//! 把安装包拆成绿色版，不运行安装程序、不写注册表。
//!
//! 支持的封装：
//! - 7z 自解压包，例如 QQ：程序在 Files 目录下
//! - NSIS 安装包，内层再套一个 7z，例如微信的 install.7z
//! - 其他 7-Zip 能识别的格式
//!
//! 做法：用 7-Zip 解开外层，顶层若有 .7z/.zip 再解一层，
//! 然后找到主程序，把它所在的目录整体移到目标位置。

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

use crate::{config::Config, paths};

/// 系统里单独安装的 7-Zip
pub const DEFAULT_7Z: &str = r"C:\Program Files\7-Zip\7z.exe";

/// 安装程序捆绑的 7-Zip，相对安装目录。精简版 7zr.exe 解不开 NSIS 安装包，所以捆绑完整的 7z.exe 和 7z.dll
pub const BUNDLED_7Z: &str = r"7-Zip\7z.exe";

/// 要用的 7-Zip：配置里指定的 > 安装目录里捆绑的 > 系统里装的
pub fn sevenzip(cfg: &Config) -> PathBuf {
    if let Some(p) = &cfg.sevenzip {
        return p.clone();
    }
    if let Ok(dir) = paths::install_dir() {
        let bundled = dir.join(BUNDLED_7Z);
        if bundled.exists() {
            return bundled;
        }
    }
    PathBuf::from(DEFAULT_7Z)
}

/// 不太可能是主程序的文件名片段
const NOT_MAIN: &[&str] = &[
    "unins", "uninst", "setup", "install", "update", "upgrade", "crash", "report", "helper",
    "vc_redist", "vcredist", "dxsetup", "repair", "elevat", "bugreport", "7z",
];

/// 解包到 app_dir，返回主程序的完整路径。
/// exe_name 为 None 时列出候选，交给 pick 选择。
pub fn unpack(
    sevenzip: &Path,
    installer: &Path,
    app_dir: &Path,
    exe_name: Option<&str>,
    force: bool,
    pick: &dyn Fn(&[Candidate]) -> Result<usize>,
) -> Result<PathBuf> {
    if !sevenzip.exists() {
        bail!("找不到 7-Zip: {}。重新运行 IsoBox 安装程序可以修复，也可以在配置里用 sevenzip = '...' 指定", sevenzip.display());
    }
    if !installer.exists() {
        bail!("找不到安装包: {}", installer.display());
    }
    if app_dir.exists() {
        if !force {
            bail!("{} 已存在，加 --force 覆盖。覆盖只替换程序文件，不影响聊天记录等数据", app_dir.display());
        }
        std::fs::remove_dir_all(app_dir)
            .with_context(|| format!("删除旧程序目录失败，确认软件已退出: {}", app_dir.display()))?;
    }

    let stage = app_dir.with_file_name("_unpack");
    if stage.exists() {
        std::fs::remove_dir_all(&stage)?;
    }

    let result = (|| -> Result<PathBuf> {
        extract_all(sevenzip, installer, &stage)?;
        let exe = match exe_name {
            Some(name) => find_shallowest(&stage, name)?.with_context(|| {
                format!("解包后找不到 {name}。可以查看 {} 确认主程序名", stage.display())
            })?,
            None => {
                let cands = candidates(&stage);
                if cands.is_empty() {
                    bail!("解包后没有找到任何 exe，这个安装包可能不是常见格式，可以改用 --install");
                }
                cands[pick(&cands)?].path.clone()
            }
        };
        let exe_file = exe.file_name().context("主程序没有文件名")?.to_owned();
        let root = exe.parent().context("主程序没有父目录")?;
        std::fs::rename(root, app_dir)
            .with_context(|| format!("移动 {} 到 {} 失败", root.display(), app_dir.display()))?;
        Ok(app_dir.join(exe_file))
    })();

    // 成功时清理临时目录；失败时保留，方便排查
    if result.is_ok() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

/// 解开安装包到 stage；顶层若有 .7z/.zip（例如微信的 install.7z）再解一层
fn extract_all(sevenzip: &Path, installer: &Path, stage: &Path) -> Result<()> {
    println!("解开外层: {}", installer.display());
    extract(sevenzip, installer, stage)?;
    for entry in std::fs::read_dir(stage)? {
        let p = entry?.path();
        let is_archive = p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("7z") || e.eq_ignore_ascii_case("zip"));
        if is_archive {
            let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
            println!("解开内层: {}", p.file_name().unwrap_or_default().to_string_lossy());
            extract(sevenzip, &p, &stage.join("_inner").join(stem))?;
        }
    }
    Ok(())
}

