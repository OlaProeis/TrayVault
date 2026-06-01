//! Settings panel input: toggles, text fields, theme chips, scroll.

use crate::app::App;
use crate::config::ThemeMode;
use crate::log;
use crate::ui::search_edit::{caret_at_click, has_selection, selection_range};
use crate::ui::settings::{settings_content_height, GITHUB_REPO_URL};
use crate::ui::text::GlyphCache;
use crate::ui::titlebar;
use crate::ui::widgets::WidgetRect;
use crate::ui::{format_max_image_edit, SettingsFocus, UiState};
use crate::win32::clipboard::ClipboardMonitor;
use crate::win32::ffi::{HWND, VK_BACK, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RIGHT};
use crate::win32::hotkey::HotkeyHandle;
use crate::win32::shell;
use crate::win32::tray::TrayIcon;

const HEADER_H: f32 = 48.0;
const FOOTER_H: f32 = 36.0;
const WHEEL_DELTA: f32 = 120.0;

/// Tray + hotkey hooks needed for settings side effects.
pub struct SettingsHooks {
    pub hwnd: HWND,
    pub hotkey: *mut HotkeyHandle,
    pub tray: *mut TrayIcon,
}

impl SettingsHooks {
    pub fn hotkey_ptr(&self) -> *mut HotkeyHandle {
        self.hotkey
    }

    pub fn tray_ptr(&self) -> *mut TrayIcon {
        self.tray
    }
}

pub fn handle_settings_wheel(ui: &mut UiState, delta: i32, viewport_height: i32) -> bool {
    let viewport_h =
        (viewport_height.max(0) as f32 - titlebar::TITLE_BAR_HEIGHT - HEADER_H - FOOTER_H).max(0.0);
    let max_scroll = (settings_content_height() - viewport_h).max(0.0);
    let scroll_amount = -(delta as f32) / WHEEL_DELTA * 36.0;
    ui.settings_scroll = (ui.settings_scroll + scroll_amount).clamp(0.0, max_scroll);
    true
}

/// Commit pending fields and close the settings overlay.
pub fn close_settings(
    app: &mut App,
    ui: &mut UiState,
    hooks: &SettingsHooks,
    monitor: &mut ClipboardMonitor,
) {
    let _ = commit_text_field(app, ui, hooks, monitor);
    ui.show_settings = false;
    ui.settings_focus = SettingsFocus::None;
    ui.settings_caret = 0;
    ui.settings_sel_anchor = 0;
    ui.settings_error = None;
}

