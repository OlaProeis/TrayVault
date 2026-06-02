# History List Scroll & Lookup Performance

Optimizations for the virtualized history list after the layout cache (Task 35): fewer repaints during wheel bursts and faster viewport culling / hit-testing on large lists.

Related: [`ui-perf-caches.md`](ui-perf-caches.md) (layout cache), [`render-performance.md`](render-performance.md) (wheel repaint coalescing detail), [`thumbnail-cache.md`](thumbnail-cache.md).

## Wheel scroll repaint coalescing (~60 Hz)

**Problem:** Each `MouseWheel` notch made `handle_input` return true, so `main` called `InvalidateRect` on every event. High-resolution wheels could trigger many full paints per second.

**Fix:**

| Behavior | Detail |
|----------|--------|
| Scroll offset | Updated on every wheel event (responsive feel) |
| Repaint gate | `SCROLL_REPAINT_MIN_MS` (16 ms) via `wheel_scroll_repaint` |
| Burst | First event invalidates immediately; faster events set `needs_scroll_repaint` and schedule at most one `request_window_repaint` |
| Post-paint | `clear_scroll_repaint_after_paint` and `take_deferred_scroll_repaint` flush pending frames |
| Unchanged | Scrollbar thumb drag still repaints every move |

**Key files:** `src/ui/mod.rs` (`WheelRepaint`, throttle flags), `src/ui/input.rs` (`MouseWheel`), `src/main.rs` (`on_paint`).

## Binary search on layout rows

**Problem:** With cached `Vec<EntryLayout>`, `visible_layout_range` and `hit_test_entry` still scanned all rows linearly.

**Fix:** Layouts are sorted by `y_offset`.

| Helper | Role |
|--------|------|
| `first_layout_from_top` | First index with `y_offset + height >= view_top` |
| `first_layout_below_y` | First index with `y_offset > view_bottom` (exclusive end) |
| `layout_index_at_content_y` | Row under content-space Y, respecting card height (`height - 8.0`) |

`visible_layout_range` keeps `VIRTUAL_MARGIN` and `end.max(start + 1)` semantics. `hit_test_entry` maps screen Y → content Y, binary-searches the row, then applies the same `WidgetRect` as draw.

**Tests:** Brute-force linear references in `history.rs` (`visible_layout_range_matches_linear_reference`, `hit_test_entry_matches_linear_*`).

**Key files:** `src/ui/history.rs`.

## Still synchronous (Task 39)

On `ThumbCache` miss with `image_pixels == None`, `draw_thumbnail` still calls `store.read_blob` on the UI thread. See next task in handover.
