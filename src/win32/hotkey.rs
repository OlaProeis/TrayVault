//! Global hotkey registration and string parsing.
//!
//! Parses config strings like `"Alt+V"` into modifier + virtual-key
//! combinations and registers them with `RegisterHotKey` on the main window.

use crate::error::{ClipError, Result};
use crate::log;
use crate::win32::ffi::{
    RegisterHotKey, UnregisterHotKey, ERROR_HOTKEY_ALREADY_REGISTERED, HWND, INT, MOD_ALT,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, UINT, VK_BACK, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_F1, VK_F12, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SPACE, VK_TAB, VK_UP,
};
use crate::win32::last_error;

/// `RegisterHotKey` identifier for TrayVault's toggle hotkey.
pub const TRAYVAULT_HOTKEY_ID: INT = 1;

/// Parsed modifier + virtual-key combination for a global hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u32,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            ctrl: false,
            alt: true,
            shift: false,
            win: false,
            vk: letter_vk(b'V'),
        }
    }
}

impl Hotkey {
    /// Bitmask for `RegisterHotKey` (`fsModifiers`), including `MOD_NOREPEAT`.
    pub fn modifiers(&self) -> UINT {
        let mut flags = MOD_NOREPEAT;
        if self.alt {
            flags |= MOD_ALT;
        }
        if self.ctrl {
            flags |= MOD_CONTROL;
        }
        if self.shift {
            flags |= MOD_SHIFT;
        }
        if self.win {
            flags |= MOD_WIN;
        }
        flags
    }

    /// Canonical display form (e.g. `"Alt+V"`).
    pub fn display_string(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.win {
            parts.push("Win");
        }
        let key = key_token(self.vk);
        parts.push(&key);
        parts.join("+")
    }
}

/// Register `hotkey` on `hwnd`. Returns [`ClipError::HotkeyConflict`] when taken.
pub fn register_hotkey(hwnd: HWND, hotkey: &Hotkey) -> Result<()> {
    // SAFETY: `hwnd` is the live main window; `vk` is a valid virtual-key code.
    let ok = unsafe { RegisterHotKey(hwnd, TRAYVAULT_HOTKEY_ID, hotkey.modifiers(), hotkey.vk) };
    if ok != 0 {
        return Ok(());
    }

    let err = last_error("RegisterHotKey");
    if let ClipError::Win32 { code, .. } = &err {
        if *code == ERROR_HOTKEY_ALREADY_REGISTERED {
            return Err(ClipError::HotkeyConflict {
                hotkey: hotkey.display_string(),
            });
        }
    }
    Err(err)
}

/// Unregister TrayVault's global hotkey from `hwnd` (no-op if not registered).
pub fn unregister_hotkey(hwnd: HWND) -> Result<()> {
    // SAFETY: `hwnd` is the live main window.
    let ok = unsafe { UnregisterHotKey(hwnd, TRAYVAULT_HOTKEY_ID) };
    if ok != 0 {
        Ok(())
    } else {
        Err(last_error("UnregisterHotKey"))
    }
}

/// Tracks whether a global hotkey is currently registered.
pub struct HotkeyHandle {
    hotkey: Hotkey,
    registered: bool,
}

#[allow(dead_code)] // `hotkey` / `reregister` retained for non-settings callers
impl HotkeyHandle {
    pub fn new() -> Self {
        Self {
            hotkey: Hotkey::default(),
            registered: false,
        }
    }

    pub fn is_registered(&self) -> bool {
        self.registered
    }

    pub fn hotkey(&self) -> &Hotkey {
        &self.hotkey
    }

    /// Parse `config_str`, register on `hwnd`, or return an error without changing state.
    pub fn try_register(&mut self, hwnd: HWND, config_str: &str) -> Result<()> {
        let parsed = parse_hotkey_or_default(config_str);
        register_hotkey(hwnd, &parsed)?;
        self.hotkey = parsed;
        self.registered = true;
        log::info(&format!(
            "global hotkey registered: {}",
            self.hotkey.display_string()
        ));
        Ok(())
    }

