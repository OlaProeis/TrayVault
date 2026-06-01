//! Immediate-mode widget primitives and layout helpers.

#![allow(dead_code)] // primitives wired incrementally; Button/ScrollableList used by settings later

pub use crate::ui::pixmap::{fill_rect, rgba_to_color, Pixmap};

use crate::ui::text::GlyphCache;
use crate::ui::theme::Theme;

pub const PADDING: f32 = 12.0;

/// Draw a pill-shaped toggle track plus a rounded knob.
///
/// `toggle_x/y/w/h` are the outer track bounds.  Corners are cut by 2 px to
/// approximate a stadium/capsule shape using plain axis-aligned rectangles.
#[allow(clippy::too_many_arguments)]
pub fn draw_toggle(
    pixmap: &mut Pixmap,
    toggle_x: f32,
    toggle_y: f32,
    toggle_w: f32,
    toggle_h: f32,
    enabled: bool,
    track_on: [u8; 4],
    track_off: [u8; 4],
) {
    let c = rgba_to_color(if enabled { track_on } else { track_off });
    // Pill track: three rects that cut the four corners by 2 px.
    fill_rect(
        pixmap,
        toggle_x + 2.0,
        toggle_y,
        toggle_w - 4.0,
        toggle_h,
        c,
    );
    fill_rect(
        pixmap,
        toggle_x + 1.0,
        toggle_y + 1.0,
        toggle_w - 2.0,
        toggle_h - 2.0,
        c,
    );
    fill_rect(
        pixmap,
        toggle_x,
        toggle_y + 2.0,
        toggle_w,
        toggle_h - 4.0,
        c,
    );

    // Rounded knob (knob_size × knob_size circle approx via three rects).
    let knob_size = toggle_h - 4.0;
    let knob_y = toggle_y + 2.0;
    let knob_x = if enabled {
        toggle_x + toggle_w - knob_size - 2.0
    } else {
        toggle_x + 2.0
    };
    let w = rgba_to_color([0xFF, 0xFF, 0xFF, 0xFF]);
    fill_rect(pixmap, knob_x + 2.0, knob_y, knob_size - 4.0, knob_size, w);
    fill_rect(
        pixmap,
        knob_x + 1.0,
        knob_y + 1.0,
        knob_size - 2.0,
        knob_size - 2.0,
        w,
    );
    fill_rect(pixmap, knob_x, knob_y + 2.0, knob_size, knob_size - 4.0, w);
}

/// Per-frame UI context for hover/active tracking and event emission.
pub struct UiContext<'a> {
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub mouse_left_down: bool,
    pub mouse_left_pressed: bool,
    pub mouse_right_down: bool,
    pub theme: &'a Theme,
    pub cache: &'a mut GlyphCache,
    next_id: u32,
    pub hot_widget: u32,
    pub active_widget: u32,
    pub events: Vec<WidgetEvent>,
}

