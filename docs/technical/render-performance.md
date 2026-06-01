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

**Not done (future):** Reuse a persistent RGBA scratch buffer on `UiState` (reallocate on resize only), or draw BGRA natively into the DIB — would eliminate the remaining per-frame RGBA allocation.

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
