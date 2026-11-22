//! 开机自启：HKCU\...\Run。
//!
//! 注意 UIPI：普通权限进程无法拦截管理员权限窗口的按键。
//! 若需要覆盖提权程序，须改用「任务计划程序 + 使用最高权限运行」启动本程序，
//! 这属于部署方式，不改变此处注册表逻辑。

use crate::util::wide;
use std::io;
use std::ptr;
use windows_sys::Win32::System::Registry::*;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "KeyRemapper";

pub fn is_enabled() -> bool {
    unsafe {
        let sub = wide(RUN_KEY);
        let mut key: HKEY = std::mem::zeroed();
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_QUERY_VALUE, &mut key) != 0 {
            return false;
        }
        let name = wide(VALUE_NAME);
        let mut size: u32 = 0;
        let rc = RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut size,
        );
        RegCloseKey(key);
        rc == 0
    }
}

pub fn set_enabled(on: bool) -> io::Result<()> {
    unsafe {
        let sub = wide(RUN_KEY);
        let mut key: HKEY = std::mem::zeroed();
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE, &mut key);
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        let name = wide(VALUE_NAME);
        let rc = if on {
            let exe = std::env::current_exe()?;
            let data = wide(&format!("\"{}\"", exe.display()));
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                (data.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        // 2 = ERROR_FILE_NOT_FOUND，取消自启时值本就不存在，视为成功。
        if rc == 0 || rc == 2 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(rc as i32))
        }
    }
}
