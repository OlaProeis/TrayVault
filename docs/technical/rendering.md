# Rendering Pipeline (hand-rolled pixmap + GDI text)

Task 8–9 modules: `src/ui/pixmap.rs`, `src/ui/render.rs`, `src/ui/text.rs`, `src/ui/theme.rs`. Glyph rasterization: `src/win32/glyph_raster.rs` (Task 16). Views and interaction: see [`ui-views.md`](ui-views.md).

## Flow

1. `WM_PAINT` → `WindowCallbacks::on_paint` → [`render_app`](../../src/ui/render.rs) builds an RGBA pixmap via `Pixmap::new`, then writes **BGRA** directly into the GDI DIB section (`GdiBuffer::bits_mut`) via `widgets::write_rgba_to_bgra` (single pass, no intermediate buffer).
2. Solid fills, 1:1 blits, and bilinear image scaling use [`pixmap.rs`](../../src/ui/pixmap.rs) (`fill_rect`, `blit`, `scale_bilinear_rgba`). Clipboard/blob **BGRA** image bytes are converted to RGBA in `thumb_cache.rs` and `preview.rs` before blit (see [`pixmap-rasterizer.md`](pixmap-rasterizer.md)).
3. Text is rasterized with Win32 GDI (`AddFontMemResourceEx` + bundled `assets/Roboto-Regular.ttf`, glyph cache keyed by `(char, size)`).
4. `GdiBuffer::present_internal` blits via `StretchDIBits` (see `window-gdi.md`).

Repaint is event-driven: `App::needs_repaint` is set on capture/show. While the window is visible, a 2 s `WM_TIMER` (started in `show_window`, stopped in `hide_window`) calls `on_timer_tick` for relative-time labels and invalidates only when dirty. See [`window-gdi.md`](window-gdi.md) (Relative-time timer).

## Pixmap rasterizer

See [`pixmap-rasterizer.md`](pixmap-rasterizer.md) for `Pixmap`/`Color`, `fill_rect`, `blit`, and bilinear image scaling.

## Text rasterization (`text.rs` + `glyph_raster.rs`)

Glyphs come from [`glyph_raster::rasterize_glyph`](../../src/win32/glyph_raster.rs):

- Load Roboto from memory via `AddFontMemResourceEx` (kept alive for the process; cached `HFONT` per size).
- Measure with `GetTextMetricsW` (ascent/height), `GetTextExtentPoint32W` (advance), and `GetCharABCWidthsFloatW` (left/right bearings, used to pad the cell so overhanging ink is not clipped).
- Ink: draw the glyph **baseline-aligned** (`SetTextAlign(TA_LEFT | TA_BASELINE)` then `TextOutW`) as white-on-black onto an offscreen 32-bpp DIB sized to the metrics cell plus a small margin, then **trim to the inked bounding box**. Reported `left`/`top` are offsets from the pen origin / baseline.
- Fallback: Segoe UI if Roboto returns empty ink (e.g. glyphs not in Roboto).
- U+2026 (`…`) uses a hand-stamped three-dot glyph (truncate suffix) so ellipsis always renders.

> ⚠️ **`GdiFlush` is mandatory.** GDI batches drawing into a `CreateDIBSection` bitmap, so `glyph_raster::draw_text_cell` calls `GdiFlush()` after `TextOutW` and **before** reading the section bits. Skipping it returns half-written scanlines — text appears "horizontally shredded" with dropped rows. (The window back-buffer in `gdi.rs` is exempt because it writes its DIB via CPU memcpy, never GDI drawing calls.) The DIB section is created with a **null `HDC`** so it is always a true 32-bpp section, regardless of the monochrome memory DC.

Coverage (alpha8) bitmaps are blended manually into the pixmap in `blend_pixel`.

### Color blending

`Color` channels are **normalized 0..=1**. Before blending glyph coverage, convert via `color.to_color_u8()` and use byte values (0–255). Treating normalized channels as 0–255 produces near-invisible text.

### Glyph placement (screen Y-down)

The glyph is drawn with its baseline on a known row, then trimmed. `left`/`top`
are the trimmed bitmap's offset from the pen origin / baseline (screen Y-down, so
`top` is negative for ink above the baseline). `text.rs` places each glyph at:

```text
screen_x = round(pen_x + glyph.left)
screen_y = round(baseline_y + glyph.top)
pen_x   += glyph.advance_width
```

### Cache

`GlyphCache` keys rasterized glyphs by `(char, size_px.to_bits())`. `measure()` and `draw()` advance the pen by `advance_width`. `caret_index_from_x()` maps a horizontal click to a UTF-8 byte index in a string. The title-bar search field uses **14px** for both `input_box` draw and caret measurement via [`search_edit.rs`](../../src/ui/search_edit.rs) (see [`ui-views.md`](ui-views.md)).

## Themes

- Palettes: `Theme::light()` / `Theme::dark()` in `theme.rs` (muted slate accent/selection, explicit RGBA fields).
- **Dark** — accent `#58626E`, selection `#35373A` (subtle row/chip highlight on `#2D2D2D` cards).
- **Light** — accent `#5F6B7A`, selection `#EBEDF0`.
- Accent is used for active filter chips, selected-row left bar, focused search underline, and scroll thumb. Selection is used for hovered/selected list rows.
- Config `theme = Light | Dark | System`; **System** reads `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme` (`1` = light).

## Text helpers

- `format_relative_time(created_at, now_millis)` — "Just now", "N min ago", "Yesterday", "N days ago", then a short date.
- `truncate_to_width` — ellipsis truncation for entry previews.
