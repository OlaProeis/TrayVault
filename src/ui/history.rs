//! Virtualized history list and entry card rendering.

use std::collections::HashSet;

use crate::ui::pixmap::{blit, Pixmap};

use crate::app::App;
use crate::models::{ClipEntry, EntryKind};
use crate::store::Store;
use crate::ui::text::{
    draw_lines, format_relative_time, line_height, truncate_to_width, wrap_text_lines, GlyphCache,
};
use crate::ui::thumb_cache::ThumbCache;
use crate::ui::widgets::{
    card, divider, draw_expand_button, fill_rect, rgba_to_color, WidgetRect, PADDING,
};

pub const TEXT_ROW_HEIGHT: f32 = 66.0;
pub const TEXT_PREVIEW_FONT_SIZE: f32 = 14.0;
pub const TEXT_META_FONT_SIZE: f32 = 12.0;
pub const TEXT_COLLAPSED_MAX_LINES: usize = 10;
const TEXT_CARD_PAD_TOP: f32 = 8.0;
const TEXT_CARD_PAD_BOTTOM: f32 = 18.0;
const TEXT_META_GAP: f32 = 6.0;
const EXPAND_BUTTON_TOP_PAD: f32 = 2.0;
const EXPAND_BUTTON_SIZE: f32 = 20.0;
/// Space between expand button and pin badge when both are shown.
const EXPAND_PIN_GAP: f32 = 4.0;
pub const IMAGE_THUMB_MAX_HEIGHT: f32 = 120.0;
/// Hard caps so wide windows do not build multi-megapixel list thumbnails.
const MAX_THUMB_WIDTH: f32 = 800.0;
/// Height cap must allow ~800px-wide landscape thumbs (width-only bumps had no visible effect).
const MAX_THUMB_HEIGHT: f32 = 520.0;
/// Horizontal inset inside a history card (matches `draw_history_list` x + 10 each side).
pub const CARD_INNER_PAD: f32 = 20.0;
pub const PINNED_DIVIDER_HEIGHT: f32 = 24.0;
pub const VIRTUAL_MARGIN: f32 = 80.0;

/// Draw a 7×7 diamond (rotated square) pin badge centred at `(cx, cy)`.
fn draw_pin_badge(pixmap: &mut Pixmap, cx: f32, cy: f32, color: crate::ui::pixmap::Color) {
    fill_rect(pixmap, cx + 3.0, cy, 1.0, 1.0, color);
    fill_rect(pixmap, cx + 2.0, cy + 1.0, 3.0, 1.0, color);
    fill_rect(pixmap, cx + 1.0, cy + 2.0, 5.0, 1.0, color);
    fill_rect(pixmap, cx, cy + 3.0, 7.0, 1.0, color);
    fill_rect(pixmap, cx + 1.0, cy + 4.0, 5.0, 1.0, color);
    fill_rect(pixmap, cx + 2.0, cy + 5.0, 3.0, 1.0, color);
    fill_rect(pixmap, cx + 3.0, cy + 6.0, 1.0, 1.0, color);
}

/// Inner width available for entry content (thumbnail/text) from client width.
pub fn entry_inner_width(client_width: f32) -> f32 {
    let content_w = (client_width - PADDING * 2.0).max(0.0);
    (content_w - CARD_INNER_PAD).max(0.0)
}

/// Fit box for list thumbnails (grows with card width, capped for CPU/memory).
fn thumb_fit_box(max_w: f32) -> (f32, f32) {
    let w = max_w.clamp(0.0, MAX_THUMB_WIDTH);
    let h = IMAGE_THUMB_MAX_HEIGHT.max(w * 0.65).min(MAX_THUMB_HEIGHT);
    (w, h)
}

/// Pixel size after scale-to-fit (for layout and thumbnail cache keys).
pub(crate) fn thumb_target_size(img_w: u32, img_h: u32, max_w: f32) -> (u32, u32) {
    if img_w == 0 || img_h == 0 {
        return (0, 0);
    }
    let scale = image_thumb_scale(img_w, img_h, max_w);
    (
        ((img_w as f32 * scale).round() as u32).max(1),
        ((img_h as f32 * scale).round() as u32).max(1),
    )
}

pub(crate) fn image_thumb_scale(img_w: u32, img_h: u32, max_w: f32) -> f32 {
    if img_w == 0 || img_h == 0 || max_w <= 0.0 {
        return 1.0;
    }
    let (box_w, box_h) = thumb_fit_box(max_w);
    (box_w / img_w as f32).min(box_h / img_h as f32)
}

