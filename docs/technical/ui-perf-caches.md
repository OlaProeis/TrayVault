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

## List layout cache

**Problem:** `build_list_layout` walked every filtered entry and computed text-wrap heights via `wrap_text_lines` on every paint, mouse-move, wheel event, and scrollbar hit-test — often twice per input event (input handler + paint). Drawing is virtualized; layout was not.

**Invalidation key:** `(App::entries_version, UiState::filter, App::filter_query, UiState::expanded_version, thumb_max_w bucket)`

| Input | Source |
|-------|--------|
| `entries_version`, `filter`, `query` | Same as display-indices cache (layout runs after `refresh_display_indices`) |
| `expanded_version` | `UiState` — bumped via `bump_expanded_version()` when expand/collapse toggles |
| `thumb_max_w bucket` | `entry_inner_width(client_w) as u32` — same bucket as `ThumbCache::set_width_bucket` |

**Flow:**

1. Call sites invoke `refresh_list_layout` (render, input, scroll_bar).
2. `list_layout_key_matches` compares the current key to `UiState::list_layout_key`.
3. On match, leave `cached_list_layout` unchanged.
4. On miss, call `build_list_layout`, store result and key, increment `list_layout_rebuild_count` (tests).

**Semantics:** `y_offset` / `height` values match uncached `build_list_layout` output. Scrollbar and hit-test paths clone from the cache only when they need an owned copy (`layout_for_list`).

**Key files:** [`history.rs`](../../src/ui/history.rs) (`refresh_list_layout`, `build_list_layout`), [`mod.rs`](../../src/ui/mod.rs) (key + `expanded_version`), [`render.rs`](../../src/ui/render.rs), [`input.rs`](../../src/ui/input.rs), [`scroll_bar.rs`](../../src/ui/scroll_bar.rs).

## Virtualized layout lookup (binary search)

**Problem:** With Task 35 caching a stable `Vec<EntryLayout>`, paint and hit-test still scanned every row linearly for viewport culling (`visible_layout_range`) and mouse hits (`hit_test_entry`).

**Fix:** Layouts are sorted by `y_offset`. `visible_layout_range` uses two binary searches: first index with `y_offset + height >= view_top`, first with `y_offset > view_bottom` (same semantics as before, including `VIRTUAL_MARGIN` and `end.max(start + 1)`). `hit_test_entry` maps screen Y to content Y and binary-searches the containing row, then applies the same card rect (`height - 8.0`) as draw.

**Tests:** Brute-force linear references in `history.rs` tests compare both helpers on synthetic and `build_list_layout` fixtures.

**Key files:** [`history.rs`](../../src/ui/history.rs) (`first_layout_from_top`, `first_layout_below_y`, `layout_index_at_content_y`).

## Per-entry text row height cache

**Problem:** Even when the list-layout cache hits, a miss still ran `wrap_text_lines` up to three times per text entry (`text_wrap_layout` probe, `text_preview_visible_lines` height wrap, `visible_text_lines` draw wrap). Entry text is stable between captures; only expand/collapse, card width, or entry mutation should recompute a single row's layout.

**Invalidation key (per entry):** `(ClipEntry::hash, expanded flag, thumb_max_w bucket)`

| Input | Source |
|-------|--------|
| `content_hash` | `ClipEntry::hash` — content change on capture/dedup |
| `expanded` | `UiState::expanded_text_entries` contains entry id |
| `thumb_max_w bucket` | `entry_inner_width(client_w) as u32` |

**Global invalidation:** `EntryHeightCache::sync_entries_version` clears the entire map when `App::entries_version` changes (capture, delete, pin toggle, cap prune).

**Flow:**

1. `refresh_list_layout` syncs the height cache to `entries_version`, then calls `build_list_layout`.
2. For text/rich-text rows, `cached_text_entry_layout` looks up `(entry.id → key → height, show_control, draw_lines)` via `text_card_layout` (at most two wrap passes: full-width probe, then reduced width when expand control is shown).
3. On key match, return cached layout (skip wrap).
4. On miss, compute once, store height + pre-wrapped draw lines, return.
5. `EntryLayout::text_draw_lines` / `text_show_control` feed `draw_text_card` so paint does not re-wrap.

**Pairing with list-layout cache:** When `expanded_version` or `thumb_max_w` changes, the list layout rebuilds but unchanged rows still hit the per-entry cache. Image row heights are not cached (derive from dimensions + `thumb_max_w` without text wrap).

**Key files:** [`history.rs`](../../src/ui/history.rs) (`text_card_layout`, `cached_text_entry_layout`, `build_list_layout`), [`mod.rs`](../../src/ui/mod.rs) (`EntryHeightCache`, `EntryHeightKey`). Full layout pipeline: [`text-card-layout.md`](text-card-layout.md).

## Hover repaint gating

**Problem:** Main-view `MouseMove` always returned true from `handle_input`, triggering `WM_PAINT` + layout work on every pixel of cursor travel even when hover highlight could not change.

**Fix:** After `hover_key_at`, compare to `UiState::hover_key`. Unchanged → return false (no `InvalidateRect`). Changed → store key and repaint. Scrollbar gutter hover still repaints (via `touch_scrollbar` + `gutter_hit`). Settings, help, and image preview overlays still repaint every move.

**Semantics:** Card hover uses the same bounds as `hit_test_entry`; moving within one card keeps the same `entry_index` and needs no extra frame. Filter chip, settings gear, and close button are part of `HoverKey`.

**Key files:** [`input.rs`](../../src/ui/input.rs) (`mouse_move_needs_repaint`, `hover_key_at`), [`mod.rs`](../../src/ui/mod.rs) (`HoverKey`), [`ui-views.md`](ui-views.md) (History card hover).

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

- Unit tests in `src/ui/mod.rs` (`display_indices_cache_tests`) and `src/ui/history.rs` (`entry_height_cache_*`, list-layout cache tests) and `src/ui/preview.rs` (`preview_cache_tests`)
- `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
