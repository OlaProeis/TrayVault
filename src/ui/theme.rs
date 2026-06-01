//! Light/dark theme palettes and resolution from config + Windows registry.

use crate::config::ThemeMode;
use crate::error::Result;
use crate::win32::{ffi, last_error, wide};

use ffi::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, ERROR_SUCCESS, HKEY, HKEY_CURRENT_USER, KEY_READ,
    REG_DWORD,
};

/// Resolved RGBA palette for rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub background: [u8; 4],
    pub card: [u8; 4],
    pub accent: [u8; 4],
    pub text_primary: [u8; 4],
    pub text_secondary: [u8; 4],
    pub divider: [u8; 4],
    pub pinned: [u8; 4],
    pub selection: [u8; 4],
    /// Drop-shadow color for floating surfaces (context menu, modal).
    pub shadow: [u8; 4],
    /// Destructive-action text color (e.g. "Delete" in context menus).
    pub danger: [u8; 4],
}

impl Theme {
    pub fn light() -> Self {
        Self {
            background: [0xF4, 0xF4, 0xF5, 0xFF],
            card: [0xFF, 0xFF, 0xFF, 0xFF],
            accent: [0x4A, 0x72, 0xF5, 0xFF],
            text_primary: [0x18, 0x18, 0x1B, 0xFF],
            text_secondary: [0x71, 0x71, 0x7A, 0xFF],
            divider: [0xE4, 0xE4, 0xE7, 0xFF],
            pinned: [0xF5, 0x9E, 0x0B, 0xFF],
            selection: [0xEC, 0xEC, 0xF2, 0xFF],
            shadow: [0xC4, 0xC4, 0xCC, 0xFF],
            danger: [0xDC, 0x26, 0x26, 0xFF],
        }
    }

    pub fn dark() -> Self {
        Self {
            background: [0x18, 0x18, 0x1B, 0xFF],
            card: [0x27, 0x27, 0x2A, 0xFF],
            accent: [0x5B, 0x8A, 0xF5, 0xFF],
            text_primary: [0xF4, 0xF4, 0xF5, 0xFF],
            text_secondary: [0x8B, 0x8F, 0xA8, 0xFF],
            divider: [0x3F, 0x3F, 0x46, 0xFF],
            pinned: [0xFB, 0xBF, 0x24, 0xFF],
            selection: [0x2C, 0x2C, 0x38, 0xFF],
            shadow: [0x09, 0x09, 0x0D, 0xFF],
            danger: [0xF8, 0x71, 0x71, 0xFF],
        }
    }
}

/// Pick the active palette from config, optionally reading the Windows theme.
pub fn resolve_theme(mode: ThemeMode) -> Theme {
    resolve_theme_with_system(mode, read_apps_use_light_theme().ok())
}

/// Testable theme resolution with an injected system preference.
pub fn resolve_theme_with_system(mode: ThemeMode, system_light: Option<bool>) -> Theme {
    match mode {
        ThemeMode::Light => Theme::light(),
        ThemeMode::Dark => Theme::dark(),
        ThemeMode::System => match system_light {
            Some(true) => Theme::light(),
            Some(false) => Theme::dark(),
            None => Theme::dark(),
        },
    }
}

/// Read `HKCU\...\Personalize\AppsUseLightTheme` (`1` = light, `0` = dark).
pub fn read_apps_use_light_theme() -> Result<bool> {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value_name = wide("AppsUseLightTheme");

    let mut key: HKEY = 0;
    // SAFETY: `subkey` is NUL-terminated UTF-16; `key` is a valid out-pointer.
    let open_status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut key) };
    if open_status != ERROR_SUCCESS as i32 {
        return Err(last_error("RegOpenKeyExW"));
    }

    let mut value_type = 0u32;
    let mut data: u32 = 0;
    let mut data_len = std::mem::size_of::<u32>() as u32;

    // SAFETY: `key` is open; `value_name` is valid UTF-16; `data` holds a DWORD.
    let query_status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            (&mut data as *mut u32).cast(),
            &mut data_len,
        )
    };

    // SAFETY: `key` was opened successfully.
    let _ = unsafe { RegCloseKey(key) };

    if query_status != ERROR_SUCCESS as i32 {
        return Err(last_error("RegQueryValueExW"));
    }
    if value_type != REG_DWORD || data_len != std::mem::size_of::<u32>() as u32 {
        return Err(crate::error::ClipError::Other(
            "AppsUseLightTheme: expected REG_DWORD".into(),
        ));
    }

    Ok(data != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeMode;

    #[test]
    fn resolve_theme_forced_modes_ignore_system() {
        assert_eq!(
            resolve_theme_with_system(ThemeMode::Light, Some(false)),
            Theme::light()
        );
        assert_eq!(
            resolve_theme_with_system(ThemeMode::Dark, Some(true)),
            Theme::dark()
        );
    }

    #[test]
    fn resolve_theme_system_follows_registry_value() {
        assert_eq!(
            resolve_theme_with_system(ThemeMode::System, Some(true)),
            Theme::light()
        );
        assert_eq!(
            resolve_theme_with_system(ThemeMode::System, Some(false)),
            Theme::dark()
        );
        assert_eq!(
            resolve_theme_with_system(ThemeMode::System, None),
            Theme::dark()
        );
    }

    #[test]
    fn light_and_dark_palettes_differ() {
        assert_ne!(Theme::light().background, Theme::dark().background);
        assert_eq!(Theme::light().accent, [0x4A, 0x72, 0xF5, 0xFF]);
        assert_eq!(Theme::dark().accent, [0x5B, 0x8A, 0xF5, 0xFF]);
    }
}
