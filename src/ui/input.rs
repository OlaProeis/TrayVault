//! Route Win32 input events to app/UI state.

use crate::app::App;
use crate::models::EntryKind;
use crate::ui::history::{
    build_list_layout, ensure_index_visible, entry_inner_width, hit_test_entry,
    total_content_height,
};
use crate::ui::scroll_bar::{layout_for_list, scroll_offset_for_thumb_y};
use crate::ui::search::{
    content_viewport_height, footer_total_height, header_total_height, hit_test_filter_chip,
    move_selection, refresh_display_indices, sync_selection_to_display,
};
use crate::ui::search_edit::{caret_at_click, has_selection, selection_range};
use crate::ui::settings_input::{
    close_settings, handle_settings_char, handle_settings_key_down, handle_settings_key_enter,
    handle_settings_mouse_up, handle_settings_wheel, SettingsHooks,
};
use crate::ui::widgets::{context_menu_labels, hit_test_context_menu, PADDING};
use crate::ui::EntryFilter;
use crate::ui::{HoverKey, UiState};
use crate::win32::clipboard::ClipboardMonitor;
use crate::win32::ffi::{
    GetKeyState, HWND, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_LEFT,
    VK_OEM_2, VK_RETURN, VK_RIGHT, VK_UP,
};
use crate::win32::window::{hide_window, InputEvent};

const WHEEL_DELTA: f32 = 120.0;

/// Client-area size in pixels (from GDI back-buffer).
#[derive(Clone, Copy)]
pub struct Viewport {
    pub hwnd: HWND,
    pub width: i32,
    pub height: i32,
}

/// Process one input event; returns true when the caller should repaint.
pub fn handle_input(
    event: InputEvent,
    app: &mut App,
    ui: &mut UiState,
    viewport: Viewport,
    monitor: &mut ClipboardMonitor,
    settings_hooks: Option<&SettingsHooks>,
) -> bool {
    match event {
        InputEvent::MouseMove(x, y) => {
            ui.set_mouse(x as f32, y as f32);
            if ui.show_settings || ui.show_help || ui.preview_entry_id.is_some() {
                return true;
            }
            if ui.scrollbar_drag_grab_y.is_some() {
                update_scrollbar_drag(app, ui, viewport, y as f32);
                ui.touch_scrollbar();
                return true;
            }
            if gutter_hits_scrollbar(app, ui, viewport, x as f32, y as f32) {
                ui.touch_scrollbar();
            }
            ui.hover_key = hover_key_at(app, ui, viewport, x as f32, y as f32);
            // Repaint every move so card hover tracks the cursor; HoverKey alone is too coarse
            // (card draw uses tighter bounds than a row-only hit test).
            true
        }
        InputEvent::LButtonDown(x, y) => {
            ui.on_left_down(x as f32, y as f32);
            dismiss_context_menu_unless_hit(app, ui, x as f32, y as f32);
            if !ui.show_settings && !ui.show_help && ui.preview_entry_id.is_none() {
                try_scrollbar_press(app, ui, viewport, x as f32, y as f32);
            }
            true
        }
        InputEvent::LButtonUp(x, y) => {
            ui.on_left_up(x as f32, y as f32);
            let suppress = ui.scrollbar_suppress_click;
            ui.scrollbar_drag_grab_y = None;
            ui.scrollbar_suppress_click = false;
            if handle_title_bar_close(app, viewport, ui, x as f32, y as f32) {
                return true;
            }
            if ui.show_settings {
                if ui
                    .settings_button_rect
                    .is_some_and(|r| r.contains(x as f32, y as f32))
                {
                    if let Some(hooks) = settings_hooks {
                        close_settings(app, ui, hooks, monitor);
                    } else {
                        ui.show_settings = false;
                        ui.settings_focus = crate::ui::SettingsFocus::None;
                    }
                    return true;
                }
                if let Some(hooks) = settings_hooks {
                    let mut cache = std::mem::take(&mut ui.glyph_cache);
                    handle_settings_mouse_up(
                        app, ui, &mut cache, x as f32, y as f32, hooks, monitor,
                    );
                    ui.glyph_cache = cache;
                }
            } else if !suppress {
                handle_main_mouse_up(app, ui, x as f32, y as f32, viewport, monitor);
            }
            true
        }
        InputEvent::LButtonDblClk(x, y) => {
            ui.set_mouse(x as f32, y as f32);
            try_open_image_preview(app, ui, viewport, x as f32, y as f32);
            true
        }
        InputEvent::RButtonDown(x, y) => {
            ui.on_right_down(x as f32, y as f32);
            true
        }
        InputEvent::RButtonUp(_, _) => {
            ui.on_right_up();
            true
        }
        InputEvent::MouseWheel(_, _, delta) => {
            if ui.show_settings {
                return handle_settings_wheel(ui, delta as i32, viewport.height);
            }
            let scroll_amount = -(delta as f32) / WHEEL_DELTA * 48.0;
            refresh_display_indices(app, ui);
            let thumb_max_w = entry_inner_width(viewport.width.max(0) as f32);
            let layouts = build_list_layout(
                &ui.display_indices,
                &app.entries,
                thumb_max_w,
                &mut ui.glyph_cache,
                &ui.expanded_text_entries,
            );
            let content_h = total_content_height(&layouts);
            let viewport_h = content_viewport_height(viewport.height.max(0) as f32);
            ui.scroll_offset =
                (ui.scroll_offset + scroll_amount).clamp(0.0, (content_h - viewport_h).max(0.0));
            ui.touch_scrollbar();
            true
        }
        InputEvent::KeyDown(vk) => handle_key_down(vk, app, ui, viewport, monitor, settings_hooks),
        InputEvent::Char(ch) => handle_char(ch, app, ui, settings_hooks, monitor),
    }
}

