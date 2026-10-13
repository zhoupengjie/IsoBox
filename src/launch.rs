//! 以沙盒用户身份启动程序，窗口出现在当前桌面上，机制与 runas 相同。

use std::path::Path;

use anyhow::{Context, Result};
use windows::{
    Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessWithLogonW, LOGON_WITH_PROFILE,
            PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
        },
        UI::WindowsAndMessaging::SW_HIDE,
    },
    core::{PCWSTR, PWSTR},
};

use crate::{child::Child, wide::to_wide};

/// 以 user 身份启动程序。第一次以该身份启动时，系统会为它创建个人文件夹。
pub fn launch_as(user: &str, password: &str, exe: &Path, args: &str, workdir: &Path) -> Result<Child> {
    launch_impl(user, password, exe, args, workdir, false)
}

/// 同上，但隐藏窗口，用于在后台跑 cmd、reg 之类的命令行工具
pub fn launch_as_hidden(user: &str, password: &str, exe: &Path, args: &str, workdir: &Path) -> Result<Child> {
    launch_impl(user, password, exe, args, workdir, true)
}

fn launch_impl(
    user: &str,
    password: &str,
    exe: &Path,
    args: &str,
    workdir: &Path,
    hidden: bool,
) -> Result<Child> {
    let user_w = to_wide(user);
    let domain_w = to_wide("."); // 本机账户
    let pass_w = to_wide(password);
    let mut cmdline = to_wide(format!("\"{}\" {}", exe.display(), args).trim_end());
    let workdir_w = to_wide(&workdir.to_string_lossy());

    let mut si = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    if hidden {
        si.dwFlags |= STARTF_USESHOWWINDOW;
        si.wShowWindow = SW_HIDE.0 as u16;
    }
    let mut pi = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessWithLogonW(
            PCWSTR(user_w.as_ptr()),
            PCWSTR(domain_w.as_ptr()),
            PCWSTR(pass_w.as_ptr()),
            LOGON_WITH_PROFILE,
            PCWSTR::null(),
            Some(PWSTR(cmdline.as_mut_ptr())),
            CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR(workdir_w.as_ptr()),
            &si,
            &mut pi,
        )
    }
    .with_context(|| format!("以 {user} 身份启动 {} 失败", exe.display()))?;

    unsafe {
        let _ = CloseHandle(pi.hThread);
    }
    Ok(Child::new(pi.dwProcessId, pi.hProcess))
}
