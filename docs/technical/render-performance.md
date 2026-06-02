# Render Performance Optimizations

Paint-path and text-layout optimizations in the immediate-mode UI. Pipeline overview: [`rendering.md`](rendering.md). Pixmap rasterizer: [`pixmap-rasterizer.md`](pixmap-rasterizer.md).

## Direct BGRA handoff (GDI DIB)

**Problem:** Each `WM_PAINT` allocated an RGBA `Pixmap`, then a second full-frame BGRA `Vec` (`rgba_to_bgra`), then `copy_from_slice` into the GDI DIB — ~2× frame buffer allocation plus an extra memcpy every repaint (including every `WM_MOUSEMOVE` hover update).

**Fix:** Keep drawing into an RGBA `Pixmap`; convert RGBA→BGRA in one pass **directly** into `GdiBuffer::bits_mut()` via `widgets::write_rgba_to_bgra`. No intermediate BGRA `Vec`, no separate DIB copy.

| Stage | Location |
|-------|----------|
| Render + convert | `render_app(..., bgra_dst: &mut [u8])` in `src/ui/render.rs` |
| In-place channel swap | `write_rgba_to_bgra` in `src/ui/widgets.rs` |
| Paint callback | `on_paint` in `src/main.rs` passes `gdi.bits_mut()[..needed]` |

Widget drawing still uses RGBA (`Pixmap`, theme colors as R,G,B,A). Only the final handoff to GDI swaps R/B. Visual output is unchanged in light and dark themes.

## Persistent RGBA scratch buffer

**Problem:** After direct BGRA handoff, each `WM_PAINT` still allocated a full RGBA `Pixmap` via `Pixmap::new` — costly on large windows and frequent repaints (scroll, hover).

**Fix:** `UiState` holds `scratch: Option<Pixmap>` and `scratch_size: (u32, u32)`. `render_app` calls `take_scratch` / `return_scratch` around the paint body so the same pixel `Vec` is reused when client size is unchanged; resize triggers `Pixmap::new` once. Background fill and `write_rgba_to_bgra` into the GDI DIB are unchanged.

| Stage | Location |
|-------|----------|
| Scratch fields + `take_scratch` / `return_scratch` | `src/ui/mod.rs` |
| Reuse in paint path | `render_app` in `src/ui/render.rs` |

**Tests:** `render_app_produces_bgra_buffer`, `render_app_reuses_scratch_buffer_at_same_size` (stable `data().as_ptr()` across two paints).

**Not done:** Drawing BGRA natively into the DIB (skip RGBA scratch entirely) remains optional follow-on.

## Hover repaint gating

**Problem:** Main-view `MouseMove` returned true from `handle_input` on every pixel, triggering full `WM_PAINT` even when card/chip/button hover could not change.

**Fix:** Compare `hover_key_at` result to `UiState::hover_key`; skip `InvalidateRect` when unchanged. Scrollbar gutter hover and settings/help/preview overlays still repaint every move.

**Key files:** `src/ui/input.rs` (`mouse_move_needs_repaint`), `src/ui/mod.rs` (`HoverKey`). Details: [`ui-perf-caches.md`](ui-perf-caches.md) (Hover repaint gating), [`ui-views.md`](ui-views.md) (History card hover).

## Wheel scroll repaint coalescing (~60 Hz)

**Problem:** Each `MouseWheel` notch returned true from `handle_input`, so `main` called `InvalidateRect` on every event. High-resolution wheels could enqueue many full paints per second (raster + thumbs even when list layout was cached).

**Fix:** `scroll_offset` updates on every wheel event; repaints are gated with `SCROLL_REPAINT_MIN_MS` (16 ms). First event in a burst invalidates immediately; faster events set `needs_scroll_repaint` and schedule at most one `request_window_repaint`. `on_paint` clears the pending flag and may request one follow-up frame when the interval elapses. Scrollbar thumb drag is unchanged (still repaints every move).

| Stage | Location |
|-------|----------|
| Gate + flags | `wheel_scroll_repaint`, `clear_scroll_repaint_after_paint`, `take_deferred_scroll_repaint` in `src/ui/mod.rs` |
| Wheel handler | `MouseWheel` in `src/ui/input.rs` |
| Post-paint scheduling | `on_paint` in `src/main.rs` |

## Glyph cache: borrow-based lookups

**Problem:** `GlyphCache::get` returned `cached.clone()`, copying the full alpha bitmap `Vec` on every cache hit. `measure`, `caret_index_from_x`, and `truncate_to_width` triggered those clones even when only `advance_width` was needed. `truncate_to_width` re-measured growing string prefixes each character → O(n²) glyph clones per truncated card per frame.

**Fix:**

| Change | Location |
|--------|----------|
| `get(ch, size_px) -> &CachedGlyph` via `HashMap::entry().or_insert_with()` | `src/ui/text.rs` |
| `advance(ch, size_px) -> f32` for metric-only callers | `src/ui/text.rs` |
| `measure` / `caret_index_from_x` use `advance()` | `src/ui/text.rs` |
| `truncate_to_width` accumulates advances in O(n) | `src/ui/text.rs` |
| `draw()` borrows glyph, passes reference to `draw_glyph` | `src/ui/text.rs` |

Cache keys remain `(char, size_px.to_bits())`. Raster output and truncation behavior are byte-identical to before; only allocation patterns changed.

**Test:** `measure_equals_sum_of_advances` asserts the advance-only path matches full-string `measure()`.
