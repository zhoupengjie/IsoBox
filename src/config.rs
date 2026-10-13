use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// 隔离方式
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// 以独立的 isobox 账户运行，需要先执行 isobox-setup install
    #[default]
    User,
    /// 在 AppContainer 里运行，不建账户、不改磁盘权限
    #[value(name = "appcontainer")]
    AppContainer,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub sandbox_user: String,
    /// 交换文件夹。不写时使用下载文件夹里的 IsoBox，见 exchange_dir()
    #[serde(rename = "exchange_dir")]
    pub exchange_dir_raw: Option<PathBuf>,
    #[serde(default)]
    pub deny_paths: Vec<PathBuf>,
    /// deny_paths 为空时是否自动保护所有内置的非系统盘。手动增删过保护位置后会写成 false
    #[serde(rename = "protect_auto")]
    pub protect_auto_raw: Option<bool>,
    /// 软件没有单独指定 mode 时使用的方式
    #[serde(default)]
    pub default_mode: Mode,
    /// 7-Zip 路径，unpack 时使用
    pub sevenzip: Option<PathBuf>,
    #[serde(default)]
    pub apps: BTreeMap<String, App>,
}

#[derive(Debug, Deserialize)]
pub struct App {
    /// 普通安装：主程序的完整路径。
    /// 绿色版（portable = true）：只写主程序文件名，例如 'QQ.exe'
    pub exe: PathBuf,
    /// 为 true 时是拆包得到的绿色版：账户模式下放在沙盒用户个人文件夹的 Apps\<名字> 目录，
    /// AppContainer 模式下放在容器文件夹的 app 目录
    #[serde(default)]
    pub portable: bool,
    /// 快捷方式上显示的名字，例如“微信”。不写就用配置里的英文名
    pub display: Option<String>,
    #[serde(default)]
    pub args: String,
    /// 工作目录，缺省为 exe 所在目录
    pub workdir: Option<PathBuf>,
    /// 覆盖全局 default_mode
    pub mode: Option<Mode>,
    /// AppContainer 模式下授予的能力
    #[serde(default = "default_capabilities")]
    pub capabilities: Vec<String>,
    /// AppContainer 模式下额外允许只读访问的目录，例如装在 Program Files 以外的软件目录
    #[serde(default)]
    pub extra_read: Vec<PathBuf>,
    /// 程序所在库的盘符，例如 'D'。不写时：绿色版在旧位置（沙盒用户个人文件夹的 Apps），
    /// 原版安装程序装的软件以 exe 的完整路径为准
    pub library: Option<String>,
}

pub fn default_capabilities() -> Vec<String> {
    ["internetClient", "privateNetworkClientServer", "microphone", "webcam"]
        .map(String::from)
        .to_vec()
}

impl App {
    pub fn mode(&self, cfg: &Config) -> Mode {
        self.mode.unwrap_or(cfg.default_mode)
    }

    /// 工作目录：配置里指定的，或主程序所在目录。exe 传入解析后的完整路径。
    pub fn workdir_for(&self, exe: &Path) -> Result<PathBuf> {
        match &self.workdir {
            Some(w) => Ok(w.clone()),
            None => Ok(exe.parent().context("exe 路径没有父目录")?.to_path_buf()),
        }
    }

    /// 所在库的盘符
    pub fn library_drive(&self) -> Option<char> {
        self.library
            .as_deref()
            .and_then(|s| s.chars().next())
            .filter(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
    }

    /// 绿色版主程序的文件名
    pub fn exe_name(&self) -> Result<String> {
        let name = self.exe.file_name().context("exe 没有文件名")?;
        if self.exe.components().count() != 1 {
            anyhow::bail!("绿色版的 exe 只写文件名，例如 'QQ.exe'，不要带路径");
        }
        Ok(name.to_string_lossy().to_string())
    }
}

/// 配置里软件的英文名只能用小写字母、数字、短横线和下划线，也用作文件夹名
pub fn validate_app_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        anyhow::bail!("软件名 {name:?} 不合法：只能用小写英文字母、数字、短横线和下划线，例如 baidunetdisk");
    }
    Ok(())
}

/// 在配置文件末尾追加一个软件。保留原文件的注释和格式，只追加文本。
pub fn append_app(path: &Path, name: &str, fields: &[(&str, toml::Value)]) -> Result<()> {
    validate_app_name(name)?;
    let mut text = std::fs::read_to_string(path)?;
    if text.contains(&format!("[apps.{name}]")) {
        anyhow::bail!("配置里已经有 {name}");
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("\n# 由 isobox-setup add 添加\n[apps.{name}]\n"));
    for (k, v) in fields {
        // toml::Value 的 Display 会按 TOML 规则加引号和转义
        text.push_str(&format!("{k} = {v}\n"));
    }
    // 写入前确认结果仍是合法配置，避免把配置文件弄坏
    toml::from_str::<Config>(text.trim_start_matches('\u{feff}'))
        .context("追加后的配置无法解析，已放弃写入")?;
    std::fs::write(path, text).with_context(|| format!("写入 {} 失败", path.display()))
}

