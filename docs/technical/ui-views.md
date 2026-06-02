# UI Views and Interaction (immediate-mode)

Task 9 modules: `src/ui/{widgets,history,search,search_edit,titlebar,preview,settings,input,render,mod}.rs`; input routed from `src/win32/window.rs`.

## Render flow

1. `WM_PAINT` → `render_app(app, ui, size)` in [`render.rs`](../../src/ui/render.rs).
2. `UiState::begin_frame()` clears per-frame flags (`mouse_left_pressed`, `hot_widget`) before layout.
3. Full-window background fill, then **scrollable content** (virtualized history or empty state) in the region below `header_total_height()`.
4. **Sticky header** — [`draw_sticky_header()`](../../src/ui/search.rs) redrawn on top of content: opaque search/filter band (`theme.background`), then title bar (28px drag region + ×) and search bar + filter chips. Settings overlay uses the same title-bar-on-top pattern after its scrollable rows.
5. Context menu, image preview, and help draw above the header when active.
6. RGBA → BGRA handoff unchanged (see [`rendering.md`](rendering.md)).

History cards are not scissor-clipped during draw; partially scrolled rows can paint above `content_top`. Redrawing the header after the list keeps the chrome opaque and fixed while the list scrolls underneath. Do not move header paint before the list without adding clip rects — that was the original scroll-over-header bug.

**Paint is display-only.** Hover highlights and right-click menu *opening* are resolved during paint from mouse position. **Left-click actions** (filter chips, row copy, context-menu items, settings gear) are handled on `LButtonUp` in [`input.rs`](../../src/ui/input.rs) — not during paint — because `begin_frame()` clears `mouse_left_pressed` before widgets would see it. On `LButtonDown`, the context menu is dismissed only when the press is **outside** the menu (`dismiss_context_menu_unless_hit`); clearing the menu unconditionally on down would prevent `LButtonUp` from ever selecting Pin/Copy/Delete.

## State split

| Layer | Owns |
|-------|------|
| `App` | History entries, `filter_query`, `selected_index`, pin/delete/copy actions, persistence |
| `UiState` | Scroll, `EntryFilter` chip, help/preview/settings flags, context menu, mouse, `display_indices`, search focus/caret/selection (`search_sel_anchor`) |
| `App::entries_version` | Monotonic counter bumped on capture (accept), delete, pin toggle, and cap prune via `set_max_entries`; drives display-index cache invalidation |

Filtered display order: pinned entries first (MRU among pinned), then unpinned. Built by `build_display_indices()` in [`mod.rs`](../../src/ui/mod.rs). **`refresh_display_indices()`** ([`search.rs`](../../src/ui/search.rs)) rebuilds only when `(entries_version, filter, query)` changes — paint, mouse-move, and scroll call refresh but skip the O(N) filter pass on cache hits (important when a search query lowercases every text entry preview).

## Virtualized history

[`history.rs`](../../src/ui/history.rs):

- Per-item height: `TEXT_ROW_HEIGHT` (56px) for text; image rows use scaled thumbnail height + 36px metadata.
- `build_list_layout()` takes `thumb_max_w` from `entry_inner_width(client_width)` so layout matches draw scale.
- `visible_layout_range()` returns indices intersecting viewport ± `VIRTUAL_MARGIN` (80px).
- Only visible cards are laid out and drawn; count stored in `UiState::last_visible_count`.
- `hit_test_entry()` maps client coordinates to an entry index for mouse-up, double-click preview, and `HoverKey` tracking.

### History scrollbar

[`scroll_bar.rs`](../../src/ui/scroll_bar.rs) — custom vertical thumb for the history list (no Win32 scrollbar control):

| Constant | Value | Role |
|----------|-------|------|
| `GUTTER_W` | 12px | Hit zone for hover reveal and track clicks (wider than the visible thumb) |
| `THUMB_W` | 8px | Visible thumb width (centered in gutter, 2px inset each side) |
| `VISIBLE_HOLD_MS` | 1000 | Full opacity after wheel/drag/gutter hover |
| `FADE_MS` | 300 | Fade-out duration before hidden |

**Auto-hide:** Hidden by default. `UiState::touch_scrollbar()` extends `scrollbar_visible_until` (GetTickCount) on wheel scroll, gutter hover, and thumb drag. [`render.rs`](../../src/ui/render.rs) draws the thumb only when opacity &gt; 0; mid-fade repaints are scheduled from `on_paint` in [`main.rs`](../../src/main.rs).

