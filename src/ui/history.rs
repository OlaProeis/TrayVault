//! Virtualized history list and entry card rendering.

use std::collections::HashSet;

use crate::ui::pixmap::{blit, fill_rect, Color, Pixmap};

use crate::app::App;
use crate::models::{ClipEntry, EntryKind};
use crate::ui::text::{
    draw_lines, format_relative_time, line_height, truncate_to_width, wrap_text_lines, GlyphCache,
};
use crate::ui::thumb_cache::ThumbCache;
use crate::ui::thumb_loader::{ThumbLoadRequest, ThumbLoader};
use crate::ui::widgets::{card, divider, draw_expand_button, rgba_to_color, WidgetRect, PADDING};

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
/// Minimum fit-box height on narrow cards (px).
pub const IMAGE_THUMB_MAX_HEIGHT: f32 = 120.0;
/// Hard caps so wide windows do not build multi-megapixel list thumbnails.
const MAX_THUMB_WIDTH: f32 = 1200.0;
const MAX_THUMB_HEIGHT: f32 = 900.0;
/// Fit-box height as a fraction of card width (~4:3 screenshots can use full width).
const THUMB_BOX_HEIGHT_RATIO: f32 = 0.85;
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
    let h = IMAGE_THUMB_MAX_HEIGHT
        .max(w * THUMB_BOX_HEIGHT_RATIO)
        .min(MAX_THUMB_HEIGHT);
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
    /// Pre-wrapped preview lines for text/rich-text rows (`None` for image rows).
    pub text_draw_lines: Option<Vec<String>>,
    /// Whether the expand/collapse control is shown (text/rich-text rows only).
    pub text_show_control: bool,
}

/// Unified text-card wrap result: at most two `wrap_text_lines` passes (probe, then reduced width).
#[derive(Clone, Debug)]
struct TextCardLayout {
    show_control: bool,
    draw_lines: Vec<String>,
    visible_line_count: usize,
}

impl TextCardLayout {
    fn height(&self) -> f32 {
        let line_step = line_height(TEXT_PREVIEW_FONT_SIZE);
        let text_block_h = self.visible_line_count as f32 * line_step;
        let meta_h = line_height(TEXT_META_FONT_SIZE);
        (TEXT_CARD_PAD_TOP + text_block_h + TEXT_META_GAP + meta_h + TEXT_CARD_PAD_BOTTOM)
            .max(TEXT_ROW_HEIGHT)
    }
}

/// Compute per-item heights and cumulative Y offsets for the display list.
pub fn build_list_layout(
    display_indices: &[usize],
    entries: &[ClipEntry],
    thumb_max_w: f32,
    cache: &mut GlyphCache,
    expanded_entries: &HashSet<u64>,
    height_cache: &mut crate::ui::EntryHeightCache,
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
        let (height, text_draw_lines, text_show_control) = match entry.kind {
            EntryKind::Image => {
                let h = image_row_height(entry, thumb_max_w);
                (h, None, false)
            }
            EntryKind::Text | EntryKind::RichText => {
                let expanded = expanded_entries.contains(&entry.id);
                let (h, show_control, lines) =
                    cached_text_entry_layout(entry, thumb_max_w, cache, expanded, height_cache);
                (h, Some(lines), show_control)
            }
        };
        let show_divider = has_divider && pos + 1 == pinned_count;
        layouts.push(EntryLayout {
            entry_index,
            y_offset: y,
            height,
            show_pinned_divider_after: show_divider,
            text_draw_lines,
            text_show_control,
        });
        y += height;
        if show_divider {
            y += PINNED_DIVIDER_HEIGHT;
        }
    }
    layouts
}

