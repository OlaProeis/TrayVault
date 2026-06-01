# Configuration

Module: `src/config.rs`. Settings file: `%LOCALAPPDATA%\TrayVault\config.toml`.

## Parser

Hand-rolled minimal TOML subset (pure `std`, no `serde`/`toml`):

- Top-level `key = value` lines only — no tables or arrays.
- Blank lines and `#` comment lines are ignored.
- Split on the first `=`; trim key/value whitespace.
- Value types: double-quoted strings (`\"`, `\\` escapes), booleans (`true`/`false`), integers, floats, and the `theme` enum (`Light`, `Dark`, `System`).

## Settings

| Key | Type | Default |
|-----|------|---------|
| `max_entries` | u32 | 500 |
| `deduplicate_global` | bool | false |
| `hotkey` | string | `"Alt+V"` |
| `autostart` | bool | false |
| `theme` | enum | System |
| `capture_images` | bool | true |
| `capture_rich_text` | bool | true |
| `close_on_copy` | bool | true |
| `show_in_taskbar` | bool | true |
| `pause_capture` | bool | false |
| `max_image_size_mb` | f32 | 5.0 |
| `window_x` | int (optional) | omitted — Windows default placement |
| `window_y` | int (optional) | omitted — Windows default placement |
| `window_client_w` | u32 | 900 |
| `window_client_h` | u32 | 640 |

When `show_in_taskbar` is `true`, the main window appears on the Windows taskbar while it is visible; when `false`, the window stays a tool window even when open. Hidden windows never show on the taskbar. See [`window-gdi.md`](window-gdi.md) (Taskbar visibility).

## Window placement

The main window’s **client size** and optional **screen position** are persisted so restarts (dev rebuild, release install, reboot + relaunch) reopen where the user left off. Implementation: [`window-gdi.md`](window-gdi.md) (Window placement persistence), [`app-orchestration.md`](app-orchestration.md) (`persist_window_geometry`, `shutdown`).

| Key | Stored value | Notes |
|-----|--------------|-------|
| `window_client_w` / `window_client_h` | Client pixels | Always written on save; defaults 900×640 |
| `window_x` / `window_y` | Outer window top-left (screen coords) | Written as a pair after the user moves the window; omitted on first run |

**Bounds** (`src/config.rs`):

- `clamp_client_dimensions` — 400×320 minimum, 8192×8192 maximum (applied on load via `normalize_window_fields` and on capture).
- Position clamping (in `window.rs`) — at least 48px of the outer window must remain on the virtual screen (`GetSystemMetrics` `SM_*VIRTUALSCREEN*`).

**When values are saved**

| Event | Handler |
|-------|---------|
| User finishes drag-resize or move | `WM_EXITSIZEMOVE` → `App::persist_window_geometry` |
| Tray **Quit** or abnormal post-loop shutdown | `App::shutdown(hwnd, …)` → `capture_geometry_into_config` then `Config::save` |

**Not saved:** while the window is minimized (`IsIconic` — last good placement is kept). **Unchanged by hide-to-tray:** closing the window chrome or hotkey hide does not write placement; geometry stays in memory until quit or `WM_EXITSIZEMOVE`.

**Load quirks:** if only one of `window_x` / `window_y` is present, both are cleared and Windows default placement (`CW_USEDEFAULT`) is used for position; size keys still apply.

## Load / save

- `Config::load_or_default(path)` — missing file returns defaults; per-line parse errors are logged with `log::warn` and leave the default for that key; valid keys still apply. On load, previous default hotkeys (`Ctrl+Shift+V`, `Ctrl+Win+V`, `Ctrl+Alt+V`) are upgraded to `Alt+V` and saved back to disk; `normalize_window_fields` clamps client size and clears orphan position keys.
- `Config::save(path)` — writes all keys in canonical order via tmp + rename (same pattern as metadata). `window_x` / `window_y` are appended only when both are set.
- The data directory is created before read/write via `Store::data_dir()`.

Startup in `main.rs` loads config before `Window::create(hinstance, &config)`. Autostart is synced to the Run key on launch; see [`autostart-startup.md`](autostart-startup.md). Runtime edits go through the settings panel; see [`settings-panel.md`](settings-panel.md). Window placement is not exposed in the settings UI — it updates automatically from user move/resize and quit.

## Errors

Strict validation for callers that must reject bad config: `validate_config(&Config) -> Result<()>` returns `ClipError::Config`.
