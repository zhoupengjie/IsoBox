//! 汇总当前状态，供图形界面和 `isobox status --json` 使用。只读，不需要管理员权限。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    account, acl,
    config::{Config, Mode},
    drives, icons, library, paths, procs, shortcut,
    start::{self, Exe},
    theme,
};

#[derive(Debug, Serialize)]
pub struct Overview {
    pub install_dir: String,
    pub config_path: String,
    pub account: Account,
    pub exchange_dir: Option<String>,
    pub exchange_exists: bool,
    pub apps: Vec<AppStatus>,
    pub libraries: Vec<LibStatus>,
    pub drives: Vec<DriveStatus>,
    pub protected: Vec<Protected>,
    pub backups: Vec<Backup>,
    pub theme_watcher: bool,
    pub tray_autostart: bool,
}

#[derive(Debug, Serialize)]
pub struct Account {
    pub user: String,
    pub exists: bool,
    pub sid: Option<String>,
    pub profile: Option<String>,
    /// 账户已存在，但安装没做完时缺的那一步。界面据此显示“修复”
    pub gap: Option<String>,
}

/// 隔离账户已存在时，install 的各步是否都做完了，返回缺的第一项。
/// install 遇错即停，交换文件夹是最后两步，所以前面任何一步失败都会表现为交换文件夹没建好。
/// 受保护的位置不在检查之列：自动保护时新接上的硬盘不算“没装完”
pub fn install_gap(cfg: &Config, sid: &str) -> Option<String> {
    if account::profile_path(sid).ok().flatten().is_none() {
        return Some("隔离账户还没有个人文件夹".into());
    }
    if library::get(drives::system_drive(), sid).is_none() {
        return Some("系统盘的库还没建好".into());
    }
    match cfg.exchange_dir() {
        Ok(x) if x.exists() && acl::has_explicit_allow(&x, sid) => None,
        Ok(x) => Some(format!("交换文件夹 {} 还没建好", x.display())),
        Err(e) => Some(format!("{e:#}")),
    }
}

