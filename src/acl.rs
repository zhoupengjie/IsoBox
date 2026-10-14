//! 权限相关：锁定目录、读写目录权限的文本形式、检查禁止规则、备份和还原。

use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW,
                ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
                SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
            },
            ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
            UNPROTECTED_DACL_SECURITY_INFORMATION,
        },
    },
    core::{BOOL, PCWSTR, PWSTR},
};

use crate::wide::to_wide;

pub const ADMINISTRATORS: &str = "*S-1-5-32-544";
pub const SYSTEM: &str = "*S-1-5-18";
pub const USERS: &str = "*S-1-5-32-545";

/// 运行 icacls，失败时附上它的输出
pub fn icacls(args: &[&str]) -> Result<String> {
    let out = Command::new("icacls").args(args).output().context("无法运行 icacls")?;
    let text = crate::cmd::decode_console(&out.stdout);
    if !out.status.success() {
        bail!("icacls {} 失败: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

/// 和系统的 Program Files 一样：管理员和 SYSTEM 完全控制，普通用户只读。不继承上级权限。
pub fn lock_readonly(dir: &Path) -> Result<()> {
    let d = dir.display().to_string();
    icacls(&[
        &d,
        "/inheritance:r",
        "/grant:r",
        &format!("{ADMINISTRATORS}:(OI)(CI)F"),
        &format!("{SYSTEM}:(OI)(CI)F"),
        &format!("{USERS}:(OI)(CI)RX"),
        "/q",
    ])?;
    Ok(())
}

/// 库的 apps 目录：在只读的基础上，允许沙盒用户读写
pub fn lock_apps_dir(dir: &Path, sandbox_sid: &str) -> Result<()> {
    lock_readonly(dir)?;
    icacls(&[&dir.display().to_string(), "/grant", &format!("*{sandbox_sid}:(OI)(CI)M"), "/q"])?;
    Ok(())
}

/// 让目录下所有文件重新继承上级权限。移动文件夹后使用：同盘移动会保留旧位置继承来的权限
pub fn reset_tree(dir: &Path) -> Result<()> {
    icacls(&[&dir.display().to_string(), "/reset", "/t", "/c", "/q"])?;
    Ok(())
}

/// 读取目录自身权限的文本形式（SDDL，只含 DACL），包括“是否继承上级”的标志
pub fn get_sddl(path: &Path) -> Result<String> {
    let p = to_wide(&path.to_string_lossy());
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let rc = unsafe {
        GetNamedSecurityInfoW(PCWSTR(p.as_ptr()), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, None, None, None, None, &mut sd)
    };
    if rc.is_err() {
        bail!("读取 {} 的权限失败: {rc:?}", path.display());
    }
    let mut s = PWSTR::null();
    let r = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, DACL_SECURITY_INFORMATION, &mut s, None)
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    r.context("转换权限文本失败")?;
    let out = unsafe { s.to_string() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
    }
    Ok(out?)
}

/// 按文本形式设置目录自身的权限。可继承的规则会传播到子项
pub fn set_sddl(path: &Path, sddl: &str) -> Result<()> {
    let s = to_wide(sddl);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(s.as_ptr()), SDDL_REVISION_1, &mut sd, None) }
        .context("解析权限文本失败")?;
    let result = (|| -> Result<()> {
        let mut present = BOOL(0);
        let mut defaulted = BOOL(0);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) }?;
        let mut control = 0u16;
        let mut rev = 0u32;
        unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut rev) }?;
        let protect = if control & SE_DACL_PROTECTED.0 != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let p = to_wide(&path.to_string_lossy());
        let rc = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(p.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | protect,
                None,
                None,
                Some(dacl),
                None,
            )
        };
        if rc.is_err() {
            bail!("设置 {} 的权限失败: {rc:?}", path.display());
        }
        Ok(())
    })();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    result
}

/// 目录自身是否有一条针对 sid 的显式禁止规则（不算继承来的）
pub fn has_explicit_deny(path: &Path, sid: &str) -> bool {
    get_sddl(path).is_ok_and(|s| sddl_has_explicit_deny(&s, sid))
}