    /// Parse `config_str` strictly (no fallback), register on `hwnd`.
    pub fn try_register_strict(&mut self, hwnd: HWND, config_str: &str) -> Result<()> {
        let parsed = parse_hotkey(config_str.trim())
            .ok_or_else(|| ClipError::Config(format!("Invalid hotkey syntax: {config_str}")))?;
        register_hotkey(hwnd, &parsed)?;
        self.hotkey = parsed;
        self.registered = true;
        log::info(&format!(
            "global hotkey registered: {}",
            self.hotkey.display_string()
        ));
        Ok(())
    }

    /// Unregister the previous hotkey (if any) and register `config_str`.
    pub fn reregister(&mut self, hwnd: HWND, config_str: &str) -> Result<()> {
        self.unregister(hwnd);
        self.try_register(hwnd, config_str)
    }

    /// Unregister the previous hotkey (if any) and register `config_str` strictly.
    pub fn reregister_strict(&mut self, hwnd: HWND, config_str: &str) -> Result<()> {
        self.unregister(hwnd);
        self.try_register_strict(hwnd, config_str)
    }

    /// Unregister from `hwnd` if registered; ignores unregister failures.
    pub fn unregister(&mut self, hwnd: HWND) {
        if !self.registered {
            return;
        }
        if let Err(err) = unregister_hotkey(hwnd) {
            log::warn(&format!("UnregisterHotKey failed: {err}"));
        }
        self.registered = false;
    }
}

impl Default for HotkeyHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a hotkey string like `"Alt+V"`. Returns `None` on invalid input.
pub fn parse_hotkey(s: &str) -> Option<Hotkey> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut win = false;
    let mut vk: Option<u32> = None;

    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        if apply_modifier(part, &mut ctrl, &mut alt, &mut shift, &mut win) {
            continue;
        }
        let code = parse_key_token(part)?;
        if vk.is_some() {
            return None;
        }
        vk = Some(code);
    }

    vk.map(|code| Hotkey {
        ctrl,
        alt,
        shift,
        win,
        vk: code,
    })
}

/// Parse `s`, logging a warning and returning the default hotkey on failure.
pub fn parse_hotkey_or_default(s: &str) -> Hotkey {
    match parse_hotkey(s) {
        Some(hk) => hk,
        None => {
            log::warn(&format!(
                "invalid hotkey `{s}`; using default {}",
                Hotkey::default().display_string()
            ));
            Hotkey::default()
        }
    }
}

fn apply_modifier(
    part: &str,
    ctrl: &mut bool,
    alt: &mut bool,
    shift: &mut bool,
    win: &mut bool,
) -> bool {
    match part.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => {
            *ctrl = true;
            true
        }
        "alt" => {
            *alt = true;
            true
        }
        "shift" => {
            *shift = true;
            true
        }
        "win" | "super" | "windows" => {
            *win = true;
            true
        }
        _ => false,
    }
}

fn parse_key_token(part: &str) -> Option<u32> {
    if part.len() == 1 {
        let ch = part.chars().next()?;
        if ch.is_ascii_alphabetic() {
            return Some(letter_vk(ch.to_ascii_uppercase() as u8));
        }
        if ch.is_ascii_digit() {
            return Some(digit_vk(ch as u8));
        }
        return None;
    }

    match part.to_ascii_lowercase().as_str() {
        "space" => Some(VK_SPACE as u32),
        "tab" => Some(VK_TAB as u32),
        "enter" | "return" => Some(VK_RETURN as u32),
        "esc" | "escape" => Some(VK_ESCAPE as u32),
        "backspace" | "back" => Some(VK_BACK as u32),
        "delete" | "del" => Some(VK_DELETE as u32),
        "insert" | "ins" => Some(VK_INSERT as u32),
        "home" => Some(VK_HOME as u32),
        "end" => Some(VK_END as u32),
        "pageup" | "pgup" | "prior" => Some(VK_PRIOR as u32),
        "pagedown" | "pgdn" | "next" => Some(VK_NEXT as u32),
        "up" => Some(VK_UP as u32),
        "down" => Some(VK_DOWN as u32),
        "left" => Some(VK_LEFT as u32),
        "right" => Some(VK_RIGHT as u32),
        name if name.len() >= 2 && name.starts_with('f') => parse_function_key(name),
        _ => None,
    }
}