/// 安装前预览：识别安装包类型，试着解包，列出主程序候选。不需要管理员权限，解到临时目录，用完删除
#[derive(Debug, serde::Serialize)]
pub struct Preview {
    pub kind: String,
    /// 根据文件名猜的英文名，例如 BaiduNetdisk_7.50.exe 得到 baidunetdisk
    pub guessed_name: String,
    /// 能否快速安装（拆包）
    pub unpackable: bool,
    pub error: Option<String>,
    pub candidates: Vec<PreviewCandidate>,
    /// 解包后的总大小
    pub size: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct PreviewCandidate {
    pub file: String,
    pub rel_path: String,
    pub size: u64,
    pub likely_helper: bool,
}

/// 给用户看的相对路径：去掉内层压缩包的临时目录名，例如 _inner\install\Weixin.exe 显示为 Weixin.exe
fn display_rel(rel: &Path) -> String {
    let parts: Vec<_> = rel.components().collect();
    let skip = if parts.first().is_some_and(|c| c.as_os_str() == "_inner") { 2 } else { 0 };
    let shown: PathBuf = parts.into_iter().skip(skip).collect();
    shown.display().to_string()
}

pub fn guess_name(installer: &Path) -> String {
    let stem = installer.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
    let word: String = stem.chars().skip_while(|c| !c.is_ascii_lowercase()).take_while(|c| c.is_ascii_lowercase()).collect();
    if word.is_empty() { "app".into() } else { word }
}

pub fn preview(sevenzip: &Path, installer: &Path) -> Preview {
    let kind = installer_kind(sevenzip, installer);
    let mut p = Preview {
        kind: format!("{kind:?}"),
        guessed_name: guess_name(installer),
        unpackable: false,
        error: None,
        candidates: Vec::new(),
        size: 0,
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let stage = std::env::temp_dir().join(format!("isobox-preview-{}-{nanos}", std::process::id()));
    match extract_all(sevenzip, installer, &stage) {
        Ok(()) => {
            let cands = candidates(&stage);
            p.unpackable = !cands.is_empty();
            if cands.is_empty() {
                p.error = Some("解包后没有找到任何程序，建议使用原版安装程序".into());
            }
            p.size = crate::library::dir_size(&stage);
            p.candidates = cands
                .into_iter()
                .map(|c| PreviewCandidate {
                    file: c.path.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    rel_path: display_rel(c.path.strip_prefix(&stage).unwrap_or(&c.path)),
                    size: c.size,
                    likely_helper: c.likely_helper,
                })
                .collect();
        }
        Err(e) => p.error = Some(format!("{e:#}")),
    }
    let _ = std::fs::remove_dir_all(&stage);
    p
}

/// 安装包类型，决定怎样预填安装目录
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    Nsis,
    Inno,
    Msi,
    Unknown,
}

/// 识别安装包类型：.msi 看扩展名；NSIS 用 7-Zip 识别；Inno Setup 看文件开头的特征字符串
pub fn installer_kind(sevenzip: &Path, installer: &Path) -> InstallerKind {
    if installer.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi")) {
        return InstallerKind::Msi;
    }
    if let Ok(out) = Command::new(sevenzip).args(["l", "-slt"]).arg(installer).output()
        && String::from_utf8_lossy(&out.stdout).lines().any(|l| l.trim() == "Type = Nsis") {
            return InstallerKind::Nsis;
        }
    // Inno Setup 的启动程序很小，特征字符串在文件前几 MB 里
    if let Ok(mut f) = std::fs::File::open(installer) {
        use std::io::Read;
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let n = f.read(&mut buf).unwrap_or(0);
        let hay = &buf[..n];
        let has = |needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
        if has(b"Inno Setup Setup Data") || has(b"JR.Inno.Setup") {
            return InstallerKind::Inno;
        }
    }
    InstallerKind::Unknown
}