impl<'a> UiContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mouse_x: f32,
        mouse_y: f32,
        mouse_left_down: bool,
        mouse_left_pressed: bool,
        mouse_right_down: bool,
        theme: &'a Theme,
        cache: &'a mut GlyphCache,
        active_widget: u32,
    ) -> Self {
        Self {
            mouse_x,
            mouse_y,
            mouse_left_down,
            mouse_left_pressed,
            mouse_right_down,
            theme,
            cache,
            next_id: 1,
            hot_widget: 0,
            active_widget,
            events: Vec::new(),
        }
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn pointer_over(&self, rect: WidgetRect) -> bool {
        rect.contains(self.mouse_x, self.mouse_y)
    }

    fn emit_click(&mut self, id: u32) {
        self.events.push(WidgetEvent::Clicked(id));
    }

    fn emit_right_click(&mut self, id: u32) {
        self.events.push(WidgetEvent::RightClicked(id));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetEvent {
    Clicked(u32),
    RightClicked(u32),
    TextChanged(u32),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WidgetRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl WidgetRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Horizontal row with fixed padding between children.
pub fn row<'a>(
    ctx: &mut UiContext<'a>,
    x: f32,
    y: f32,
    _height: f32,
    gap: f32,
    children: impl FnOnce(&mut UiContext<'a>, f32, f32) -> f32,
) -> f32 {
    children(ctx, x + PADDING, y + PADDING / 2.0) + gap
}

/// Vertical column; returns total height consumed.
pub fn column<'a>(
    ctx: &mut UiContext<'a>,
    x: f32,
    y: f32,
    width: f32,
    gap: f32,
    children: impl FnOnce(&mut UiContext<'a>, f32, f32, f32) -> f32,
) -> f32 {
    children(ctx, x, y, width) + gap
}

pub fn divider(ctx: &mut UiContext<'_>, pixmap: &mut Pixmap, x: f32, y: f32, width: f32) {
    fill_rect(pixmap, x, y, width, 1.0, rgba_to_color(ctx.theme.divider));
}

pub fn toggle_row(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    label: &str,
    x: f32,
    y: f32,
    width: f32,
    enabled: bool,
) -> (WidgetRect, bool) {
    let id = ctx.alloc_id();
    let row_h = 32.0;
    let rect = WidgetRect {
        x,
        y,
        w: width,
        h: row_h,
    };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }

    ctx.cache.draw(
        pixmap,
        label,
        x,
        y + 22.0,
        14.0,
        rgba_to_color(ctx.theme.text_primary),
    );

    let toggle_w = 44.0;
    let toggle_h = 22.0;
    let toggle_x = x + width - toggle_w;
    let toggle_y = y + 4.0;
    draw_toggle(
        pixmap,
        toggle_x,
        toggle_y,
        toggle_w,
        toggle_h,
        enabled,
        ctx.theme.accent,
        ctx.theme.divider,
    );

    let clicked = hovered && ctx.mouse_left_pressed;
    if clicked {
        ctx.emit_click(id);
    }

    (rect, clicked)
}

pub fn button(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    label: &str,
    x: f32,
    y: f32,
    min_w: f32,
    height: f32,
) -> (WidgetRect, bool) {
    let id = ctx.alloc_id();
    let text_w = ctx.cache.measure(label, 13.0);
    let w = (text_w + PADDING * 2.0).max(min_w);
    let rect = WidgetRect { x, y, w, h: height };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }

    // Outlined button: border rect first, then inset background.
    let border_color = if hovered {
        ctx.theme.accent
    } else {
        ctx.theme.divider
    };
    fill_rect(pixmap, x, y, w, height, rgba_to_color(border_color));
    let bg = if hovered {
        ctx.theme.selection
    } else {
        ctx.theme.card
    };
    fill_rect(
        pixmap,
        x + 1.0,
        y + 1.0,
        w - 2.0,
        height - 2.0,
        rgba_to_color(bg),
    );

    // Accent text signals the button is interactive regardless of hover state.
    let text_color = rgba_to_color(ctx.theme.accent);
    let text_x = x + (w - text_w) / 2.0;
    ctx.cache
        .draw(pixmap, label, text_x, y + height * 0.65, 13.0, text_color);

    let clicked = hovered && ctx.mouse_left_pressed;
    if clicked {
        ctx.emit_click(id);
        ctx.active_widget = id;
    }

    (rect, clicked)
}

/// Square icon-only button (compact alternative to [`button`]).
pub fn icon_button(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    icon: &str,
    x: f32,
    y: f32,
    size: f32,
    icon_size: f32,
) -> (WidgetRect, bool) {
    let id = ctx.alloc_id();
    let rect = WidgetRect {
        x,
        y,
        w: size,
        h: size,
    };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }

    let border_color = if hovered {
        ctx.theme.accent
    } else {
        ctx.theme.divider
    };
    fill_rect(pixmap, x, y, size, size, rgba_to_color(border_color));
    let bg = if hovered {
        ctx.theme.selection
    } else {
        ctx.theme.card
    };
    fill_rect(
        pixmap,
        x + 1.0,
        y + 1.0,
        size - 2.0,
        size - 2.0,
        rgba_to_color(bg),
    );

    let icon_w = ctx.cache.measure(icon, icon_size);
    let icon_color = rgba_to_color(if hovered {
        ctx.theme.accent
    } else {
        ctx.theme.text_secondary
    });
    ctx.cache.draw(
        pixmap,
        icon,
        x + (size - icon_w) / 2.0,
        y + size * 0.72,
        icon_size,
        icon_color,
    );

    let clicked = hovered && ctx.mouse_left_pressed;
    if clicked {
        ctx.emit_click(id);
        ctx.active_widget = id;
    }

    (rect, clicked)
}

