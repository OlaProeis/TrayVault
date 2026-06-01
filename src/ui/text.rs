//! GDI text layout, glyph cache, and relative time labels.

use std::collections::HashMap;

use crate::ui::pixmap::{Color, Pixmap};
use crate::win32::glyph_raster::{self, RasterizedGlyph};

/// Rasterized glyph bitmap plus placement metrics.
struct CachedGlyph {
    width: usize,
    height: usize,
    advance_width: f32,
    left: f32,
    top: f32,
    pixels: Vec<u8>,
}

impl From<RasterizedGlyph> for CachedGlyph {
    fn from(g: RasterizedGlyph) -> Self {
        Self {
            width: g.width,
            height: g.height,
            advance_width: g.advance_width,
            left: g.left,
            top: g.top,
            pixels: g.pixels,
        }
    }
}

/// `(char, size_px)` → cached raster.
#[derive(Default)]
pub struct GlyphCache {
    entries: HashMap<(char, u32), CachedGlyph>,
}

impl std::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl GlyphCache {
    /// Horizontal advance for one glyph at `size_px` (cache fill on miss, no bitmap clone).
    pub fn advance(&mut self, ch: char, size_px: f32) -> f32 {
        self.get(ch, size_px).advance_width
    }

    pub fn measure(&mut self, text: &str, size_px: f32) -> f32 {
        let mut x = 0.0f32;
        for ch in text.chars() {
            x += self.advance(ch, size_px);
        }
        x
    }

    /// Byte index in `text` for a horizontal click at `click_x` (text starts at `text_x`).
    pub fn caret_index_from_x(
        &mut self,
        text: &str,
        size_px: f32,
        click_x: f32,
        text_x: f32,
    ) -> usize {
        let rel = (click_x - text_x).max(0.0);
        let mut pen_x = 0.0f32;
        let mut byte_idx = 0usize;
        for (i, ch) in text.char_indices() {
            let advance = self.advance(ch, size_px);
            if pen_x + advance * 0.5 >= rel {
                return byte_idx;
            }
            pen_x += advance;
            byte_idx = i + ch.len_utf8();
        }
        text.len()
    }

    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        x: f32,
        baseline_y: f32,
        size_px: f32,
        color: Color,
    ) {
        let mut pen_x = x;
        for ch in text.chars() {
            let glyph = self.get(ch, size_px);
            draw_glyph(pixmap, glyph, pen_x, baseline_y, color);
            pen_x += glyph.advance_width;
        }
    }

    fn get(&mut self, ch: char, size_px: f32) -> &CachedGlyph {
        let key = (ch, size_px.to_bits());
        self.entries.entry(key).or_insert_with(|| {
            glyph_raster::rasterize_glyph(ch, size_px)
                .unwrap_or_else(|_| RasterizedGlyph {
                    width: 0,
                    height: 0,
                    advance_width: size_px * 0.5,
                    left: 0.0,
                    top: 0.0,
                    pixels: Vec::new(),
                })
                .into()
        })
    }
}

fn draw_glyph(pixmap: &mut Pixmap, glyph: &CachedGlyph, x: f32, baseline_y: f32, color: Color) {
    if glyph.width == 0 || glyph.height == 0 || glyph.pixels.len() != glyph.width * glyph.height {
        return;
    }

    let start_x = (x + glyph.left).round() as i32;
    let start_y = (baseline_y + glyph.top).round() as i32;
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;

    for row in 0..glyph.height {
        for col in 0..glyph.width {
            let alpha = glyph.pixels[row * glyph.width + col];
            if alpha == 0 {
                continue;
            }
            let px = start_x + col as i32;
            let py = start_y + row as i32;
            if px < 0 || py < 0 || px >= width || py >= height {
                continue;
            }
            blend_pixel(pixmap, px as u32, py as u32, color, alpha as f32 / 255.0);
        }
    }
}