fn extract(sevenzip: &Path, archive: &Path, dest: &Path) -> Result<()> {
    let out = Command::new(sevenzip)
        .arg("x")
        .arg("-y")
        .arg("-bso0")
        .arg("-bsp0")
        .arg(format!("-o{}", dest.display()))
        .arg(archive)
        .output()
        .context("无法运行 7-Zip")?;
    if !out.status.success() {
        bail!(
            "7-Zip 解不开 {}。这个安装包不是 7-Zip 能识别的格式，可以改用 --install。\n{}",
            archive.display(),
            crate::cmd::decode_console(&out.stderr).trim()
        );
    }
    Ok(())
}

/// 广度优先查找，返回层级最浅的同名文件。跳过 NSIS 的插件目录。
fn find_shallowest(root: &Path, name: &str) -> Result<Option<PathBuf>> {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let mut subdirs = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let p = entry.path();
            let fname = entry.file_name().to_string_lossy().to_string();
            if entry.file_type()?.is_dir() {
                if !fname.starts_with('$') {
                    subdirs.push(p);
                }
            } else if fname.eq_ignore_ascii_case(name) {
                return Ok(Some(p));
            }
        }
        subdirs.sort();
        queue.extend(subdirs);
    }
    Ok(None)
}

/// 主程序候选
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    pub depth: usize,
    /// 文件名像卸载、更新、崩溃上报之类的辅助程序
    pub likely_helper: bool,
}

/// 列出目录下所有 exe，按“像不像主程序”排序：先排除辅助程序，再按层级浅、体积大排。
pub fn candidates(root: &Path) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_lowercase();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !name.starts_with('$') {
                    queue.push_back((p, depth + 1));
                }
            } else if name.ends_with(".exe") {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                let likely_helper = NOT_MAIN.iter().any(|k| name.contains(k));
                out.push(Candidate { path: p, size, depth, likely_helper });
            }
        }
    }
    sort_candidates(&mut out);
    out
}

pub fn is_likely_helper(file_name: &str) -> bool {
    let n = file_name.to_lowercase();
    NOT_MAIN.iter().any(|k| n.contains(k))
}

pub fn sort_candidates(c: &mut [Candidate]) {
    c.sort_by(|a, b| {
        a.likely_helper
            .cmp(&b.likely_helper)
            .then(a.depth.cmp(&b.depth))
            .then(b.size.cmp(&a.size))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_program_ranks_first() {
        let root = std::env::temp_dir().join(format!("isobox-cand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("Uninstall.exe"), vec![0u8; 5000]).unwrap();
        std::fs::write(root.join("App.exe"), vec![0u8; 3000]).unwrap();
        std::fs::write(root.join("tiny.exe"), vec![0u8; 10]).unwrap();
        std::fs::write(root.join("sub").join("Deep.exe"), vec![0u8; 90000]).unwrap();
        std::fs::write(root.join("sub").join("crashpad_handler.exe"), vec![0u8; 90000]).unwrap();
        let c = candidates(&root);
        let names: Vec<String> = c.iter().map(|x| x.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names[0], "App.exe");
        assert_eq!(names[1], "tiny.exe");
        assert_eq!(names[2], "Deep.exe");
        assert!(c.iter().find(|x| x.path.ends_with("Uninstall.exe")).unwrap().likely_helper);
        assert!(c.iter().find(|x| x.path.ends_with("crashpad_handler.exe")).unwrap().likely_helper);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod guess_tests {
    use super::*;

    #[test]
    fn guesses_names_from_installer_files() {
        assert_eq!(guess_name(Path::new(r"D:\x\WeChatWin_4.1.15.exe")), "wechatwin");
        assert_eq!(guess_name(Path::new("QQ_9.9.36_260924_x64_01.exe")), "qq");
        assert_eq!(guess_name(Path::new("BaiduNetdisk_7.50.0.132.exe")), "baidunetdisk");
        assert_eq!(guess_name(Path::new("123.exe")), "app");
    }
}
