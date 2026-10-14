//! 运行系统自带的命令行程序（reg、icacls、robocopy、powershell）。
//!
//! 它们的输出用控制台代码页（中文系统是 GBK），直接继承到日志或管道里，会和 Rust 写的 UTF-8 混在一起，
//! 界面和安装程序就显示成乱码。所以一律先收集输出，转成 Unicode 再打印。

use std::process::{Command, ExitStatus};

use anyhow::{Context, Result};
use windows::Win32::Globalization::{CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};

/// 把命令行程序的输出转成文字：合法的 UTF-8 原样使用（例如设置了 UTF-8 输出的 PowerShell），否则按 OEM 代码页转换
pub fn decode_console(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    let n = unsafe { MultiByteToWideChar(CP_OEMCP, flags, bytes, None) };
    if n <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; n as usize];
    let n = unsafe { MultiByteToWideChar(CP_OEMCP, flags, bytes, Some(&mut wide)) };
    String::from_utf16_lossy(&wide[..n.max(0) as usize])
}

/// 运行命令，把它的输出缩进四格打印出来。返回退出状态，由调用者判断成败
pub fn run_print(cmd: &mut Command) -> Result<ExitStatus> {
    let out = cmd.output().with_context(|| format!("无法运行 {}", cmd.get_program().to_string_lossy()))?;
    for bytes in [&out.stdout, &out.stderr] {
        for line in decode_console(bytes).lines().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
            println!("    {line}");
        }
    }
    Ok(out.status)
}

#[cfg(test)]
mod tests {
    use super::decode_console;

    #[test]
    fn keeps_utf8_and_ascii() {
        assert_eq!(decode_console(b"ok\r\n"), "ok\r\n");
        assert_eq!(decode_console("已完成".as_bytes()), "已完成");
        assert_eq!(decode_console(b""), "");
    }
}