pub fn filter_chip(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    label: &str,
    x: f32,
    y: f32,
    selected: bool,
) -> (WidgetRect, bool) {
    let id = ctx.alloc_id();
    let text_w = ctx.cache.measure(label, 12.0);
    let w = text_w + PADDING * 1.5;
    let h = 22.0;
    let rect = WidgetRect { x, y, w, h };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }

    let bg = if selected {
        ctx.theme.accent
    } else if hovered {
        ctx.theme.selection
    } else {
        ctx.theme.card
    };
    fill_rect(pixmap, x, y, w, h, rgba_to_color(bg));

    let text_color = if selected {
        crate::ui::pixmap::Color::from_rgba8(255, 255, 255, 255)
    } else {
        rgba_to_color(ctx.theme.text_secondary)
    };
    ctx.cache.draw(
        pixmap,
        label,
        x + PADDING * 0.75,
        y + 15.0,
        12.0,
        text_color,
    );

    let clicked = hovered && ctx.mouse_left_pressed;
    if clicked {
        ctx.emit_click(id);
    }

    (rect, clicked)
}

pub fn card(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    selected: bool,
) -> WidgetRect {
    let id = ctx.alloc_id();
    let rect = WidgetRect {
        x,
        y,
        w: width,
        h: height,
    };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }

    let bg = if selected || hovered {
        ctx.theme.selection
    } else {
        ctx.theme.card
    };
    fill_rect(pixmap, x, y, width, height, rgba_to_color(bg));

    // 1 px bottom border acts as a visual separator between consecutive entries.
    fill_rect(
        pixmap,
        x,
        y + height - 1.0,
        width,
        1.0,
        rgba_to_color(ctx.theme.divider),
    );

    if selected {
        fill_rect(pixmap, x, y, 3.0, height, rgba_to_color(ctx.theme.accent));
    } else if hovered {
        fill_rect(pixmap, x, y, 2.0, height, rgba_to_color(ctx.theme.divider));
    }

    if hovered && ctx.mouse_right_down {
        ctx.emit_right_click(id);
    }

    rect
}

pub struct InputBoxResult {
    pub rect: WidgetRect,
    pub focused: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn input_box(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    value: &str,
    placeholder: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    focused: bool,
    text_size_px: f32,
    scroll_x: f32,
    selection: Option<(usize, usize)>,
    text_y_offset: f32,
) -> InputBoxResult {
    let id = ctx.alloc_id();
    let rect = WidgetRect {
        x,
        y,
        w: width,
        h: height,
    };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }
    if hovered && ctx.mouse_left_pressed {
        ctx.emit_click(id);
        ctx.active_widget = id;
    }

    fill_rect(pixmap, x, y, width, height, rgba_to_color(ctx.theme.card));
    if focused || hovered {
        fill_rect(
            pixmap,
            x,
            y + height - 2.0,
            width,
            2.0,
            rgba_to_color(ctx.theme.accent),
        );
    }

    let display = if value.is_empty() && !focused {
        placeholder
    } else {
        value
    };
    let color = if value.is_empty() && !focused {
        rgba_to_color(ctx.theme.text_secondary)
    } else {
        rgba_to_color(ctx.theme.text_primary)
    };
    let text_x = x + 8.0;
    let text_y = y + height * 0.65 + text_y_offset;
    let inner_w = (width - 16.0).max(0.0);

