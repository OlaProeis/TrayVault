//! System tray icon, context menu, and balloon notifications.
//!
//! Uses `Shell_NotifyIconW` with a custom callback message routed through
//! [`crate::win32::window::WM_TRAY_CALLBACK`].

#![allow(dead_code)] // balloon API used by Task 11 (hotkey conflict notifications)

use std::cell::Cell;
use std::mem;
use std::ptr;
use std::sync::OnceLock;

use crate::error::{ClipError, Result};
use crate::log;
use crate::win32::ffi;
use crate::win32::window::WM_TRAY_CALLBACK;
use crate::win32::{last_error, wide};

use ffi::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, DestroyMenu, GetCursorPos,
    GetTickCount, LoadIconW, PostMessageW, SetForegroundWindow, Shell_NotifyIconW, TrackPopupMenu,
    DWORD, HICON, HINSTANCE, HWND, IDI_APPLICATION, LPARAM, LR_DEFAULTSIZE, MF_CHECKED, MF_ENABLED,
    MF_SEPARATOR, MF_STRING, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_KEYSELECT, NIN_SELECT, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4, POINT, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_CONTEXTMENU,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WPARAM,
};

/// Tray context-menu command ids (low word of `WM_COMMAND` wParam).
pub const IDM_TRAY_SHOW: usize = 1001;
pub const IDM_TRAY_PAUSE: usize = 1002;
pub const IDM_TRAY_SETTINGS: usize = 1003;
pub const IDM_TRAY_QUIT: usize = 1004;

const TRAY_ICON_ID: u32 = 1;
const TRAY_TIP: &str = "TrayVault";
/// Collapse duplicate toggle notifications for one physical click (`WM_LBUTTONUP` +
/// `NIN_SELECT`, or click + double-click) when using [`NOTIFYICON_VERSION_4`].
const TRAY_TOGGLE_DEBOUNCE_MS: u32 = 400;

/// Embedded fallback icon shipped with the repo (`assets/icon.ico`).
const EMBEDDED_ICON: &[u8] = include_bytes!("../../assets/icon.ico");

/// System tray icon lifecycle (`NIM_ADD` / `NIM_MODIFY` / `NIM_DELETE`).
pub struct TrayIcon {
    hwnd: HWND,
    icon: HICON,
    added: bool,
    last_toggle_tick: Cell<u32>,
}

impl TrayIcon {
    /// Create the tray icon and register it with the shell.
    pub fn new(hwnd: HWND, _hinstance: HINSTANCE) -> Result<Self> {
        let icon = load_tray_icon()?;
        let mut tray = Self {
            hwnd,
            icon,
            added: false,
            last_toggle_tick: Cell::new(0),
        };
        tray.add()?;
        Ok(tray)
    }

