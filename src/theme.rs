//! 把当前账户的深浅色模式和强调色同步给沙盒用户。
//!
//! 这些设置按用户保存在各自的注册表里。软件以沙盒用户身份运行，读到的是沙盒用户的设置，
//! 而沙盒用户从不登录桌面，所以一直是默认的浅色。
//!
//! 写入沙盒用户的注册表不需要管理员权限：以沙盒用户身份运行一次 reg.exe 即可。
//! 写完后广播 WM_SETTINGCHANGE("ImmersiveColorSet")，正在运行的软件会重新读取主题。

use std::{ffi::c_void, path::Path};

use anyhow::{Context, Result};
use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, WAIT_OBJECT_0, WPARAM},
        System::{
            Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_BINARY, REG_DWORD, REG_NOTIFY_CHANGE_LAST_SET,
                REG_VALUE_TYPE, RRF_RT_ANY, RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue,
                RegOpenKeyExW,
            },
            Threading::{CreateEventW, CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE, WaitForMultipleObjects},
        },
        UI::WindowsAndMessaging::{SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE},
    },
    core::PCWSTR,
};

use crate::{cred, launch, wide::to_wide};

/// 要同步的注册表项和值
const SETTINGS: &[(&str, &[&str])] = &[
    (
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        &["AppsUseLightTheme", "SystemUsesLightTheme", "EnableTransparency", "ColorPrevalence"],
    ),
    (
        r"Software\Microsoft\Windows\DWM",
        &["AccentColor", "ColorizationColor", "ColorizationAfterglow", "ColorPrevalence"],
    ),
    (
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent",
        &["AccentPalette", "AccentColorMenu", "StartColorMenu"],
    ),
];

/// 监视进程的单实例标记
const WATCHER_MUTEX: &str = r"Local\IsoBoxThemeWatcher";

/// 读取当前账户的一个注册表值，转成 reg add 的参数。只处理 DWORD 和二进制。
fn read_value(key: &str, name: &str) -> Option<(&'static str, String)> {
    let key_w = to_wide(key);
    let name_w = to_wide(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut buf = [0u8; 256];
    let mut size = buf.len() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_w.as_ptr()),
            PCWSTR(name_w.as_ptr()),
            RRF_RT_ANY,
            Some(&mut ty),
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    if rc.is_err() {
        return None;
    }
    let data = &buf[..size as usize];
    if ty == REG_DWORD && data.len() == 4 {
        Some(("REG_DWORD", u32::from_le_bytes(data.try_into().ok()?).to_string()))
    } else if ty == REG_BINARY {
        Some(("REG_BINARY", data.iter().map(|b| format!("{b:02x}")).collect()))
    } else {
        None
    }
}

/// CreateProcessWithLogonW 的命令行最长 1024 个字符，留出余量
const MAX_CMDLINE: usize = 900;

/// 生成若干行 cmd 命令，每行依次执行几条 reg add，且不超过命令行长度上限
fn build_commands() -> Vec<String> {
    let mut batches = Vec::new();
    let mut current = String::new();
    for (key, names) in SETTINGS {
        for name in *names {
            let Some((ty, data)) = read_value(key, name) else { continue };
            let part = format!(r#"reg add "HKCU\{key}" /v {name} /t {ty} /d {data} /f >nul"#);
            if !current.is_empty() && current.len() + part.len() + 3 > MAX_CMDLINE {
                batches.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push_str(" & ");
            }
            current.push_str(&part);
        }
    }
    if !current.is_empty() {
        // 记下同步时间，排查时可以确认同步是否真的发生过。值在沙盒用户自己的注册表里，卸载时随个人文件夹删除
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        current.push_str(&format!(
            r#" & reg add "HKCU\Software\IsoBox" /v LastThemeSync /t REG_SZ /d {stamp} /f >nul"#
        ));
        batches.push(current);
    }
    batches
}

/// 把当前账户的主题设置写进沙盒用户的注册表，然后通知正在运行的程序刷新。
pub fn sync(user: &str, password: &str) -> Result<()> {
    let sys32 = Path::new(r"C:\Windows\System32");
    for cmd in build_commands() {
        // /s 让 cmd 只去掉最外层的一对引号，里面的引号原样保留
        let args = format!("/d /s /c \"{cmd}\"");
        let code = launch::launch_as_hidden(user, password, &sys32.join("cmd.exe"), &args, sys32)?
            .wait()
            .context("等待主题同步完成失败")?;
        if code != 0 {
            anyhow::bail!("同步主题时 reg 返回 {code}");
        }
    }
    broadcast();
    Ok(())
}

/// 通知所有顶层窗口主题已变化。同一个桌面上的沙盒用户窗口也会收到。
pub fn broadcast() {
    let param = to_wide("ImmersiveColorSet");
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND(0xffff as *mut c_void), // HWND_BROADCAST
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(param.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            3000,
            None,
        );
    }
}

