//! 以管理员身份运行 isobox-setup：弹出系统的 UAC 确认，窗口隐藏，输出写进临时日志后读回。
//! 图形界面本身始终以普通权限运行。

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED},
        System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
        UI::{
            Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::{HRESULT, PCWSTR},
};

use isobox::{paths, wide::to_wide};

#[derive(Debug, Serialize)]
pub struct AdminResult {
    pub code: i32,
    pub output: String,
    /// 用户在 UAC 窗口里点了“否”
    pub cancelled: bool,
}

/// 安装目录里的 isobox-setup.exe。开发时如果还没部署，就用和界面同目录的那个
fn setup_exe() -> Result<PathBuf> {
    let installed = paths::expected_install_dir().join("isobox-setup.exe");
    if installed.exists() {
        return Ok(installed);
    }
    let local = paths::install_dir()?.join("isobox-setup.exe");
    if local.exists() {
        return Ok(local);
    }
    bail!("找不到 isobox-setup.exe，请先部署 IsoBox")
}

/// 按 Windows 命令行规则给参数加引号
fn quote(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_string();
    }
    let mut s = String::from("\"");
    let mut slashes = 0;
    for c in a.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                s.push_str(&"\\".repeat(slashes * 2 + 1));
                s.push('"');
                slashes = 0;
            }
            _ => {
                s.push_str(&"\\".repeat(slashes));
                s.push(c);
                slashes = 0;
            }
        }
    }
    s.push_str(&"\\".repeat(slashes * 2));
    s.push('"');
    s
}

pub fn run_setup(args: &[String]) -> Result<AdminResult> {
    let exe = setup_exe()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let log = std::env::temp_dir().join(format!("isobox-admin-{nanos}.log"));
    let mut all: Vec<String> = vec!["--log".into(), log.display().to_string(), "--non-interactive".into()];
    all.extend(args.iter().cloned());
    let params = all.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");

    let verb = to_wide("runas");
    let file = to_wide(&exe.to_string_lossy());
    let params_w = to_wide(&params);
    let dir = to_wide(&exe.parent().unwrap_or(&exe).to_string_lossy());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params_w.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
            return Ok(AdminResult { code: -1, output: "已取消：没有在 UAC 窗口里确认".into(), cancelled: true });
        }
        return Err(e).context("无法以管理员身份运行 isobox-setup");
    }
    let mut code = 0u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
    }
    let output = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    Ok(AdminResult { code: code as i32, output, cancelled: false })
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quotes_like_the_windows_command_line_parser() {
        assert_eq!(quote("add"), "add");
        assert_eq!(quote(r"D:\x y\a.exe"), r#""D:\x y\a.exe""#);
        assert_eq!(quote(r"D:\"), r"D:\");
        assert_eq!(quote(r"C:\Program Files\"), r#""C:\Program Files\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(""), r#""""#);
    }
}
