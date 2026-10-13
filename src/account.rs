//! 本地账户相关的查询：SID、个人文件夹位置、文件所有者。

use std::{ffi::c_void, path::{Path, PathBuf}};

use anyhow::{Context, Result, bail};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::{
            Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
            LookupAccountNameW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
        },
        System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
    },
    core::{PCWSTR, PWSTR},
};

use crate::wide::to_wide;

/// 把用户名解析成 SID 字符串，例如 S-1-5-21-xxx-1001。
pub fn lookup_sid(user: &str) -> Result<String> {
    let name_w = to_wide(user);
    let mut sid_len = 0u32;
    let mut dom_len = 0u32;
    let mut use_ = SID_NAME_USE::default();
    // 第一次调用只为取得缓冲区大小，预期会失败
    let _ = unsafe {
        LookupAccountNameW(PCWSTR::null(), PCWSTR(name_w.as_ptr()), None, &mut sid_len, None, &mut dom_len, &mut use_)
    };
    if sid_len == 0 {
        bail!("找不到用户 {user}");
    }
    let mut sid_buf = vec![0u8; sid_len as usize];
    let mut dom_buf = vec![0u16; dom_len as usize];
    unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            PCWSTR(name_w.as_ptr()),
            Some(PSID(sid_buf.as_mut_ptr() as *mut _)),
            &mut sid_len,
            Some(PWSTR(dom_buf.as_mut_ptr())),
            &mut dom_len,
            &mut use_,
        )
    }
    .with_context(|| format!("查询 {user} 的 SID 失败"))?;
    sid_to_string(PSID(sid_buf.as_mut_ptr() as *mut _))
}

pub fn sid_to_string(sid: PSID) -> Result<String> {
    let mut s = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(sid, &mut s)?;
        let out = s.to_string();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
        Ok(out?)
    }
}

/// 从注册表 ProfileList 读出账户个人文件夹的位置。账户从未登录过时返回 None。
pub fn profile_path(sid: &str) -> Result<Option<PathBuf>> {
    let key = to_wide(&format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}"));
    let value = to_wide("ProfileImagePath");
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    if rc.is_err() {
        return Ok(None);
    }
    let len = (size as usize / 2).saturating_sub(1);
    Ok(Some(PathBuf::from(String::from_utf16_lossy(&buf[..len]))))
}

/// 文件或文件夹所有者的 SID 字符串。
pub fn owner_sid(path: &Path) -> Result<String> {
    let path_w = to_wide(&path.to_string_lossy());
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let rc = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_w.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        )
    };
    if rc.is_err() {
        bail!("读取 {} 的所有者失败: {rc:?}", path.display());
    }
    let s = sid_to_string(owner);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    s
}