fn parse_function_key(name: &str) -> Option<u32> {
    let num: u32 = name[1..].parse().ok()?;
    if !(1..=12).contains(&num) {
        return None;
    }
    Some((VK_F1 as u32) + (num - 1))
}

fn letter_vk(upper: u8) -> u32 {
    debug_assert!(upper.is_ascii_uppercase());
    upper as u32
}

fn digit_vk(digit: u8) -> u32 {
    debug_assert!(digit.is_ascii_digit());
    digit as u32
}

fn key_token(vk: u32) -> String {
    if (vk as u8).is_ascii_uppercase() {
        return ((vk as u8) as char).to_string();
    }
    if (vk as u8).is_ascii_digit() {
        return ((vk as u8) as char).to_string();
    }
    if vk >= VK_F1 as u32 && vk <= VK_F12 as u32 {
        return format!("F{}", vk - VK_F1 as u32 + 1);
    }
    match vk as i32 {
        VK_SPACE => "Space".into(),
        VK_TAB => "Tab".into(),
        VK_RETURN => "Enter".into(),
        VK_ESCAPE => "Escape".into(),
        VK_BACK => "Backspace".into(),
        VK_DELETE => "Delete".into(),
        VK_INSERT => "Insert".into(),
        VK_HOME => "Home".into(),
        VK_END => "End".into(),
        VK_PRIOR => "PageUp".into(),
        VK_NEXT => "PageDown".into(),
        VK_UP => "Up".into(),
        VK_DOWN => "Down".into(),
        VK_LEFT => "Left".into(),
        VK_RIGHT => "Right".into(),
        _ => format!("0x{vk:X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_hotkey_string() {
        let hk = parse_hotkey("Alt+V").unwrap();
        assert_eq!(hk, Hotkey::default());
    }

    #[test]
    fn parse_case_insensitive_with_spaces() {
        let hk = parse_hotkey(" alt + v ").unwrap();
        assert_eq!(hk, Hotkey::default());
    }

    #[test]
    fn parse_alt_letter() {
        let hk = parse_hotkey("Alt+V").unwrap();
        assert!(hk.alt);
        assert!(!hk.ctrl);
        assert_eq!(hk.vk, letter_vk(b'V'));
    }

    #[test]
    fn parse_win_function_key() {
        let hk = parse_hotkey("Win+F12").unwrap();
        assert!(hk.win);
        assert_eq!(hk.vk, VK_F12 as u32);
    }

    #[test]
    fn parse_named_key() {
        let hk = parse_hotkey("Ctrl+Space").unwrap();
        assert!(hk.ctrl);
        assert_eq!(hk.vk, VK_SPACE as u32);
    }

    #[test]
    fn parse_digit_key() {
        let hk = parse_hotkey("Ctrl+1").unwrap();
        assert_eq!(hk.vk, b'1' as u32);
    }

    #[test]
    fn parse_rejects_missing_key() {
        assert!(parse_hotkey("Ctrl+Shift").is_none());
    }

    #[test]
    fn parse_rejects_unknown_token() {
        assert!(parse_hotkey("Ctrl+Foo+V").is_none());
    }

    #[test]
    fn parse_rejects_multiple_keys() {
        assert!(parse_hotkey("Ctrl+V+B").is_none());
    }

    #[test]
    fn parse_rejects_empty_string() {
        assert!(parse_hotkey("").is_none());
    }

    #[test]
    fn display_string_round_trip() {
        let original = Hotkey::default();
        assert_eq!(
            original.display_string(),
            parse_hotkey("Alt+V").unwrap().display_string()
        );
    }

    #[test]
    fn modifiers_include_norepeat() {
        let hk = Hotkey::default();
        assert_ne!(hk.modifiers() & MOD_NOREPEAT, 0);
        assert_eq!(hk.modifiers() & MOD_CONTROL, 0);
        assert_ne!(hk.modifiers() & MOD_ALT, 0);
    }
}