fn handle_key_down(
    vk: u32,
    app: &mut App,
    ui: &mut UiState,
    viewport: Viewport,
    monitor: &mut ClipboardMonitor,
    settings_hooks: Option<&SettingsHooks>,
) -> bool {
    use crate::win32::window::shift_held;

    if ui.show_settings {
        if vk == VK_ESCAPE as u32 {
            if let Some(hooks) = settings_hooks {
                close_settings(app, ui, hooks, monitor);
            } else {
                ui.show_settings = false;
                ui.settings_focus = crate::ui::SettingsFocus::None;
                ui.settings_caret = 0;
                ui.settings_sel_anchor = 0;
            }
            return true;
        }
        if vk == VK_RETURN as u32 {
            if let Some(hooks) = settings_hooks {
                return handle_settings_key_enter(app, ui, hooks, monitor);
            }
        }
        if handle_settings_key_down(vk, ui, shift_held(), control_held()) {
            return true;
        }
        return false;
    }

    if ui.preview_entry_id.is_some() {
        if vk == VK_ESCAPE as u32 {
            ui.preview_entry_id = None;
            ui.preview_cache = None;
            return true;
        }
        return false;
    }

    if ui.show_help {
        if vk == VK_ESCAPE as u32 {
            ui.show_help = false;
            return true;
        }
        return false;
    }

    match vk as i32 {
        VK_F1 => {
            ui.show_help = !ui.show_help;
            true
        }
        VK_OEM_2 if shift_held() => {
            ui.show_help = !ui.show_help;
            true
        }
        VK_UP => {
            refresh_display_indices(app, ui);
            move_selection(app, ui, -1);
            clamp_scroll_to_selection(app, ui, viewport);
            true
        }
        VK_DOWN => {
            refresh_display_indices(app, ui);
            move_selection(app, ui, 1);
            clamp_scroll_to_selection(app, ui, viewport);
            true
        }
        VK_RETURN => {
            if let Some(id) = app.selected_entry_id() {
                if copy_and_maybe_hide(app, id, viewport, monitor) {
                    hide_window(viewport.hwnd);
                }
            }
            true
        }
        VK_ESCAPE => {
            if !app.filter_query.is_empty() {
                app.filter_query.clear();
                ui.search_caret = 0;
                ui.search_sel_anchor = 0;
                refresh_display_indices(app, ui);
                sync_selection_to_display(app, ui);
                true
            } else {
                hide_window(viewport.hwnd);
                app.on_hide_window();
                true
            }
        }
        VK_DELETE => {
            if ui.search_focused {
                if delete_search_selection(app, ui) || {
                    let end = next_char_boundary(&app.filter_query, ui.search_caret);
                    if end > ui.search_caret {
                        app.filter_query.drain(ui.search_caret..end);
                        true
                    } else {
                        false
                    }
                } {
                    refresh_display_indices(app, ui);
                    sync_selection_to_display(app, ui);
                }
            } else if let Some(id) = app.selected_entry_id() {
                app.delete_entry(id);
                refresh_display_indices(app, ui);
                sync_selection_to_display(app, ui);
            }
            true
        }
        VK_LEFT if ui.search_focused => {
            ui.search_caret = prev_char_boundary(&app.filter_query, ui.search_caret);
            if !shift_held() {
                ui.search_sel_anchor = ui.search_caret;
            }
            true
        }
        VK_RIGHT if ui.search_focused => {
            ui.search_caret = next_char_boundary(&app.filter_query, ui.search_caret);
            if !shift_held() {
                ui.search_sel_anchor = ui.search_caret;
            }
            true
        }
        VK_HOME if ui.search_focused => {
            ui.search_caret = 0;
            if !shift_held() {
                ui.search_sel_anchor = 0;
            }
            true
        }
        VK_END if ui.search_focused => {
            ui.search_caret = app.filter_query.len();
            if !shift_held() {
                ui.search_sel_anchor = ui.search_caret;
            }
            true
        }
        _ => {
            if control_held() && ui.search_focused && (vk == b'A' as u32 || vk == b'a' as u32) {
                ui.search_sel_anchor = 0;
                ui.search_caret = app.filter_query.len();
                return true;
            }
            if control_held() && (vk == b'P' as u32 || vk == b'p' as u32) {
                if let Some(id) = app.selected_entry_id() {
                    app.toggle_pin(id);
                    refresh_display_indices(app, ui);
                }
                return true;
            }
            false
        }
    }
}

