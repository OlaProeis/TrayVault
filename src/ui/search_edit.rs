//! Search field text layout: metrics, horizontal scroll, and selection range.

use crate::ui::text::GlyphCache;
use crate::ui::widgets::WidgetRect;

/// Must match [`crate::ui::widgets::input_box`] draw size for the title-bar search field.
pub const SEARCH_TEXT_SIZE_PX: f32 = 14.0;
pub const SEARCH_PAD_X: f32 = 8.0;

/// Inner width available for glyphs inside the search box.
pub fn search_text_inner_width(rect: WidgetRect) -> f32 {
    (rect.w - SEARCH_PAD_X * 2.0).max(0.0)
}

/// Left edge where search text is drawn (client coords).
pub fn search_text_x(rect: WidgetRect) -> f32 {
    rect.x + SEARCH_PAD_X
}

/// Horizontal scroll so the caret stays visible when the query overflows the box.
pub fn search_scroll_x(cache: &mut GlyphCache, query: &str, inner_w: f32, caret: usize) -> f32 {
    if inner_w <= 0.0 || query.is_empty() {
        return 0.0;
    }
    let total = cache.measure(query, SEARCH_TEXT_SIZE_PX);
    if total <= inner_w {
        return 0.0;
    }
    let caret_x = cache.measure(&query[..caret.min(query.len())], SEARCH_TEXT_SIZE_PX);
    let max_scroll = (total - inner_w).max(0.0);
    if caret_x + 2.0 <= inner_w {
        0.0
    } else {
        (caret_x - inner_w + 8.0).clamp(0.0, max_scroll)
    }
}

/// Pixel X for the caret bar in client coordinates.
pub fn search_caret_pixel_x(
    cache: &mut GlyphCache,
    query: &str,
    rect: WidgetRect,
    caret: usize,
    scroll_x: f32,
) -> f32 {
    let text_x = search_text_x(rect);
    let prefix = &query[..caret.min(query.len())];
    text_x + cache.measure(prefix, SEARCH_TEXT_SIZE_PX) - scroll_x
}

/// Map a click inside the search field to a UTF-8 byte index (accounts for scroll).
pub fn caret_at_click(
    cache: &mut GlyphCache,
    query: &str,
    rect: WidgetRect,
    click_x: f32,
) -> usize {
    let text_x = search_text_x(rect);
    let inner_w = search_text_inner_width(rect);
    let rough = cache.caret_index_from_x(query, SEARCH_TEXT_SIZE_PX, click_x, text_x);
    let scroll = search_scroll_x(cache, query, inner_w, rough);
    cache.caret_index_from_x(query, SEARCH_TEXT_SIZE_PX, click_x, text_x - scroll)
}

/// Normalized `(start, end)` byte range; empty when `start == end`.
pub fn selection_range(anchor: usize, caret: usize) -> (usize, usize) {
    if anchor == caret {
        (caret, caret)
    } else if anchor < caret {
        (anchor, caret)
    } else {
        (caret, anchor)
    }
}

pub fn has_selection(anchor: usize, caret: usize) -> bool {
    anchor != caret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::text::GlyphCache;

    #[test]
    fn scroll_zero_when_text_fits() {
        let mut cache = GlyphCache::default();
        let scroll = search_scroll_x(&mut cache, "hi", 200.0, 2);
        assert_eq!(scroll, 0.0);
    }

    #[test]
    fn scroll_follows_caret_when_overflow() {
        let mut cache = GlyphCache::default();
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        let inner = 80.0;
        let caret = long.len();
        let scroll = search_scroll_x(&mut cache, long, inner, caret);
        assert!(scroll > 0.0);
        let total = cache.measure(long, SEARCH_TEXT_SIZE_PX);
        assert!(scroll <= (total - inner).max(0.0) + 0.01);
    }
}
