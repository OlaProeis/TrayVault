# Async Thumbnail Loading

Modules: `src/ui/thumb_loader.rs`, `src/ui/history.rs` (`draw_thumbnail`), `src/ui/mod.rs` (`ThumbLoadState`), `src/main.rs` (callback), `src/win32/window.rs` (`WM_THUMB_READY`). Cache overview: [`thumbnail-cache.md`](thumbnail-cache.md).

## Purpose

Persisted image entries have no in-memory `image_pixels` after capture persist (see [`storage.md`](storage.md)). Loading full `.dib` blobs and bilinear downscaling inside `WM_PAINT` blocked the message loop on fast scroll through image-heavy history.

Disk I/O and scale now run on a dedicated **thumb-loader worker thread**. The UI thread enqueues loads, draws a placeholder on miss, and repaints when results arrive via a custom window message.

## Architecture

```
WM_PAINT (miss, no image_pixels)
  → ThumbLoader::request(...)
  → draw placeholder
Worker thread
  → read_blob_at(data_dir, hash)
  → build_thumbnail_pixmap (BGRA→RGBA, bilinear scale)
  → reply mpsc (pixmap or failure) + PostMessageW(hwnd, WM_THUMB_READY)
WM_THUMB_READY
  → drain replies → clear inflight; insert on success → InvalidateRect
```

| Component | Role |
|-----------|------|
| `ThumbLoader` | Owned by `App`; `request` / `drain_replies` / `set_notify_hwnd` / `join` |
| `ThumbLoadState` on `UiState` | `inflight: HashSet<(entry_id, dst_w, dst_h)>`; `generation` for stale reply drop |
| `WM_THUMB_READY` | `WM_APP + 2`; handled in `WindowCallbacks::on_thumb_ready` |
| `ThumbCache::insert` | Inserts pre-built `Arc<Pixmap>` from worker (LRU eviction unchanged) |
| `ThumbLoadReply::pixmap` | `Some` on success; `None` when read/scale failed — still clears `inflight` so the next paint retries |

## Paint path (`draw_thumbnail`)

1. **`ThumbCache::get`** — hit → blit, return (no I/O).
2. **`entry.image_pixels` present** — sync `get_or_build` (recent captures still in RAM).
3. **Disk-backed miss** — enqueue one request per key if not `inflight`; draw divider-colored placeholder; **never** call `Store::read_blob` on the UI thread.

Copy-back and the image preview modal still use `Store::read_blob` on their own paths (unchanged).

## Stale reply handling

Each request carries `generation` from `ThumbLoadState`. When the thumb width bucket changes (window resize), `reset_on_width_change()` bumps generation and clears `inflight`. Replies with a mismatched generation are ignored in `UiState::apply_thumb_replies`.

## Failed loads and retries

If `read_blob_at` or `build_thumbnail_pixmap` fails (missing blob, decode error, dimension mismatch), the worker still sends a reply with `pixmap: None` and posts `WM_THUMB_READY`. `apply_thumb_replies` always removes the `(entry_id, dst_w, dst_h)` key from `inflight` but only inserts into `ThumbCache` on success. The next `WM_PAINT` can enqueue a new request — important when the thumb worker ran before `PersistAll` finished, or when a blob was temporarily missing.

## Lifecycle

- `App::new` spawns `ThumbLoader` with the store data directory.
- `main.rs` calls `thumb_loader.set_notify_hwnd(hwnd)` after window creation.
- `App::join_storage` joins both the storage worker and thumb loader on exit.

## Related

- LRU cache keys and sizing: [`thumbnail-cache.md`](thumbnail-cache.md)
- Blob storage layout: [`storage.md`](storage.md)
- Message-loop callback pattern: [`message-loop-callbacks.md`](message-loop-callbacks.md)