fn control_held() -> bool {
    // SAFETY: documented GetKeyState usage.
    unsafe { (GetKeyState(VK_CONTROL) as u16) & 0x8000 != 0 }
}

fn delete_search_selection(app: &mut App, ui: &mut UiState) -> bool {
    if !has_selection(ui.search_sel_anchor, ui.search_caret) {
        return false;
    }
    let (start, end) = selection_range(ui.search_sel_anchor, ui.search_caret);
    app.filter_query.drain(start..end);
    ui.search_caret = start;
    ui.search_sel_anchor = start;
    true
}

fn handle_char(
    ch: u32,
    app: &mut App,
    ui: &mut UiState,
    settings_hooks: Option<&SettingsHooks>,
    _monitor: &mut ClipboardMonitor,
) -> bool {
    if ui.show_settings {
        if settings_hooks.is_some() {
            return handle_settings_char(app, ui, ch);
        }
        return false;
    }
    if ui.show_help || ui.preview_entry_id.is_some() {
        return false;
    }
    if control_held() {
        return false;
    }
    if !ui.search_focused {
        return false;
    }

    let c = char::from_u32(ch).unwrap_or('\0');
    match c {
        '\x08' => {
            if delete_search_selection(app, ui)
                || (ui.search_caret > 0 && {
                    let start = prev_char_boundary(&app.filter_query, ui.search_caret);
                    app.filter_query.drain(start..ui.search_caret);
                    ui.search_caret = start;
                    ui.search_sel_anchor = start;
                    true
                })
            {
                refresh_display_indices(app, ui);
                sync_selection_to_display(app, ui);
            }
            true
        }
        '\x1b' => false,
        c if !c.is_control() => {
            if delete_search_selection(app, ui) {
                // replaced selection before insert
            }
            app.filter_query.insert(ui.search_caret, c);
            ui.search_caret += c.len_utf8();
            ui.search_sel_anchor = ui.search_caret;
            refresh_display_indices(app, ui);
            sync_selection_to_display(app, ui);
            true
        }
        _ => false,
    }
}