**Drag:** `LButtonDown` on the thumb sets `scrollbar_drag_grab_y` and calls `SetCapture(hwnd)` so moves outside the client area still update scroll. `MouseMove` maps Y → `scroll_offset`. Track click (gutter, not thumb) pages ~90% of the viewport. `LButtonUp` clears grab state only — **do not** call `ReleaseCapture()` from input handlers; it sends `WM_CAPTURECHANGED` synchronously and re-enters `WndProc` while `RefCell` borrows are active (same failure mode as `SendMessage(SC_MOVE)`). Windows releases capture on button-up; external capture loss is handled via deferred `WM_CAPTURECHANGED` in [`window.rs`](../../src/win32/window.rs) (`InputEvent::CaptureLost`). `scrollbar_suppress_click` prevents accidental row copy on release.

**Resize conflict:** The thumb sits in the right 6px resize strip. [`window.rs`](../../src/win32/window.rs) `on_nc_client_override` returns `HTCLIENT` for the gutter over the scroll track (corners keep resize grips). `UiState::last_content_height` from the last paint feeds NC hit-test without rebuilding layout.

### History card hover

Hover highlight and row hit-testing must share **identical card bounds** — the same rectangle [`card()`](../../src/ui/widgets.rs) uses during paint:

| Dimension | Value | Notes |
|-----------|-------|-------|
| `x` | `PADDING` (`content_x`) | Horizontal list inset |
| `w` | `client_width - 2 × PADDING` (`content_w`) | Matches draw |
| `y` | `content_top + layout.y_offset - scroll_offset` | Screen Y of row |
| `h` | `layout.height - 4.0` | Card is 4px shorter than layout slot (gap between rows) |

[`draw_history_list`](../../src/ui/history.rs) calls `card(..., layout.height - 4.0)`; [`hit_test_entry`](../../src/ui/history.rs) must pass the same `content_x`, `content_w`, and height. Do **not** hit-test the full layout row (`h = layout.height`, full client width) — that leaves a 4px band below each card where the cursor is still “over” the row but paint shows no hover, so highlight **lags behind** when moving up until `HoverKey` finally changes.

**Repaint:** [`handle_input`](../../src/ui/input.rs) `MouseMove` on the main view compares [`HoverKey`](../../src/ui/mod.rs) before/after `hover_key_at`; unchanged key skips `InvalidateRect` (card hit-test bounds match draw, so moving within one card does not need extra frames). Still repaints when the scrollbar gutter is hit (reveal/fade), when `HoverKey` changes (card, filter chip, settings, close), or on settings/help/preview overlays.

**Regression fixed (2026):** Full-row hit test + `HoverKey`-gated repaint caused sticky highlight on the row below when moving the cursor upward.

### Image list thumbnails

List previews are **scale-to-fit** inside the card’s inner width (client width minus `PADDING` and `CARD_INNER_PAD`). Both **width and height** caps matter: raising only `MAX_THUMB_WIDTH` does not enlarge typical landscape screenshots if `MAX_THUMB_HEIGHT` is too low.

| Constant | Value | Role |
|----------|-------|------|
| `entry_inner_width()` | derived | Max width passed into scale/layout (`content_w − 20`) |
| `MAX_THUMB_WIDTH` | 1200px | Hard cap on displayed thumbnail width |
| `MAX_THUMB_HEIGHT` | 900px | Hard cap on displayed thumbnail height |
| `IMAGE_THUMB_MAX_HEIGHT` | 120px | Minimum fit-box height on narrow windows |
| Fit box height | `max(120, inner_w × 0.85)` capped at 900 | Grows with window until height cap binds |

Scale: `min(box_w / img_w, box_h / img_h)` (upscale allowed). Example 1817×1476 at inner_w ≈ 856 → ~856×695 display pixels (full card width). Pixel filter: bilinear (`scale_bilinear_rgba` in [`pixmap.rs`](../../src/ui/pixmap.rs)).

**Cache ([`thumb_cache.rs`](../../src/ui/thumb_cache.rs)):** `ThumbCache::get()` consults `(entry_id, dst_w, dst_h)` without reading pixels or disk. On a miss, full-resolution **BGRA** pixels (from `image_pixels` or a one-time blob load) are converted to RGBA, downscaled once via bilinear filtering into a `Pixmap`, and stored in `UiState::thumb_cache`. Subsequent repaints (e.g. mouse-move) blit from cache and skip blob I/O. Cache clears when `set_width_bucket()` sees a new integer `thumb_max_w` (window resize). Do not key by entry id alone — cap changes would reuse stale small bitmaps.