fn scaled_thumb_height(img_w: u32, img_h: u32, max_w: f32) -> f32 {
    if img_h == 0 {
        return thumb_fit_box(max_w).1;
    }
    image_thumb_scale(img_w, img_h, max_w) * img_h as f32
}

/// Result of drawing the visible history slice: visible count plus optional right-click target.
pub type HistoryDrawResult = (usize, Option<(u64, f32, f32)>);

/// Layout metadata for one displayed entry.
#[derive(Clone, Debug)]
pub struct EntryLayout {
    pub entry_index: usize,
    pub y_offset: f32,
    pub height: f32,
    pub show_pinned_divider_after: bool,
}

/// Compute per-item heights and cumulative Y offsets for the display list.
pub fn build_list_layout(
    display_indices: &[usize],
    entries: &[ClipEntry],
    thumb_max_w: f32,
    cache: &mut GlyphCache,
    expanded_entries: &HashSet<u64>,
) -> Vec<EntryLayout> {
    let mut layouts = Vec::with_capacity(display_indices.len());
    let mut y = 0.0f32;
    let pinned_count = display_indices
        .iter()
        .filter(|&&idx| entries[idx].is_pinned)
        .count();
    let has_divider = pinned_count > 0 && pinned_count < display_indices.len();

    for (pos, &entry_index) in display_indices.iter().enumerate() {
        let entry = &entries[entry_index];
        let height = entry_row_height(entry, thumb_max_w, cache, expanded_entries);
        let show_divider = has_divider && pos + 1 == pinned_count;
        layouts.push(EntryLayout {
            entry_index,
            y_offset: y,
            height,
            show_pinned_divider_after: show_divider,
        });
        y += height;
        if show_divider {
            y += PINNED_DIVIDER_HEIGHT;
        }
    }
    layouts
}

pub fn total_content_height(layouts: &[EntryLayout]) -> f32 {
    layouts
        .last()
        .map(|l| {
            l.y_offset
                + l.height
                + if l.show_pinned_divider_after {
                    PINNED_DIVIDER_HEIGHT
                } else {
                    0.0
                }
        })
        .unwrap_or(0.0)
}

pub fn entry_row_height(
    entry: &ClipEntry,
    thumb_max_w: f32,
    cache: &mut GlyphCache,
    expanded_entries: &HashSet<u64>,
) -> f32 {
    match entry.kind {
        EntryKind::Image => {
            if let Some(img) = &entry.image {
                let thumb_h = scaled_thumb_height(img.width, img.height, thumb_max_w);
                thumb_h + 46.0
            } else {
                TEXT_ROW_HEIGHT
            }
        }
        EntryKind::Text | EntryKind::RichText => text_card_height(
            entry,
            thumb_max_w,
            cache,
            expanded_entries.contains(&entry.id),
        ),
    }
}

/// Text wrap width and whether the expand/collapse control is shown (top-right of card).
fn text_wrap_layout(content_width: f32, cache: &mut GlyphCache, entry: &ClipEntry) -> (f32, bool) {
    let base_w = (content_width - 16.0).max(0.0);
    let probe = wrap_text_lines(
        cache,
        entry_display_text(entry),
        TEXT_PREVIEW_FONT_SIZE,
        base_w,
    );
    let show_control = probe.len() > TEXT_COLLAPSED_MAX_LINES;
    if !show_control {
        return (base_w, false);
    }
    let mut reserve = EXPAND_BUTTON_SIZE + 4.0;
    if entry.is_pinned {
        reserve += 7.0 + EXPAND_PIN_GAP;
    }
    ((base_w - reserve).max(0.0), true)
}

fn text_card_height(
    entry: &ClipEntry,
    content_width: f32,
    cache: &mut GlyphCache,
    expanded: bool,
) -> f32 {
    let (text_w, _) = text_wrap_layout(content_width, cache, entry);
    let visible_lines = text_preview_visible_lines(cache, entry, text_w, expanded);
    let line_step = line_height(TEXT_PREVIEW_FONT_SIZE);
    let text_block_h = visible_lines as f32 * line_step;
    let meta_h = line_height(TEXT_META_FONT_SIZE);
    (TEXT_CARD_PAD_TOP + text_block_h + TEXT_META_GAP + meta_h + TEXT_CARD_PAD_BOTTOM)
        .max(TEXT_ROW_HEIGHT)
}