fn prev_char_boundary(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())]
        .char_indices()
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    if pos >= text.len() {
        return text.len();
    }
    text[pos..]
        .char_indices()
        .nth(1)
        .map(|(i, _)| pos + i)
        .unwrap_or(text.len())
}

/// Close the context menu on left press outside the menu; keep it open for item clicks.
fn dismiss_context_menu_unless_hit(app: &App, ui: &mut UiState, x: f32, y: f32) {
    let Some(menu) = ui.context_menu.clone() else {
        return;
    };
    let entry = app.entries.iter().find(|e| e.id == menu.entry_id);
    let items = context_menu_labels(entry.is_some_and(|e| e.is_pinned));
    if hit_test_context_menu(&mut ui.glyph_cache, menu.x, menu.y, x, y, &items).is_none() {
        ui.context_menu = None;
    }
}

fn handle_main_mouse_up(
    app: &mut App,
    ui: &mut UiState,
    x: f32,
    y: f32,
    viewport: Viewport,
    monitor: &mut ClipboardMonitor,
) {
    if !ui.mouse_left_pressed {
        return;
    }

    if let Some(menu) = ui.context_menu.clone() {
        let entry = app.entries.iter().find(|e| e.id == menu.entry_id);
        let items = context_menu_labels(entry.is_some_and(|e| e.is_pinned));
        if let Some(choice) =
            hit_test_context_menu(&mut ui.glyph_cache, menu.x, menu.y, x, y, &items)
        {
            match choice {
                0 => {
                    if copy_and_maybe_hide(app, menu.entry_id, viewport, monitor) {
                        hide_window(viewport.hwnd);
                    }
                }
                1 => {
                    app.toggle_pin(menu.entry_id);
                    refresh_display_indices(app, ui);
                }
                2 => {
                    app.delete_entry(menu.entry_id);
                    refresh_display_indices(app, ui);
                    sync_selection_to_display(app, ui);
                }
                _ => {}
            }
            ui.context_menu = None;
            return;
        }
        ui.context_menu = None;
        return;
    }

    if ui.settings_button_rect.is_some_and(|r| r.contains(x, y)) {
        ui.open_settings(&app.config);
        return;
    }

    if let Some(rect) = ui.search_input_rect {
        if rect.contains(x, y) {
            ui.search_focused = true;
            ui.search_caret = caret_at_click(&mut ui.glyph_cache, &app.filter_query, rect, x);
            ui.search_sel_anchor = ui.search_caret;
            return;
        }
    }

    let client_h = viewport.height.max(0) as f32;
    if let Some(filter) = hit_test_filter_chip(&mut ui.glyph_cache, x, y, client_h) {
        if ui.filter != filter {
            ui.filter = filter;
            refresh_display_indices(app, ui);
            sync_selection_to_display(app, ui);
        }
        return;
    }

    for &(entry_id, rect) in &ui.expand_button_rects {
        if rect.contains(x, y) {
            if ui.expanded_text_entries.contains(&entry_id) {
                ui.expanded_text_entries.remove(&entry_id);
            } else {
                ui.expanded_text_entries.insert(entry_id);
            }
            return;
        }
    }

    refresh_display_indices(app, ui);
    let thumb_max_w = entry_inner_width(viewport.width.max(0) as f32);
    let layouts = build_list_layout(
        &ui.display_indices,
        &app.entries,
        thumb_max_w,
        &mut ui.glyph_cache,
        &ui.expanded_text_entries,
    );
    let content_top = header_total_height();
    let content_w = list_content_width(viewport);
    if let Some(idx) = hit_test_entry(
        &layouts,
        ui.scroll_offset,
        content_top,
        PADDING,
        content_w,
        x,
        y,
    ) {
        app.selected_index = idx;
        let entry_id = app.entries[idx].id;
        if copy_and_maybe_hide(app, entry_id, viewport, monitor) {
            hide_window(viewport.hwnd);
        }
    }
}