**Preview modal** ([`preview.rs`](../../src/ui/preview.rs)): scale-to-fit over the full client area (not the list caps above). On cache miss: BGRA → RGBA, bilinear downscale, store in `UiState::preview_cache` (`PreviewImageCache`); cache hits 1:1 `blit` the pre-scaled pixmap. Pixels from `image_pixels` or `Store::read_blob` on miss. Cache clears on Esc (`input.rs`). See [`ui-perf-caches.md`](ui-perf-caches.md).

**Perf:** `UiState::glyph_cache` persists across paints (text not re-rasterized every frame) and is reused for input hit-tests (`hit_test_filter_chip`, `hit_test_context_menu`, `caret_at_click` in [`input.rs`](../../src/ui/input.rs)) so mouse-move hover does not cold-rasterize chip/menu labels every frame. `preview_cache` avoids per-frame decode/scale while the modal is open. Main-view `MouseMove` gates repaints on `HoverKey` change (see **History card hover** and [`ui-perf-caches.md`](ui-perf-caches.md)). Avoid re-entrant Win32 calls during input (`SendMessageW(SC_MOVE)`, `ReleaseCapture()`) — see [`window-gdi.md`](window-gdi.md) and [`message-loop-callbacks.md`](message-loop-callbacks.md).

## Widgets

[`widgets.rs`](../../src/ui/widgets.rs) — immediate-mode primitives sharing `UiContext` (mouse, theme, glyph cache, hover/active widget ids):

- `Card`, `InputBox`, filter chips, dividers, `Button`, `ScrollableList`, `draw_context_menu`.
- **Hover:** history cards use `theme.selection` when the mouse is over the **card rect** (same bounds as `hit_test_entry`; keyboard-selected row also gets a teal accent bar on the left).
- `context_menu_labels(is_pinned)` — shared `["Copy", "Pin"|"Unpin", "Delete"]` for draw and hit-test (must stay in sync).
- `hit_test_context_menu()` mirrors `draw_context_menu` layout (28px rows, width from glyph measure).

## Pinning and entry context menu

**Data:** `ClipEntry::is_pinned` persisted in `entries.dat` (see `storage.md`). **Actions:** `App::toggle_pin(entry_id)` in [`app.rs`](../../src/app.rs); **Ctrl+P** on the selected row in [`input.rs`](../../src/ui/input.rs).

**Display order:** `build_display_indices()` ([`mod.rs`](../../src/ui/mod.rs)) puts pinned entries first (MRU among pinned), then unpinned. Filter chip **Pinned** shows only `is_pinned` rows. When both pinned and unpinned are visible, [`history.rs`](../../src/ui/history.rs) draws a **Pinned** divider after the last pinned card (`show_pinned_divider_after`).

**Context menu lifecycle:**

1. **Open:** While `mouse_right_down` and the cursor is over a card, [`draw_history_list`](../../src/ui/history.rs) sets `right_click`; [`render.rs`](../../src/ui/render.rs) stores `UiState::context_menu { entry_id, x, y }`. Menu stays open after right-button release until dismissed or an item is chosen.
2. **Dismiss on outside left press:** `LButtonDown` → `dismiss_context_menu_unless_hit` — clears menu only if the press misses all menu rows.
3. **Item action on left up:** `handle_main_mouse_up` hit-tests with `context_menu_labels` + `hit_test_context_menu`; choice 1 calls `toggle_pin` and `refresh_display_indices`. Missed click clears the menu and returns (no row copy underneath).

**Regression fixed (2025):** Clearing `context_menu` unconditionally on `LButtonDown` removed the menu before `LButtonUp` could select Pin — menu looked fine but pinning never ran. Keyboard **Ctrl+P** still worked because it bypasses the menu.

## Search and filters

[`search.rs`](../../src/ui/search.rs) owns filter chip layout (`header_total_height()` = title bar + filter row + padding), [`draw_filter_row()`](../../src/ui/search.rs) for filter chips, and [`draw_sticky_header()`](../../src/ui/search.rs) for the composite chrome redrawn after the history list. Search input lives in the title bar ([`titlebar.rs`](../../src/ui/titlebar.rs)). Layout helpers (14px metrics, horizontal scroll, click-to-caret with scroll, selection range) live in [`search_edit.rs`](../../src/ui/search_edit.rs). There is **no Win32 edit control** — the field is an `input_box` widget (scrolled draw + optional selection highlight) plus a software caret in `titlebar`.

