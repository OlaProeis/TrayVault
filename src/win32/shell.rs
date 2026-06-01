//! Shell helpers (open URLs in the default browser).

use crate::error::Result;
use crate::win32::ffi::{ShellExecuteW, HWND, INT, SW_SHOWNORMAL};
use crate::win32::{last_error, wide};

/// Open `url` in the user's default browser.
pub fn open_url(url: &str) -> Result<()> {
    let operation = wide("open");
    let file = wide(url);
    // SAFETY: null HWND is documented for ShellExecuteW; wide buffers live for the call.
    let ret = unsafe {
        ShellExecuteW(
            0 as HWND,
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL as INT,
        )
    };
    // Values <= 32 indicate failure (Win32 SE_ERR_* codes).
    if ret <= 32 {
        Err(last_error("ShellExecuteW"))
    } else {
        Ok(())
    }
}