fn copy_and_maybe_hide(
    app: &mut App,
    entry_id: u64,
    viewport: Viewport,
    monitor: &mut ClipboardMonitor,
) -> bool {
    app.copy_entry_to_clipboard(entry_id, viewport.hwnd, monitor)
        .unwrap_or(false)
}

fn handle_title_bar_close(app: &mut App, viewport: Viewport, ui: &UiState, x: f32, y: f32) -> bool {
    if !ui.mouse_left_pressed {
        return false;
    }
    if ui.close_button_rect.is_some_and(|rect| rect.contains(x, y)) {
        hide_window(viewport.hwnd);
        app.on_hide_window();
        return true;
    }
    false
}

fn try_open_image_preview(app: &App, ui: &mut UiState, viewport: Viewport, x: f32, y: f32) {
    refresh_display_indices(app, ui);
    let thumb_max_w = entry_inner_width(viewport.width.max(0) as f32);
    let layouts = build_list_layout(
        &ui.display_indices,
        &app.entries,
        thumb_max_w,
        &mut ui.glyph_cache,
        &ui.expanded_text_entries,
    );
    let content_top = header_total_height();
    let content_w = list_content_width(viewport);
    if let Some(idx) = hit_test_entry(
        &layouts,
        ui.scroll_offset,
        content_top,
        PADDING,
        content_w,
        x,
        y,
    ) {
        if app.entries[idx].kind == EntryKind::Image {
            ui.preview_entry_id = Some(app.entries[idx].id);
        }
    }
}

fn clamp_scroll_to_selection(app: &App, ui: &mut UiState, viewport: Viewport) {
    let thumb_max_w = entry_inner_width(viewport.width.max(0) as f32);
    let layouts = build_list_layout(
        &ui.display_indices,
        &app.entries,
        thumb_max_w,
        &mut ui.glyph_cache,
        &ui.expanded_text_entries,
    );
    let viewport_h = content_viewport_height(viewport.height.max(0) as f32);
    ui.scroll_offset =
        ensure_index_visible(&layouts, app.selected_index, ui.scroll_offset, viewport_h);
    ui.touch_scrollbar();
}

fn gutter_hits_scrollbar(app: &App, ui: &mut UiState, viewport: Viewport, x: f32, y: f32) -> bool {
    let client_w = viewport.width.max(0) as f32;
    let client_h = viewport.height.max(0) as f32;
    layout_for_list(app, ui, client_w, client_h).is_some_and(|(_, bar)| bar.gutter_contains(x, y))
}

fn try_scrollbar_press(app: &App, ui: &mut UiState, viewport: Viewport, x: f32, y: f32) {
    let client_w = viewport.width.max(0) as f32;
    let client_h = viewport.height.max(0) as f32;
    let Some((_layouts, bar)) = layout_for_list(app, ui, client_w, client_h) else {
        return;
    };
    if !bar.track_contains(x, y) {
        return;
    }
    ui.scrollbar_suppress_click = true;
    ui.touch_scrollbar();
    if bar.thumb_contains(x, y) {
        ui.scrollbar_drag_grab_y = Some(y - bar.thumb_y);
        return;
    }
    let page = bar.track_h * 0.9;
    if y < bar.thumb_y {
        ui.scroll_offset = (ui.scroll_offset - page).max(0.0);
    } else if y > bar.thumb_y + bar.thumb_h {
        ui.scroll_offset = (ui.scroll_offset + page).min(bar.max_scroll);
    }
}