fn text_preview_visible_lines(
    cache: &mut GlyphCache,
    entry: &ClipEntry,
    text_width: f32,
    expanded: bool,
) -> usize {
    let all_lines = wrap_text_lines(
        cache,
        entry_display_text(entry),
        TEXT_PREVIEW_FONT_SIZE,
        text_width,
    );
    let total = all_lines.len();
    if expanded || total <= TEXT_COLLAPSED_MAX_LINES {
        total.max(1)
    } else {
        TEXT_COLLAPSED_MAX_LINES
    }
}

fn visible_text_lines(
    cache: &mut GlyphCache,
    entry: &ClipEntry,
    text_width: f32,
    expanded: bool,
) -> (Vec<String>, bool) {
    let all_lines = wrap_text_lines(
        cache,
        entry_display_text(entry),
        TEXT_PREVIEW_FONT_SIZE,
        text_width,
    );
    let total = all_lines.len();
    let show_control = total > TEXT_COLLAPSED_MAX_LINES;
    if expanded || !show_control {
        return (all_lines, show_control);
    }

    let mut visible: Vec<String> = all_lines
        .into_iter()
        .take(TEXT_COLLAPSED_MAX_LINES)
        .collect();
    if let Some(last) = visible.last_mut() {
        *last = truncate_to_width(cache, last, TEXT_PREVIEW_FONT_SIZE, text_width);
    }
    (visible, true)
}

/// Return layout indices that intersect the viewport (plus margin).
pub fn visible_layout_range(
    layouts: &[EntryLayout],
    scroll_top: f32,
    viewport_h: f32,
) -> (usize, usize) {
    if layouts.is_empty() {
        return (0, 0);
    }
    let view_top = (scroll_top - VIRTUAL_MARGIN).max(0.0);
    let view_bottom = scroll_top + viewport_h + VIRTUAL_MARGIN;

    let start = layouts
        .iter()
        .position(|l| l.y_offset + l.height >= view_top)
        .unwrap_or(0);
    let end = layouts
        .iter()
        .position(|l| l.y_offset > view_bottom)
        .unwrap_or(layouts.len());
    (start, end.max(start + 1).min(layouts.len()))
}

pub fn entry_display_text(entry: &ClipEntry) -> &str {
    match entry.kind {
        EntryKind::Image => "(empty)",
        EntryKind::RichText | EntryKind::Text => entry.text.as_deref().unwrap_or("(empty)").trim(),
    }
}