/// 权限文本里是否有一条针对 sid 的显式禁止规则。带 ID 标志的是继承来的，不算
pub fn sddl_has_explicit_deny(sddl: &str, sid: &str) -> bool {
    sddl_has_explicit(sddl, "D", sid)
}

/// 位置上有没有一条针对 sid 的显式允许规则，例如交换文件夹的授权
pub fn has_explicit_allow(path: &Path, sid: &str) -> bool {
    get_sddl(path).is_ok_and(|s| sddl_has_explicit(&s, "A", sid))
}

fn sddl_has_explicit(sddl: &str, kind: &str, sid: &str) -> bool {
    sddl.split('(').skip(1).any(|ace| {
        let ace = ace.trim_end_matches(')');
        let parts: Vec<&str> = ace.split(';').collect();
        parts.len() >= 6
            && parts[0] == kind
            && !parts[1].contains("ID")
            && parts[5].eq_ignore_ascii_case(sid)
    })
}

/// 备份目录（或整个盘）的权限：目录自身存为文本，子项用 icacls /save。
/// 返回备份所在的文件夹
pub fn backup(target: &Path, backup_root: &Path) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let label: String = target
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    let dir = backup_root.join(format!("{stamp}-{label}"));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("target.txt"), target.to_string_lossy().as_bytes())?;
    std::fs::write(dir.join("self.sddl"), get_sddl(target)?)?;
    let children = dir.join("children.acl");
    let pattern = target.join("*");
    // /c：遇到系统保护的文件继续；返回码非零也可能只是部分文件被跳过，所以只检查文件是否生成
    let _ = Command::new("icacls")
        .args([&pattern.display().to_string(), "/save", &children.display().to_string(), "/t", "/c", "/q"])
        .output();
    if !children.exists() {
        bail!("备份 {} 的子项权限失败", target.display());
    }
    Ok(dir)
}

/// 用 backup 生成的文件夹还原权限
pub fn restore(backup_dir: &Path) -> Result<PathBuf> {
    let target = PathBuf::from(
        std::fs::read_to_string(backup_dir.join("target.txt")).context("备份里没有 target.txt")?.trim(),
    );
    let sddl = std::fs::read_to_string(backup_dir.join("self.sddl")).context("备份里没有 self.sddl")?;
    set_sddl(&target, sddl.trim())?;
    let children = backup_dir.join("children.acl");
    if children.exists() {
        let out = Command::new("icacls")
            .args([&target.display().to_string(), "/restore", &children.display().to_string(), "/c", "/q"])
            .output()
            .context("无法运行 icacls")?;
        if !out.status.success() {
            eprintln!(
                "注意：部分子项还原失败，通常是系统保护的文件，可以忽略: {}",
                crate::cmd::decode_console(&out.stdout).trim()
            );
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "S-1-5-21-1111111111-2222222222-3333333333-1005";

    #[test]
    fn explicit_deny_on_root_is_found() {
        let root = "D:AI(D;OICI;FA;;;S-1-5-21-1111111111-2222222222-3333333333-1005)(A;;FA;;;BA)(A;OICIIO;GA;;;BA)";
        assert!(sddl_has_explicit_deny(root, SID));
        assert!(!sddl_has_explicit_deny(root, "S-1-5-21-1-2-3-1001"));
    }

    #[test]
    fn inherited_deny_is_ignored() {
        let child = "D:AI(D;OICIID;FA;;;S-1-5-21-1111111111-2222222222-3333333333-1005)(A;ID;FA;;;BA)";
        assert!(!sddl_has_explicit_deny(child, SID));
    }

    #[test]
    fn allow_is_not_deny() {
        let x = "D:AI(A;OICI;0x1301bf;;;S-1-5-21-1111111111-2222222222-3333333333-1005)";
        assert!(!sddl_has_explicit_deny(x, SID));
        assert!(sddl_has_explicit(x, "A", SID));
        assert!(!sddl_has_explicit("D:AI(A;OICIID;0x1301bf;;;S-1-5-21-1111111111-2222222222-3333333333-1005)", "A", SID));
    }
}