fn update_scrollbar_drag(app: &App, ui: &mut UiState, viewport: Viewport, y: f32) {
    let grab = match ui.scrollbar_drag_grab_y {
        Some(g) => g,
        None => return,
    };
    let client_w = viewport.width.max(0) as f32;
    let client_h = viewport.height.max(0) as f32;
    let Some((_layouts, bar)) = layout_for_list(app, ui, client_w, client_h) else {
        return;
    };
    let thumb_y = (y - grab).clamp(bar.track_top, bar.track_top + bar.track_h - bar.thumb_h);
    ui.scroll_offset = scroll_offset_for_thumb_y(&bar, thumb_y);
}

fn hover_key_at(app: &App, ui: &mut UiState, viewport: Viewport, x: f32, y: f32) -> HoverKey {
    let close = ui.close_button_rect.is_some_and(|r| r.contains(x, y));
    let settings = ui.settings_button_rect.is_some_and(|r| r.contains(x, y));
    let client_h = viewport.height.max(0) as f32;
    let filter_chip = {
        let cache = &mut ui.glyph_cache;
        hit_test_filter_chip(cache, x, y, client_h).map(|filter| match filter {
            EntryFilter::All => 1,
            EntryFilter::Text => 2,
            EntryFilter::Images => 3,
            EntryFilter::Pinned => 4,
        })
    }
    .unwrap_or(0);

    let content_bottom = client_h - footer_total_height();
    let entry_index = if y >= header_total_height() && y < content_bottom {
        refresh_display_indices(app, ui);
        let thumb_max_w = entry_inner_width(viewport.width.max(0) as f32);
        let layouts = build_list_layout(
            &ui.display_indices,
            &app.entries,
            thumb_max_w,
            &mut ui.glyph_cache,
            &ui.expanded_text_entries,
        );
        let content_top = header_total_height();
        let content_w = list_content_width(viewport);
        hit_test_entry(
            &layouts,
            ui.scroll_offset,
            content_top,
            PADDING,
            content_w,
            x,
            y,
        )
    } else {
        None
    };

    HoverKey {
        entry_index,
        filter_chip,
        settings,
        close,
    }
}

fn list_content_width(viewport: Viewport) -> f32 {
    (viewport.width.max(0) as f32 - PADDING * 2.0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::{next_char_boundary, prev_char_boundary};

    #[test]
    fn next_char_boundary_ascii() {
        let s = "hello";
        assert_eq!(next_char_boundary(s, 0), 1);
        assert_eq!(next_char_boundary(s, 4), 5);
        assert_eq!(next_char_boundary(s, 5), 5);
    }

    #[test]
    fn next_char_boundary_multibyte_and_combining() {
        let accented = "aéb";
        assert_eq!(next_char_boundary(accented, 0), 1);
        assert_eq!(next_char_boundary(accented, 1), 3);
        assert_eq!(next_char_boundary(accented, 3), 4);

        let combining = "e\u{0301}x";
        assert_eq!(next_char_boundary(combining, 0), 1);
        assert_eq!(next_char_boundary(combining, 1), 3);
        assert_eq!(next_char_boundary(combining, 3), 4);
    }

    #[test]
    fn next_char_boundary_emoji() {
        let s = "hi👋";
        assert_eq!(next_char_boundary(s, 0), 1);
        assert_eq!(next_char_boundary(s, 2), 6);
        assert_eq!(next_char_boundary(s, 6), 6);
    }

    #[test]
    fn prev_next_char_boundary_round_trip() {
        let samples = ["hello", "café", "hi👋there", "e\u{0301}z"];
        for s in samples {
            let mut pos = 0;
            while pos < s.len() {
                let next = next_char_boundary(s, pos);
                assert!(next > pos || next == s.len(), "stuck at {pos} in {s:?}");
                if next < s.len() {
                    assert_eq!(prev_char_boundary(s, next), pos);
                }
                pos = next;
            }
        }
    }
}