pub fn handle_settings_mouse_up(
    app: &mut App,
    ui: &mut UiState,
    cache: &mut GlyphCache,
    x: f32,
    y: f32,
    hooks: &SettingsHooks,
    monitor: &mut ClipboardMonitor,
) -> bool {
    let rects = ui.settings_rects.clone();
    let mut changed = false;

    if rect_clicked(rects.back, x, y) {
        close_settings(app, ui, hooks, monitor);
        return true;
    }

    if rect_clicked(rects.pause, x, y) {
        let paused = !app.pause_capture;
        if apply_pause(app, ui, hooks, monitor, paused) {
            changed = true;
        }
    } else if rect_clicked(rects.deduplicate, x, y) {
        let v = !app.config.deduplicate_global;
        if app
            .set_config_bool(|c| &mut c.deduplicate_global, v)
            .is_ok()
        {
            ui.settings_error = None;
            changed = true;
        }
    } else if rect_clicked(rects.capture_images, x, y) {
        let v = !app.config.capture_images;
        if app.set_config_bool(|c| &mut c.capture_images, v).is_ok() {
            monitor.set_config(app.config.capture_config());
            ui.settings_error = None;
            changed = true;
        }
    } else if rect_clicked(rects.capture_rich_text, x, y) {
        let v = !app.config.capture_rich_text;
        if app.set_config_bool(|c| &mut c.capture_rich_text, v).is_ok() {
            monitor.set_config(app.config.capture_config());
            ui.settings_error = None;
            changed = true;
        }
    } else if rect_clicked(rects.close_on_copy, x, y) {
        let v = !app.config.close_on_copy;
        if app.set_config_bool(|c| &mut c.close_on_copy, v).is_ok() {
            ui.settings_error = None;
            changed = true;
        }
    } else if rect_clicked(rects.show_in_taskbar, x, y) {
        let v = !app.config.show_in_taskbar;
        if app.set_config_bool(|c| &mut c.show_in_taskbar, v).is_ok() {
            if app.is_window_visible() {
                crate::win32::window::set_taskbar_button_visible(hooks.hwnd, v);
            }
            ui.settings_error = None;
            changed = true;
        }
    } else if rect_clicked(rects.autostart, x, y) {
        changed = toggle_autostart(app, ui);
    } else if rect_clicked(rects.theme_light, x, y) {
        changed = apply_theme(app, ui, ThemeMode::Light);
    } else if rect_clicked(rects.theme_dark, x, y) {
        changed = apply_theme(app, ui, ThemeMode::Dark);
    } else if rect_clicked(rects.theme_system, x, y) {
        changed = apply_theme(app, ui, ThemeMode::System);
    } else if rect_clicked(rects.max_entries, x, y) {
        commit_text_field(app, ui, hooks, monitor);
        focus_settings_field(ui, cache, SettingsFocus::MaxEntries, rects.max_entries, x);
        changed = true;
    } else if rect_clicked(rects.hotkey, x, y) {
        commit_text_field(app, ui, hooks, monitor);
        focus_settings_field(ui, cache, SettingsFocus::Hotkey, rects.hotkey, x);
        changed = true;
    } else if rect_clicked(rects.max_image_mb, x, y) {
        commit_text_field(app, ui, hooks, monitor);
        focus_settings_field(
            ui,
            cache,
            SettingsFocus::MaxImageSizeMb,
            rects.max_image_mb,
            x,
        );
        changed = true;
    } else if rect_clicked(rects.github, x, y) {
        if let Err(err) = shell::open_url(GITHUB_REPO_URL) {
            log::warn(&format!("open GitHub repo: {err}"));
            ui.settings_error = Some("Could not open the GitHub link in your browser.".into());
        } else {
            ui.settings_error = None;
        }
        changed = true;
    } else {
        if commit_text_field(app, ui, hooks, monitor) {
            changed = true;
        }
        ui.settings_focus = SettingsFocus::None;
        ui.settings_caret = 0;
        ui.settings_sel_anchor = 0;
    }

    changed
}

fn focus_settings_field(
    ui: &mut UiState,
    cache: &mut GlyphCache,
    focus: SettingsFocus,
    rect: Option<WidgetRect>,
    click_x: f32,
) {
    ui.settings_focus = focus;
    let text = ui.settings_field_text(focus);
    ui.settings_caret = rect.map_or(text.len(), |r| caret_at_click(cache, text, r, click_x));
    ui.settings_sel_anchor = ui.settings_caret;
}

pub fn handle_settings_key_down(
    vk: u32,
    ui: &mut UiState,
    shift_held: bool,
    control_held: bool,
) -> bool {
    if ui.settings_focus == SettingsFocus::None {
        return false;
    }

    match vk {
        v if v == VK_BACK as u32 => handle_settings_backspace(ui),
        v if v == VK_DELETE as u32 => handle_settings_delete(ui),
        v if v == VK_LEFT as u32 => {
            let focus = ui.settings_focus;
            let text = ui.settings_field_text(focus);
            ui.settings_caret = prev_char_boundary(text, ui.settings_caret);
            if !shift_held {
                ui.settings_sel_anchor = ui.settings_caret;
            }
            true
        }
        v if v == VK_RIGHT as u32 => {
            let focus = ui.settings_focus;
            let text = ui.settings_field_text(focus);
            ui.settings_caret = next_char_boundary(text, ui.settings_caret);
            if !shift_held {
                ui.settings_sel_anchor = ui.settings_caret;
            }
            true
        }
        v if v == VK_HOME as u32 => {
            ui.settings_caret = 0;
            if !shift_held {
                ui.settings_sel_anchor = 0;
            }
            true
        }
        v if v == VK_END as u32 => {
            let len = ui.settings_field_text(ui.settings_focus).len();
            ui.settings_caret = len;
            if !shift_held {
                ui.settings_sel_anchor = len;
            }
            true
        }
        _ => {
            if control_held && (vk == b'A' as u32 || vk == b'a' as u32) {
                ui.settings_sel_anchor = 0;
                ui.settings_caret = ui.settings_field_text(ui.settings_focus).len();
                return true;
            }
            false
        }
    }
}