### Sticky header

| Constant / fn | Value / role |
|---------------|--------------|
| `TITLE_BAR_HEIGHT` | 28px — search field + settings icon + hide-to-tray × ([`titlebar.rs`](../../src/ui/titlebar.rs)) |
| `FILTER_ROW_HEIGHT` | 36px — All / Text / Images / Pinned chips |
| `header_total_height()` | Title bar + filter row + 8px padding; top of scroll viewport |
| `draw_sticky_header()` | Fills below title bar with `theme.background`, then `draw_title_bar` + `draw_filter_row` |

Called from [`render.rs`](../../src/ui/render.rs) **after** `draw_history_list` / empty state so scrolled entry cards cannot cover the filter row or × button.

### Search state

| Field | Location | Role |
|-------|----------|------|
| `filter_query` | `App` | Filter string; changing it invalidates the display-index cache and rebuilds on the next `refresh_display_indices` |
| `search_focused` | `UiState` | When true, `WM_CHAR` edits the query; accent underline + caret drawn |
| `search_caret` | `UiState` | UTF-8 byte index for insert/delete and caret draw |
| `search_sel_anchor` | `UiState` | Selection anchor; active range when `search_sel_anchor != search_caret` |
| `search_input_rect` | `UiState` | Last-painted search bounds; used for mouse-up hit test |

Defaults: `UiState::new()` sets `search_focused = true`. Each time the window is shown (`show_main_window` in [`main.rs`](../../src/main.rs)), focus is restored, `search_caret` is set to `filter_query.len()`, and `search_sel_anchor` matches the caret (collapsed selection).

### Text layout (`search_edit.rs`)

| Constant / fn | Role |
|---------------|------|
| `SEARCH_TEXT_SIZE_PX` | **14.0** — must match [`input_box`](../../src/ui/widgets.rs) draw size for the title-bar search field (caret and glyphs share metrics) |
| `search_scroll_x()` | Horizontal offset when the query is wider than the inner box; keeps the caret visible |
| `search_caret_pixel_x()` | Client X for the 1.5px caret bar (prefix width minus scroll) |
| `caret_at_click()` | Maps click X → byte index via [`caret_index_from_x`](../../src/ui/text.rs), accounting for scroll |
| `selection_range()` / `has_selection()` | Normalized `(start, end)` for highlight and edit operations |

Overflowing text is drawn at `text_x - scroll_x` inside `input_box`; the caret uses the same scroll so it stays aligned with visible glyphs.

### Focus, caret, selection, and typing

- **Paint:** When `search_focused`, `draw_title_bar` passes `scroll_x` and an optional selection range into `input_box` (selection fill uses `theme.selection`), then draws a 1.5px accent caret via `search_caret_pixel_x`.
- **Click:** On `LButtonUp`, if `(x, y)` is inside `search_input_rect`, set `search_focused = true`, `search_caret = caret_at_click(...)`, and collapse selection (`search_sel_anchor = search_caret`). The search field uses `HTCAPTION` for window drag ([`window-gdi.md`](window-gdi.md)), so clicks there arrive via `WM_NCLBUTTONUP` → synthetic `LButtonUp` when movement ≤4px; settings/close still use client `LButtonUp`. Do **not** rely on `input_box` click during paint — `begin_frame()` clears `mouse_left_pressed` before widgets run (same pattern as settings rects; see **Paint is display-only** above).
- **Drag:** Click-and-drag in the search field moves the window (native caption drag); a drag beyond the click slop does not focus the field.
- **Type:** [`handle_char`](../../src/ui/input.rs) runs only when `search_focused` and no overlay is open. If a range is selected, typing or backspace/delete removes it first. Printable chars `insert` at `search_caret`; backspace (`WM_CHAR` `\x08`) removes the previous codepoint (or the selection).
- **Caret keys (search focused):** [`handle_key_down`](../../src/ui/input.rs) — **Left** / **Right** / **Home** / **End** move the caret by UTF-8 codepoint (`prev_char_boundary` / `next_char_boundary`); without **Shift**, `search_sel_anchor` follows the caret; with **Shift**, the anchor stays fixed to extend the selection. **Delete** forward-deletes the codepoint after the caret, or the selection (does **not** delete the selected history entry when search is focused). When search is **not** focused, **Delete** deletes the selected entry.
- **Ctrl+A (search focused):** Selects the entire `filter_query` (`search_sel_anchor = 0`, `search_caret = len`).
- **Navigate:** ↑ / ↓ always move selection on the **filtered** list (`move_selection`); they do not move the text caret regardless of search focus.
- **Esc:** Clears `filter_query` and resets `search_caret` and `search_sel_anchor` to 0 (or hides the window if the query is already empty).

