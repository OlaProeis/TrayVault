//! Image preview modal and help overlay.

use std::sync::Arc;

use crate::app::App;
use crate::store::Store;
use crate::ui::pixmap::{bgra_to_rgba, blit, scale_bilinear_rgba, Color, Pixmap};
use crate::ui::text::GlyphCache;
use crate::ui::theme::Theme;
use crate::ui::widgets::{fill_rect, rgba_to_color, PADDING};

const OVERLAY_ALPHA: u8 = 200;

/// Single-slot cache of the decoded+scaled preview pixmap for the open modal.
pub struct PreviewImageCache {
    entry_id: u64,
    dst_w: u32,
    dst_h: u32,
    pixmap: Arc<Pixmap>,
}

impl std::fmt::Debug for PreviewImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewImageCache")
            .field("entry_id", &self.entry_id)
            .field("dst_w", &self.dst_w)
            .field("dst_h", &self.dst_h)
            .finish()
    }
}

impl PreviewImageCache {
    pub fn get(&self, entry_id: u64, dst_w: u32, dst_h: u32) -> Option<Arc<Pixmap>> {
        if self.entry_id == entry_id && self.dst_w == dst_w && self.dst_h == dst_h {
            Some(Arc::clone(&self.pixmap))
        } else {
            None
        }
    }

    fn store(&mut self, entry_id: u64, dst_w: u32, dst_h: u32, pixmap: Pixmap) -> Arc<Pixmap> {
        self.entry_id = entry_id;
        self.dst_w = dst_w;
        self.dst_h = dst_h;
        self.pixmap = Arc::new(pixmap);
        Arc::clone(&self.pixmap)
    }
}

pub fn draw_image_preview(
    pixmap: &mut Pixmap,
    cache: &mut GlyphCache,
    preview_cache: &mut Option<PreviewImageCache>,
    theme: &Theme,
    app: &App,
    store: &Store,
    entry_id: u64,
) {
    let width = pixmap.width() as f32;
    let height = pixmap.height() as f32;

    fill_rect(
        pixmap,
        0.0,
        0.0,
        width,
        height,
        Color::from_rgba8(0, 0, 0, OVERLAY_ALPHA),
    );

    let Some(entry) = app.entries.iter().find(|e| e.id == entry_id) else {
        return;
    };
    let Some(img) = &entry.image else {
        return;
    };

    let (dst_w, dst_h, dx, dy) = preview_layout(width, height, img.width, img.height);

    let scaled = preview_cache
        .as_ref()
        .and_then(|slot| slot.get(entry_id, dst_w, dst_h))
        .or_else(|| build_and_store_preview(preview_cache, entry, store, dst_w, dst_h));

    let Some(scaled) = scaled else {
        draw_centered_label(cache, pixmap, theme, "Image unavailable", width, height);
        return;
    };

    blit(pixmap, &scaled, dx, dy);

    let label = format!("{}×{} — Esc to close", img.width, img.height);
    let label_w = cache.measure(&label, 14.0);
    cache.draw(
        pixmap,
        &label,
        (width - label_w) / 2.0,
        height - 32.0,
        14.0,
        rgba_to_color(theme.text_primary),
    );
}

fn preview_layout(window_w: f32, window_h: f32, img_w: u32, img_h: u32) -> (u32, u32, f32, f32) {
    let max_w = window_w - PADDING * 4.0;
    let max_h = window_h - 80.0;
    let scale = (max_w / img_w as f32).min(max_h / img_h as f32).min(1.0);
    let dw = img_w as f32 * scale;
    let dh = img_h as f32 * scale;
    let dst_w = (img_w as f32 * scale).round().max(1.0) as u32;
    let dst_h = (img_h as f32 * scale).round().max(1.0) as u32;
    let dx = (window_w - dw) / 2.0;
    let dy = (window_h - dh) / 2.0;
    (dst_w, dst_h, dx, dy)
}

fn build_and_store_preview(
    preview_cache: &mut Option<PreviewImageCache>,
    entry: &crate::models::ClipEntry,
    store: &Store,
    dst_w: u32,
    dst_h: u32,
) -> Option<Arc<Pixmap>> {
    let img = entry.image.as_ref()?;
    let entry_id = entry.id;
    let pixel_buf = entry
        .image_pixels
        .as_deref()
        .map(|p| p.to_vec())
        .or_else(|| store.read_blob(&img.hash, img.width, img.height))?;

    let pixmap = build_preview_pixmap(&pixel_buf, img.width, img.height, dst_w, dst_h)?;
    let arc = match preview_cache {
        Some(slot) => slot.store(entry_id, dst_w, dst_h, pixmap),
        None => {
            let arc = Arc::new(pixmap);
            *preview_cache = Some(PreviewImageCache {
                entry_id,
                dst_w,
                dst_h,
                pixmap: Arc::clone(&arc),
            });
            arc
        }
    };
    Some(arc)
}