pub fn handle_settings_backspace(ui: &mut UiState) -> bool {
    if ui.settings_focus == SettingsFocus::None {
        return false;
    }
    if delete_settings_selection(ui) {
        return true;
    }
    let focus = ui.settings_focus;
    let caret = ui.settings_caret;
    let start = prev_char_boundary(ui.settings_field_text(focus), caret);
    if start == caret {
        return false;
    }
    ui.settings_field_text_mut(focus).drain(start..caret);
    ui.settings_caret = start;
    ui.settings_sel_anchor = start;
    true
}

fn handle_settings_delete(ui: &mut UiState) -> bool {
    if ui.settings_focus == SettingsFocus::None {
        return false;
    }
    if delete_settings_selection(ui) {
        return true;
    }
    let focus = ui.settings_focus;
    let caret = ui.settings_caret;
    let text = ui.settings_field_text(focus);
    let end = next_char_boundary(text, caret);
    if end <= caret {
        return false;
    }
    ui.settings_field_text_mut(focus).drain(caret..end);
    ui.settings_sel_anchor = caret;
    true
}

fn delete_settings_selection(ui: &mut UiState) -> bool {
    if ui.settings_focus == SettingsFocus::None {
        return false;
    }
    if !has_selection(ui.settings_sel_anchor, ui.settings_caret) {
        return false;
    }
    let focus = ui.settings_focus;
    let (start, end) = selection_range(ui.settings_sel_anchor, ui.settings_caret);
    ui.settings_field_text_mut(focus).drain(start..end);
    ui.settings_caret = start;
    ui.settings_sel_anchor = start;
    true
}

pub fn handle_settings_char(_app: &mut App, ui: &mut UiState, ch: u32) -> bool {
    if ui.settings_focus == SettingsFocus::None {
        return false;
    }

    let c = char::from_u32(ch).unwrap_or('\0');
    match c {
        '\x08' => handle_settings_backspace(ui),
        '\r' | '\n' => false,
        c if !c.is_control() => insert_settings_char(ui, c),
        _ => false,
    }
}

fn insert_settings_char(ui: &mut UiState, c: char) -> bool {
    if !settings_char_allowed(ui.settings_focus, c) {
        return false;
    }
    delete_settings_selection(ui);
    let focus = ui.settings_focus;
    let insert_at = ui.settings_caret.min(ui.settings_field_text(focus).len());
    ui.settings_field_text_mut(focus).insert(insert_at, c);
    ui.settings_caret = insert_at + c.len_utf8();
    ui.settings_sel_anchor = ui.settings_caret;
    true
}

fn settings_char_allowed(focus: SettingsFocus, c: char) -> bool {
    match focus {
        SettingsFocus::None => false,
        SettingsFocus::MaxEntries => c.is_ascii_digit(),
        SettingsFocus::MaxImageSizeMb => c.is_ascii_digit() || c == '.',
        SettingsFocus::Hotkey => true,
    }
}

fn prev_char_boundary(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    if pos == 0 {
        return 0;
    }
    text[..pos].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next_char_boundary(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    if pos >= text.len() {
        return text.len();
    }
    text[pos..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| pos + offset)
}

pub fn handle_settings_key_enter(
    app: &mut App,
    ui: &mut UiState,
    hooks: &SettingsHooks,
    monitor: &mut ClipboardMonitor,
) -> bool {
    commit_text_field(app, ui, hooks, monitor)
}

fn rect_clicked(rect: Option<WidgetRect>, x: f32, y: f32) -> bool {
    rect.is_some_and(|r| r.contains(x, y))
}

fn apply_pause(
    app: &mut App,
    ui: &mut UiState,
    hooks: &SettingsHooks,
    monitor: &mut ClipboardMonitor,
    paused: bool,
) -> bool {
    match app.set_pause_capture(paused) {
        Ok(()) => {
            monitor.set_config(app.config.capture_config());
            ui.settings_error = None;
            // SAFETY: `tray` valid for message-loop lifetime.
            let tray = unsafe { &*hooks.tray_ptr() };
            let _ = tray.set_tooltip(if paused {
                "TrayVault (capture paused)"
            } else {
                "TrayVault"
            });
            true
        }
        Err(err) => {
            ui.settings_error = Some(format!("Pause setting failed: {err}"));
            false
        }
    }
}

fn apply_theme(app: &mut App, ui: &mut UiState, theme: ThemeMode) -> bool {
    match app.set_theme(theme) {
        Ok(()) => {
            ui.settings_error = None;
            true
        }
        Err(err) => {
            ui.settings_error = Some(format!("Theme failed: {err}"));
            false
        }
    }
}

fn toggle_autostart(app: &mut App, ui: &mut UiState) -> bool {
    let previous = app.config.autostart;
    let desired = !previous;
    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            ui.settings_error = Some(format!("Could not resolve executable path: {err}"));
            return false;
        }
    };

    if let Err(err) = app.set_autostart(desired, &exe_path) {
        app.config.autostart = previous;
        ui.settings_error = Some(format!("Autostart failed: {err}"));
        log::warn(&format!("autostart toggle reverted: {err}"));
        false
    } else {
        ui.settings_error = None;
        true
    }
}

