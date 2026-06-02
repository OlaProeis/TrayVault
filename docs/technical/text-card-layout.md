# Text Card Layout

History list text/rich-text cards share one layout computation for row height and paint. Word-wrap runs at most twice per entry (full-width probe, then reduced width when the expand control reserves space).

## Problem

Previously, `history.rs` called `wrap_text_lines` up to three times per text entry on layout rebuild and again on every painted card:

1. Probe at full width to decide expand-button reserve.
2. Re-wrap at reduced width for height.
3. Re-wrap at draw time for visible cards.

## Unified layout (`text_card_layout`)

**File:** [`history.rs`](../../src/ui/history.rs)

`text_card_layout(cache, entry, content_width, expanded)` returns:

| Field | Use |
|-------|-----|
| `show_control` | Expand/collapse button visible when wrapped lines exceed `TEXT_COLLAPSED_MAX_LINES` (10) |
| `draw_lines` | Lines to paint (truncated + ellipsis on last line when collapsed) |
| `visible_line_count` | Line count for card height |

**Wrap passes:**

1. Wrap at `base_w = content_width - 16`.
2. If line count > 10, reserve expand button width (plus pin-badge gap when pinned) and re-wrap at reduced `text_w`.
3. If no expand control, reuse the probe result (single pass).

Height uses the same padding constants as draw: `TEXT_CARD_PAD_TOP/BOTTOM`, `TEXT_META_GAP`, `TEXT_PREVIEW_FONT_SIZE`, `TEXT_META_FONT_SIZE`, minimum `TEXT_ROW_HEIGHT`.

## Caching and draw handoff

**Per-entry cache:** `EntryHeightCache` in [`mod.rs`](../../src/ui/mod.rs) stores `(height, show_control, draw_lines)` keyed by `(ClipEntry::hash, expanded, thumb_max_w bucket)`. Cleared when `App::entries_version` changes.

**List layout:** `build_list_layout` calls `cached_text_entry_layout`, which hits the per-entry cache or runs `text_card_layout` once. Results populate `EntryLayout`:

- `text_draw_lines` — pre-wrapped lines for paint (`None` on image rows)
- `text_show_control` — expand button visibility

**Draw path:** `draw_history_list` passes cached lines into `draw_text_card`; paint does not call `wrap_text_lines` for list cards.

## Related docs

- Per-entry cache invalidation and list-layout pairing: [`ui-perf-caches.md`](ui-perf-caches.md)
- Card bounds / hover hit-test alignment: [`ui-views.md`](ui-views.md) (History card hover)
- `wrap_text_lines`, `truncate_to_width`: [`text.rs`](../../src/ui/text.rs)

## Verification

- `cargo test` — `history::tests::{text_card_height_matches_unified_layout, list_layout_carries_prewrapped_draw_lines, entry_height_cache_*}`