fn build_preview_pixmap(
    pixels: &[u8],
    img_w: u32,
    img_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Option<Pixmap> {
    if img_w == 0 || img_h == 0 {
        return None;
    }
    let expected = (img_w as usize)
        .checked_mul(img_h as usize)?
        .checked_mul(4)?;
    if pixels.len() != expected {
        return None;
    }

    let rgba = bgra_to_rgba(pixels);

    if dst_w == img_w && dst_h == img_h {
        return Pixmap::from_vec(rgba, dst_w, dst_h);
    }

    let scaled = scale_bilinear_rgba(&rgba, img_w, img_h, dst_w, dst_h);
    Pixmap::from_vec(scaled, dst_w, dst_h)
}

pub fn draw_help_overlay(pixmap: &mut Pixmap, cache: &mut GlyphCache, theme: &Theme) {
    let width = pixmap.width() as f32;
    let height = pixmap.height() as f32;

    fill_rect(
        pixmap,
        0.0,
        0.0,
        width,
        height,
        Color::from_rgba8(0, 0, 0, OVERLAY_ALPHA),
    );

    let panel_w = 360.0f32.min(width - PADDING * 4.0);
    let panel_h = 320.0f32.min(height - PADDING * 4.0);
    let px = (width - panel_w) / 2.0;
    let py = (height - panel_h) / 2.0;

    fill_rect(pixmap, px, py, panel_w, panel_h, rgba_to_color(theme.card));

    let lines = [
        "TrayVault Help",
        "",
        "↑ / ↓     Move selection",
        "PgUp/Dn   Scroll one page",
        "Enter     Copy selected entry",
        "Esc       Clear search / close",
        "F1 / ?    Toggle this help",
        "Settings  Open settings panel",
        "Ctrl+P    Pin / unpin selected",
        "Delete    Delete selected entry",
        "",
        "Click entry to copy",
        "Double-click image to preview",
        "Right-click for context menu",
    ];

    let mut y = py + 28.0;
    for (i, line) in lines.iter().enumerate() {
        let size = if i == 0 { 16.0 } else { 13.0 };
        let color = if i == 0 {
            rgba_to_color(theme.text_primary)
        } else {
            rgba_to_color(theme.text_secondary)
        };
        cache.draw(pixmap, line, px + PADDING, y, size, color);
        y += if i == 0 { 28.0 } else { 20.0 };
    }
}

fn draw_centered_label(
    cache: &mut GlyphCache,
    pixmap: &mut Pixmap,
    theme: &Theme,
    text: &str,
    width: f32,
    height: f32,
) {
    let w = cache.measure(text, 14.0);
    cache.draw(
        pixmap,
        text,
        (width - w) / 2.0,
        height / 2.0,
        14.0,
        rgba_to_color(theme.text_primary),
    );
}

#[cfg(test)]
mod preview_cache_tests {
    use super::*;

    fn solid_bgra(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&bgra);
        }
        v
    }

    #[test]
    fn cache_hit_when_entry_and_dims_match() {
        let pixels = solid_bgra(80, 40, [0, 128, 255, 255]);
        let pixmap = build_preview_pixmap(&pixels, 80, 40, 40, 20).expect("scaled");
        let slot = PreviewImageCache {
            entry_id: 7,
            dst_w: 40,
            dst_h: 20,
            pixmap: Arc::new(pixmap),
        };
        let a = slot.get(7, 40, 20).expect("hit");
        let b = slot.get(7, 40, 20).expect("hit");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(slot.get(7, 41, 20).is_none());
        assert!(slot.get(8, 40, 20).is_none());
    }

    #[test]
    fn preview_layout_matches_blit_scaled_rounding() {
        let (dst_w, dst_h, dx, dy) = preview_layout(800.0, 600.0, 100, 50);
        assert_eq!(dst_w, 100);
        assert_eq!(dst_h, 50);
        assert!((dx - 350.0).abs() < 0.01);
        assert!((dy - 275.0).abs() < 0.01);

        let (dst_w, dst_h, _, _) = preview_layout(200.0, 200.0, 1000, 800);
        assert!(dst_w <= 168);
        assert!(dst_h <= 120);
    }
}
