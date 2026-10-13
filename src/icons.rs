//! 从程序文件里提取图标，转成 PNG 的 data URL，给图形界面显示。

use std::{ffi::c_void, path::Path};

use base64::Engine;
use windows::{
    Win32::{
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
            GetDIBits, GetObjectW, HGDIOBJ,
        },
        UI::{
            Shell::SHDefExtractIconW,
            WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO},
        },
    },
    core::PCWSTR,
};

use crate::wide::to_wide;

/// 提取 exe 的第一个图标，size 是期望的边长。读不到时返回 None
pub fn png_data_url(exe: &Path, size: u32) -> Option<String> {
    let png = png_bytes(exe, size)?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

pub fn png_bytes(exe: &Path, size: u32) -> Option<Vec<u8>> {
    let path = to_wide(&exe.to_string_lossy());
    let mut icon = HICON::default();
    let hr = unsafe { SHDefExtractIconW(PCWSTR(path.as_ptr()), 0, 0, Some(&mut icon), None, size) };
    if hr.is_err() || icon.is_invalid() {
        return None;
    }
    let result = icon_to_png(icon);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    result
}

fn icon_to_png(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let color = info.hbmColor;
    let mask = info.hbmMask;
    let result = (|| {
        if color.is_invalid() {
            return None; // 单色图标，很少见，不处理
        }
        let mut bm = BITMAP::default();
        let n = unsafe { GetObjectW(HGDIOBJ(color.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) };
        if n == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // 负数表示从上到下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = unsafe { CreateCompatibleDC(None) };
        let lines = unsafe { GetDIBits(dc, color, 0, h, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS) };
        unsafe {
            let _ = DeleteDC(dc);
        }
        if lines == 0 {
            return None;
        }
        // BGRA 转 RGBA。老式图标没有透明通道（全是 0），这时按不透明处理
        let has_alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().ok()?;
            writer.write_image_data(&px).ok()?;
        }
        Some(out)
    })();
    unsafe {
        if !color.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(color.0));
        }
        if !mask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(mask.0));
        }
    }
    result
}
