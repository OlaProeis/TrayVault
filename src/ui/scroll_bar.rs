//! Vertical history scrollbar geometry, fade visibility, and NC hit-test helpers.

use crate::app::App;
use crate::ui::history::{build_list_layout, entry_inner_width, total_content_height, EntryLayout};
use crate::ui::search::{content_viewport_height, header_total_height, refresh_display_indices};
use crate::ui::theme::Theme;
use crate::ui::widgets::{fill_rect, rgba_to_color, Pixmap};
use crate::ui::UiState;
use crate::win32::ffi::GetTickCount;

/// Right-edge hit zone (wider than the visible thumb for hover reveal).
pub const GUTTER_W: f32 = 12.0;
pub const THUMB_W: f32 = 4.0;
const THUMB_MARGIN: f32 = 2.0;
const MIN_THUMB_H: f32 = 24.0;

pub const VISIBLE_HOLD_MS: u32 = 1000;
pub const FADE_MS: u32 = 300;

/// Must match [`crate::win32::window::RESIZE_BORDER`] for NC override logic.
const RESIZE_BORDER: i32 = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarLayout {
    pub track_top: f32,
    pub track_h: f32,
    pub thumb_x: f32,
    pub thumb_y: f32,
    pub thumb_w: f32,
    pub thumb_h: f32,
    pub gutter_x: f32,
    pub gutter_w: f32,
    pub max_scroll: f32,
}

pub fn tick_now() -> u32 {
    // SAFETY: documented kernel32 export.
    unsafe { GetTickCount() }
}

impl UiState {
    /// Extend scrollbar visibility (wheel, gutter hover, drag).
    pub fn touch_scrollbar(&mut self) {
        let now = tick_now();
        self.scrollbar_visible_until = now.saturating_add(VISIBLE_HOLD_MS + FADE_MS);
    }

    /// 0..=1 opacity for the thumb; 0 when fully hidden.
    pub fn scrollbar_opacity(&self, now: u32) -> f32 {
        if self.scrollbar_visible_until == 0 || now >= self.scrollbar_visible_until {
            return 0.0;
        }
        let fade_start = self.scrollbar_visible_until.saturating_sub(FADE_MS);
        if now <= fade_start {
            return 1.0;
        }
        (self.scrollbar_visible_until.saturating_sub(now) as f32) / FADE_MS as f32
    }

    /// True while the thumb is mid-fade (caller should schedule another repaint).
    pub fn scrollbar_fading(&self, now: u32) -> bool {
        let o = self.scrollbar_opacity(now);
        o > 0.0 && o < 1.0
    }

    pub fn scrollbar_dragging(&self) -> bool {
        self.scrollbar_drag_grab_y.is_some()
    }
}

pub fn layout_from_heights(
    client_w: f32,
    content_top: f32,
    viewport_h: f32,
    content_height: f32,
    scroll_offset: f32,
) -> Option<ScrollbarLayout> {
    if viewport_h <= 0.0 || content_height <= viewport_h {
        return None;
    }

    let thumb_h = (viewport_h * viewport_h / content_height)
        .max(MIN_THUMB_H)
        .min(viewport_h);
    let thumb_travel = (viewport_h - thumb_h).max(0.0);
    let max_scroll = (content_height - viewport_h).max(1.0);
    let thumb_y = content_top + (scroll_offset / max_scroll) * thumb_travel;
    let thumb_x = client_w - THUMB_MARGIN - THUMB_W;

    Some(ScrollbarLayout {
        track_top: content_top,
        track_h: viewport_h,
        thumb_x,
        thumb_y,
        thumb_w: THUMB_W,
        thumb_h,
        gutter_x: client_w - GUTTER_W,
        gutter_w: GUTTER_W,
        max_scroll,
    })
}

/// Build list layout metrics and scrollbar geometry for the main history view.
pub fn layout_for_list(
    app: &App,
    ui: &mut UiState,
    client_w: f32,
    client_h: f32,
) -> Option<(Vec<EntryLayout>, ScrollbarLayout)> {
    refresh_display_indices(app, ui);
    let thumb_max_w = entry_inner_width(client_w);
    let layouts = build_list_layout(
        &ui.display_indices,
        &app.entries,
        thumb_max_w,
        &mut ui.glyph_cache,
        &ui.expanded_text_entries,
    );
    let content_height = total_content_height(&layouts);
    let content_top = header_total_height();
    let viewport_h = content_viewport_height(client_h);
    let bar = layout_from_heights(
        client_w,
        content_top,
        viewport_h,
        content_height,
        ui.scroll_offset,
    )?;
    Some((layouts, bar))
}

pub fn scroll_offset_for_thumb_y(layout: &ScrollbarLayout, thumb_y: f32) -> f32 {
    let thumb_travel = (layout.track_h - layout.thumb_h).max(0.0);
    if thumb_travel <= 0.0 {
        return 0.0;
    }
    let rel = (thumb_y - layout.track_top).clamp(0.0, thumb_travel);
    (rel / thumb_travel) * layout.max_scroll
}

