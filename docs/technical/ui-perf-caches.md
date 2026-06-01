# UI Interaction Performance Caches

TrayVault avoids redundant work on hot input and paint paths by caching filtered list indices and reusing the persistent glyph cache for measurements.

## Display indices cache

**Problem:** `refresh_display_indices` rebuilt the pinned-first filtered list on every paint, mouse-move, scroll, and keyboard navigation. With an active search query, `build_display_indices` lowercases every text entry preview (plus HTML and source app) — O(N) allocations for N up to `max_entries` (500). Image entries skip string matching when the query is non-empty (see `ui-views.md`).

**Invalidation key:** `(App::entries_version, UiState::filter, App::filter_query)`

| Input | Source |
|-------|--------|
| `entries_version` | `App` — bumped on capture (accept), delete, pin toggle, and cap prune via `set_max_entries` |
| `filter` | Filter chip on `UiState` |
| `query` | Search string on `App` |

**Flow:**

1. Call sites still invoke `refresh_display_indices` (render, input, filter/search handlers).
2. `display_indices_key_matches` compares the current triple to `UiState::display_indices_key`.
3. On match, return without calling `build_display_indices`.
4. On miss, rebuild `display_indices`, store the key (query cloned once), increment `display_indices_rebuild_count` (tests).

**Semantics:** Pinned-first order and case-insensitive substring matching on text entries are unchanged. Image entries are omitted from filtered results while the query is non-empty. Empty filter results are cached (empty `display_indices` is valid).

**Key files:** [`app.rs`](../../src/app.rs) (`entries_version`), [`mod.rs`](../../src/ui/mod.rs) (`build_display_indices`, key + tests), [`search.rs`](../../src/ui/search.rs) (`refresh_display_indices`).

## Glyph cache reuse for hit-tests

**Problem:** `hover_key_at` (every `WM_MOUSEMOVE`), `dismiss_context_menu_unless_hit`, and `handle_main_mouse_up` each used a cold `GlyphCache::default()`. Filter chips, context menu rows, and search click-to-caret re-rasterized label glyphs through GDI on every call.

**Fix:** Pass `&mut ui.glyph_cache` into `hit_test_filter_chip`, `hit_test_context_menu`, and `caret_at_click` — the same cache warmed during paint.

**Borrow pattern:** In `hover_key_at`, read title-bar rects first, borrow `glyph_cache` only for chip measurement, then run list layout hit-test without overlapping `&mut UiState` borrows.

**Semantics:** Hit-test widths match draw (same font, same cache entries).

**Key file:** [`input.rs`](../../src/ui/input.rs).

## Image preview pixmap cache

**Problem (before cache):** `draw_image_preview` cloned full-resolution pixels (`entry.image_pixels` or `store.read_blob`), decoded BGRA→RGBA, and ran image scaling on every `WM_PAINT`. With the preview modal open, `WM_MOUSEMOVE` triggers repaints, so large images were re-decoded and re-scaled every frame.

**Invalidation key:** `(entry_id, dst_w, dst_h)` — target dimensions derived from window size and image dimensions (same scale math as before).

| Event | Action |
|-------|--------|
| Cache hit | 1:1 `blit` of cached scaled `Pixmap` at centered `(dx, dy)` |
| Entry change | Key mismatch → decode once, scale once, replace slot |
| Window resize | `dst_w`/`dst_h` change → rebuild |
| Esc / dismiss | `preview_entry_id = None` clears `preview_cache` in `input.rs` |

**Flow:**

1. Compute `(dst_w, dst_h, dx, dy)` from pixmap size and image dimensions.
2. On `(entry_id, dst_w, dst_h)` match in `UiState::preview_cache`, blit cached pixmap.
3. On miss, read pixels (in-memory or blob fallback), `scale_bilinear_rgba` once, store in single-slot cache, blit.

**Semantics:** Nearest-neighbor scaling unchanged; "Image unavailable" path preserved when pixels are missing.

**Key files:** [`preview.rs`](../../src/ui/preview.rs) (`PreviewImageCache`, `draw_image_preview`), [`mod.rs`](../../src/ui/mod.rs) (`preview_cache` field), [`input.rs`](../../src/ui/input.rs) (clear on Esc).

## Related docs

- List layout and hover bounds: [`ui-views.md`](ui-views.md)
- Glyph cache paint optimizations: [`render-performance.md`](render-performance.md)
- History thumbnails (separate cache): [`thumbnail-cache.md`](thumbnail-cache.md)
- In-memory pixel lifecycle after capture: [`storage.md`](storage.md)
- Visibility-gated relative-time timer: [`window-gdi.md`](window-gdi.md)

## Verification

- Unit tests in `src/ui/mod.rs` (`display_indices_cache_tests`) and `src/ui/preview.rs` (`preview_cache_tests`)
- `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
