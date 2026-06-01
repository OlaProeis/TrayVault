//! Windows Run-key autostart registration for TrayVault.

use std::path::Path;

use crate::error::Result;
use crate::log;
use crate::win32::{registry_error, wide};

use super::ffi::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, BYTE, DWORD,
    ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WRITE, REG_SZ,
};

const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "TrayVault";

/// Enable or disable autostart via `HKCU\...\Run\TrayVault`.
///
/// When enabled, writes a quoted command line with `--minimized` so the app
/// starts tray-only after sign-in.
pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    let exe = exe_path.to_string_lossy();
    if enabled {
        enable_autostart(&exe)
    } else {
        disable_autostart()
    }
}

fn enable_autostart(exe_path: &str) -> Result<()> {
    let command = autostart_command(exe_path);
    let data = wide(&command);
    let byte_len = (data.len() * std::mem::size_of::<u16>()) as DWORD;

    let subkey = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);

    let mut key: HKEY = 0;
    // SAFETY: `subkey` and `value_name` are NUL-terminated UTF-16; `key` is valid out-pointer.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null_mut(),
            0,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS as i32 {
        let err = registry_error("RegCreateKeyExW", status);
        log::error(&format!("autostart enable failed: {err}"));
        return Err(err);
    }

    // SAFETY: `key` is open; `data` is a valid REG_SZ buffer including the NUL terminator.
    let set_status = unsafe {
        RegSetValueExW(
            key,
            value_name.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr().cast::<BYTE>(),
            byte_len,
        )
    };

    // SAFETY: `key` was opened successfully.
    let _ = unsafe { RegCloseKey(key) };

    if set_status != ERROR_SUCCESS as i32 {
        let err = registry_error("RegSetValueExW", set_status);
        log::error(&format!("autostart enable failed: {err}"));
        return Err(err);
    }

    log::info(&format!("autostart enabled: {command}"));
    Ok(())
}

fn disable_autostart() -> Result<()> {
    let subkey = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);

    let mut key: HKEY = 0;
    // SAFETY: `subkey` is NUL-terminated UTF-16; `key` is a valid out-pointer.
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if open_status != ERROR_SUCCESS as i32 {
        if open_status == ERROR_FILE_NOT_FOUND as i32 {
            log::info("autostart already disabled (Run key missing)");
            return Ok(());
        }
        let err = registry_error("RegOpenKeyExW", open_status);
        log::error(&format!("autostart disable failed: {err}"));
        return Err(err);
    }

    // SAFETY: `key` is open; `value_name` is valid UTF-16.
    let delete_status = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };

    // SAFETY: `key` was opened successfully.
    let _ = unsafe { RegCloseKey(key) };

    if delete_status != ERROR_SUCCESS as i32 && delete_status != ERROR_FILE_NOT_FOUND as i32 {
        let err = registry_error("RegDeleteValueW", delete_status);
        log::error(&format!("autostart disable failed: {err}"));
        return Err(err);
    }

    log::info("autostart disabled");
    Ok(())
}

/// Build the Run-key command line: quoted exe path plus `--minimized`.
pub fn autostart_command(exe_path: &str) -> String {
    format!("\"{exe_path}\" --minimized")
}

/// Apply `config.autostart` to the Run key at startup.
pub fn sync_autostart_from_config(enabled: bool, exe_path: &Path) {
    match set_autostart(enabled, exe_path) {
        Ok(()) => {}
        Err(err) => log::warn(&format!("autostart registry sync failed: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_command_quotes_path_and_adds_minimized_flag() {
        assert_eq!(
            autostart_command(r"C:\Program Files\TrayVault\trayvault.exe"),
            r#""C:\Program Files\TrayVault\trayvault.exe" --minimized"#
        );
    }

    #[test]
    fn autostart_command_handles_simple_path() {
        assert_eq!(
            autostart_command(r"C:\trayvault.exe"),
            r#""C:\trayvault.exe" --minimized"#
        );
    }
}
