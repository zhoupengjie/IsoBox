//! 把沙盒用户的密码存进当前用户的 Windows 凭据管理器，启动时读出。

use anyhow::{Context, Result};
use windows::{
    Win32::Security::Credentials::{
        CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW,
        CredFree, CredReadW, CredWriteW,
    },
    core::{PCWSTR, PWSTR},
};

use crate::wide::to_wide;

fn target(user: &str) -> Vec<u16> {
    to_wide(&format!("isobox:{user}"))
}

pub fn store(user: &str, password: &str) -> Result<()> {
    let mut target = target(user);
    let mut uname = to_wide(user);
    let blob: Vec<u16> = password.encode_utf16().collect();
    let cred = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: (blob.len() * 2) as u32,
        CredentialBlob: blob.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(uname.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0) }.context("写入凭据管理器失败")?;
    Ok(())
}

/// 删除凭据。返回 false 表示本来就不存在。
pub fn delete(user: &str) -> Result<bool> {
    let target = target(user);
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(true),
        // ERROR_NOT_FOUND
        Err(e) if e.code().0 as u32 == 0x8007_0490 => Ok(false),
        Err(e) => Err(e).context("删除凭据失败"),
    }
}

pub fn load(user: &str) -> Result<String> {
    let target = target(user);
    let mut p: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut p) }.with_context(
        || format!("凭据管理器里没有 isobox:{user}，请先运行 isobox store-password"),
    )?;
    let password = unsafe {
        let c = &*p;
        let slice = std::slice::from_raw_parts(
            c.CredentialBlob as *const u16,
            c.CredentialBlobSize as usize / 2,
        );
        let s = String::from_utf16(slice);
        CredFree(p as *const _);
        s
    }?;
    Ok(password)
}
