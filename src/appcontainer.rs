//! AppContainer 模式：不创建账户，不改数据盘权限。
//!
//! AppContainer 进程只能访问显式授权给“所有应用程序包”或该容器 SID 的对象。
//! 非系统盘根目录和用户目录默认都没有这类授权，所以天然读不到；
//! Program Files 和 Windows 目录默认有，所以软件能正常加载。
//!
//! 留在系统里的只有：
//! - %LOCALAPPDATA%\Packages\<容器名> 文件夹，软件的数据都写在这里
//! - HKCU 下的容器注册表配置
//! - 交换目录和 extra_read 目录上针对容器 SID 的授权
//!
//! `reset` 会把这三样全部清掉。

use std::{
    ffi::c_void,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL, LocalFree,
            SetHandleInformation,
        },
        Security::{
            DeriveCapabilitySidsFromName, FreeSid,
            Isolation::{
                CreateAppContainerProfile, DeleteAppContainerProfile,
                DeriveAppContainerSidFromAppContainerName, GetAppContainerFolderPath,
            },
            PSID, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
        },
        System::{
            Com::CoTaskMemFree,
            Threading::{
                CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
                EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList,
                LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION,
                STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
            },
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};

pub use crate::child::Child;
use crate::{account::sid_to_string, wide::to_wide};

const SE_GROUP_ENABLED: u32 = 0x0000_0004;

/// 记录已授权路径的文件，放在容器文件夹里，避免每次启动都重写 ACL
const GRANTS_FILE: &str = "isobox-grants.txt";

pub fn container_name(app: &str) -> String {
    format!("isobox.{app}")
}

/// 一个已存在的容器配置。SID 在 Drop 时释放。
pub struct Container {
    pub name: String,
    sid: PSID,
    pub sid_string: String,
    /// 本次是否新建
    pub created: bool,
}

impl Drop for Container {
    fn drop(&mut self) {
        unsafe {
            FreeSid(self.sid);
        }
    }
}

impl Container {
    /// 打开容器，不存在就创建。
    pub fn ensure(name: &str) -> Result<Self> {
        let name_w = to_wide(name);
        let desc_w = to_wide("isobox AppContainer");
        let already = HRESULT::from_win32(ERROR_ALREADY_EXISTS.0);
        let (sid, created) = match unsafe {
            CreateAppContainerProfile(
                PCWSTR(name_w.as_ptr()),
                PCWSTR(name_w.as_ptr()),
                PCWSTR(desc_w.as_ptr()),
                None,
            )
        } {
            Ok(sid) => (sid, true),
            Err(e) if e.code() == already => (Self::derive(name)?, false),
            Err(e) => return Err(e).with_context(|| format!("创建 AppContainer {name} 失败")),
        };
        let sid_string = sid_to_string(sid)?;
        Ok(Self { name: name.into(), sid, sid_string, created })
    }

    fn derive(name: &str) -> Result<PSID> {
        let name_w = to_wide(name);
        unsafe { DeriveAppContainerSidFromAppContainerName(PCWSTR(name_w.as_ptr())) }
            .with_context(|| format!("推导 {name} 的 SID 失败"))
    }

    /// 只读取，不创建。容器不存在时返回 None。
    pub fn open_existing(name: &str) -> Result<Option<Self>> {
        // 派生 SID 总能成功，所以用容器文件夹是否存在来判断
        let sid = Self::derive(name)?;
        let sid_string = sid_to_string(sid)?;
        let c = Self { name: name.into(), sid, sid_string, created: false };
        Ok(if c.folder().is_ok() { Some(c) } else { None })
    }

    /// 容器专属的数据文件夹，通常是 %LOCALAPPDATA%\Packages\<名字>\AC
    pub fn folder(&self) -> Result<PathBuf> {
        let sid_w = to_wide(&self.sid_string);
        unsafe {
            let p = GetAppContainerFolderPath(PCWSTR(sid_w.as_ptr()))
                .context("找不到容器文件夹")?;
            let s = p.to_string();
            CoTaskMemFree(Some(p.0 as *const c_void));
            Ok(PathBuf::from(s?))
        }
    }

