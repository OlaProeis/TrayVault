//! Cached, pre-downscaled image thumbnails for the history list.
//!
//! Avoids re-allocating full-resolution pixmaps and CPU-scaling huge images on every
//! `WM_PAINT` / mouse-move repaint.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ui::pixmap::{bgra_to_rgba, scale_bilinear_rgba, Pixmap};

use crate::ui::history::thumb_target_size;

const MAX_ENTRIES: usize = 64;

type ThumbKey = (u64, u32, u32);

/// LRU-backed cache of display-sized thumbnails keyed by `(entry_id, dst_w, dst_h)`.
#[derive(Default)]
pub struct ThumbCache {
    width_bucket: u32,
    entries: HashMap<ThumbKey, Arc<Pixmap>>,
    /// Front = evict first; back = most recently used.
    lru: Vec<ThumbKey>,
}

impl std::fmt::Debug for ThumbCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbCache")
            .field("width_bucket", &self.width_bucket)
            .field("entries", &self.entries.len())
            .field("lru", &self.lru.len())
            .finish()
    }
}

impl ThumbCache {
    /// Current width bucket (card inner width as `u32`).
    pub fn width_bucket(&self) -> u32 {
        self.width_bucket
    }

    /// Drop cached pixmaps when the card inner width changes (window resize).
    pub fn set_width_bucket(&mut self, thumb_max_w: f32) {
        let bucket = thumb_max_w as u32;
        if self.width_bucket != bucket {
            self.entries.clear();
            self.lru.clear();
            self.width_bucket = bucket;
        }
    }

    /// Insert a pre-built thumbnail (async loader path).
    pub fn insert(&mut self, entry_id: u64, dst_w: u32, dst_h: u32, pixmap: Arc<Pixmap>) {
        let key = (entry_id, dst_w, dst_h);
        self.evict_one_if_full();
        self.entries.insert(key, Arc::clone(&pixmap));
        self.touch(key);
    }

    /// Pixels-free cache lookup. Returns a cached thumbnail when `(entry_id, dst_w, dst_h)` matches.
    pub fn get(
        &mut self,
        entry_id: u64,
        img_w: u32,
        img_h: u32,
        thumb_max_w: f32,
    ) -> Option<Arc<Pixmap>> {
        let (dst_w, dst_h) = thumb_target_size(img_w, img_h, thumb_max_w);
        if dst_w == 0 || dst_h == 0 {
            return None;
        }
        let key = (entry_id, dst_w, dst_h);
        let arc = self.entries.get(&key).map(Arc::clone)?;
        self.touch(key);
        Some(arc)
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
        if let Some(cached) = self.entries.get(&key).map(Arc::clone) {
            self.touch(key);
            return Some(cached);
        }
        let pixmap = build_thumbnail_pixmap(pixels, img_w, img_h, dst_w, dst_h)?;
        self.evict_one_if_full();
        let arc = Arc::new(pixmap);
        self.entries.insert(key, Arc::clone(&arc));
        self.touch(key);
        Some(arc)
    }

    fn touch(&mut self, key: ThumbKey) {
        if let Some(i) = self.lru.iter().position(|&k| k == key) {
            self.lru.remove(i);
        }
        self.lru.push(key);
    }

    fn evict_one_if_full(&mut self) {
        if self.entries.len() < MAX_ENTRIES {
            return;
        }
        let Some(victim) = self.lru.first().copied() else {
            return;
        };
        self.lru.remove(0);
        self.entries.remove(&victim);
    }
}

pub(crate) fn build_thumbnail_pixmap(
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

    #[test]
    fn lru_evicts_oldest_entry_not_entire_cache() {
        let pixels = solid(80, 40, [0, 128, 255, 255]);
        let mut cache = ThumbCache::default();
        cache.set_width_bucket(200.0);

        for id in 1..=(MAX_ENTRIES as u64) {
            cache
                .get_or_build(id, &pixels, 80, 40, 200.0)
                .expect("thumb");
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);

        cache.get(1, 80, 40, 200.0).expect("touch id 1");
        cache
            .get_or_build(MAX_ENTRIES as u64 + 1, &pixels, 80, 40, 200.0)
            .expect("insert");

        assert!(
            cache.get(1, 80, 40, 200.0).is_some(),
            "recently touched entry should remain"
        );
        assert!(
            cache.get(2, 80, 40, 200.0).is_none(),
            "oldest untouched entry should be evicted"
        );
        assert!(
            cache.get(MAX_ENTRIES as u64 + 1, 80, 40, 200.0).is_some(),
            "new entry should be cached"
        );
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
    }
}