    fn add(&mut self) -> Result<()> {
        let mut data = notify_data(self.hwnd, self.icon);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        set_wide_field(&mut data.szTip, TRAY_TIP);

        // SAFETY: `data` matches `NOTIFYICONDATAW` layout; `hwnd` is valid.
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &mut data) };
        if ok == 0 {
            return Err(last_error("Shell_NotifyIconW(NIM_ADD)"));
        }

        data.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: upgrade to extended balloon-capable version.
        let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &mut data) };

        self.added = true;
        log::info("tray icon added");
        Ok(())
    }

    /// Remove the icon from the notification area (safe to call multiple times).
    pub fn remove(&mut self) {
        if !self.added {
            return;
        }
        let mut data = notify_data(self.hwnd, self.icon);
        // SAFETY: `NIM_DELETE` only needs hwnd/uID/cbSize.
        let ok = unsafe { Shell_NotifyIconW(NIM_DELETE, &mut data) };
        if ok == 0 {
            log::warn("Shell_NotifyIconW(NIM_DELETE) failed");
        } else {
            log::info("tray icon removed");
        }
        self.added = false;
    }

    /// Show a one-shot balloon notification (e.g. hotkey registration failure).
    pub fn show_balloon(&self, title: &str, message: &str) -> Result<()> {
        let mut data = notify_data(self.hwnd, self.icon);
        data.uFlags = NIF_INFO | NIF_TIP | NIF_ICON;
        set_wide_field(&mut data.szTip, TRAY_TIP);
        set_wide_field64(&mut data.szInfoTitle, title);
        set_wide_field256(&mut data.szInfo, message);
        data.dwInfoFlags = NIIF_INFO;

        // SAFETY: balloon fields populated; `hwnd`/`hIcon` valid.
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut data) };
        if ok == 0 {
            return Err(last_error("Shell_NotifyIconW(NIM_MODIFY balloon)"));
        }
        Ok(())
    }

    /// Update the tooltip (e.g. when capture is paused).
    pub fn set_tooltip(&self, tip: &str) -> Result<()> {
        let mut data = notify_data(self.hwnd, self.icon);
        data.uFlags = NIF_TIP | NIF_ICON;
        set_wide_field(&mut data.szTip, tip);

        // SAFETY: tooltip-only modify.
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut data) };
        if ok == 0 {
            return Err(last_error("Shell_NotifyIconW(NIM_MODIFY tip)"));
        }
        Ok(())
    }

    /// Handle a tray callback (`WM_TRAY_CALLBACK`) event.
    ///
    /// With [`NOTIFYICON_VERSION_4`], the notification kind is in `LOWORD(lParam)` and
    /// anchor coordinates (for menu placement) are in `wParam` via `GET_X/Y_LPARAM`.
    ///
    /// Left-click toggle notifications are debounced (400 ms) — see `docs/technical/system-tray.md`.
    pub fn handle_callback(
        &self,
        wparam: WPARAM,
        lparam: LPARAM,
        pause_capture: bool,
    ) -> Option<TrayMenuAction> {
        let event = loword(lparam as u32) as u32;
        match event {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => {
                if self.take_toggle_if_due() {
                    Some(TrayMenuAction::ToggleWindow)
                } else {
                    None
                }
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                let (x, y) = tray_menu_coords(wparam, lparam);
                show_context_menu(self.hwnd, x, y, pause_capture);
                None
            }
            _ => None,
        }
    }

    fn take_toggle_if_due(&self) -> bool {
        // SAFETY: `GetTickCount` is a documented kernel32 export.
        let now = unsafe { GetTickCount() };
        let last = self.last_toggle_tick.get();
        if !should_accept_toggle(now, last, TRAY_TOGGLE_DEBOUNCE_MS) {
            return false;
        }
        self.last_toggle_tick.set(now);
        true
    }

    /// Map a tray menu command id to an action.
    pub fn menu_action(command_id: u32) -> Option<TrayMenuAction> {
        match command_id as usize {
            IDM_TRAY_SHOW => Some(TrayMenuAction::ShowWindow),
            IDM_TRAY_PAUSE => Some(TrayMenuAction::TogglePause),
            IDM_TRAY_SETTINGS => Some(TrayMenuAction::Settings),
            IDM_TRAY_QUIT => Some(TrayMenuAction::Quit),
            _ => None,
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
        // Icon handle is process-lifetime (shared with the window class); not destroyed here.
        self.icon = 0;
    }
}

/// Actions triggered from the tray icon or its context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayMenuAction {
    ToggleWindow,
    ShowWindow,
    TogglePause,
    Settings,
    Quit,
}

/// Show a one-shot tray balloon (e.g. hotkey registration failure in Task 11).
pub fn show_tray_notification(tray: &TrayIcon, title: &str, message: &str) {
    if let Err(err) = tray.show_balloon(title, message) {
        log::warn(&format!("tray balloon failed: {err}"));
    }
}

fn notify_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: mem::size_of::<NOTIFYICONDATAW>() as DWORD,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: icon,
        ..unsafe { mem::zeroed() }
    }
}

fn set_wide_field(buf: &mut [u16; 128], text: &str) {
    copy_wide_truncated(buf, text);
}

fn set_wide_field64(buf: &mut [u16; 64], text: &str) {
    let wide_text = wide(text);
    let len = wide_text.len().saturating_sub(1).min(buf.len());
    buf[..len].copy_from_slice(&wide_text[..len]);
    if len < buf.len() {
        buf[len] = 0;
    }
}