/// Rebuild list layout only when layout inputs change; otherwise leave `cached_list_layout` as-is.
pub fn refresh_list_layout(app: &App, ui: &mut crate::ui::UiState, thumb_max_w: f32) {
    crate::ui::search::refresh_display_indices(app, ui);
    if crate::ui::list_layout_key_matches(app, ui, thumb_max_w) {
        return;
    }
    ui.entry_height_cache
        .sync_entries_version(app.entries_version);
    ui.cached_list_layout = build_list_layout(
        &ui.display_indices,
        &app.entries,
        thumb_max_w,
        &mut ui.glyph_cache,
        &ui.expanded_text_entries,
        &mut ui.entry_height_cache,
    );
    ui.store_list_layout_key(app, thumb_max_w);
    ui.list_layout_rebuild_count += 1;
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

/// Height of one history row (see [`build_list_layout`] for the primary layout path).
#[allow(dead_code)]
pub fn entry_row_height(
    entry: &ClipEntry,
    thumb_max_w: f32,
    cache: &mut GlyphCache,
    expanded_entries: &HashSet<u64>,
    height_cache: &mut crate::ui::EntryHeightCache,
) -> f32 {
    match entry.kind {
        EntryKind::Image => image_row_height(entry, thumb_max_w),
        EntryKind::Text | EntryKind::RichText => {
            let expanded = expanded_entries.contains(&entry.id);
            cached_text_entry_layout(entry, thumb_max_w, cache, expanded, height_cache).0
        }
    }
}

fn image_row_height(entry: &ClipEntry, thumb_max_w: f32) -> f32 {
    if let Some(img) = &entry.image {
        let thumb_h = scaled_thumb_height(img.width, img.height, thumb_max_w);
        thumb_h + 46.0
    } else {
        TEXT_ROW_HEIGHT
    }
}

fn cached_text_entry_layout(
    entry: &ClipEntry,
    content_width: f32,
    cache: &mut GlyphCache,
    expanded: bool,
    height_cache: &mut crate::ui::EntryHeightCache,
) -> (f32, bool, Vec<String>) {
    let height_key = crate::ui::EntryHeightKey {
        content_hash: entry.hash,
        expanded,
        thumb_max_w_bucket: content_width as u32,
    };
    if let Some(cached) = height_cache.get_layout(entry.id, &height_key) {
        return cached;
    }
    let layout = text_card_layout(cache, entry, content_width, expanded);
    let height = layout.height();
    height_cache.store_layout(
        entry.id,
        height_key,
        height,
        layout.show_control,
        layout.draw_lines.clone(),
    );
    (height, layout.show_control, layout.draw_lines)
}

/// Single wrap pass (or probe + one reduced-width pass when expand control is shown).
fn text_card_layout(
    cache: &mut GlyphCache,
    entry: &ClipEntry,
    content_width: f32,
    expanded: bool,
) -> TextCardLayout {
    let base_w = (content_width - 16.0).max(0.0);
    let text = entry_display_text(entry);

    let probe = wrap_text_lines(cache, text, TEXT_PREVIEW_FONT_SIZE, base_w);
    let show_control = probe.len() > TEXT_COLLAPSED_MAX_LINES;

    let text_w = if show_control {
        let mut reserve = EXPAND_BUTTON_SIZE + 4.0;
        if entry.is_pinned {
            reserve += 7.0 + EXPAND_PIN_GAP;
        }
        (base_w - reserve).max(0.0)
    } else {
        base_w
    };

    let all_lines = if show_control {
        wrap_text_lines(cache, text, TEXT_PREVIEW_FONT_SIZE, text_w)
    } else {
        probe
    };

    let total = all_lines.len();
    let visible_line_count = if expanded || total <= TEXT_COLLAPSED_MAX_LINES {
        total.max(1)
    } else {
        TEXT_COLLAPSED_MAX_LINES
    };

    let draw_lines = if expanded || !show_control {
        all_lines
    } else {
        let mut visible: Vec<String> = all_lines
            .into_iter()
            .take(TEXT_COLLAPSED_MAX_LINES)
            .collect();
        if let Some(last) = visible.last_mut() {
            *last = truncate_to_width(cache, last, TEXT_PREVIEW_FONT_SIZE, text_w);
        }
        visible
    };

    TextCardLayout {
        show_control,
        draw_lines,
        visible_line_count,
    }
}

#[cfg(test)]
fn text_card_height(
    entry: &ClipEntry,
    content_width: f32,
    cache: &mut GlyphCache,
    expanded: bool,
) -> f32 {
    text_card_layout(cache, entry, content_width, expanded).height()
}

/// First layout index whose slot intersects `view_top` (`y_offset + height >= view_top`).
fn first_layout_from_top(layouts: &[EntryLayout], view_top: f32) -> usize {
    let mut lo = 0;
    let mut hi = layouts.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if layouts[mid].y_offset + layouts[mid].height < view_top {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= layouts.len() {
        0
    } else {
        lo
    }
}

/// First layout index strictly below `y` (`y_offset > y`), or `layouts.len()`.
fn first_layout_below_y(layouts: &[EntryLayout], y: f32) -> usize {
    let mut lo = 0;
    let mut hi = layouts.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if layouts[mid].y_offset <= y {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
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

    let start = first_layout_from_top(layouts, view_top);
    let end = first_layout_below_y(layouts, view_bottom);
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
    thumb_loader: &ThumbLoader,
    thumb_load_state: &mut crate::ui::ThumbLoadState,
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
            thumb_loader,
            thumb_load_state,
            thumb_cache,
            content_x + 10.0,
            screen_y + 4.0,
            content_w - 20.0,
            layout.height - 12.0,
            now,
            expanded_entries.contains(&entry.id),
            layout.text_draw_lines.as_deref(),
            layout.text_show_control,
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
    thumb_loader: &ThumbLoader,
    thumb_load_state: &mut crate::ui::ThumbLoadState,
    thumb_cache: &mut ThumbCache,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    now_millis: u64,
    expanded: bool,
    text_draw_lines: Option<&[String]>,
    text_show_control: bool,
    expand_button_rects: &mut Vec<(u64, WidgetRect)>,
) {
    match entry.kind {
        EntryKind::Image => {
            draw_image_card(
                pixmap,
                ctx,
                entry,
                thumb_loader,
                thumb_load_state,
                thumb_cache,
                x,
                y,
                width,
                height,
            );
        }
        EntryKind::Text | EntryKind::RichText => {
            let lines = text_draw_lines.expect("text layout supplies draw lines");
            draw_text_card(
                pixmap,
                ctx,
                entry,
                x,
                y,
                width,
                now_millis,
                expanded,
                lines,
                text_show_control,
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
    lines: &[String],
    show_control: bool,
    expand_button_rects: &mut Vec<(u64, WidgetRect)>,
) {
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
        lines,
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
    thumb_loader: &ThumbLoader,
    thumb_load_state: &mut crate::ui::ThumbLoadState,
    thumb_cache: &mut ThumbCache,
    x: f32,
    y: f32,
    width: f32,
    _height: f32,
) {
    let Some(img) = &entry.image else {
        let layout = text_card_layout(ctx.cache, entry, width, false);
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
            &layout.draw_lines,
            layout.show_control,
            &mut noop,
        );
        return;
    };

    draw_thumbnail(
        pixmap,
        thumb_cache,
        thumb_loader,
        thumb_load_state,
        entry,
        width,
        x,
        y,
        rgba_to_color(ctx.theme.divider),
    );

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

#[allow(clippy::too_many_arguments)]
fn draw_thumbnail(
    pixmap: &mut Pixmap,
    thumb_cache: &mut ThumbCache,
    thumb_loader: &ThumbLoader,
    load_state: &mut crate::ui::ThumbLoadState,
    entry: &ClipEntry,
    max_w: f32,
    x: f32,
    y: f32,
    placeholder: Color,
) {
    let Some(img) = &entry.image else {
        return;
    };

    if let Some(src) = thumb_cache.get(entry.id, img.width, img.height, max_w) {
        blit(pixmap, src.as_ref(), x, y);
        return;
    }

    let (dst_w, dst_h) = thumb_target_size(img.width, img.height, max_w);
    if dst_w == 0 || dst_h == 0 {
        return;
    }

    // Recent captures may still hold pixels in memory — build synchronously (fast path).
    if let Some(pixels) = entry.image_pixels.as_deref() {
        if let Some(src) = thumb_cache.get_or_build(entry.id, pixels, img.width, img.height, max_w)
        {
            blit(pixmap, src.as_ref(), x, y);
        }
        return;
    }

    // Disk-backed image: enqueue async load; never call `read_blob` on the UI thread.
    let key = (entry.id, dst_w, dst_h);
    if !load_state.inflight.contains(&key) {
        load_state.inflight.insert(key);
        thumb_loader.request(ThumbLoadRequest {
            entry_id: entry.id,
            hash: img.hash.clone(),
            img_w: img.width,
            img_h: img.height,
            dst_w,
            dst_h,
            generation: load_state.generation,
        });
    }

    fill_rect(pixmap, x, y, dst_w as f32, dst_h as f32, placeholder);
}

/// Layout index under `content_y` (content-space), if inside card bounds (`height - 8.0`).
fn layout_index_at_content_y(layouts: &[EntryLayout], content_y: f32) -> Option<usize> {
    let mut lo = 0;
    let mut hi = layouts.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if layouts[mid].y_offset <= content_y {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return None;
    }
    let idx = lo - 1;
    let layout = &layouts[idx];
    let card_bottom = layout.y_offset + layout.height - 8.0;
    if content_y >= layout.y_offset && content_y < card_bottom {
        Some(idx)
    } else {
        None
    }
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
    if layouts.is_empty() {
        return None;
    }
    let content_y = y - viewport_top + scroll_offset;
    let idx = layout_index_at_content_y(layouts, content_y)?;
    let layout = &layouts[idx];
    let screen_y = viewport_top + layout.y_offset - scroll_offset;
    let rect = WidgetRect {
        x: content_x,
        y: screen_y,
        w: content_w,
        h: layout.height - 8.0,
    };
    if rect.contains(x, y) {
        Some(layout.entry_index)
    } else {
        None
    }
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

    #[cfg(test)]
    fn visible_layout_range_linear(
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

    #[cfg(test)]
    fn hit_test_entry_linear(
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

    fn synthetic_layouts(heights: &[f32]) -> Vec<EntryLayout> {
        let mut y = 0.0f32;
        heights
            .iter()
            .enumerate()
            .map(|(i, &h)| {
                let layout = EntryLayout {
                    entry_index: i,
                    y_offset: y,
                    height: h,
                    show_pinned_divider_after: false,
                    text_draw_lines: None,
                    text_show_control: false,
                };
                y += h;
                layout
            })
            .collect()
    }

    #[test]
    fn visible_layout_range_matches_linear_reference() {
        let layouts = synthetic_layouts(&[40.0, 80.0, 30.0, 120.0, 50.0]);
        let scroll_tops = [0.0, 25.0, 100.0, 200.0, 500.0];
        let viewport_heights = [50.0, 120.0, 200.0, 400.0];
        for &scroll_top in &scroll_tops {
            for &viewport_h in &viewport_heights {
                assert_eq!(
                    visible_layout_range(&layouts, scroll_top, viewport_h),
                    visible_layout_range_linear(&layouts, scroll_top, viewport_h),
                    "scroll_top={scroll_top} viewport_h={viewport_h}"
                );
            }
        }
    }

    #[test]
    fn hit_test_entry_matches_linear_reference() {
        let layouts = synthetic_layouts(&[60.0, 90.0, 45.0]);
        let content_top = 80.0;
        let content_x = 10.0;
        let content_w = 260.0;
        let scroll_offsets = [0.0, 30.0, 70.0];
        let xs = [
            content_x + 5.0,
            content_x + content_w - 1.0,
            content_x - 1.0,
        ];
        for &scroll_offset in &scroll_offsets {
            for &x in &xs {
                for y in (0..300).step_by(7) {
                    let y = y as f32;
                    assert_eq!(
                        hit_test_entry(
                            &layouts,
                            scroll_offset,
                            content_top,
                            content_x,
                            content_w,
                            x,
                            y,
                        ),
                        hit_test_entry_linear(
                            &layouts,
                            scroll_offset,
                            content_top,
                            content_x,
                            content_w,
                            x,
                            y,
                        ),
                        "scroll={scroll_offset} x={x} y={y}"
                    );
                }
            }
        }
    }

    #[test]
    fn hit_test_entry_matches_linear_on_built_layouts() {
        let entries: Vec<_> = (0..12).map(text_entry).collect();
        let indices: Vec<_> = (0..12).collect();
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        let layouts = build_list_layout(
            &indices,
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        let content_top = 100.0;
        let content_x = PADDING;
        let content_w = 280.0;
        for scroll_offset in [0.0, 50.0, 150.0] {
            for y in (0..800).step_by(5) {
                let y = y as f32;
                let x = content_x + 40.0;
                assert_eq!(
                    hit_test_entry(
                        &layouts,
                        scroll_offset,
                        content_top,
                        content_x,
                        content_w,
                        x,
                        y,
                    ),
                    hit_test_entry_linear(
                        &layouts,
                        scroll_offset,
                        content_top,
                        content_x,
                        content_w,
                        x,
                        y,
                    ),
                    "scroll={scroll_offset} y={y}"
                );
            }
        }
    }

    #[test]
    fn virtualization_limits_visible_range() {
        let entries: Vec<_> = (0..100).map(text_entry).collect();
        let indices: Vec<_> = (0..100).collect();
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        let layouts = build_list_layout(
            &indices,
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
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
        assert!(h <= 900);
    }

    #[test]
    fn tall_screenshot_uses_full_card_width() {
        let max_w = 856.0;
        let (w, h) = thumb_target_size(1817, 1476, max_w);
        assert!(
            w >= 850,
            "4:3-ish screenshot should fill card width, got {w}×{h}"
        );
        assert!(h >= 690, "expected ~full-width scale, got {w}×{h}");
    }

    #[test]
    fn hit_test_uses_card_bounds_not_full_row() {
        let entries: Vec<_> = (0..2).map(text_entry).collect();
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        let layouts = build_list_layout(
            &[0, 1],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
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
        let mut height_cache = crate::ui::EntryHeightCache::default();
        let layouts = build_list_layout(
            &[0, 1],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        assert!(layouts[0].show_pinned_divider_after);
        assert!(!layouts[1].show_pinned_divider_after);
    }

    #[test]
    fn refresh_list_layout_skips_rebuild_on_cache_hit() {
        use crate::app::App;
        use crate::config::Config;
        use crate::store::{LoadResult, Store};
        use std::fs;

        let dir =
            std::env::temp_dir().join(format!("trayvault-layout-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let mut app = App::new(
            Config::default(),
            LoadResult::default(),
            Store::open_for_test(dir.clone()),
        );
        for id in 0..5 {
            app.entries.push(text_entry(id));
        }
        app.entries_version = 1;

        let mut ui = crate::ui::UiState::default();
        let thumb_max_w = 280.0;

        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 1);
        assert_eq!(ui.cached_list_layout.len(), 5);
        let first_y = ui.cached_list_layout[0].y_offset;

        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 1);
        assert_eq!(ui.cached_list_layout.len(), 5);
        assert_eq!(ui.cached_list_layout[0].y_offset, first_y);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_list_layout_rebuilds_on_input_change() {
        use crate::app::App;
        use crate::config::Config;
        use crate::store::{LoadResult, Store};
        use std::fs;

        let dir =
            std::env::temp_dir().join(format!("trayvault-layout-cache-inv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let mut app = App::new(
            Config::default(),
            LoadResult::default(),
            Store::open_for_test(dir.clone()),
        );
        app.entries.push(text_entry(0));
        app.entries.push(text_entry(1));
        app.entries_version = 1;

        let mut ui = crate::ui::UiState::default();
        let thumb_max_w = 280.0;

        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 1);

        app.entries_version = app.entries_version.wrapping_add(1);
        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 2);

        ui.filter = crate::ui::EntryFilter::Text;
        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 3);

        app.filter_query = "entry-1".into();
        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 4);

        ui.bump_expanded_version();
        refresh_list_layout(&app, &mut ui, thumb_max_w);
        assert_eq!(ui.list_layout_rebuild_count, 5);

        refresh_list_layout(&app, &mut ui, 400.0);
        assert_eq!(ui.list_layout_rebuild_count, 6);

        let _ = fs::remove_dir_all(&dir);
    }

    fn long_text_entry(id: u64) -> ClipEntry {
        let text = "word ".repeat(80);
        ClipEntry {
            id,
            created_at: id,
            kind: EntryKind::Text,
            text: Some(text.clone()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash: hash_text(&text),
        }
    }

    #[test]
    fn entry_height_cache_hits_on_repeated_layout_build() {
        let entries = vec![long_text_entry(1), long_text_entry(2)];
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        height_cache.sync_entries_version(1);

        let layouts_a = build_list_layout(
            &[0, 1],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        let misses_first = height_cache.miss_count;
        let hits_first = height_cache.hit_count;
        assert_eq!(misses_first, 2);
        assert_eq!(hits_first, 0);

        let layouts_b = build_list_layout(
            &[0, 1],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        assert_eq!(layouts_a[0].height, layouts_b[0].height);
        assert_eq!(layouts_a[1].height, layouts_b[1].height);
        assert_eq!(height_cache.miss_count, misses_first);
        assert_eq!(height_cache.hit_count, hits_first + 2);
    }

    #[test]
    fn entry_height_cache_misses_on_expand_or_width_change() {
        let entries = vec![long_text_entry(1)];
        let mut cache = GlyphCache::default();
        let mut expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        height_cache.sync_entries_version(1);

        let collapsed_h = build_list_layout(
            &[0],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        )[0]
        .height;
        assert_eq!(height_cache.miss_count, 1);

        expanded.insert(1);
        let expanded_h = build_list_layout(
            &[0],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        )[0]
        .height;
        assert!(expanded_h >= collapsed_h);
        assert_eq!(height_cache.miss_count, 2);

        let narrow_h = build_list_layout(
            &[0],
            &entries,
            200.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        )[0]
        .height;
        assert!(narrow_h >= expanded_h);
        assert_eq!(height_cache.miss_count, 3);
    }

    #[test]
    fn entry_height_cache_clears_on_entries_version_bump() {
        let entries = vec![long_text_entry(1)];
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        height_cache.sync_entries_version(1);

        build_list_layout(
            &[0],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        assert_eq!(height_cache.miss_count, 1);
        assert_eq!(height_cache.hit_count, 0);

        height_cache.sync_entries_version(2);
        build_list_layout(
            &[0],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        assert_eq!(height_cache.miss_count, 2);
        assert_eq!(height_cache.hit_count, 0);
    }

    #[test]
    fn text_card_height_matches_unified_layout() {
        let mut cache = GlyphCache::default();
        let short = text_entry(1);
        let long = long_text_entry(2);

        for (entry, expanded) in [(&short, false), (&long, false), (&long, true)] {
            let via_helper = text_card_height(entry, 280.0, &mut cache, expanded);
            let via_layout = text_card_layout(&mut cache, entry, 280.0, expanded).height();
            assert!(
                (via_helper - via_layout).abs() < f32::EPSILON,
                "entry {} expanded={expanded}",
                entry.id
            );
        }

        let short_h = text_card_height(&short, 280.0, &mut cache, false);
        assert!(short_h >= TEXT_ROW_HEIGHT);
        let collapsed = text_card_height(&long, 280.0, &mut cache, false);
        let expanded_h = text_card_height(&long, 280.0, &mut cache, true);
        assert!(expanded_h >= collapsed);
    }

    #[test]
    fn entry_row_height_matches_build_list_layout() {
        let entries = vec![long_text_entry(1)];
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        height_cache.sync_entries_version(1);

        let layout_h = build_list_layout(
            &[0],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        )[0]
        .height;
        height_cache.sync_entries_version(1);
        let row_h = entry_row_height(&entries[0], 280.0, &mut cache, &expanded, &mut height_cache);
        assert!((layout_h - row_h).abs() < f32::EPSILON);
    }

    #[test]
    fn list_layout_carries_prewrapped_draw_lines() {
        let entries = vec![long_text_entry(1)];
        let mut cache = GlyphCache::default();
        let expanded = HashSet::new();
        let mut height_cache = crate::ui::EntryHeightCache::default();
        height_cache.sync_entries_version(1);

        let layouts = build_list_layout(
            &[0],
            &entries,
            280.0,
            &mut cache,
            &expanded,
            &mut height_cache,
        );
        let lines = layouts[0]
            .text_draw_lines
            .as_ref()
            .expect("text row should carry draw lines");
        assert!(layouts[0].text_show_control);
        assert_eq!(lines.len(), TEXT_COLLAPSED_MAX_LINES);
    }
}
