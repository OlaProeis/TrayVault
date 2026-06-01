# Image Thumbnail Cache

Modules: `src/ui/thumb_cache.rs`, `src/ui/history.rs` (draw path). UI overview: [`ui-views.md`](ui-views.md) (Image list thumbnails).

## Purpose

History list image cards show scale-to-fit previews. Full-resolution BGRA pixels can be multi-MB; downscaling every `WM_PAINT` / mouse-move repaint is expensive. `ThumbCache` stores pre-downscaled `Pixmap` values keyed by `(entry_id, dst_w, dst_h)`.

## Cache key and sizing

Destination size is computed by `thumb_target_size(img_w, img_h, thumb_max_w)` in `history.rs` — it depends only on image dimensions and card inner width, **not** on pixel bytes. Hard caps: `MAX_THUMB_WIDTH` 800px, `MAX_THUMB_HEIGHT` 520px.

Downscale filter: **bilinear** (`scale_bilinear_rgba` in `pixmap.rs`) — same `(entry_id, dst_w, dst_h)` cache keys and memory footprint as nearest-neighbor; smoother edges on aggressive downscales (especially text/UI screenshots). Rebuild cached thumbs after deploy by resizing the window or restarting the app.

| API | When to use |
|-----|-------------|
| `ThumbCache::get(entry_id, img_w, img_h, thumb_max_w)` | Pixels-free lookup; returns `Option<Arc<Pixmap>>` on hit |
| `ThumbCache::get_or_build(..., pixels, ...)` | Miss path: BGRA → RGBA, bilinear scale, insert into cache |

Cache clears when `set_width_bucket(thumb_max_w)` sees a new integer card width (window resize). Eviction: when `MAX_ENTRIES` (64) is exceeded, the map is cleared (LRU-ish).

## Cache-first paint path

For persisted images (`entry.image_pixels == None`), pixels live on disk in `blobs/<hash>.dib`. The draw path in `draw_thumbnail` (`history.rs`):

1. Call `thumb_cache.get(...)` — on hit, blit and **return** (no disk I/O).
2. On miss: load pixels from `image_pixels` or `Store::read_blob`, then `get_or_build`.

This avoids re-reading full `.dib` blobs on every repaint when the scaled thumbnail is already cached (e.g. during `WM_MOUSEMOVE` hover repaints).

## Related

- Blob load on demand: [`storage.md`](storage.md) (in-memory `image_pixels` cleared after capture persist)
- Preview modal cache (separate single slot): [`ui-perf-caches.md`](ui-perf-caches.md)
- BGRA → RGBA conversion: [`pixmap-rasterizer.md`](pixmap-rasterizer.md)