fn set_wide_field256(buf: &mut [u16; 256], text: &str) {
    let wide_text = wide(text);
    let len = wide_text.len().saturating_sub(1).min(buf.len());
    buf[..len].copy_from_slice(&wide_text[..len]);
    if len < buf.len() {
        buf[len] = 0;
    }
}

fn copy_wide_truncated(buf: &mut [u16; 128], text: &str) {
    let wide_text = wide(text);
    let len = wide_text.len().saturating_sub(1).min(buf.len());
    buf[..len].copy_from_slice(&wide_text[..len]);
    if len < buf.len() {
        buf[len] = 0;
    }
}

fn show_context_menu(hwnd: HWND, x: i32, y: i32, pause_capture: bool) {
    // SAFETY: standard popup menu creation.
    let menu = unsafe { CreatePopupMenu() };
    if menu == 0 {
        log::warn("CreatePopupMenu failed for tray menu");
        return;
    }

    let show_label = wide("Open");
    let pause_label = wide(if pause_capture {
        "Resume capture"
    } else {
        "Pause capture"
    });
    let settings_label = wide("Settings");
    let quit_label = wide("Quit");

    // SAFETY: `menu` is valid; labels are NUL-terminated UTF-16.
    unsafe {
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_ENABLED,
            IDM_TRAY_SHOW,
            show_label.as_ptr(),
        );
        let pause_flags = if pause_capture {
            MF_STRING | MF_CHECKED | MF_ENABLED
        } else {
            MF_STRING | MF_ENABLED
        };
        let _ = AppendMenuW(menu, pause_flags, IDM_TRAY_PAUSE, pause_label.as_ptr());
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_ENABLED,
            IDM_TRAY_SETTINGS,
            settings_label.as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_ENABLED,
            IDM_TRAY_QUIT,
            quit_label.as_ptr(),
        );

        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            x,
            y,
            0,
            hwnd,
            ptr::null(),
        );
        // Required so the menu dismisses when clicking elsewhere.
        let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
        let _ = DestroyMenu(menu);
    }
}

/// Screen coordinates for tray context-menu placement.
///
/// With [`NOTIFYICON_VERSION_4`], `LOWORD(lParam)` is the notification kind. For
/// mouse-driven context menus (`WM_CONTEXTMENU` / `WM_RBUTTONUP`), `wParam` is
/// often **not** reliable screen coordinates (it may be the icon id), which
/// places the menu on the wrong monitor. The cursor is at the tray icon when the
/// user right-clicks, so [`GetCursorPos`] is used for those events. Keyboard
/// activation (`NIN_KEYSELECT`) supplies a valid anchor in `wParam`.
fn tray_menu_coords(wparam: WPARAM, lparam: LPARAM) -> (i32, i32) {
    let event = loword(lparam as u32) as u32;
    match event {
        NIN_KEYSELECT => (get_x_lparam(wparam), get_y_lparam(wparam)),
        WM_CONTEXTMENU | WM_RBUTTONUP => {
            cursor_pos().unwrap_or_else(|| (get_x_lparam(wparam), get_y_lparam(wparam)))
        }
        _ => cursor_pos().unwrap_or((0, 0)),
    }
}

fn cursor_pos() -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    // SAFETY: `pt` is valid stack storage.
    let ok = unsafe { GetCursorPos(&mut pt) };
    if ok == 0 {
        None
    } else {
        Some((pt.x, pt.y))
    }
}

/// Returns whether a toggle notification should run (debounces duplicate shell events).
fn should_accept_toggle(now: u32, last: u32, debounce_ms: u32) -> bool {
    last == 0 || now.wrapping_sub(last) >= debounce_ms
}

fn is_toggle_event(event: u32) -> bool {
    matches!(
        event,
        WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT
    )
}

fn loword(v: u32) -> u16 {
    (v & 0xFFFF) as u16
}

fn get_x_lparam(l: WPARAM) -> i32 {
    let low = (l as u32) & 0xFFFF;
    low as i16 as i32
}

fn get_y_lparam(l: WPARAM) -> i32 {
    let high = ((l as u32) >> 16) & 0xFFFF;
    high as i16 as i32
}

fn load_tray_icon() -> Result<HICON> {
    load_app_icon()
}

