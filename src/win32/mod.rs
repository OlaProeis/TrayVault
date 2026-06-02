//! Safe-ish wrappers over the raw [`ffi`] surface.
//!
//! The raw bindings in [`ffi`] are all `unsafe`. This module provides the small
//! set of foundational helpers used across the app and establishes the pattern
//! every later Win32 module should follow: keep `unsafe` blocks tight, check the
//! documented failure value, and translate failures into [`ClipError`] by
//! calling [`last_error`] *immediately* after the failing call.

pub mod autostart;
pub mod clipboard;
pub mod ffi;
pub mod gdi;
pub mod glyph_raster;
pub mod hotkey;
pub mod shell;
pub mod tray;
pub mod wic;
pub mod window;

use crate::error::{ClipError, Result};
use ffi::{GetCurrentProcessId, GetCurrentThreadId, GetLastError, GetModuleHandleW, HINSTANCE};

/// Capture the calling thread's last Win32 error as a [`ClipError::Win32`].
///
/// Must be called *immediately* after the failing API: any intervening Win32
/// call may overwrite the thread-local error code.
pub fn last_error(api: &'static str) -> ClipError {
    // SAFETY: `GetLastError` reads thread-local state and is always safe to call.
    let code = unsafe { GetLastError() };
    ClipError::Win32 { api, code }
}

/// Translate a registry API return code into [`ClipError::Registry`].
///
/// Registry functions return the error code directly (not via `GetLastError`).
pub fn registry_error(op: &'static str, code: i32) -> ClipError {
    ClipError::Registry {
        op,
        code: code as u32,
    }
}

/// Encode a string as a NUL-terminated UTF-16 buffer for the `*W` Win32 APIs.
///
/// Keep the returned `Vec` alive for as long as the pointer is in use.
#[allow(dead_code)] // first used by the window module in Task 2
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// PID of the current process.
pub fn current_process_id() -> u32 {
    // SAFETY: no arguments, no failure mode.
    unsafe { GetCurrentProcessId() }
}

/// Thread id of the calling thread.
pub fn current_thread_id() -> u32 {
    // SAFETY: no arguments, no failure mode.
    unsafe { GetCurrentThreadId() }
}

/// `HINSTANCE` of the running executable, used for window-class registration.
pub fn current_module_handle() -> Result<HINSTANCE> {
    // SAFETY: passing null asks for the handle of the file used to create the
    // calling process, which is always valid for an .exe.
    let handle = unsafe { GetModuleHandleW(std::ptr::null()) };
    if handle == 0 {
        Err(last_error("GetModuleHandleW"))
    } else {
        Ok(handle)
    }
}