    if let Some((sel_start, sel_end)) = selection {
        if sel_start < sel_end && !value.is_empty() {
            let before = &value[..sel_start.min(value.len())];
            let selected = &value[sel_start.min(value.len())..sel_end.min(value.len())];
            let sel_x = text_x + ctx.cache.measure(before, text_size_px) - scroll_x;
            let sel_w = ctx.cache.measure(selected, text_size_px);
            if sel_w > 0.0 {
                fill_rect(
                    pixmap,
                    sel_x,
                    y + 3.0,
                    sel_w.min(inner_w.max(0.0)),
                    height - 6.0,
                    rgba_to_color(ctx.theme.selection),
                );
            }
        }
    }

    ctx.cache.draw(
        pixmap,
        display,
        text_x - scroll_x,
        text_y,
        text_size_px,
        color,
    );

    InputBoxResult {
        rect,
        focused: focused || ctx.active_widget == id,
    }
}

/// Virtualized scroll container metadata.
pub struct ScrollableList {
    pub content_height: f32,
    pub viewport_height: f32,
    pub scroll_offset: f32,
}

impl ScrollableList {
    pub fn clamp_scroll(&mut self) {
        let max_scroll = (self.content_height - self.viewport_height).max(0.0);
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);
    }

    pub fn scroll_by(&mut self, delta: f32) {
        self.scroll_offset -= delta;
        self.clamp_scroll();
    }

    /// Visible Y range in content coordinates.
    pub fn visible_y_range(&self) -> (f32, f32) {
        let top = self.scroll_offset;
        let bottom = self.scroll_offset + self.viewport_height;
        (top, bottom)
    }
}

pub fn draw_context_menu(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    items: &[&str],
) {
    let item_h = 28.0;
    let mut max_w = 120.0f32;
    for item in items {
        max_w = max_w.max(ctx.cache.measure(item, 13.0) + PADDING * 2.0);
    }
    let menu_h = item_h * items.len() as f32;
    let menu_w = max_w;

    // Hard shadow (3 px offset, solid dark).
    fill_rect(
        pixmap,
        x + 3.0,
        y + 3.0,
        menu_w,
        menu_h,
        rgba_to_color(ctx.theme.shadow),
    );
    // 1 px border via over-sized rect + background on top.
    fill_rect(
        pixmap,
        x - 1.0,
        y - 1.0,
        menu_w + 2.0,
        menu_h + 2.0,
        rgba_to_color(ctx.theme.divider),
    );
    fill_rect(pixmap, x, y, menu_w, menu_h, rgba_to_color(ctx.theme.card));

    for (i, label) in items.iter().enumerate() {
        let iy = y + i as f32 * item_h;
        let rect = WidgetRect {
            x,
            y: iy,
            w: menu_w,
            h: item_h,
        };
        if rect.contains(ctx.mouse_x, ctx.mouse_y) {
            fill_rect(
                pixmap,
                x,
                iy,
                menu_w,
                item_h,
                rgba_to_color(ctx.theme.selection),
            );
        }
        // Item separator (skip first row — top border covers it).
        if i > 0 {
            fill_rect(
                pixmap,
                x + 8.0,
                iy,
                menu_w - 16.0,
                1.0,
                rgba_to_color(ctx.theme.divider),
            );
        }
        // "Delete" uses danger color; everything else uses primary.
        let text_color = if *label == "Delete" {
            rgba_to_color(ctx.theme.danger)
        } else {
            rgba_to_color(ctx.theme.text_primary)
        };
        ctx.cache
            .draw(pixmap, label, x + PADDING, iy + 19.0, 13.0, text_color);
    }
}

/// Labels for the history entry context menu (index 1 toggles pin).
pub fn context_menu_labels(is_pinned: bool) -> [&'static str; 3] {
    if is_pinned {
        ["Copy", "Unpin", "Delete"]
    } else {
        ["Copy", "Pin", "Delete"]
    }
}

