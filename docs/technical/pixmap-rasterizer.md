# Hand-Rolled Pixmap Rasterizer

`src/ui/pixmap.rs` — RGBA8 CPU drawing for the immediate-mode UI. Replaced `tiny-skia` (Task 15); no paths, strokes, gradients, anti-aliasing, or PNG decode. Pipeline overview: [`rendering.md`](rendering.md).

## Buffer layout

- Top-down **RGBA8** (`width × height × 4` bytes), row-major.
- `Pixmap::new(w, h)` allocates; `Pixmap::from_vec(vec, w, h)` validates length (caller must supply RGBA).
- `Pixmap::from_bgra_vec(vec, w, h)` — build from **BGRA8** (clipboard capture, blob store, copy-back pipeline).
- `render.rs` converts finished UI pixels to **BGRA** via `widgets::write_rgba_to_bgra` directly into the GDI DIB back-buffer before present.

## BGRA vs RGBA (clipboard images)

Windows DIB / clipboard image bytes are **BGRA** end-to-end in TrayVault (`parse_dib_to_bgra`, `image_pixels`, blob `.dib` files). The UI pixmap is **RGBA**. Callers must convert before blitting captured images:

| Stage | Format |
|-------|--------|
| Capture / persist / copy-back | BGRA |
| `ThumbCache`, preview modal | BGRA → RGBA via `bgra_to_rgba`, then `scale_bilinear_rgba` + `blit` (preview caches the scaled result in `PreviewImageCache`) |
| Final window present | RGBA → BGRA via `write_rgba_to_bgra` into `GdiBuffer::bits_mut()` |

Skipping the display conversion swaps red and blue (wrong skin tones, inverted icon colors).

## API surface

| Function | Use |
|----------|-----|
| `fill_rect` | Solid rectangles (cards, title bar, overlays); clips to bounds; no AA |
| `blit` | 1:1 copy — history list thumbnails (`history.rs`) |
| `blit_scaled` | Bilinear scale blit (legacy helper; preview uses pre-scaled cache + `blit`) |
| `scale_bilinear_rgba` | Bilinear resize **RGBA8** raw pixels — `thumb_cache.rs` and `preview.rs` after `bgra_to_rgba` |
| `scale_nearest_rgba` | Nearest-neighbor resize (legacy; tests / optional callers) |
| `bgra_to_rgba` | Swap R/B channels — clipboard/blob pixels → pixmap |
| `Pixmap::from_bgra_vec` | Convenience BGRA → RGBA wrapper (tests / optional callers) |
| `Color::from_rgba8` / `to_color_u8` | Theme colors + glyph alpha blend in `text.rs` |
| `rgba_to_color` | `[u8;4]` theme fields → `Color` |

## Call sites

- **Widgets** — re-exports `fill_rect`, `Pixmap` from `widgets.rs` for all views.
- **History** — `blit` for cached thumbs from `ThumbCache`.
- **Preview** — on cache miss: `bgra_to_rgba`, `scale_bilinear_rgba`, store in `PreviewImageCache`, then 1:1 `blit`; on hit, blit cached pixmap only.
- **Thumb cache** — `bgra_to_rgba`, then `scale_bilinear_rgba` when source size ≠ target thumb size.

## Dependencies

Zero crates: pure `std`. Removing `tiny-skia` dropped ~20 transitive deps (`png`, `flate2`, `miniz_oxide`, etc.) that were unused.

## Tests (`pixmap.rs`)

- `bgra_to_rgba` red/blue swap
- `fill_rect` writes/clips as expected
- `blit` 2×2 copy
- `blit_scaled` / `scale_bilinear_rgba` dimension and blend checks
- `scale_nearest_rgba` dimension check

## Out of scope (by design)

Rounded rects, strokes, gradients. Image downscale uses bilinear filtering; nearest-neighbor remains for legacy callers.
