//! 列出正在运行的进程，用于判断某个隔离软件是否在运行。
//! 隔离账户的进程有的能读到完整路径，有的读不到，读不到时只能按文件名判断。

use std::path::{Path, PathBuf};

use windows::{
    Win32::{
        Foundation::CloseHandle,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
            },
            Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
        },
    },
    core::PWSTR,
};

#[derive(Debug, Clone)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub path: Option<PathBuf>,
}

pub fn list() -> Vec<Proc> {
    let mut out = Vec::new();
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return out;
    };
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while ok {
        let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        let name = String::from_utf16_lossy(&e.szExeFile[..end]);
        out.push(Proc { pid: e.th32ProcessID, path: image_path(e.th32ProcessID), name });
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snap);
    }
    out
}

fn image_path(pid: u32) -> Option<PathBuf> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    let r = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size) };
    unsafe {
        let _ = CloseHandle(h);
    }
    r.ok()?;
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])))
}

/// 主程序是否在运行：路径相同；读不到路径的进程按文件名判断
pub fn is_running(procs: &[Proc], exe: &Path) -> bool {
    let want_name = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let want_path = exe.to_string_lossy().to_lowercase();
    procs.iter().any(|p| match &p.path {
        Some(path) => path.to_string_lossy().to_lowercase() == want_path,
        None => p.name.to_lowercase() == want_name,
    })
}
