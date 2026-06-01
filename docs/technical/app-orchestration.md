# App Orchestration

Module: `src/app.rs`. Clipboard read/write helpers: `src/win32/clipboard.rs`. Bootstrap: `src/main.rs`.

## Role

`App` is the central state owner on the main thread. It holds the in-memory history, applies config limits, coordinates storage jobs, and exposes message-loop hooks. The clipboard monitor reads formats only; history logic lives here.

## State

| Field | Purpose |
|-------|---------|
| `config` | Loaded settings (`config.rs`) |
| `entries` | Most-recent-first `ClipEntry` list |
| `selected_index` | UI selection (Task 9) |
| `filter_query` | Search filter string; edited when `UiState::search_focused` (caret, selection anchor, scroll — see `ui-views.md`) |
| `pause_capture` | Runtime pause mirror |
| `store` | Storage worker handle |
| `hash_index` | Ref-counted SHA-256 map for O(1) global dedup |
| `next_id` | Monotonic entry id allocator |
| `window_visible` / `needs_repaint` | Visibility and dirty flag for repaint-on-demand |

## Bootstrap (`main.rs`)

1. Load `Config` and `Store::load_initial()`.
2. `Window::create(hinstance, &config)` (applies saved placement) + `SharedClipboardMonitor` with `config.capture_config()`.
3. Construct `App::new(config, loaded, store)`; call `apply_capture_config` on the monitor.
4. Register tray icon and global hotkey.
5. Wire callbacks (paint, input, clipboard, tray, hotkey):
   - `WM_CLIPBOARDUPDATE` → `monitor.try_capture()` → `App::on_clipboard_captured`
   - `WM_TIMER` → `App::on_timer_tick` (timer runs only while window visible — see `window-gdi.md`)
6. **Startup visibility:** call `show_main_window()` unless `--minimized` is on the command line (autostart). See [`autostart-startup.md`](autostart-startup.md) and [`system-tray.md`](system-tray.md).
7. Enter the message loop.

## Capture path

`ClipboardMonitor::try_capture()` returns `Result<Option<ClipEntry>>` (no in-monitor history).

`App::on_clipboard_captured`:

1. Skip if paused.
2. Dedup: consecutive same-kind+hash via `hash::is_duplicate_entry`; global via `hash_index` when `deduplicate_global` is on.
3. Assign `id`, insert at front, update `hash_index`.
4. `prune_to_cap` — remove oldest **non-pinned** entries when `len > max_entries`.
5. For each pruned image entry: `enqueue_removed_entry_delete` — blob hash only when no surviving entry still references it.
6. `store.enqueue_persist(&entries)`; for image captures, clear `image_pixels` on the new entry immediately after enqueue (worker job owns pixel clone for blob write).
7. `enqueue_prune_orphans` when anything was pruned.

Later persists (pin, delete, cap change) clone metadata only — entries no longer carry pixel buffers in memory.

## Prune-to-cap

`prune_to_cap(entries, max_entries)` scans from the tail (oldest). Pinned entries are never removed. If all entries are pinned and count exceeds the cap, no further removals occur.

## Pin / unpin / delete

| API | Behavior |
|-----|----------|
| `toggle_pin(entry_id)` | Flip `is_pinned`, `enqueue_persist`, set `needs_repaint` |
| `delete_entry(entry_id)` | Remove from `entries`, update `hash_index`, enqueue blob delete only when unreferenced + persist |

UI triggers: right-click context menu (Pin/Unpin), **Ctrl+P**, or **Delete** for removal. Context menu input rules (do not clear menu on every left down) are documented in [`ui-views.md`](ui-views.md#pinning-and-entry-context-menu).

## Copy-back

`App::copy_entry_to_clipboard(entry_id, hwnd, monitor)`:

1. Resolve entry by id; load image pixels from `image_pixels` or `Store::read_blob`.
2. `monitor.mark_own_write()` — suppresses the next `WM_CLIPBOARDUPDATE`.
3. `write_entry_to_clipboard` sets formats by kind:
   - **Text** — `CF_UNICODETEXT`
   - **RichText** — `CF_UNICODETEXT` + `"HTML Format"`
   - **Image** — optional text + `CF_DIB` (top-down 32-bpp via `encode_bgra_dib`)
4. If `close_on_copy`, hide the window.

## Config wiring

`Config::capture_config()` → `CaptureConfig` in `clipboard.rs` (capture toggles + `max_image_size_bytes`). Oversized decoded images are dropped at capture time; text/HTML are kept.

## Window placement

| Method | When |
|--------|------|
| `persist_window_geometry(hwnd, config_path)` | After `WM_EXITSIZEMOVE` (callback from `wire_callbacks`) — calls `capture_geometry_into_config`, saves config if geometry changed |
| `shutdown(hwnd, config_path)` | Quit and post-loop fallback — `sync_config`, capture geometry, `Config::save`, `Store::flush` |

Capture delegates to `win32::window::capture_geometry_into_config`; clamp/load rules live in `config.rs`. Not user-editable in the settings panel. Details: [`config.md`](config.md) (Window placement), [`window-gdi.md`](window-gdi.md).

## Related

- Capture pipeline: `clipboard-capture.md`
- Dedup rules: `hashing-dedup.md`
- Persistence jobs: `storage.md`
- Settings fields: `config.md`
- Pinning and context menu UX: `ui-views.md`
