//! 启动后的子进程句柄。

use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
};

/// Drop 时只关闭句柄，不会结束进程。
pub struct Child {
    pub pid: u32,
    handle: HANDLE,
}

impl Child {
    pub(crate) fn new(pid: u32, handle: HANDLE) -> Self {
        Self { pid, handle }
    }

    /// 等待进程退出，返回退出码。
    pub fn wait(&self) -> anyhow::Result<u32> {
        unsafe {
            WaitForSingleObject(self.handle, INFINITE);
            let mut code = 0u32;
            GetExitCodeProcess(self.handle, &mut code)?;
            Ok(code)
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