/// 监视进程是否已在运行
pub fn watcher_running() -> bool {
    let name = to_wide(WATCHER_MUTEX);
    unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())).is_ok() }
}

/// 设置环境变量 ISOBOX_THEME_LOG 为文件路径时，监视进程把每一步写进这个文件。平时不写任何东西
fn log(msg: &str) {
    if let Ok(path) = std::env::var("ISOBOX_THEME_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "{t} {msg}");
        }
    }
}

/// 常驻监视：当前账户的主题一变就同步给沙盒用户。一直运行到你注销，平时阻塞在等待上，几乎不占资源。
pub fn watch(user: &str) -> Result<()> {
    let r = watch_inner(user);
    if let Err(e) = &r {
        log(&format!("exit with error: {e:#}"));
    }
    r
}

fn watch_inner(user: &str) -> Result<()> {
    log("watcher starting");
    let name = to_wide(WATCHER_MUTEX);
    let _mutex = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log("another watcher is running, exit");
        return Ok(()); // 已经有一个在运行
    }
    let password = cred::load(user)?;
    log("credential loaded");

    // 深浅色、DWM 颜色、强调色分别在三个注册表项里。每个项一个事件，哪个变了都会被唤醒。
    // 等待期间线程停在内核里，不占 CPU。
    let mut keys = Vec::new();
    let mut events = Vec::new();
    for (path, _) in SETTINGS {
        let key_w = to_wide(path);
        let mut key = HKEY::default();
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key_w.as_ptr()), None, KEY_NOTIFY, &mut key) }.is_err() {
            log(&format!("skip missing key {path}"));
            continue;
        }
        keys.push(key);
        events.push(unsafe { CreateEventW(None, false, false, None) }?);
    }
    if keys.is_empty() {
        anyhow::bail!("找不到任何主题注册表项");
    }

    let arm = |i: usize| -> Result<()> {
        unsafe { RegNotifyChangeKeyValue(keys[i], false, REG_NOTIFY_CHANGE_LAST_SET, Some(events[i]), true) }
            .ok()
            .context("注册主题变化通知失败")
    };
    let result = (|| -> Result<()> {
        for i in 0..keys.len() {
            arm(i)?;
        }
        loop {
            log("waiting for change");
            let r = unsafe { WaitForMultipleObjects(&events, false, u32::MAX) };
            let i = (r.0 - WAIT_OBJECT_0.0) as usize;
            if i >= keys.len() {
                anyhow::bail!("等待主题变化失败: {r:?}");
            }
            log(&format!("change detected in {}", SETTINGS[i].0));
            // 切换主题时会连续改好几个值，稍等再同步，并合并成一次
            std::thread::sleep(std::time::Duration::from_millis(800));
            // 通知是一次性的：重新注册触发了的那个；其他项期间若也变了，它们的事件已置位，下一轮立刻返回
            arm(i)?;
            match sync(user, &password) {
                Ok(()) => log("synced"),
                Err(e) => log(&format!("sync failed: {e:#}")),
            }
        }
    })();
    for k in keys {
        unsafe {
            let _ = RegCloseKey(k);
        }
    }
    result
}

/// 如果监视进程没在运行，就在后台启动一个。launcher 是 isobox-launch.exe 的路径。
pub fn ensure_watcher(launcher: &Path) {
    if watcher_running() || !launcher.exists() {
        return;
    }
    // 不用 std::process::Command：它总是让子进程继承父进程所有可继承的句柄，
    // 包括调用者的输出管道，常驻的监视进程会一直占着管道，让调用者以为命令没结束。
    // 这里直接调用 CreateProcessW 并关闭句柄继承。
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CREATE_NEW_PROCESS_GROUP, CreateProcessW, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
        },
    };
    let mut cmdline = to_wide(&format!("\"{}\" --watch-theme", launcher.display()));
    let dir = to_wide(&launcher.parent().unwrap_or(Path::new(".")).to_string_lossy());
    let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut pi = PROCESS_INFORMATION::default();
    let ok = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(windows::core::PWSTR(cmdline.as_mut_ptr())),
            None,
            None,
            false,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR(dir.as_ptr()),
            &si,
            &mut pi,
        )
    };
    if ok.is_ok() {
        unsafe {
            let _ = CloseHandle(pi.hThread);
            let _ = CloseHandle(pi.hProcess);
        }
    }
}