fn blend_pixel(pixmap: &mut Pixmap, x: u32, y: u32, color: Color, alpha: f32) {
    let w = pixmap.width();
    let idx = (y * w + x) as usize * 4;
    let data = pixmap.data_mut();
    if idx + 3 >= data.len() {
        return;
    }

    // `Color` channels are normalized 0..=1; pixmap stores straight RGBA8.
    let src = color.to_color_u8();
    let src_a = alpha * (f32::from(src.alpha()) / 255.0);
    if src_a <= 0.0 {
        return;
    }
    let inv = 1.0 - src_a;
    let dst_r = f32::from(data[idx]);
    let dst_g = f32::from(data[idx + 1]);
    let dst_b = f32::from(data[idx + 2]);
    let dst_a = f32::from(data[idx + 3]) / 255.0;

    let out_r = f32::from(src.red()) * src_a + dst_r * inv;
    let out_g = f32::from(src.green()) * src_a + dst_g * inv;
    let out_b = f32::from(src.blue()) * src_a + dst_b * inv;
    let out_a = (src_a + dst_a * inv).min(1.0);

    data[idx] = out_r.round().clamp(0.0, 255.0) as u8;
    data[idx + 1] = out_g.round().clamp(0.0, 255.0) as u8;
    data[idx + 2] = out_b.round().clamp(0.0, 255.0) as u8;
    data[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Vertical distance between baselines for multiline text at `size_px`.
pub fn line_height(size_px: f32) -> f32 {
    (size_px * 1.35).ceil()
}

/// Word-wrap `text` to `max_width`, preserving explicit newlines.
pub fn wrap_text_lines(
    cache: &mut GlyphCache,
    text: &str,
    size_px: f32,
    max_width: f32,
) -> Vec<String> {
    if max_width <= 0.0 {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        wrap_paragraph(cache, paragraph, size_px, max_width, &mut lines);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_paragraph(
    cache: &mut GlyphCache,
    paragraph: &str,
    size_px: f32,
    max_width: f32,
    lines: &mut Vec<String>,
) {
    let space_w = cache.advance(' ', size_px);
    let mut current = String::new();
    let mut current_w = 0.0f32;

    for word in paragraph.split_whitespace() {
        let word_w = cache.measure(word, size_px);
        if current.is_empty() {
            if word_w <= max_width {
                current.push_str(word);
                current_w = word_w;
            } else {
                push_wrapped_chars(cache, word, size_px, max_width, lines);
            }
            continue;
        }

        if current_w + space_w + word_w <= max_width {
            current.push(' ');
            current.push_str(word);
            current_w += space_w + word_w;
        } else {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
            if word_w <= max_width {
                current.push_str(word);
                current_w = word_w;
            } else {
                push_wrapped_chars(cache, word, size_px, max_width, lines);
            }
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
}

fn push_wrapped_chars(
    cache: &mut GlyphCache,
    text: &str,
    size_px: f32,
    max_width: f32,
    lines: &mut Vec<String>,
) {
    let mut current = String::new();
    let mut current_w = 0.0f32;
    for ch in text.chars() {
        let adv = cache.advance(ch, size_px);
        if !current.is_empty() && current_w + adv > max_width {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
        }
        current.push(ch);
        current_w += adv;
    }
    if !current.is_empty() {
        lines.push(current);
    }
}

/// Draw `lines` stacked vertically starting at `first_baseline_y`.
pub fn draw_lines(
    cache: &mut GlyphCache,
    pixmap: &mut Pixmap,
    lines: &[String],
    x: f32,
    first_baseline_y: f32,
    size_px: f32,
    color: Color,
) {
    let step = line_height(size_px);
    for (i, line) in lines.iter().enumerate() {
        cache.draw(
            pixmap,
            line,
            x,
            first_baseline_y + i as f32 * step,
            size_px,
            color,
        );
    }
}

/// Truncate `text` to fit `max_width` using an ellipsis suffix.
pub fn truncate_to_width(
    cache: &mut GlyphCache,
    text: &str,
    size_px: f32,
    max_width: f32,
) -> String {
    if cache.measure(text, size_px) <= max_width {
        return text.to_string();
    }

    let ellipsis_w = cache.advance('…', size_px);
    let mut out = String::new();
    let mut width = 0.0f32;
    for ch in text.chars() {
        let adv = cache.advance(ch, size_px);
        if width + adv + ellipsis_w > max_width {
            break;
        }
        width += adv;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Format a capture timestamp relative to `now_millis` (Unix epoch ms).
pub fn format_relative_time(created_at: u64, now_millis: u64) -> String {
    if created_at >= now_millis {
        return "Just now".into();
    }

    let delta_ms = now_millis.saturating_sub(created_at);
    if delta_ms < 60_000 {
        return "Just now".into();
    }

    let delta_min = delta_ms / 60_000;
    if delta_min < 60 {
        return if delta_min == 1 {
            "1 min ago".into()
        } else {
            format!("{delta_min} min ago")
        };
    }

    let created_day = millis_to_day(created_at);
    let now_day = millis_to_day(now_millis);
    if created_day + 1 == now_day {
        return "Yesterday".into();
    }

    let day_span = now_day.saturating_sub(created_day);
    if (2..7).contains(&day_span) {
        return format!("{day_span} days ago");
    }

    format_date(created_at)
}

fn millis_to_day(ms: u64) -> u64 {
    ms / 86_400_000
}

fn format_date(ms: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = ms / 1000;
    let days = secs / 86_400;
    let mut y = 1970i32;
    let mut remaining = days;
    loop {
        let year_days = if is_leap(y) { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        y += 1;
    }
    let month_lengths = month_lengths(y);
    let mut month = 0usize;
    for (idx, len) in month_lengths.iter().enumerate() {
        if remaining < *len {
            month = idx;
            break;
        }
        remaining -= *len;
    }
    let day = remaining + 1;
    format!("{} {day}, {y}", MONTHS[month])
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn month_lengths(year: i32) -> [u64; 12] {
    let feb = if is_leap(year) { 29 } else { 28 };
    [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fontdue baseline captured before Task 16 (14 px Roboto Regular).
    struct FontdueBaseline {
        width: usize,
        height: usize,
        advance: f32,
        left: f32,
        top: f32,
        nz: usize,
        sum: u32,
    }

    const FONTDUE_BASELINES: &[(char, FontdueBaseline)] = &[
        (
            'a',
            FontdueBaseline {
                width: 7,
                height: 9,
                advance: 7.615,
                left: 0.745,
                top: -8.0,
                nz: 47,
                sum: 6238,
            },
        ),
        (
            'v',
            FontdueBaseline {
                width: 7,
                height: 8,
                advance: 6.781,
                left: 0.226,
                top: -8.0,
                nz: 33,
                sum: 4157,
            },
        ),
        (
            '…',
            FontdueBaseline {
                width: 8,
                height: 3,
                advance: 9.365,
                left: 1.012,
                top: -2.0,
                nz: 19,
                sum: 1429,
            },
        ),
    ];

    fn ellipsis_baseline_checks(g: &RasterizedGlyph, expected: &FontdueBaseline) {
        assert!(g.advance_width > 0.0, "ellipsis expected positive advance");
        assert!(
            g.pixels.iter().any(|&b| b > 0),
            "ellipsis expected visible pixels"
        );
        let nz = g.pixels.iter().filter(|&&b| b > 0).count();
        assert!(
            nz >= 3,
            "ellipsis should compose at least three dot strokes, got {nz}"
        );
        let _ = expected;
    }

    fn now_ms() -> u64 {
        1_700_000_000_000
    }

    #[test]
    fn relative_time_just_now() {
        assert_eq!(format_relative_time(now_ms() - 5_000, now_ms()), "Just now");
        assert_eq!(
            format_relative_time(now_ms() - 30_000, now_ms()),
            "Just now"
        );
    }

    #[test]
    fn relative_time_minutes() {
        assert_eq!(
            format_relative_time(now_ms() - 60_000, now_ms()),
            "1 min ago"
        );
        assert_eq!(
            format_relative_time(now_ms() - 120_000, now_ms()),
            "2 min ago"
        );
    }

    #[test]
    fn relative_time_yesterday() {
        let now = 1_704_067_200_000u64; // 2024-01-01 00:00 UTC approx
        let yesterday = now - 86_400_000;
        assert_eq!(format_relative_time(yesterday, now), "Yesterday");
    }

    #[test]
    fn relative_time_days_ago() {
        let now = 1_704_067_200_000u64;
        let three_days = now - 3 * 86_400_000;
        assert_eq!(format_relative_time(three_days, now), "3 days ago");
    }

    #[test]
    fn measure_equals_sum_of_advances() {
        let mut cache = GlyphCache::default();
        let text = "Hello world";
        let measured = cache.measure(text, 14.0);
        let sum: f32 = text.chars().map(|ch| cache.advance(ch, 14.0)).sum();
        assert!((measured - sum).abs() < f32::EPSILON);
    }

    #[test]
    fn wrap_preserves_explicit_newlines() {
        let mut cache = GlyphCache::default();
        let lines = wrap_text_lines(&mut cache, "line one\nline two", 14.0, 400.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "line one");
        assert_eq!(lines[1], "line two");
    }

    #[test]
    fn wrap_breaks_long_unbroken_token() {
        let mut cache = GlyphCache::default();
        let long = "abcdefghijklmnopqrstuvwxyz";
        let lines = wrap_text_lines(&mut cache, long, 14.0, 80.0);
        assert!(lines.len() > 1);
        assert!(lines
            .iter()
            .all(|line| cache.measure(line, 14.0) <= 80.0 + 1.0));
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let mut cache = GlyphCache::default();
        let long = "abcdefghijklmnopqrstuvwxyz";
        let truncated = truncate_to_width(&mut cache, long, 14.0, 80.0);
        assert!(truncated.ends_with('…'));
        assert!(cache.measure(&truncated, 14.0) <= 80.0 + 1.0);
    }

    #[test]
    fn draw_produces_dark_pixels_on_white_background() {
        let mut cache = GlyphCache::default();
        let mut pixmap = Pixmap::new(120, 32).expect("pixmap");
        for px in pixmap.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        }

        cache.draw(
            &mut pixmap,
            "Hello",
            4.0,
            24.0,
            14.0,
            Color::from_rgba8(26, 26, 26, 255),
        );

        let has_dark = pixmap.data().chunks_exact(4).any(|px| {
            let luma = u16::from(px[0]) + u16::from(px[1]) + u16::from(px[2]);
            luma < 3 * 200
        });
        assert!(has_dark, "expected visible glyph pixels after draw");
    }

    #[test]
    fn glyphs_share_baseline_on_same_line() {
        let a = glyph_raster::rasterize_glyph('a', 14.0).expect("a");
        let v = glyph_raster::rasterize_glyph('v', 14.0).expect("v");
        assert_eq!(
            a.top.round(),
            v.top.round(),
            "x-height letters should share the same top offset from baseline"
        );
    }

    #[test]
    fn gdi_metrics_track_fontdue_baseline() {
        for &(ch, ref expected) in FONTDUE_BASELINES {
            let g = glyph_raster::rasterize_glyph(ch, 14.0).expect("rasterize");
            if ch == '…' {
                ellipsis_baseline_checks(&g, expected);
                continue;
            }
            let advance_tol = if ch == '…' { 5.0 } else { 2.0 };
            assert!(
                (g.advance_width - expected.advance).abs() <= advance_tol,
                "ch={ch:?} advance {} vs fontdue {}",
                g.advance_width,
                expected.advance
            );
            assert!(
                g.width.abs_diff(expected.width) <= 2,
                "ch={ch:?} width {} vs fontdue {}",
                g.width,
                expected.width
            );
            assert!(
                g.height.abs_diff(expected.height) <= 2,
                "ch={ch:?} height {} vs fontdue {}",
                g.height,
                expected.height
            );
            assert!(
                (g.left - expected.left).abs() <= 2.0,
                "ch={ch:?} left {} vs fontdue {}",
                g.left,
                expected.left
            );
            assert!(
                (g.top - expected.top).abs() <= 2.0,
                "ch={ch:?} top {} vs fontdue {}",
                g.top,
                expected.top
            );
            let nz = g.pixels.iter().filter(|&&b| b > 0).count();
            let sum: u32 = g.pixels.iter().map(|&b| u32::from(b)).sum();
            assert!(nz > 0, "ch={ch:?} expected visible pixels");
            assert!(
                nz >= expected.nz / 2,
                "ch={ch:?} coverage nz={nz} vs fontdue {}",
                expected.nz
            );
            assert!(
                sum >= expected.sum / 3,
                "ch={ch:?} ink sum={sum} vs fontdue {}",
                expected.sum
            );
        }
    }
}
