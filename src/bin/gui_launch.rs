//! 给快捷方式用的启动器：没有控制台窗口，出错时弹出提示框。
//! 用法：
//!   isobox-launch.exe <软件名>      启动软件，例如 isobox-launch.exe wechat
//!   isobox-launch.exe --watch-theme  常驻后台，把深浅色切换同步给沙盒用户（由启动软件时自动拉起）

#![windows_subsystem = "windows"]

use isobox::{config::Config, start, theme, wide::to_wide};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    core::PCWSTR,
};

fn main() {
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        // 后台监视进程出错时不弹框打扰用户，直接退出，下次启动软件时会重新拉起
        Some("--watch-theme") => {
            let cfg = Config::load(&Config::default_path());
            if let Err(e) = &cfg
                && let Ok(p) = std::env::var("ISOBOX_THEME_LOG") {
                    let _ = std::fs::write(p, format!("config load failed: {e:#}\n"));
                }
            let _ = cfg.and_then(|cfg| theme::watch(&cfg.sandbox_user));
            return;
        }
        Some(app) => run(app),
        None => Err(anyhow::anyhow!("用法：isobox-launch.exe <软件名>，例如 isobox-launch.exe wechat")),
    };
    if let Err(e) = result {
        let text = to_wide(&format!("{e:#}"));
        let title = to_wide("IsoBox 启动失败");
        unsafe {
            MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR);
        }
    }
}

fn run(app: &str) -> anyhow::Result<()> {
    let cfg = Config::load(&Config::default_path())?;
    start::start(&cfg, app, None, "", None, true)?;
    Ok(())
}
