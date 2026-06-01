# Autostart and Startup Behavior

Modules: `src/win32/autostart.rs`, `src/main.rs`, `src/ui/settings.rs`. Registry FFI: `src/win32/ffi.rs` (advapi32).

## Run-key autostart

TrayVault registers with Windows login via the current-user Run key:

- **Key:** `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`
- **Value name:** `TrayVault`
- **Value data (enabled):** `"<exe-path>" --minimized` (REG_SZ, quoted path)

`set_autostart(enabled, exe_path)` creates or opens the Run key with `RegCreateKeyExW` / `RegOpenKeyExW`, then writes via `RegSetValueExW` or deletes via `RegDeleteValueW`. Failures return `ClipError::Registry` and are logged.

At startup, `main.rs` calls `sync_autostart_from_config(config.autostart, exe_path)` so the registry matches the persisted setting (warn-only on failure).

## Settings integration

The settings panel exposes a **Start with Windows** toggle. On click, `App::set_autostart` updates the Run key and persists; registry failures revert the toggle and set `UiState.settings_error`. See [`settings-panel.md`](settings-panel.md) for the full settings UI.

## `--minimized` flag

Parsed in `main.rs` via `std::env::args()`. When present, the app starts tray-only (window hidden until hotkey or tray click). Without the flag, a normal launch shows the main window immediately. The flag is embedded in the Run-key command so sign-in launches stay tray-only until the user invokes hotkey or tray click.

## System theme

`theme = System` reads `AppsUseLightTheme` from the Windows personalize registry key at render time. See [`rendering.md`](rendering.md#themes).
