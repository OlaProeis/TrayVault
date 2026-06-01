# Global Hotkey

Task 11 module: `src/win32/hotkey.rs`; wired from `src/main.rs` and `src/win32/window.rs`.

## Configuration

- Setting: `hotkey` in `config.toml` (default `"Alt+V"`).
- Parsed at startup via `HotkeyHandle::try_register`.
- Invalid strings log a warning and fall back to the default combination.

## String format

Split on `+` (whitespace trimmed). Case-insensitive modifiers:

| Token | Flag |
|-------|------|
| `Ctrl`, `Control` | `MOD_CONTROL` |
| `Alt` | `MOD_ALT` |
| `Shift` | `MOD_SHIFT` |
| `Win`, `Super`, `Windows` | `MOD_WIN` |

The key token may be a single letter/digit (`V`, `1`), a function key (`F1`–`F12`), or a name (`Space`, `Tab`, `Enter`, `Escape`, etc.).

## Registration

- `RegisterHotKey` on the main window with id `TRAYVAULT_HOTKEY_ID` (`1`).
- `MOD_NOREPEAT` is always set so holding the chord does not spam `WM_HOTKEY`.
- On conflict (`ERROR_HOTKEY_ALREADY_REGISTERED`), logs the error, shows a one-shot tray balloon, and continues without a global hotkey.
- `HotkeyHandle::reregister` unregisters then re-registers (for settings changes in Task 13).

## Runtime behavior

| Input | Action |
|-------|--------|
| Global hotkey (`WM_HOTKEY`) | Toggle main window show/hide (same path as tray left-click) |

Unregister on quit and normal shutdown via `HotkeyHandle::unregister`.

## FFI

See [`ffi.rs`](../../src/win32/ffi.rs): `RegisterHotKey`, `UnregisterHotKey`, `MOD_*`, `VK_*`, `ERROR_HOTKEY_ALREADY_REGISTERED`.

`WM_HOTKEY` is dispatched in [`window.rs`](../../src/win32/window.rs) to `WindowCallbacks::on_hotkey`.