#[derive(Debug, Serialize)]
pub struct AppStatus {
    pub name: String,
    pub display: String,
    /// library：在库里；legacy：旧位置（隔离账户个人文件夹）；installed：原版安装程序装的；appcontainer
    pub kind: String,
    pub library: Option<char>,
    pub exe: Option<String>,
    pub found: bool,
    pub running: bool,
    pub startup: bool,
    pub icon: Option<String>,
    /// 还不能启动时的原因
    pub problem: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LibStatus {
    pub drive: char,
    pub root: String,
    pub apps: Vec<String>,
    pub free: u64,
    pub total: u64,
}

#[derive(Debug, Serialize)]
pub struct DriveStatus {
    pub letter: char,
    pub kind: String,
    pub label: String,
    pub fs: String,
    pub ready: bool,
    pub total: u64,
    pub free: u64,
    pub system: bool,
    pub protect_blocker: Option<String>,
    pub library_blocker: Option<String>,
    pub protected: bool,
    pub has_library: bool,
}

#[derive(Debug, Serialize)]
pub struct Protected {
    pub path: String,
    /// 禁止规则确实存在
    pub active: bool,
    /// 写在配置里
    pub in_config: bool,
}

#[derive(Debug, Serialize)]
pub struct Backup {
    pub dir: String,
    pub target: String,
    pub time: u64,
}

pub fn overview(cfg_path: &Path, cfg: &Config, with_icons: bool) -> Overview {
    let sid = account::lookup_sid(&cfg.sandbox_user).ok();
    let profile = sid.as_deref().and_then(|s| account::profile_path(s).ok().flatten());
    let libs = sid.as_deref().map(library::discover).unwrap_or_default();
    let all_drives = drives::list();
    let procs = procs::list();
    let startup_apps = shortcut::startup_apps();
    let link_names = shortcut::link_names();

    let apps = cfg
        .apps
        .iter()
        .map(|(name, a)| {
            let mode = a.mode(cfg);
            let kind = match (mode, a.library_drive(), a.portable) {
                (Mode::AppContainer, _, _) => "appcontainer",
                (Mode::User, Some(_), _) => "library",
                (Mode::User, None, true) => "legacy",
                (Mode::User, None, false) => "installed",
            };
            let (exe, found, problem) = match start::resolve_exe(cfg, name, a, mode) {
                Ok(Exe::Found(p)) => (Some(p), true, None),
                Ok(Exe::Unchecked(p)) => (Some(p), false, None),
                Ok(Exe::Missing(p)) => (Some(p.clone()), false, Some(format!("找不到 {}", p.display()))),
                Ok(Exe::NotReady(m)) => (None, false, Some(m)),
                Err(e) => (None, false, Some(format!("{e:#}"))),
            };
            let running = exe.as_deref().is_some_and(|p| procs::is_running(&procs, p));
            let icon = if with_icons {
                exe.as_deref()
                    .filter(|_| found)
                    .and_then(|p| icons::png_data_url(p, 48))
                    .or_else(|| shortcut::icon_path(name).ok().and_then(|p| icons::png_data_url(&p, 48)))
            } else {
                None
            };
            AppStatus {
                name: name.clone(),
                display: a
                    .display
                    .clone()
                    .or_else(|| link_names.get(name).cloned())
                    .unwrap_or_else(|| name.clone()),
                kind: kind.into(),
                library: a.library_drive(),
                exe: exe.map(|p| p.display().to_string()),
                found,
                running,
                startup: startup_apps.iter().any(|s| s == name),
                icon,
                problem,
            }
        })
        .collect();

    let libraries = libs
        .iter()
        .map(|l| {
            let d = all_drives.iter().find(|d| d.letter == l.drive);
            LibStatus {
                drive: l.drive,
                root: l.root.display().to_string(),
                apps: l.apps(),
                free: d.map(|d| d.free).unwrap_or(0),
                total: d.map(|d| d.total).unwrap_or(0),
            }
        })
        .collect();

    // 受保护的位置：配置里写的，加上所有盘的根目录，只要有禁止规则就列出
    let mut protected: Vec<Protected> = Vec::new();
    let mut candidates: Vec<(PathBuf, bool)> = cfg.deny_paths.iter().map(|p| (p.clone(), true)).collect();
    candidates.extend(all_drives.iter().filter(|d| d.ready && d.acl).map(|d| (PathBuf::from(d.root()), false)));
    for (p, in_config) in candidates {
        let key = p.to_string_lossy().trim_end_matches('\\').to_lowercase();
        if protected.iter().any(|x| x.path.trim_end_matches('\\').to_lowercase() == key) {
            continue;
        }
        let active = sid.as_deref().is_some_and(|s| acl::has_explicit_deny(&p, s));
        if active || in_config {
            protected.push(Protected { path: p.display().to_string(), active, in_config });
        }
    }

    let drives_out = all_drives
        .iter()
        .map(|d| DriveStatus {
            letter: d.letter,
            kind: d.describe_kind().into(),
            label: d.label.clone(),
            fs: d.fs.clone(),
            ready: d.ready,
            total: d.total,
            free: d.free,
            system: d.system,
            protect_blocker: d.protect_blocker().map(String::from),
            library_blocker: d.library_blocker().map(String::from),
            protected: protected.iter().any(|p| p.active && paths::same_path(Path::new(&p.path), Path::new(&d.root()))),
            has_library: libs.iter().any(|l| l.drive == d.letter),
        })
        .collect();

    let install_dir = paths::install_dir().unwrap_or_default();
    let exchange = cfg.exchange_dir().ok();
    Overview {
        install_dir: install_dir.display().to_string(),
        config_path: cfg_path.display().to_string(),
        account: Account {
            user: cfg.sandbox_user.clone(),
            exists: sid.is_some(),
            gap: sid.as_deref().and_then(|s| install_gap(cfg, s)),
            sid,
            profile: profile.map(|p| p.display().to_string()),
        },
        exchange_exists: exchange.as_deref().is_some_and(Path::exists),
        exchange_dir: exchange.map(|p| p.display().to_string()),
        apps,
        libraries,
        drives: drives_out,
        protected,
        backups: backups(&paths::expected_install_dir().join("backup")),
        theme_watcher: theme::watcher_running(),
        tray_autostart: shortcut::tray_autostart_enabled(),
    }
}

fn backups(root: &Path) -> Vec<Backup> {
    let mut v: Vec<Backup> = std::fs::read_dir(root)
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let dir = e.path();
                    let target = std::fs::read_to_string(dir.join("target.txt")).ok()?.trim().to_string();
                    let time = e.file_name().to_string_lossy().split('-').next()?.parse().ok()?;
                    Some(Backup { dir: dir.display().to_string(), target, time })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|b| std::cmp::Reverse(b.time));
    v
}
