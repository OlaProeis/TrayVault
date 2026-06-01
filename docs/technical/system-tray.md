# System Tray Integration

Task 10 module: `src/win32/tray.rs`; wired from `src/main.rs` and `src/win32/window.rs`.

## Icon lifecycle

- Icon design: white clipboard outline + gold padlock on a deep-indigo background (`assets/icon.ico`). Multi-size ICO: 16×16, 32×32, 48×48, 256×256.
- Icon loaded from **embedded bytes** only: `EMBEDDED_ICON = include_bytes!("../../assets/icon.ico")` in `tray.rs`, parsed by `load_icon_from_ico_bytes` (`CreateIconFromResourceEx`, picks the largest ICO entry). No runtime `LoadImageW(LR_LOADFROMFILE)` — a compile-time path only exists on the build machine and fails on end-user installs.
- **`pub fn load_app_icon()`** in `tray.rs` is the shared entry point used by both the tray and the main window. It calls `load_icon_from_ico_bytes(EMBEDDED_ICON)` with a fallback to `LoadIconW(IDI_APPLICATION)` (`log::warn` on embedded parse failure).
- Registered with `Shell_NotifyIconW(NIM_ADD)` using callback message [`WM_TRAY_CALLBACK`](../../src/win32/window.rs) (`WM_APP + 1`).
- Upgraded to `NOTIFYICON_VERSION_4` for balloon support.
- Removed on quit and in `Drop` via `NIM_DELETE`.

## User interactions

| Input | Action |
|-------|--------|
| Left-click tray icon | Toggle main window show/hide (debounced — see [Toggle debounce](#toggle-debounce)) |
| Right-click tray icon | Context menu at cursor |
| Menu: Open | Show window, focus search |
| Menu: Pause/Resume capture | Toggle `App::pause_capture`, update monitor + tooltip |
| Menu: Settings | Show window, open settings overlay |
| Menu: Quit | Save config, flush storage, remove tray icon, exit |
| Window close (X) | Hide to tray (does not quit) |

## NOTIFYICON_VERSION_4 callback parsing

The shell registers version 4 via `NIM_SETVERSION`. Callback parameters differ from legacy tray icons:

- **`LOWORD(lParam)`** — notification kind (`WM_LBUTTONUP`, `WM_CONTEXTMENU`, `NIN_SELECT`, `NIN_KEYSELECT`, etc.).
- **`wParam`** — anchor coordinates for some events via `GET_X_LPARAM` / `GET_Y_LPARAM`, but **not** reliably for mouse-driven context menus (it may hold the icon id instead).

`TrayIcon::handle_callback` in [`tray.rs`](../../src/win32/tray.rs) maps events as follows:

| `LOWORD(lParam)` | Action |
|------------------|--------|
| `WM_LBUTTONUP`, `WM_LBUTTONDBLCLK`, `NIN_SELECT`, `NIN_KEYSELECT` | Toggle window show/hide (debounced — see below) |
| `WM_RBUTTONUP`, `WM_CONTEXTMENU` | Show context menu |

## Toggle debounce

With [`NOTIFYICON_VERSION_4`], left-click activation must listen for **both** legacy mouse messages (`WM_LBUTTONUP`, `WM_LBUTTONDBLCLK`) and v4 notifications (`NIN_SELECT`, `NIN_KEYSELECT`). On modern Windows a single physical click can deliver **more than one** of those in quick succession (e.g. `WM_LBUTTONUP` plus `NIN_SELECT`, or click plus `WM_LBUTTONDBLCLK`). Without filtering, `apply_tray_action(ToggleWindow)` runs twice — show then immediate hide (window flickers, stays closed).

`TrayIcon` stores `last_toggle_tick: Cell<u32>`. `take_toggle_if_due()` in [`tray.rs`](../../src/win32/tray.rs) accepts the first toggle notification and ignores duplicates within **`TRAY_TOGGLE_DEBOUNCE_MS` (400 ms)**, using [`GetTickCount`](../../src/win32/ffi.rs) with wrapping subtraction. Right-click **Open** uses `TrayMenuAction::ShowWindow` (not toggle) and is unaffected. The global hotkey toggle in [`global-hotkey.md`](global-hotkey.md) is a separate code path with no debounce.

Pure helper `should_accept_toggle(now, last, debounce_ms)` is unit-tested in `tray.rs` (`#[cfg(test)]`).

## Legacy vs v4 left-click notes

Left-click on modern Windows often sends **`NIN_SELECT`** (`WM_USER`) rather than `WM_LBUTTONUP`; both handlers must remain in the match arm or the window cannot be reopened from the tray on some builds. Debounce collapses duplicate delivery — it does not remove either message type.

## Context menu placement

`TrackPopupMenu` needs screen coordinates on the correct monitor.

- **Mouse right-click** (`WM_CONTEXTMENU` / `WM_RBUTTONUP`): use **`GetCursorPos`** — the cursor is at the tray icon when the user clicks. Do not read `wParam` for placement; on multi-monitor setups it can be the icon id (e.g. `1, 0`) and the menu appears on the wrong screen.
- **Keyboard activation** (`NIN_KEYSELECT`): use the anchor in `wParam` (`GET_X_LPARAM` / `GET_Y_LPARAM`, sign-extended for negative coordinates on secondary monitors).

After `TrackPopupMenu`, post `WM_NULL` to the owner window so the menu dismisses when clicking elsewhere. Call `SetForegroundWindow` on the owner before showing the menu.

## Window visibility

- Normal launch shows the main window; `--minimized` (autostart) starts **hidden** with only the tray icon visible.
- Main window is created with `WS_EX_TOOLWINDOW`. While hidden it stays off the taskbar and out of Alt+Tab.
- When visible, `show_in_taskbar` (config, default `true`) controls whether `WS_EX_TOOLWINDOW` is cleared so the window appears on the taskbar like a normal app. `set_taskbar_button_visible()` toggles `GWL_EXSTYLE` and refreshes the frame via `SetWindowPos`.
- `Window::show(show_in_taskbar)` calls `ShowWindow`, focus helpers, then applies taskbar visibility. `hide()` clears the taskbar button before `ShowWindow(SW_HIDE)` and syncs with `App::on_show_window` / `on_hide_window`.

## Balloon notifications

- `TrayIcon::show_balloon(title, message)` via `NIM_MODIFY` + `NIF_INFO`.
- `show_tray_notification()` wrapper logs failures; used for hotkey registration conflict alerts.

## Shutdown path

Quit (tray menu) → `quit_app()` in [`main.rs`](../../src/main.rs). Post-loop teardown joins the storage worker and returns exit code 0. Full sequence, idempotent guards, and the `quitting` flag are documented in [`app-lifecycle.md`](app-lifecycle.md).

## FFI additions

See [`ffi.rs`](../../src/win32/ffi.rs): `Shell_NotifyIconW`, `NOTIFYICONDATAW`, menu APIs (`CreatePopupMenu`, `AppendMenuW`, `TrackPopupMenu`), `CreateIconFromResourceEx`, `LoadIconW`, `GetCursorPos`, `GetTickCount` (toggle debounce), `SetForegroundWindow`, `BringWindowToTop`, `NIN_SELECT`, `NIN_KEYSELECT`. (`LoadImageW` remains in `ffi.rs` for other callers; tray does not use it.)

Menu commands arrive as `WM_COMMAND` in [`window.rs`](../../src/win32/window.rs) dispatch.