    /// 给容器授予某个目录的访问权限。perm 是 icacls 权限，例如 M 或 RX。
    /// 已授权过的路径记录在容器文件夹里，不会重复写 ACL。
    pub fn grant(&self, path: &Path, perm: &str) -> Result<()> {
        let record = self.folder()?.join(GRANTS_FILE);
        let line = format!("{perm}\t{}", path.display());
        let existing = std::fs::read_to_string(&record).unwrap_or_default();
        if existing.lines().any(|l| l == line) {
            return Ok(());
        }
        icacls(&[
            &path.display().to_string(),
            "/grant",
            &format!("*{}:(OI)(CI){perm}", self.sid_string),
            "/q",
        ])
        .with_context(|| {
            format!(
                "给容器授权 {} 失败。如果它在 Program Files 里，需要管理员终端执行一次 isobox run",
                path.display()
            )
        })?;
        std::fs::write(&record, format!("{existing}{line}\n"))?;
        Ok(())
    }

    /// 撤销所有授权并删除容器配置和数据文件夹。
    pub fn delete(self) -> Result<()> {
        let mut errors = Vec::new();
        if let Ok(folder) = self.folder() {
            let record = std::fs::read_to_string(folder.join(GRANTS_FILE)).unwrap_or_default();
            for line in record.lines() {
                let Some((_, path)) = line.split_once('\t') else { continue };
                if !Path::new(path).exists() {
                    continue;
                }
                // 只改顶层目录，Windows 会把变化传播到继承它的子项
                if let Err(e) = icacls(&[path, "/remove", &format!("*{}", self.sid_string), "/q"]) {
                    errors.push(format!("撤销 {path} 的授权失败: {e:#}"));
                }
            }
        }
        let name_w = to_wide(&self.name);
        if let Err(e) = unsafe { DeleteAppContainerProfile(PCWSTR(name_w.as_ptr())) } {
            errors.push(format!("删除容器配置失败，确认软件已完全退出: {e}"));
        }
        if !errors.is_empty() {
            bail!(errors.join("\n"));
        }
        Ok(())
    }

    /// 在容器里启动程序。stdio 不为空时，把标准输出和标准错误写进该文件，用于排查。
    pub fn launch(
        &self,
        capabilities: &[String],
        exe: &Path,
        args: &str,
        workdir: &Path,
        stdio: Option<&Path>,
    ) -> Result<Child> {
        let caps = Capabilities::derive(capabilities)?;
        // 父进程打开的文件句柄继承给子进程，访问权限在打开时已检查，不受容器限制
        let log_file = stdio
            .map(|p| std::fs::File::create(p).with_context(|| format!("无法创建 {}", p.display())))
            .transpose()?;
        let log_handle = log_file.as_ref().map(|f| HANDLE(f.as_raw_handle()));
        if let Some(h) = log_handle {
            unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }?;
        }
        let attr_count = if log_handle.is_some() { 2 } else { 1 };
        let folder = self.folder()?;

        // Windows 会自动把 LOCALAPPDATA、TEMP、TMP 重定向到容器文件夹，这里不能再改，
        // 否则路径会叠加。APPDATA 不会被重定向，指向容器内的 Roaming，
        // 减少软件因写不进真实 AppData 而崩溃。
        let roaming = folder.join("Roaming");
        std::fs::create_dir_all(&roaming)?;
        std::fs::create_dir_all(folder.join("Temp"))?;
        let env = build_env(&[("APPDATA", roaming.display().to_string())]);

        let sc = SECURITY_CAPABILITIES {
            AppContainerSid: self.sid,
            Capabilities: caps.entries.as_ptr() as *mut SID_AND_ATTRIBUTES,
            CapabilityCount: caps.entries.len() as u32,
            Reserved: 0,
        };

