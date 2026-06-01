# Error Handling

Central error type in `src/error.rs`. Every fallible operation returns `Result<T>` (alias for `std::result::Result<T, ClipError>`); the message loop and watcher must **never panic on an OS failure** — they log and recover.

## `ClipError` variants
| Variant | Meaning |
|---------|---------|
| `Win32 { api, code }` | A Win32 call failed; `code` is `GetLastError` captured immediately after. |
| `Registry { op, code }` | A registry op failed (registry APIs return the code directly, not via `GetLastError`). |
| `HotkeyConflict { hotkey }` | A global hotkey is already registered by another app. |
| `Config(String)` | Config parse/validation failure. |
| `Io(std::io::Error)` | Filesystem/IO (storage, logging, config). |
| `Other(String)` | Any other contextual failure. |

## Behavior
- Implements `Display` (human-readable, includes hex error codes) and `std::error::Error` (`source()` exposes the inner `io::Error`).
- `From<std::io::Error>` is implemented, so `?` works directly on IO calls.
- The enum is `#[allow(dead_code)]` during early milestones because several variants are first constructed by later tasks (registry/hotkey/config).

## Rule
When wrapping a raw Win32 call, call `win32::last_error(api)` **immediately** after detecting the documented failure value, before any other Win32 call.
