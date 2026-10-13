/// 把 Rust 字符串转成以 0 结尾的 UTF-16，供 Win32 W 系列 API 使用。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