pub fn entry_preview(entry: &ClipEntry) -> String {
    match entry.kind {
        EntryKind::Image => {
            if let Some(img) = &entry.image {
                format!("Image {}×{}", img.width, img.height)
            } else {
                "Image".into()
            }
        }
        EntryKind::RichText | EntryKind::Text => entry_display_text(entry).to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_history_list(
    pixmap: &mut Pixmap,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    app: &App,
    store: &Store,
    thumb_cache: &mut ThumbCache,
    layouts: &[EntryLayout],
    scroll_offset: f32,
    viewport_top: f32,
    viewport_h: f32,
    content_x: f32,
    content_w: f32,
    mouse_x: f32,
    mouse_y: f32,
    mouse_right_down: bool,
    expanded_entries: &HashSet<u64>,
    expand_button_rects: &mut Vec<(u64, WidgetRect)>,
) -> HistoryDrawResult {
    expand_button_rects.clear();
    let (start, end) = visible_layout_range(layouts, scroll_offset, viewport_h);
    let now = ClipEntry::now_millis();
    let mut right_click = None;

    for layout in &layouts[start..end] {
        let entry = &app.entries[layout.entry_index];
        let screen_y = viewport_top + layout.y_offset - scroll_offset;
        if screen_y + layout.height < viewport_top || screen_y > viewport_top + viewport_h {
            continue;
        }

        let selected = app.selected_index == layout.entry_index;
        let card_rect = card(
            ctx,
            pixmap,
            content_x,
            screen_y,
            content_w,
            layout.height - 8.0,
            selected,
        );

        draw_entry_content(
            pixmap,
            ctx,
            entry,
            store,
            thumb_cache,
            content_x + 10.0,
            screen_y + 4.0,
            content_w - 20.0,
            layout.height - 12.0,
            now,
            expanded_entries.contains(&entry.id),
            expand_button_rects,
        );

        if card_rect.contains(mouse_x, mouse_y) && mouse_right_down {
            right_click = Some((entry.id, mouse_x, mouse_y));
        }

        if layout.show_pinned_divider_after {
            let div_y = screen_y + layout.height + 4.0;
            divider(ctx, pixmap, content_x, div_y, content_w);
            let label = "Pinned";
            ctx.cache.draw(
                pixmap,
                label,
                content_x + PADDING,
                div_y + 16.0,
                11.0,
                rgba_to_color(ctx.theme.text_secondary),
            );
        }
    }

    (end - start, right_click)
}

#[allow(clippy::too_many_arguments)]
fn draw_entry_content(
    pixmap: &mut Pixmap,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    entry: &ClipEntry,
    store: &Store,
    thumb_cache: &mut ThumbCache,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    now_millis: u64,
    expanded: bool,
    expand_button_rects: &mut Vec<(u64, WidgetRect)>,
) {
    match entry.kind {
        EntryKind::Image => {
            draw_image_card(pixmap, ctx, entry, store, thumb_cache, x, y, width, height)
        }
        EntryKind::Text | EntryKind::RichText => {
            draw_text_card(
                pixmap,
                ctx,
                entry,
                x,
                y,
                width,
                now_millis,
                expanded,
                expand_button_rects,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text_card(
    pixmap: &mut Pixmap,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    entry: &ClipEntry,
    x: f32,
    y: f32,
    width: f32,
    now_millis: u64,
    expanded: bool,
    expand_button_rects: &mut Vec<(u64, WidgetRect)>,
) {
    let (text_w, show_control) = text_wrap_layout(width, ctx.cache, entry);
    let (lines, _) = visible_text_lines(ctx.cache, entry, text_w, expanded);
    let line_step = line_height(TEXT_PREVIEW_FONT_SIZE);
    let first_baseline = y + TEXT_CARD_PAD_TOP + TEXT_PREVIEW_FONT_SIZE;

    let btn_x = if show_control {
        x + width - EXPAND_BUTTON_SIZE
    } else {
        x + width
    };

    draw_lines(
        ctx.cache,
        pixmap,
        &lines,
        x,
        first_baseline,
        TEXT_PREVIEW_FONT_SIZE,
        rgba_to_color(ctx.theme.text_primary),
    );

    if show_control {
        let btn_y = y + EXPAND_BUTTON_TOP_PAD;
        let rect = draw_expand_button(ctx, pixmap, btn_x, btn_y, EXPAND_BUTTON_SIZE, expanded);
        expand_button_rects.push((entry.id, rect));
    }

    let meta_y = first_baseline + lines.len() as f32 * line_step + TEXT_META_GAP;

    let mut meta = format_relative_time(entry.created_at, now_millis);
    if let Some(app) = &entry.source_app {
        meta = format!("{meta} · {app}");
    }
    ctx.cache.draw(
        pixmap,
        &meta,
        x,
        meta_y,
        TEXT_META_FONT_SIZE,
        rgba_to_color(ctx.theme.text_secondary),
    );

    if entry.is_pinned {
        let pin_x = if show_control {
            btn_x - EXPAND_PIN_GAP - 3.0
        } else {
            x + width - 15.0
        };
        draw_pin_badge(pixmap, pin_x, y + 3.0, rgba_to_color(ctx.theme.pinned));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_image_card(
    pixmap: &mut Pixmap,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    entry: &ClipEntry,
    store: &Store,
    thumb_cache: &mut ThumbCache,
    x: f32,
    y: f32,
    width: f32,
    _height: f32,
) {
    let Some(img) = &entry.image else {
        let mut noop = Vec::new();
        draw_text_card(
            pixmap,
            ctx,
            entry,
            x,
            y,
            width,
            ClipEntry::now_millis(),
            false,
            &mut noop,
        );
        return;
    };

    draw_thumbnail(pixmap, thumb_cache, entry, store, width, x, y);

    let label = format!("{}×{}", img.width, img.height);
    ctx.cache.draw(
        pixmap,
        &label,
        x,
        y + scaled_thumb_height(img.width, img.height, width) + 14.0,
        TEXT_META_FONT_SIZE,
        rgba_to_color(ctx.theme.text_secondary),
    );

    if entry.is_pinned {
        draw_pin_badge(
            pixmap,
            x + width - 15.0,
            y + 3.0,
            rgba_to_color(ctx.theme.pinned),
        );
    }
}

fn draw_thumbnail(
    pixmap: &mut Pixmap,
    thumb_cache: &mut ThumbCache,
    entry: &ClipEntry,
    store: &Store,
    max_w: f32,
    x: f32,
    y: f32,
) {
    let Some(img) = &entry.image else {
        return;
    };

    if let Some(src) = thumb_cache.get(entry.id, img.width, img.height, max_w) {
        blit(pixmap, src.as_ref(), x, y);
        return;
    }

    let owned_blob;
    let pixels: Option<&[u8]> = match entry.image_pixels.as_deref() {
        Some(p) => Some(p),
        None => {
            owned_blob = store.read_blob(&img.hash);
            owned_blob.as_deref()
        }
    };

    let Some(pixels) = pixels else {
        return;
    };

    let Some(src) = thumb_cache.get_or_build(entry.id, pixels, img.width, img.height, max_w) else {
        return;
    };
    blit(pixmap, src.as_ref(), x, y);
}

/// Hit-test using the same card bounds as [`draw_history_list`] / [`card`](crate::ui::widgets::card).
pub fn hit_test_entry(
    layouts: &[EntryLayout],
    scroll_offset: f32,
    viewport_top: f32,
    content_x: f32,
    content_w: f32,
    x: f32,
    y: f32,
) -> Option<usize> {
    for layout in layouts {
        let screen_y = viewport_top + layout.y_offset - scroll_offset;
        let rect = WidgetRect {
            x: content_x,
            y: screen_y,
            w: content_w,
            h: layout.height - 8.0,
        };
        if rect.contains(x, y) {
            return Some(layout.entry_index);
        }
    }
    None
}

pub fn ensure_index_visible(
    layouts: &[EntryLayout],
    entry_index: usize,
    scroll_offset: f32,
    viewport_h: f32,
) -> f32 {
    let Some(layout) = layouts.iter().find(|l| l.entry_index == entry_index) else {
        return scroll_offset;
    };
    let item_top = layout.y_offset;
    let item_bottom = item_top + layout.height;
    if item_top < scroll_offset {
        return item_top;
    }
    if item_bottom > scroll_offset + viewport_h {
        return (item_bottom - viewport_h).max(0.0);
    }
    scroll_offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_text;
    use crate::models::{ClipEntry, EntryKind};

    fn text_entry(id: u64) -> ClipEntry {
        ClipEntry {
            id,
            created_at: id,
            kind: EntryKind::Text,
            text: Some(format!("entry-{id}")),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash: hash_text(&format!("entry-{id}")),
        }
    }

    #[test]
    fn virtualization_limits_visible_range() {
        let entries: Vec<_> = (0..100).map(text_entry).collect();
        let indices: Vec<_> = (0..100).collect();
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let layouts = build_list_layout(&indices, &entries, 280.0, &mut cache, &expanded);
        let total_h = total_content_height(&layouts);
        assert!(total_h > 500.0);

        let (start, end) = visible_layout_range(&layouts, 0.0, 200.0);
        assert!(end - start < 20);
        assert!(end <= 100);
    }

    #[test]
    fn wide_image_thumb_scales_to_max_width() {
        let max_w = 200.0;
        let scale = image_thumb_scale(800, 100, max_w);
        assert!((scale - 0.25).abs() < f32::EPSILON);
        let h = scaled_thumb_height(800, 100, max_w);
        assert!((h - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn landscape_thumb_fills_width_up_to_cap() {
        let max_w = 800.0;
        let (w, h) = thumb_target_size(1354, 910, max_w);
        assert!(w >= 700, "width cap should dominate at 800px card, got {w}");
        assert!(h <= 520);
    }

    #[test]
    fn hit_test_uses_card_bounds_not_full_row() {
        let entries: Vec<_> = (0..2).map(text_entry).collect();
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let layouts = build_list_layout(&[0, 1], &entries, 280.0, &mut cache, &expanded);
        let content_top = 100.0;
        let content_x = 12.0;
        let content_w = 280.0;
        let row_h = layouts[0].height;

        // Inside first card body.
        assert_eq!(
            hit_test_entry(
                &layouts,
                0.0,
                content_top,
                content_x,
                content_w,
                20.0,
                content_top
            ),
            Some(0)
        );
        // Bottom 8px of row slot is outside the card (matches draw hover).
        assert_eq!(
            hit_test_entry(
                &layouts,
                0.0,
                content_top,
                content_x,
                content_w,
                20.0,
                content_top + row_h - 2.0
            ),
            None
        );
        // Second card.
        assert_eq!(
            hit_test_entry(
                &layouts,
                0.0,
                content_top,
                content_x,
                content_w,
                20.0,
                content_top + layouts[1].y_offset
            ),
            Some(1)
        );
    }

    #[test]
    fn pinned_divider_placed_after_last_pinned() {
        let mut e0 = text_entry(0);
        e0.is_pinned = true;
        let e1 = text_entry(1);
        let entries = vec![e0, e1];
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let layouts = build_list_layout(&[0, 1], &entries, 280.0, &mut cache, &expanded);
        assert!(layouts[0].show_pinned_divider_after);
        assert!(!layouts[1].show_pinned_divider_after);
    }
}
