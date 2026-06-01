//! Cached, pre-downscaled image thumbnails for the history list.
//!
//! Avoids re-allocating full-resolution pixmaps and CPU-scaling huge images on every
//! `WM_PAINT` / mouse-move repaint.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ui::pixmap::{bgra_to_rgba, scale_bilinear_rgba, Pixmap};

use crate::ui::history::thumb_target_size;

const MAX_ENTRIES: usize = 64;

/// LRU-ish cache of display-sized thumbnails keyed by entry id.
#[derive(Default)]
pub struct ThumbCache {
    width_bucket: u32,
    entries: HashMap<(u64, u32, u32), Arc<Pixmap>>,
}

impl std::fmt::Debug for ThumbCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbCache")
            .field("width_bucket", &self.width_bucket)
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl ThumbCache {
    /// Drop cached pixmaps when the card inner width changes (window resize).
    pub fn set_width_bucket(&mut self, thumb_max_w: f32) {
        let bucket = thumb_max_w as u32;
        if self.width_bucket != bucket {
            self.entries.clear();
            self.width_bucket = bucket;
        }
    }

    /// Pixels-free cache lookup. Returns a cached thumbnail when `(entry_id, dst_w, dst_h)` matches.
    pub fn get(
        &self,
        entry_id: u64,
        img_w: u32,
        img_h: u32,
        thumb_max_w: f32,
    ) -> Option<Arc<Pixmap>> {
        let (dst_w, dst_h) = thumb_target_size(img_w, img_h, thumb_max_w);
        if dst_w == 0 || dst_h == 0 {
            return None;
        }
        self.entries.get(&(entry_id, dst_w, dst_h)).map(Arc::clone)
    }

    pub fn get_or_build(
        &mut self,
        entry_id: u64,
        pixels: &[u8],
        img_w: u32,
        img_h: u32,
        thumb_max_w: f32,
    ) -> Option<Arc<Pixmap>> {
        let (dst_w, dst_h) = thumb_target_size(img_w, img_h, thumb_max_w);
        if dst_w == 0 || dst_h == 0 {
            return None;
        }
        let key = (entry_id, dst_w, dst_h);
        if let Some(cached) = self.entries.get(&key) {
            return Some(Arc::clone(cached));
        }
        let pixmap = build_thumbnail_pixmap(pixels, img_w, img_h, dst_w, dst_h)?;
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.clear();
        }
        let arc = Arc::new(pixmap);
        self.entries.insert(key, Arc::clone(&arc));
        Some(arc)
    }
}

fn build_thumbnail_pixmap(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&bgra);
        }
        v
    }

    #[test]
    fn downscale_reduces_dimensions() {
        let src = solid(100, 50, [255, 0, 0, 255]);
        let dst = crate::ui::pixmap::scale_bilinear_rgba(&src, 100, 50, 40, 20);
        assert_eq!(dst.len(), 40 * 20 * 4);
    }

    #[test]
    fn cache_reuses_pixmap_for_same_entry() {
        let pixels = solid(80, 40, [0, 128, 255, 255]);
        let mut cache = ThumbCache::default();
        cache.set_width_bucket(200.0);
        let a = cache
            .get_or_build(1, &pixels, 80, 40, 200.0)
            .expect("thumb");
        let b = cache
            .get_or_build(1, &pixels, 80, 40, 200.0)
            .expect("thumb");
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn get_returns_none_before_build_and_same_arc_after() {
        let pixels = solid(80, 40, [0, 128, 255, 255]);
        let mut cache = ThumbCache::default();
        cache.set_width_bucket(200.0);
        assert!(cache.get(1, 80, 40, 200.0).is_none());
        let built = cache
            .get_or_build(1, &pixels, 80, 40, 200.0)
            .expect("thumb");
        let cached = cache.get(1, 80, 40, 200.0).expect("cached");
        assert!(Arc::ptr_eq(&built, &cached));
    }
}
