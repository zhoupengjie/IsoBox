//! 库：每个盘上的固定位置 <盘符>:\Program Files\IsoBox。
//!
//! ```text
//! X:\Program Files\IsoBox\        管理员完全控制，普通用户只读，不继承上级权限
//!     library.toml                标记文件：属于哪个隔离账户
//!     apps\<软件名>\               隔离软件的程序文件，额外允许隔离账户读写
//! ```
//!
//! 系统盘上的这个目录同时也是 IsoBox 的安装目录。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{acl, drives, paths};

pub const MARKER_FILE: &str = "library.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub version: u32,
    pub sandbox_user: String,
    pub sandbox_sid: String,
    pub created: u64,
    /// 建库时上级的 Program Files 是 IsoBox 新建的。移除库时如果它空了就一并删掉
    #[serde(default)]
    pub created_parent: bool,
}

#[derive(Debug, Clone)]
pub struct Library {
    pub drive: char,
    pub root: PathBuf,
    pub marker: Marker,
}

impl Library {
    pub fn apps_dir(&self) -> PathBuf {
        self.root.join("apps")
    }

    /// 库里的软件文件夹名
    pub fn apps(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.apps_dir())
            .map(|it| {
                it.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| !n.starts_with('_'))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

pub fn root(drive: char) -> PathBuf {
    PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase())).join(paths::LIB_DIR)
}

pub fn app_dir(drive: char, app: &str) -> PathBuf {
    root(drive).join("apps").join(app)
}

fn read_marker(drive: char) -> Option<Marker> {
    let text = std::fs::read_to_string(root(drive).join(MARKER_FILE)).ok()?;
    toml::from_str(&text).ok()
}

/// 找出属于这个隔离账户的所有库。只检查各盘的固定位置，很快
pub fn discover(sid: &str) -> Vec<Library> {
    drives::list()
        .into_iter()
        .filter(|d| d.ready)
        .filter_map(|d| {
            let m = read_marker(d.letter)?;
            m.sandbox_sid.eq_ignore_ascii_case(sid).then(|| Library {
                drive: d.letter,
                root: root(d.letter),
                marker: m,
            })
        })
        .collect()
}

pub fn get(drive: char, sid: &str) -> Option<Library> {
    discover(sid).into_iter().find(|l| l.drive == drive.to_ascii_uppercase())
}

/// 在某个盘上建库。已存在时只重新设置权限
pub fn create(drive: char, user: &str, sid: &str) -> Result<Library> {
    let drive = drive.to_ascii_uppercase();
    let d = drives::get(drive).with_context(|| format!("没有 {drive} 盘"))?;
    if let Some(why) = d.library_blocker() {
        bail!("{drive} 盘不能建库：{why}");
    }
    let root = root(drive);
    let is_install_dir = paths::same_path(&root, &paths::install_dir()?);
    let mut marker = match read_marker(drive) {
        Some(m) if !m.sandbox_sid.eq_ignore_ascii_case(sid) => {
            bail!("{} 是隔离账户 {} 的库，不属于当前账户", root.display(), m.sandbox_user)
        }
        Some(m) => m,
        None => {
            // 不碰别人的文件夹：已存在且有内容、又不是 IsoBox 安装目录的，拒绝
            if root.exists() && !is_install_dir && std::fs::read_dir(&root)?.next().is_some() {
                bail!(
                    "{} 已经存在，里面有其他文件，但不是 IsoBox 的库。\n\
                     为了不改动这些文件的权限，拒绝在这里建库",
                    root.display()
                );
            }
            Marker {
                version: 1,
                sandbox_user: user.into(),
                sandbox_sid: sid.into(),
                created: now(),
                created_parent: !root.parent().is_some_and(Path::exists),
            }
        }
    };
    marker.sandbox_user = user.into();

    std::fs::create_dir_all(&root).with_context(|| format!("创建 {} 失败", root.display()))?;
    acl::lock_readonly(&root)?;
    std::fs::write(root.join(MARKER_FILE), toml::to_string(&marker)?)?;
    let apps = root.join("apps");
    std::fs::create_dir_all(&apps)?;
    acl::lock_apps_dir(&apps, sid)?;
    Ok(Library { drive, root, marker })
}

/// 移除库。库里还有软件时拒绝
pub fn remove(drive: char, sid: &str) -> Result<()> {
    let lib = get(drive, sid).with_context(|| format!("{drive} 盘上没有 IsoBox 的库"))?;
    let apps = lib.apps();
    if !apps.is_empty() {
        bail!("{drive} 盘的库里还有软件：{}。请先移除，或移动到其他库", apps.join("、"));
    }
    remove_files(&lib)
}

/// 删除库的目录结构，不检查里面有没有软件。卸载整个 IsoBox 时使用
pub fn remove_files(lib: &Library) -> Result<()> {
    let apps = lib.apps_dir();
    if apps.exists() {
        std::fs::remove_dir_all(&apps).with_context(|| format!("删除 {} 失败，确认软件已退出", apps.display()))?;
    }
    let _ = std::fs::remove_file(lib.root.join(MARKER_FILE));
    let is_install_dir = paths::install_dir().is_ok_and(|d| paths::same_path(&lib.root, &d));
    if !is_install_dir && std::fs::read_dir(&lib.root).is_ok_and(|mut it| it.next().is_none()) {
        std::fs::remove_dir(&lib.root)?;
        if lib.marker.created_parent
            && let Some(parent) = lib.root.parent()
                && std::fs::read_dir(parent).is_ok_and(|mut it| it.next().is_none()) {
                    let _ = std::fs::remove_dir(parent);
                }
    }
    Ok(())
}

/// 目录占用的空间，用于显示
pub fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(it) = std::fs::read_dir(&d) else { continue };
        for e in it.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                Err(_) => {}
            }
        }
    }
    total
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
