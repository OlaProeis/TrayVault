# Image Thumbnail Cache

Modules: `src/ui/thumb_cache.rs`, `src/ui/history.rs` (draw path). UI overview: [`ui-views.md`](ui-views.md) (Image list thumbnails).

## Purpose

History list image cards show scale-to-fit previews. Full-resolution BGRA pixels can be multi-MB; downscaling every `WM_PAINT` / mouse-move repaint is expensive. `ThumbCache` stores pre-downscaled `Pixmap` values keyed by `(entry_id, dst_w, dst_h)`.

## Cache key and sizing

Destination size is computed by `thumb_target_size(img_w, img_h, thumb_max_w)` in `history.rs` — image dimensions and card inner width, capped at `MAX_THUMB_WIDTH` 1200px / `MAX_THUMB_HEIGHT` 900px. Fit-box height is `max(120, inner_w × 0.85)` (capped at 900) so ~4:3 landscape screenshots can use the full card width; older 800×520 caps left tall captures underscaled and text unreadable.

Downscale filter: **bilinear** (`scale_bilinear_rgba` in `pixmap.rs`). Rebuild cached thumbs after deploy by resizing the window or restarting the app.

| API | When to use |
|-----|-------------|
| `ThumbCache::get(...)` | Pixels-free lookup; returns `Option<Arc<Pixmap>>` on hit; touches LRU |
| `ThumbCache::get_or_build(..., pixels, ...)` | Sync miss when `image_pixels` still in memory: BGRA → RGBA, scale, insert |
| `ThumbCache::insert(...)` | Async worker path: insert pre-built pixmap (see [`async-thumbnail-loading.md`](async-thumbnail-loading.md)) |

`set_width_bucket(thumb_max_w)` clears the map when card inner width changes (window resize).

## LRU eviction

At most `MAX_ENTRIES` (64) thumbnails. `get` / `get_or_build` promote keys on the MRU end of an internal `lru` list; at capacity, only the LRU front entry is evicted (not `entries.clear()`). Scrolling back through a long image history keeps recently seen thumbs warm.

## Cache-first paint path

For persisted images (`entry.image_pixels == None`), pixels live on disk in `blobs/<hash>.dib`. The draw path in `draw_thumbnail` (`history.rs`):

1. Call `thumb_cache.get(...)` — on hit, blit and **return** (no disk I/O).
2. If `image_pixels` is still in memory (recent capture), sync `get_or_build` and blit.
3. Otherwise enqueue an async load via `ThumbLoader`, draw a placeholder, and return without `read_blob` on the UI thread.

When the worker completes, `WM_THUMB_READY` drains replies into `ThumbCache::insert` and triggers repaint. Details: [`async-thumbnail-loading.md`](async-thumbnail-loading.md).

This avoids re-reading full `.dib` blobs on every repaint when the scaled thumbnail is already cached (e.g. during hover repaints).

## Deferred: scroll-tier quality (Task 42)

An experiment used smaller `thumb_target_size` caps while scrolling (separate cache keys). It was **rolled back**: thumbnails visibly shrank during scroll and an idle “upgrade” repaint felt slow. Task 42 is **deferred**. If revived, keep the **on-screen card size** fixed and only reduce internal decode/scale resolution (e.g. `blit_scaled` to the full layout rect), paired with Task 39 async blob loads.

## Related

- Blob load on demand: [`storage.md`](storage.md) (in-memory `image_pixels` cleared after capture persist)
- Async disk miss path: [`async-thumbnail-loading.md`](async-thumbnail-loading.md)
- Preview modal cache (separate single slot): [`ui-perf-caches.md`](ui-perf-caches.md)
- BGRA → RGBA conversion: [`pixmap-rasterizer.md`](pixmap-rasterizer.md)