/// Process-lifetime application icon (tray, window class, `WM_SETICON`, `WM_GETICON`).
static APP_ICON: OnceLock<HICON> = OnceLock::new();

/// Load the embedded application icon from `assets/icon.ico`.
///
/// Used by the system tray, window class registration, and `WM_SETICON` /
/// `WM_GETICON`. Returns the same handle on every call. Falls back to the stock
/// application icon if parsing fails.
pub fn load_app_icon() -> Result<HICON> {
    if let Some(&icon) = APP_ICON.get() {
        return if icon != 0 {
            Ok(icon)
        } else {
            Err(ClipError::Other("application icon unavailable".into()))
        };
    }
    let icon = match load_icon_from_ico_bytes(EMBEDDED_ICON) {
        Ok(h) => h,
        Err(e) => {
            log::warn(&format!(
                "failed to load embedded app icon ({e}); using stock application icon"
            ));
            load_stock_icon()?
        }
    };
    let _ = APP_ICON.set(icon);
    Ok(icon)
}

fn load_icon_from_ico_bytes(bytes: &[u8]) -> Result<HICON> {
    if bytes.len() < 6 {
        return Err(ClipError::Other("embedded icon too small".into()));
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 {
        return Err(ClipError::Other("embedded icon has no images".into()));
    }

    // Pick the largest entry (best tray appearance).
    let mut best_idx = 0usize;
    let mut best_area = 0u32;
    for i in 0..count {
        let entry_off = 6 + i * 16;
        if entry_off + 16 > bytes.len() {
            return Err(ClipError::Other("embedded icon entry truncated".into()));
        }
        let entry = &bytes[entry_off..entry_off + 16];
        let w = entry[0] as u32;
        let h = entry[1] as u32;
        let width = if w == 0 { 256 } else { w };
        let height = if h == 0 { 256 } else { h };
        let area = width * height;
        if area >= best_area {
            best_area = area;
            best_idx = i;
        }
    }

    let entry_off = 6 + best_idx * 16;
    let entry = &bytes[entry_off..entry_off + 16];
    let image_size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
    let image_offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
    if image_offset + image_size > bytes.len() {
        return Err(ClipError::Other("embedded icon image truncated".into()));
    }

    let image_bytes = &bytes[image_offset..image_offset + image_size];
    // SAFETY: `CreateIconFromResourceEx` reads `image_bytes` for icon creation.
    let icon = unsafe {
        CreateIconFromResourceEx(
            image_bytes.as_ptr() as *mut u8,
            image_size as DWORD,
            1,
            0x0003_0000,
            0,
            0,
            LR_DEFAULTSIZE,
        )
    };
    if icon == 0 {
        return Err(last_error("CreateIconFromResourceEx"));
    }
    Ok(icon)
}

fn load_stock_icon() -> Result<HICON> {
    // SAFETY: `IDI_APPLICATION` is a documented stock icon pseudo-handle.
    let icon = unsafe { LoadIconW(0, IDI_APPLICATION) };
    if icon == 0 {
        return Err(last_error("LoadIconW(IDI_APPLICATION)"));
    }
    Ok(icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_toggle_first_event() {
        assert!(should_accept_toggle(1000, 0, TRAY_TOGGLE_DEBOUNCE_MS));
    }

    #[test]
    fn should_accept_toggle_after_debounce() {
        assert!(!should_accept_toggle(1000, 900, TRAY_TOGGLE_DEBOUNCE_MS));
        assert!(should_accept_toggle(1301, 900, TRAY_TOGGLE_DEBOUNCE_MS));
    }

    #[test]
    fn should_accept_toggle_wraps_tick_count() {
        assert!(!should_accept_toggle(
            100,
            u32::MAX - 100,
            TRAY_TOGGLE_DEBOUNCE_MS
        ));
    }

    #[test]
    fn toggle_events_include_v4_and_legacy_messages() {
        assert!(is_toggle_event(NIN_SELECT));
        assert!(is_toggle_event(NIN_KEYSELECT));
        assert!(is_toggle_event(WM_LBUTTONUP));
        assert!(is_toggle_event(WM_LBUTTONDBLCLK));
        assert!(!is_toggle_event(WM_RBUTTONUP));
    }
}
