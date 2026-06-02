# Settings Panel

Modules: `src/ui/settings.rs`, `src/ui/settings_input.rs`, `src/ui/input.rs`, `src/ui/mod.rs`, `src/app.rs`, `src/main.rs`. Related: [`config.md`](config.md), [`global-hotkey.md`](global-hotkey.md), [`system-tray.md`](system-tray.md), [`autostart-startup.md`](autostart-startup.md).

## Opening settings

- **Tray:** Settings menu item shows the main window and sets `UiState.show_settings`.
- **Main UI:** Settings icon (⚙) in the title bar next to the close button (`src/ui/titlebar.rs`).
- **Close:** `Esc`, the **←** back button beside the Settings title, or the highlighted gear icon in the title bar commits pending text fields and exits the overlay.

`UiState::open_settings` copies current config into edit buffers, clears search focus, and clears errors.

## Layout

- Fixed header: back arrow + “Settings” title (does not scroll).
- Scrollable rows below with visible labels and short hint text for numeric fields and non-obvious toggles (no tooltips).
- Title bar in settings mode hides the search field so keyboard input routes to settings text fields.

## Controls

| Control | Config key | Runtime effect |
|---------|------------|----------------|
| Pause capture | `pause_capture` | `App::set_pause_capture`, `CaptureConfig`, tray tooltip |
| Max entries | `max_entries` | Immediate prune via `App::set_max_entries` |
| Deduplicate globally | `deduplicate_global` | Future capture dedup in `App::on_clipboard_captured` |
| Hotkey | `hotkey` | `HotkeyHandle::reregister_strict`; revert on parse/register failure |
| Theme | `theme` | `resolve_theme` on next paint (`Light` / `Dark` / `System` chips) |
| Capture images | `capture_images` | `ClipboardMonitor::set_config` |
| Capture rich text | `capture_rich_text` | `ClipboardMonitor::set_config` |
| Close on copy | `close_on_copy` | `App::copy_entry_to_clipboard` hide behavior |
| Show in taskbar when open | `show_in_taskbar` | Toggles `WS_EX_TOOLWINDOW` on the main window while visible; applies immediately if the window is open. See [`window-gdi.md`](window-gdi.md) (Taskbar visibility). |
| Max image size (MB) | `max_image_size_mb` | `CaptureConfig.max_image_size_bytes` |
| Image blob codec | `image_blob_codec` | `"png"` (default, lossless) or `"jpeg"`; affects **new** blob writes only |
| JPEG quality | `jpeg_quality` | 1–100 (default 90); shown only when codec is JPEG; lossy paste when JPEG |
| Start with Windows | `autostart` | Run key via `App::set_autostart`; revert + inline error on failure |

## About (read-only)

At the bottom of the scrollable list (after a separator):

- **Version** — `env!("CARGO_PKG_VERSION")` from `Cargo.toml` (`settings::APP_VERSION`).
- **GitHub** — clickable row opens `https://github.com/OlaProeis/TrayVault` in the default browser via `win32::shell::open_url` (`ShellExecuteW`). On failure, `UiState.settings_error` shows a short inline message.

Constants: `settings::GITHUB_REPO_URL`, `settings::GITHUB_LINK_LABEL` (display text).

## Text fields (`max_entries`, `hotkey`, `max_image_size_mb`) commit on **Enter**, focus change, or when closing settings. Invalid values show `UiState.settings_error` and restore the last good edit buffer. Focused fields show an accent caret, support click-to-place caret, horizontal scroll when overflowing, text selection (Shift+arrows, Ctrl+A), and Backspace/Delete — same behavior as the search field (`search_edit.rs`).

## Persistence

`App::persist_config` writes `config.toml` after each successful change. Pause state is also synced on shutdown via `App::sync_config`.

## Pause / tray sync

`app.pause_capture` and `config.pause_capture` stay aligned. Tray **Pause capture** / **Resume capture** and the settings toggle both call `App::set_pause_capture` or `toggle_pause_capture`, refresh `CaptureConfig`, and update the tray tooltip.

## Hotkey changes

Settings use strict parsing (`parse_hotkey`, no silent default). `HotkeyHandle::reregister_strict` unregisters the old global hotkey before registering the new combination. On failure, the previous hotkey is re-registered and the hotkey text field reverts.

## Scroll

The settings list scrolls with the mouse wheel (`settings_scroll` in `UiState`) when content exceeds the viewport.

## Hit testing

`SettingsRects` stores per-frame `WidgetRect` values for toggles, inputs, and theme chips. `settings_input.rs` handles clicks, keyboard input, and wheel events when `SettingsHooks` (hwnd, hotkey, tray pointers) is passed from `main.rs`.