impl ScrollbarLayout {
    pub fn thumb_contains(&self, x: f32, y: f32) -> bool {
        x >= self.thumb_x
            && x < self.thumb_x + self.thumb_w
            && y >= self.thumb_y
            && y < self.thumb_y + self.thumb_h
    }

    pub fn gutter_contains(&self, x: f32, y: f32) -> bool {
        x >= self.gutter_x
            && x < self.gutter_x + self.gutter_w
            && y >= self.track_top
            && y < self.track_top + self.track_h
    }

    pub fn track_contains(&self, x: f32, y: f32) -> bool {
        x >= self.gutter_x
            && x < self.gutter_x + self.gutter_w
            && y >= self.track_top
            && y < self.track_top + self.track_h
    }
}

/// When true, `WM_NCHITTEST` should return `HTCLIENT` instead of `HTRIGHT` for the scrollbar gutter.
pub fn overrides_right_edge_resize(
    client_w: i32,
    client_h: i32,
    cx: i32,
    cy: i32,
    layout: &ScrollbarLayout,
) -> bool {
    if cx < client_w - RESIZE_BORDER {
        return false;
    }
    if cy < RESIZE_BORDER || cy >= client_h - RESIZE_BORDER {
        return false;
    }
    layout.track_contains(cx as f32, cy as f32)
}

pub fn overrides_right_edge_resize_for_state(
    ui: &UiState,
    cx: i32,
    cy: i32,
    client_w: i32,
    client_h: i32,
) -> bool {
    if ui.show_settings || ui.show_help || ui.preview_entry_id.is_some() {
        return false;
    }
    let content_top = header_total_height();
    let viewport_h = content_viewport_height(client_h as f32);
    let content_height = ui.last_content_height;
    layout_from_heights(
        client_w as f32,
        content_top,
        viewport_h,
        content_height,
        ui.scroll_offset,
    )
    .is_some_and(|layout| overrides_right_edge_resize(client_w, client_h, cx, cy, &layout))
}

pub fn draw_thumb(pixmap: &mut Pixmap, theme: &Theme, layout: &ScrollbarLayout, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    let color = faded_color(theme.background, theme.text_secondary, opacity);
    let bar_x = layout.thumb_x;
    let bar_w = layout.thumb_w;
    let thumb_y = layout.thumb_y;
    let thumb_h = layout.thumb_h;
    fill_rect(pixmap, bar_x + 1.0, thumb_y, bar_w - 2.0, thumb_h, color);
    fill_rect(pixmap, bar_x, thumb_y + 1.0, bar_w, thumb_h - 2.0, color);
}

fn faded_color(bg: [u8; 4], fg: [u8; 4], opacity: f32) -> crate::ui::pixmap::Color {
    let o = opacity.clamp(0.0, 1.0);
    rgba_to_color([
        lerp_u8(bg[0], fg[0], o),
        lerp_u8(bg[1], fg[1], o),
        lerp_u8(bg[2], fg[2], o),
        255,
    ])
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_none_when_content_fits() {
        assert!(layout_from_heights(400.0, 80.0, 300.0, 300.0, 0.0).is_none());
    }

    #[test]
    fn thumb_maps_to_scroll_offset() {
        let layout = layout_from_heights(400.0, 80.0, 200.0, 600.0, 0.0).expect("overflow");
        assert!((scroll_offset_for_thumb_y(&layout, layout.thumb_y) - 0.0).abs() < 0.01);
        let bottom = layout.track_top + layout.track_h - layout.thumb_h;
        let at_bottom = scroll_offset_for_thumb_y(&layout, bottom);
        assert!((at_bottom - layout.max_scroll).abs() < 0.5);
    }

    #[test]
    fn gutter_wider_than_thumb() {
        let layout = layout_from_heights(320.0, 70.0, 150.0, 500.0, 10.0).expect("overflow");
        assert!(layout.gutter_w >= layout.thumb_w);
        assert!(layout.gutter_contains(layout.thumb_x, layout.thumb_y));
    }

    #[test]
    fn opacity_fades_after_hold() {
        let mut ui = UiState::default();
        let start = tick_now();
        ui.scrollbar_visible_until = start.saturating_add(VISIBLE_HOLD_MS + FADE_MS);
        assert!((ui.scrollbar_opacity(start) - 1.0).abs() < f32::EPSILON);
        let mid = start.saturating_add(VISIBLE_HOLD_MS + FADE_MS / 2);
        assert!(ui.scrollbar_opacity(mid) > 0.0 && ui.scrollbar_opacity(mid) < 1.0);
        assert_eq!(
            ui.scrollbar_opacity(start.saturating_add(VISIBLE_HOLD_MS + FADE_MS)),
            0.0
        );
    }
}