        let mut size = 0usize;
        // 第一次调用只取缓冲区大小，预期返回错误
        let _ = unsafe { InitializeProcThreadAttributeList(None, attr_count, None, &mut size) };
        let mut attr_buf = vec![0u8; size];
        let attrs = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut c_void);
        unsafe { InitializeProcThreadAttributeList(Some(attrs), attr_count, None, &mut size) }?;
        let result = (|| -> Result<Child> {
            unsafe {
                UpdateProcThreadAttribute(
                    attrs,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    Some(&sc as *const _ as *const c_void),
                    size_of::<SECURITY_CAPABILITIES>(),
                    None,
                    None,
                )
            }?;
            let mut si = STARTUPINFOEXW::default();
            si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            si.lpAttributeList = attrs;
            if let Some(h) = &log_handle {
                // 只继承这一个句柄，避免把父进程的其他句柄泄露进容器
                unsafe {
                    UpdateProcThreadAttribute(
                        attrs,
                        0,
                        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                        Some(h as *const HANDLE as *const c_void),
                        size_of::<HANDLE>(),
                        None,
                        None,
                    )
                }?;
                si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
                si.StartupInfo.hStdOutput = *h;
                si.StartupInfo.hStdError = *h;
            }
            let mut cmdline = to_wide(format!("\"{}\" {}", exe.display(), args).trim_end());
            let workdir_w = to_wide(&workdir.to_string_lossy());
            let mut pi = PROCESS_INFORMATION::default();
            unsafe {
                CreateProcessW(
                    PCWSTR::null(),
                    Some(PWSTR(cmdline.as_mut_ptr())),
                    None,
                    None,
                    log_handle.is_some(),
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                    Some(env.as_ptr() as *const c_void),
                    PCWSTR(workdir_w.as_ptr()),
                    &si.StartupInfo,
                    &mut pi,
                )
            }
            .with_context(|| format!("在容器 {} 中启动 {} 失败", self.name, exe.display()))?;
            unsafe {
                let _ = CloseHandle(pi.hThread);
            }
            Ok(Child::new(pi.dwProcessId, pi.hProcess))
        })();
        unsafe { DeleteProcThreadAttributeList(attrs) };
        result
    }
}

/// 由能力名推导出的 SID 列表，Drop 时释放。
struct Capabilities {
    entries: Vec<SID_AND_ATTRIBUTES>,
}

impl Capabilities {
    fn derive(names: &[String]) -> Result<Self> {
        let mut entries = Vec::new();
        for name in names {
            let name_w = to_wide(name);
            let (mut groups, mut gcount, mut caps, mut ccount) =
                (std::ptr::null_mut::<PSID>(), 0u32, std::ptr::null_mut::<PSID>(), 0u32);
            unsafe {
                DeriveCapabilitySidsFromName(
                    PCWSTR(name_w.as_ptr()),
                    &mut groups,
                    &mut gcount,
                    &mut caps,
                    &mut ccount,
                )
            }
            .with_context(|| format!("无法识别能力 {name}"))?;
            unsafe {
                // 组 SID 用不到，直接释放
                for i in 0..gcount as usize {
                    let _ = LocalFree(Some(HLOCAL((*groups.add(i)).0)));
                }
                let _ = LocalFree(Some(HLOCAL(groups as *mut c_void)));
                for i in 0..ccount as usize {
                    entries.push(SID_AND_ATTRIBUTES {
                        Sid: *caps.add(i),
                        Attributes: SE_GROUP_ENABLED,
                    });
                }
                let _ = LocalFree(Some(HLOCAL(caps as *mut c_void)));
            }
        }
        Ok(Self { entries })
    }
}

impl Drop for Capabilities {
    fn drop(&mut self) {
        for e in &self.entries {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(e.Sid.0)));
            }
        }
    }
}

/// 用于诊断：列出能力名对应的 SID。
pub fn capability_sids(names: &[String]) -> Result<Vec<(String, Vec<String>)>> {
    let mut out = Vec::new();
    for name in names {
        let caps = Capabilities::derive(std::slice::from_ref(name))?;
        let sids = caps.entries.iter().map(|e| sid_to_string(e.Sid)).collect::<Result<_>>()?;
        out.push((name.clone(), sids));
    }
    Ok(out)
}

/// 当前环境变量加上覆盖项，拼成 CreateProcessW 需要的 UTF-16 环境块。
fn build_env(overrides: &[(&str, String)]) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !overrides.iter().any(|(o, _)| o.eq_ignore_ascii_case(k)))
        .collect();
    vars.extend(overrides.iter().map(|(k, v)| (k.to_string(), v.clone())));
    vars.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

fn icacls(args: &[&str]) -> Result<()> {
    let out = Command::new("icacls").args(args).output().context("无法运行 icacls")?;
    if !out.status.success() {
        bail!(
            "icacls {} 失败: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    Ok(())
}