Filtering is case-insensitive over text preview, HTML, and source app name (`entry_matches_filter` in [`mod.rs`](../../src/ui/mod.rs)). **Image entries are excluded whenever the query is non-empty** — they stay visible with an empty search bar (including under the **Images** chip), but typing any character removes them from results. Rationale: image cards are not meaningfully searchable (preview is `Image W×H`), while capture metadata often contains strings like `Screenshot.png` or snipping-tool process names that falsely match arbitrary letters.

### Filter chips

All / Text / Images / Pinned — **`hit_test_filter_chip()`** on mouse-up updates `UiState::filter` and calls `refresh_display_indices` (cache miss → rebuild).

## Input routing

[`input.rs`](../../src/ui/input.rs) handles `InputEvent` from the message loop:

| Input | Action |
|-------|--------|
| ↑ / ↓ | Move selection on **filtered** list; clamp scroll |
| Page Up / Page Down | Scroll history by one visible list viewport (does not move selection) |
| Enter | Copy selected entry |
| Esc | Clear search, or hide window if search empty; dismiss overlays |
| Delete | Search focused → forward-delete or delete selection; else delete selected entry |
| ← / → / Home / End | Search focused → move caret (Shift extends selection); else unhandled |
| Ctrl+A | Search focused → select all in query |
| Ctrl+P | Toggle pin |
| F1 / Shift+? | Toggle help overlay |
| Mouse move | Update cursor; repaint on main view when `HoverKey` changes or scrollbar gutter hit; overlays always repaint on move |
| **Mouse down (left)** | Dismiss context menu if press is outside it (keep menu open for item clicks) |
| **Mouse up (left)** | Search field → focus + caret (client `LButtonUp`, or synthetic from `WM_NCLBUTTONUP` when search uses caption drag); filter chip → change filter; history row → copy entry; context menu item → action; settings gear → open settings |
| Double-click image | Open preview modal |
| Right-click (hold) | Context menu opens on paint while `mouse_right_down` |
| Wheel | Scroll history; reveal scrollbar |
| Scrollbar thumb drag | Drag to scroll; stays visible while dragging |
| Scrollbar track click | Page up/down (~90% viewport); suppresses row copy |
| Title bar drag | Search field and title-bar gaps: native move via `WM_NCHITTEST` → `HTCAPTION` (not `SendMessage` from input) |
| Title bar × | Hide to tray |

`handle_main_mouse_up()` on `LButtonUp` performs hit-testing for the search field (`search_input_rect`), filters (`hit_test_filter_chip`), list rows (`hit_test_entry`), context menu (`hit_test_context_menu`), and the settings button rect stored from the last paint. Search receives `LButtonUp` either from client mouse-up (legacy path) or from `window.rs` after a caption click without drag.

Copy uses `App::copy_entry_to_clipboard()`; returns whether to hide the window (`close_on_copy`).

Settings panel clicks use the same mouse-up pattern via `handle_settings_mouse_up()` (hit rects from last paint). See [`settings-panel.md`](settings-panel.md).

## Overlays

- **Image preview** ([`preview.rs`](../../src/ui/preview.rs)): modal overlay, scale-to-fit (cached scaled pixmap), dimensions label, Esc to close (clears `preview_cache`).
- **Help** ([`preview.rs`](../../src/ui/preview.rs)): hotkey reference panel, F1/`?` toggle, Esc dismiss.
- **Settings** ([`settings.rs`](../../src/ui/settings.rs)): full settings overlay (Task 13).
- **Empty state**: friendly message when no entries or no filter matches.

## Window integration

[`window.rs`](../../src/win32/window.rs) dispatches `WM_KEYDOWN`, `WM_CHAR`, mouse, and wheel messages via `WindowCallbacks::on_input(event, hwnd, client_width, client_height)`. [`main.rs`](../../src/main.rs) wires `handle_input()` and repaints on change.
