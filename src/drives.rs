//! 识别电脑上的每个盘：类型、文件系统、是否外接，以及能不能保护、能不能建库。

use std::ffi::c_void;

use windows::{
    Win32::{
        Foundation::{CloseHandle, GENERIC_ACCESS_RIGHTS},
        Storage::FileSystem::{
            BusTypeUsb, CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{
                IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
                STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
            },
        },
    },
    core::PCWSTR,
};

use crate::wide::to_wide;

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const FILE_PERSISTENT_ACLS: u32 = 0x0000_0008;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Fixed,
    Removable,
    Network,
    CdRom,
    Other,
}

#[derive(Debug, Clone)]
pub struct Drive {
    pub letter: char,
    pub kind: Kind,
    pub label: String,
    pub fs: String,
    /// 文件系统支持权限控制（NTFS、ReFS）
    pub acl: bool,
    /// 通过 USB 连接。外接硬盘在系统里常常也报告为固定硬盘
    pub usb: bool,
    pub system: bool,
    pub total: u64,
    pub free: u64,
    /// 盘符存在但读不出卷信息，例如没插卡的读卡器
    pub ready: bool,
}

impl Drive {
    pub fn root(&self) -> String {
        format!("{}:\\", self.letter)
    }

    /// 能否给这个盘加“禁止隔离账户访问”的规则。不能时返回原因
    pub fn protect_blocker(&self) -> Option<&'static str> {
        if !self.ready {
            Some("当前不可用")
        } else if self.system {
            Some("系统盘：Windows 自身需要读它，你的用户目录已经默认隔离")
        } else if self.kind != Kind::Fixed {
            Some("不是固定硬盘")
        } else if !self.acl {
            Some("文件系统不支持权限控制")
        } else {
            None
        }
    }

    /// 能否在这个盘上建库。不能时返回原因
    pub fn library_blocker(&self) -> Option<&'static str> {
        if !self.ready {
            Some("当前不可用")
        } else if self.kind != Kind::Fixed {
            Some("不是固定硬盘")
        } else if !self.acl {
            Some("文件系统不支持权限控制")
        } else if self.usb {
            Some("外接硬盘：拔掉后库里的软件就无法启动")
        } else {
            None
        }
    }

    pub fn describe_kind(&self) -> &'static str {
        match (self.kind, self.usb) {
            (Kind::Fixed, true) => "外接硬盘",
            (Kind::Fixed, false) => "固定硬盘",
            (Kind::Removable, _) => "可移动磁盘",
            (Kind::Network, _) => "网络盘",
            (Kind::CdRom, _) => "光驱",
            (Kind::Other, _) => "其他",
        }
    }
}

pub fn system_drive() -> char {
    std::env::var("SystemDrive")
        .ok()
        .and_then(|s| s.chars().next())
        .unwrap_or('C')
        .to_ascii_uppercase()
}

/// 列出所有盘符
pub fn list() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| info((b'A' + i as u8) as char))
        .collect()
}

pub fn get(letter: char) -> Option<Drive> {
    let l = letter.to_ascii_uppercase();
    list().into_iter().find(|d| d.letter == l)
}

fn info(letter: char) -> Drive {
    let root = format!("{letter}:\\");
    let root_w = to_wide(&root);
    let kind = match unsafe { GetDriveTypeW(PCWSTR(root_w.as_ptr())) } {
        DRIVE_FIXED => Kind::Fixed,
        DRIVE_REMOVABLE => Kind::Removable,
        DRIVE_REMOTE => Kind::Network,
        DRIVE_CDROM => Kind::CdRom,
        _ => Kind::Other,
    };
    let mut label = [0u16; 261];
    let mut fs = [0u16; 64];
    let mut flags = 0u32;
    let ready = unsafe {
        GetVolumeInformationW(PCWSTR(root_w.as_ptr()), Some(&mut label), None, None, Some(&mut flags), Some(&mut fs))
    }
    .is_ok();
    let (mut free, mut total) = (0u64, 0u64);
    if ready {
        let _ = unsafe { GetDiskFreeSpaceExW(PCWSTR(root_w.as_ptr()), Some(&mut free), Some(&mut total), None) };
    }
    Drive {
        letter,
        kind,
        label: from_wide(&label),
        fs: from_wide(&fs),
        acl: ready && flags & FILE_PERSISTENT_ACLS != 0,
        usb: kind == Kind::Fixed && ready && is_usb(letter),
        system: letter == system_drive(),
        total,
        free,
        ready,
    }
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 查询卷所在磁盘的总线类型。不需要管理员权限：打开卷时不申请任何读写权限
fn is_usb(letter: char) -> bool {
    let path = to_wide(&format!(r"\\.\{letter}:"));
    let Ok(h) = (unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            GENERIC_ACCESS_RIGHTS(0).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }) else {
        return false;
    };
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        ..Default::default()
    };
    let mut out = [0u8; 1024];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const c_void),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(out.as_mut_ptr() as *mut c_void),
            out.len() as u32,
            Some(&mut returned),
            None,
        )
    }
    .is_ok();
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok || (returned as usize) < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return false;
    }
    let desc = unsafe { &*(out.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    desc.BusType == BusTypeUsb
}

pub fn human_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gb = bytes as f64 / GB;
    if gb >= 1024.0 { format!("{:.1} TB", gb / 1024.0) } else { format!("{gb:.0} GB") }
}
