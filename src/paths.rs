//! 固定位置：IsoBox 安装目录、库的相对路径、下载文件夹。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use windows::Win32::{
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
};

/// 每个盘上库的位置都是 <盘符>:\Program Files\IsoBox，系统盘上的这个目录同时也是 IsoBox 的安装目录
pub const LIB_DIR: &str = r"Program Files\IsoBox";

/// isobox.exe、isobox-setup.exe、isobox-launch.exe 所在的目录
pub fn install_dir() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .context("无法确定安装目录")?
        .to_path_buf())
}

/// IsoBox 应该装在的位置：系统盘的 Program Files\IsoBox
pub fn expected_install_dir() -> PathBuf {
    let pf = std::env::var("ProgramW6432")
        .or_else(|_| std::env::var("ProgramFiles"))
        .unwrap_or_else(|_| r"C:\Program Files".into());
    PathBuf::from(pf).join("IsoBox")
}

/// 开发调试时（例如从 target\debug 运行）设置 ISOBOX_DEV=1 可跳过安装位置检查
pub fn dev_mode() -> bool {
    std::env::var("ISOBOX_DEV").is_ok_and(|v| v == "1")
}

/// 会修改系统的命令只允许从固定的安装目录运行，避免被复制到别处的程序误操作
pub fn require_installed() -> Result<PathBuf> {
    let dir = install_dir()?;
    if dev_mode() || same_path(&dir, &expected_install_dir()) {
        return Ok(dir);
    }
    bail!(
        "IsoBox 必须安装在 {}，当前在 {}。\n请用 IsoBox 安装程序安装后，从安装目录运行",
        expected_install_dir().display(),
        dir.display()
    )
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

fn known_folder(id: &windows::core::GUID, what: &str) -> Result<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).with_context(|| format!("找不到{what}"))?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(PathBuf::from(s?))
    }
}

/// 当前用户真正的下载文件夹（可能被移到了别的盘）
pub fn downloads_dir() -> Result<PathBuf> {
    known_folder(&FOLDERID_Downloads, "下载文件夹")
}

/// 当前用户的开机启动文件夹
pub fn startup_dir() -> Result<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_Startup, "开机启动文件夹")
}

/// 默认的交换文件夹：下载文件夹里的 IsoBox
pub fn default_exchange_dir() -> Result<PathBuf> {
    Ok(downloads_dir()?.join("IsoBox"))
}

/// 路径所在的盘符，大写
pub fn drive_letter(p: &Path) -> Option<char> {
    let s = p.to_string_lossy();
    let mut c = s.chars();
    let l = c.next()?;
    (l.is_ascii_alphabetic() && c.next() == Some(':')).then(|| l.to_ascii_uppercase())
}