/// 从配置文件里删除一个软件的整段，到下一个 [ 开头的段落为止。
/// 段落上方紧挨着的那行“由 isobox-setup add 添加”注释也一并删除。
pub fn remove_app(path: &Path, name: &str) -> Result<bool> {
    let text = std::fs::read_to_string(path)?;
    let header = format!("[apps.{name}]");
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    let mut removed = false;
    for line in text.lines() {
        let t = line.trim();
        if t == header {
            skipping = true;
            removed = true;
            if out.last().is_some_and(|l| l.trim() == "# 由 isobox-setup add 添加") {
                out.pop();
            }
            continue;
        }
        if skipping && t.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            out.push(line);
        }
    }
    if removed {
        let mut new_text = out.join("\n");
        new_text.push('\n');
        toml::from_str::<Config>(new_text.trim_start_matches('\u{feff}'))
            .context("删除后的配置无法解析，已放弃写入")?;
        std::fs::write(path, new_text).with_context(|| format!("写入 {} 失败", path.display()))?;
    }
    Ok(removed)
}

/// 设置顶层（第一个 [段落] 之前）的一个配置项：已有就替换那一行，没有就插在第一个段落之前。
/// 只支持写在一行里的值
pub fn set_top_level(path: &Path, key: &str, value: toml::Value) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let new_line = format!("{key} = {value}");
    let mut out: Vec<String> = Vec::new();
    let (mut in_tables, mut done) = (false, false);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && !in_tables {
            in_tables = true;
            if !done {
                out.push(new_line.clone());
                out.push(String::new());
                done = true;
            }
        }
        if !in_tables && !done && t.split('=').next().is_some_and(|k| k.trim() == key) && !t.starts_with('#') {
            out.push(new_line.clone());
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        out.push(new_line);
    }
    let mut new_text = out.join("\n");
    new_text.push('\n');
    toml::from_str::<Config>(new_text.trim_start_matches('\u{feff}'))
        .context("修改后的配置无法解析，已放弃写入")?;
    std::fs::write(path, new_text).with_context(|| format!("写入 {} 失败", path.display()))
}

/// 设置某个软件条目里的一个字段：已有就替换那一行，没有就加在段落标题下面
pub fn set_app_field(path: &Path, name: &str, key: &str, value: toml::Value) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let header = format!("[apps.{name}]");
    let new_line = format!("{key} = {value}");
    let mut out: Vec<String> = Vec::new();
    let (mut inside, mut found_header, mut done) = (false, false, false);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if inside && !done {
                // 段落里没有这个字段：插在段落末尾（下一个段落之前、空行之前）
                let mut tail = Vec::new();
                while out.last().is_some_and(|l| l.trim().is_empty()) {
                    tail.push(out.pop().unwrap());
                }
                out.push(new_line.clone());
                out.extend(tail.into_iter().rev());
                done = true;
            }
            inside = t == header;
            found_header |= inside;
        } else if inside && !done && t.split('=').next().is_some_and(|k| k.trim() == key) {
            out.push(new_line.clone());
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !found_header {
        bail!("配置里没有 {name}");
    }
    if !done {
        out.push(new_line);
    }
    let mut new_text = out.join("\n");
    new_text.push('\n');
    toml::from_str::<Config>(new_text.trim_start_matches('\u{feff}'))
        .context("修改后的配置无法解析，已放弃写入")?;
    std::fs::write(path, new_text).with_context(|| format!("写入 {} 失败", path.display()))
}

impl Config {
    pub fn protect_auto(&self) -> bool {
        self.protect_auto_raw.unwrap_or(true)
    }

    /// 交换文件夹：配置里写了就用它，否则用下载文件夹里的 IsoBox
    pub fn exchange_dir(&self) -> Result<PathBuf> {
        match &self.exchange_dir_raw {
            Some(p) => Ok(p.clone()),
            None => crate::paths::default_exchange_dir(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置失败: {}", path.display()))?;
        // 记事本等编辑器保存的 UTF-8 文件可能带 BOM，TOML 解析器不接受它
        let text = text.trim_start_matches('\u{feff}');
        toml::from_str(text).with_context(|| format!("解析配置失败: {}", path.display()))
    }

    /// 默认配置位置：与可执行文件同目录的 isobox.toml，找不到再用当前目录。
    pub fn default_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            let p = exe.with_file_name("isobox.toml");
            if p.exists() {
                return p;
            }
        }
        PathBuf::from("isobox.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "# 注释要保留\nsandbox_user = \"isobox\"\nexchange_dir = 'D:\\x'\n\n[apps.wechat]\nportable = true\nexe = 'Weixin.exe'\n";

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("isobox-test-{name}-{}.toml", std::process::id()));
        std::fs::write(&p, BASE).unwrap();
        p
    }

