//! IsoBox 图形界面和托盘。始终以普通权限运行；需要管理员权限的操作通过 UAC 调用 isobox-setup。
//!
//! 启动方式：
//!   isobox-gui.exe          打开主窗口，同时显示托盘
//!   isobox-gui.exe --tray   只显示托盘（开机启动时使用）
//!
//! 关闭主窗口会销毁网页界面以释放内存，托盘继续运行；从托盘菜单选“退出”才真正退出。

#![windows_subsystem = "windows"]

mod elevate;

use std::path::{Path, PathBuf};

use isobox::{config::Config, paths, shortcut, start, status, theme, unpack};
use tauri::{
    AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const TRAY_ID: &str = "main";

/// 配置文件位置：环境变量 ISOBOX_CONFIG（开发调试用）> 程序旁边 > 安装目录
fn cfg_path() -> PathBuf {
    if let Ok(p) = std::env::var("ISOBOX_CONFIG") {
        return PathBuf::from(p);
    }
    let local = Config::default_path();
    if local.is_absolute() && local.exists() {
        return local;
    }
    paths::expected_install_dir().join("isobox.toml")
}

fn load_cfg() -> Result<Config, String> {
    Config::load(&cfg_path()).map_err(|e| format!("{e:#}"))
}

/// 在后台线程里跑耗时的同步操作，避免卡住界面
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn overview() -> Result<status::Overview, String> {
    blocking(|| {
        let cfg = load_cfg()?;
        Ok(status::overview(&cfg_path(), &cfg, true))
    })
    .await
}

#[tauri::command]
async fn launch_app(name: String) -> Result<String, String> {
    blocking(move || {
        let cfg = load_cfg()?;
        start::start(&cfg, &name, None, "", None, true).map(|(_, msg)| msg).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
async fn preview_installer(path: String) -> Result<unpack::Preview, String> {
    blocking(move || {
        let cfg = load_cfg()?;
        let seven = cfg.sevenzip.clone().unwrap_or_else(|| unpack::DEFAULT_7Z.into());
        if !seven.exists() {
            return Err(format!("找不到 7-Zip：{}", seven.display()));
        }
        Ok(unpack::preview(&seven, Path::new(&path)))
    })
    .await
}

#[tauri::command]
async fn admin(app: AppHandle, args: Vec<String>) -> Result<elevate::AdminResult, String> {
    let r = blocking(move || elevate::run_setup(&args).map_err(|e| format!("{e:#}"))).await;
    let _ = refresh_tray_menu(&app);
    r
}

#[tauri::command]
async fn set_app_startup(name: String, enable: bool) -> Result<(), String> {
    blocking(move || {
        let cfg = load_cfg()?;
        let a = start::get_app(&cfg, &name).map_err(|e| format!("{e:#}"))?;
        let display = a.display.clone().unwrap_or_else(|| name.clone());
        let exe = match start::resolve_exe(&cfg, &name, a, a.mode(&cfg)) {
            Ok(start::Exe::Found(p)) => Some(p),
            _ => None,
        };
        shortcut::set_app_startup(&name, &display, enable, exe.as_deref()).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
async fn set_tray_autostart(enable: bool) -> Result<(), String> {
    blocking(move || shortcut::set_tray_autostart(enable).map_err(|e| format!("{e:#}"))).await
}

/// 用资源管理器打开文件夹，或选中某个文件
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    let mut cmd = std::process::Command::new("explorer.exe");
    if p.is_file() {
        cmd.arg(format!("/select,{}", p.display()));
    } else {
        cmd.arg(p);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn refresh_tray(app: AppHandle) -> Result<(), String> {
    refresh_tray_menu(&app).map_err(|e| e.to_string())
}

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config": cfg_path().display().to_string(),
        "install_dir": paths::expected_install_dir().display().to_string(),
    })
}

/// 托盘菜单：每个软件一项“启动”，然后是打开主窗口和退出
fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    if let Ok(cfg) = load_cfg() {
        let link_names = shortcut::link_names();
        for (name, a) in &cfg.apps {
            let display = a.display.clone().or_else(|| link_names.get(name).cloned()).unwrap_or_else(|| name.clone());
            let label = format!("启动 {display}");
            menu.append(&MenuItem::with_id(app, format!("launch:{name}"), label, true, None::<&str>)?)?;
        }
        if !cfg.apps.is_empty() {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
    }
    menu.append(&MenuItem::with_id(app, "show", "打开 IsoBox", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?)?;
    Ok(menu)
}

fn refresh_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_tray_menu(app)?))?;
    }
    Ok(())
}

/// 显示主窗口；窗口已被关闭（销毁）时重新创建
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("IsoBox")
        .inner_size(980.0, 660.0)
        .min_inner_size(760.0, 520.0)
        .center()
        .build();
}

fn main() {
    let tray_only = std::env::args().any(|a| a == "--tray");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            overview,
            launch_app,
            preview_installer,
            admin,
            set_app_startup,
            set_tray_autostart,
            open_path,
            refresh_tray,
            app_info
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip("IsoBox")
                .menu(&build_tray_menu(&handle)?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    if let Some(name) = id.strip_prefix("launch:") {
                        if let Ok(cfg) = load_cfg() {
                            let _ = start::start(&cfg, name, None, "", None, true);
                        }
                    } else if id == "show" {
                        show_main(app);
                    } else if id == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // 主题监视：托盘运行期间由它负责。已有监视进程时 watch 会立即返回
            if let Ok(cfg) = load_cfg() {
                std::thread::spawn(move || {
                    let _ = theme::watch(&cfg.sandbox_user);
                });
            }

            if !tray_only {
                show_main(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("IsoBox 启动失败");

    app.run(|_app, event| {
        // 关闭最后一个窗口时不退出，托盘继续运行；托盘菜单的“退出”会带退出码，正常退出
        if let RunEvent::ExitRequested { api, code, .. } = event
            && code.is_none() {
                api.prevent_exit();
            }
    });
}