/// Hit-test an open context menu; returns the clicked item index.
pub fn hit_test_context_menu(
    cache: &mut GlyphCache,
    menu_x: f32,
    menu_y: f32,
    mouse_x: f32,
    mouse_y: f32,
    items: &[&str],
) -> Option<usize> {
    let item_h = 28.0;
    let mut max_w = 120.0f32;
    for item in items {
        max_w = max_w.max(cache.measure(item, 13.0) + PADDING * 2.0);
    }
    let menu_w = max_w;

    for (i, _label) in items.iter().enumerate() {
        let iy = menu_y + i as f32 * item_h;
        let rect = WidgetRect {
            x: menu_x,
            y: iy,
            w: menu_w,
            h: item_h,
        };
        if rect.contains(mouse_x, mouse_y) {
            return Some(i);
        }
    }
    None
}

/// Pixel-art chevron expand / collapse button; returns its rect for external hit-testing.
///
/// A `∨` (expand) or `∧` (collapse) chevron is constructed from five 2×2
/// `fill_rect` blocks — no glyph rendering, no font dependency, perfectly
/// crisp at every DPI.
///
/// Layout of the 10×6 icon centred in `size × size`:
///
/// ```text
/// Expand ∨          Collapse ∧
/// ##         ##     ##  tip  ##   ← outer pair
///    ##   ##              (flipped)
///       ##
/// ```
pub fn draw_expand_button(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    size: f32,
    expanded: bool,
) -> WidgetRect {
    let id = ctx.alloc_id();
    let rect = WidgetRect {
        x,
        y,
        w: size,
        h: size,
    };

    let hovered = ctx.pointer_over(rect);
    if hovered {
        ctx.hot_widget = id;
    }

    let border = rgba_to_color(if hovered {
        ctx.theme.accent
    } else {
        ctx.theme.divider
    });
    fill_rect(pixmap, x, y, size, size, border);
    let bg = rgba_to_color(if hovered {
        ctx.theme.selection
    } else {
        ctx.theme.card
    });
    fill_rect(pixmap, x + 1.0, y + 1.0, size - 2.0, size - 2.0, bg);

    // 10×6 chevron centred inside the button; each mark is a 2×2 pixel block.
    let c = rgba_to_color(if hovered {
        ctx.theme.accent
    } else {
        ctx.theme.text_secondary
    });
    let ix = x + (size - 10.0) / 2.0; // left edge of the 10 px icon
    let iy = y + (size - 6.0) / 2.0; // top edge of the 6 px icon
    let s = 2.0;

    if expanded {
        // ∧  tip at top, arms spread downward
        fill_rect(pixmap, ix + 4.0, iy, s, s, c); // tip
        fill_rect(pixmap, ix + 2.0, iy + 2.0, s, s, c); // mid-left
        fill_rect(pixmap, ix + 6.0, iy + 2.0, s, s, c); // mid-right
        fill_rect(pixmap, ix, iy + 4.0, s, s, c); // outer-left
        fill_rect(pixmap, ix + 8.0, iy + 4.0, s, s, c); // outer-right
    } else {
        // ∨  tip at bottom, arms spread upward
        fill_rect(pixmap, ix, iy, s, s, c); // outer-left
        fill_rect(pixmap, ix + 8.0, iy, s, s, c); // outer-right
        fill_rect(pixmap, ix + 2.0, iy + 2.0, s, s, c); // mid-left
        fill_rect(pixmap, ix + 6.0, iy + 2.0, s, s, c); // mid-right
        fill_rect(pixmap, ix + 4.0, iy + 4.0, s, s, c); // tip
    }

    rect
}

/// Convert top-down RGBA8 pixels into BGRA8 for the GDI DIB back-buffer.
pub fn write_rgba_to_bgra(rgba: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(rgba.len(), dst.len());
    debug_assert_eq!(rgba.len() % 4, 0);
    for (src, out) in rgba.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        out[0] = src[2];
        out[1] = src[1];
        out[2] = src[0];
        out[3] = src[3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_menu_pin_row_hit_matches_draw_layout() {
        let mut cache = GlyphCache::default();
        let items = context_menu_labels(false);
        let menu_x = 100.0;
        let menu_y = 200.0;
        let pin_y = menu_y + 28.0;
        assert_eq!(
            hit_test_context_menu(
                &mut cache,
                menu_x,
                menu_y,
                menu_x + 8.0,
                pin_y + 14.0,
                &items
            ),
            Some(1)
        );
    }
}