    #[test]
    fn append_then_remove_roundtrip() {
        let p = tmp("roundtrip");
        append_app(&p, "baidunetdisk", &[
            ("exe", toml::Value::from(r"C:\Users\isobox\AppData\Roaming\baidu\BaiduNetdisk.exe")),
            ("display", toml::Value::from("百度网盘")),
        ]).unwrap();
        let cfg = Config::load(&p).unwrap();
        let a = &cfg.apps["baidunetdisk"];
        assert_eq!(a.display.as_deref(), Some("百度网盘"));
        assert!(!a.portable);
        assert_eq!(a.exe, PathBuf::from(r"C:\Users\isobox\AppData\Roaming\baidu\BaiduNetdisk.exe"));
        assert!(cfg.apps.contains_key("wechat"));

        assert!(remove_app(&p, "baidunetdisk").unwrap());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("baidunetdisk"));
        assert!(!text.contains("由 isobox-setup add 添加"));
        assert!(text.contains("# 注释要保留"));
        assert!(Config::load(&p).unwrap().apps.contains_key("wechat"));
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn portable_bool_is_written_as_bool() {
        let p = tmp("bool");
        append_app(&p, "quark", &[("portable", toml::Value::from(true)), ("exe", toml::Value::from("quark.exe"))]).unwrap();
        assert!(Config::load(&p).unwrap().apps["quark"].portable);
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn rejects_duplicates_and_bad_names() {
        let p = tmp("dup");
        assert!(append_app(&p, "wechat", &[("exe", toml::Value::from("a.exe"))]).is_err());
        assert!(append_app(&p, "Bad Name", &[("exe", toml::Value::from("a.exe"))]).is_err());
        assert!(append_app(&p, "x]\n[evil", &[("exe", toml::Value::from("a.exe"))]).is_err());
        assert!(!remove_app(&p, "nothere").unwrap());
        std::fs::remove_file(p).unwrap();
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;

    #[test]
    fn set_app_field_replaces_and_inserts() {
        let p = std::env::temp_dir().join(format!("isobox-field-{}.toml", std::process::id()));
        std::fs::write(&p, "sandbox_user = \"isobox\"\n\n[apps.qq]\nportable = true\nexe = 'QQ.exe'\n\n[apps.wechat]\nportable = true\nexe = 'Weixin.exe'\nlibrary = 'C'\n").unwrap();
        set_app_field(&p, "qq", "library", toml::Value::from("D")).unwrap();
        set_app_field(&p, "wechat", "library", toml::Value::from("E")).unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.apps["qq"].library_drive(), Some('D'));
        assert_eq!(cfg.apps["wechat"].library_drive(), Some('E'));
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text.matches("library").count(), 2, "{text}");
        assert!(set_app_field(&p, "nothere", "library", toml::Value::from("C")).is_err());
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn exchange_dir_defaults_to_downloads() {
        let cfg: Config = toml::from_str("sandbox_user = \"isobox\"").unwrap();
        assert!(cfg.exchange_dir().unwrap().ends_with("IsoBox"));
        let cfg: Config = toml::from_str("sandbox_user = \"isobox\"\nexchange_dir = 'D:\\x'").unwrap();
        assert_eq!(cfg.exchange_dir().unwrap(), PathBuf::from(r"D:\x"));
    }
}

#[cfg(test)]
mod top_level_tests {
    use super::*;

    #[test]
    fn set_top_level_replaces_inserts_and_skips_comments() {
        let p = std::env::temp_dir().join(format!("isobox-top-{}.toml", std::process::id()));
        std::fs::write(&p, "sandbox_user = \"isobox\"\n# exchange_dir = 'X:\\old'\ndeny_paths = []\n\n[apps.qq]\nportable = true\nexe = 'QQ.exe'\n").unwrap();
        set_top_level(&p, "deny_paths", toml::Value::Array(vec![toml::Value::from(r"D:\")])).unwrap();
        set_top_level(&p, "exchange_dir", toml::Value::from(r"D:\Chat")).unwrap();
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.deny_paths, vec![PathBuf::from(r"D:\")]);
        assert_eq!(cfg.exchange_dir().unwrap(), PathBuf::from(r"D:\Chat"));
        assert!(cfg.apps.contains_key("qq"));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# exchange_dir = 'X:\\old'"), "{text}");
        let pos_ex = text.find("\nexchange_dir = ").expect("exchange_dir line");
        assert!(pos_ex < text.find("[apps.qq]").unwrap(), "{text}");
        std::fs::remove_file(p).unwrap();
    }
}