fn commit_text_field(
    app: &mut App,
    ui: &mut UiState,
    hooks: &SettingsHooks,
    monitor: &mut ClipboardMonitor,
) -> bool {
    match ui.settings_focus {
        SettingsFocus::None => false,
        SettingsFocus::MaxEntries => commit_max_entries(app, ui),
        SettingsFocus::Hotkey => commit_hotkey(app, ui, hooks),
        SettingsFocus::MaxImageSizeMb => commit_max_image_mb(app, ui, monitor),
    }
}

fn commit_max_entries(app: &mut App, ui: &mut UiState) -> bool {
    let text = ui.settings_edit_max_entries.trim();
    let Ok(value) = text.parse::<u32>() else {
        ui.settings_error = Some("Max entries must be a positive integer".into());
        ui.settings_edit_max_entries = app.config.max_entries.to_string();
        return false;
    };
    match app.set_max_entries(value) {
        Ok(()) => {
            ui.settings_error = None;
            true
        }
        Err(err) => {
            ui.settings_error = Some(format!("Max entries: {err}"));
            ui.settings_edit_max_entries = app.config.max_entries.to_string();
            false
        }
    }
}

fn commit_hotkey(app: &mut App, ui: &mut UiState, hooks: &SettingsHooks) -> bool {
    let new_hotkey = ui.settings_edit_hotkey.trim().to_string();
    let previous = app.config.hotkey.clone();
    if new_hotkey == previous {
        return false;
    }

    // SAFETY: `hotkey` valid for message-loop lifetime.
    let hotkey = unsafe { &mut *hooks.hotkey_ptr() };
    if let Err(err) = hotkey.reregister_strict(hooks.hwnd, &new_hotkey) {
        ui.settings_edit_hotkey = previous;
        ui.settings_error = Some(format!("Hotkey: {err}"));
        log::warn(&format!("hotkey change reverted: {err}"));
        return false;
    }

    if let Err(err) = app.set_hotkey_string(new_hotkey.clone()) {
        let _ = hotkey.reregister_strict(hooks.hwnd, &previous);
        ui.settings_edit_hotkey = previous;
        ui.settings_error = Some(format!("Hotkey save failed: {err}"));
        return false;
    }

    ui.settings_edit_hotkey = app.config.hotkey.clone();
    ui.settings_error = None;
    true
}

fn commit_max_image_mb(app: &mut App, ui: &mut UiState, monitor: &mut ClipboardMonitor) -> bool {
    let text = ui.settings_edit_max_image_mb.trim();
    let Ok(value) = text.parse::<f32>() else {
        ui.settings_error = Some("Max image size must be a number".into());
        ui.settings_edit_max_image_mb = format_max_image_edit(app.config.max_image_size_mb);
        return false;
    };
    match app.set_max_image_size_mb(value) {
        Ok(()) => {
            monitor.set_config(app.config.capture_config());
            ui.settings_edit_max_image_mb = format_max_image_edit(app.config.max_image_size_mb);
            ui.settings_error = None;
            true
        }
        Err(err) => {
            ui.settings_error = Some(format!("Max image size: {err}"));
            ui.settings_edit_max_image_mb = format_max_image_edit(app.config.max_image_size_mb);
            false
        }
    }
}
