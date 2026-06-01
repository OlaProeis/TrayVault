# Clipboard Hardening

Module: `src/win32/clipboard.rs`. See also [`clipboard-capture.md`](clipboard-capture.md) for the full capture/copy-back pipeline.

## DIB decode size bounds (`parse_dib_to_bgra`)

Clipboard DIB headers are attacker/other-app controlled. Before allocating a decoded BGRA buffer, `parse_dib_to_bgra(data, max_bytes)` validates dimensions:

1. **Checked 64-bit size** — `(width as u64).checked_mul(height).and_then(|v| v.checked_mul(4))`; returns `"DIB dimensions overflow"` on failure.
2. **Budget gate** — rejects with `"DIB pixel data exceeds size limit"` when `needed > max_bytes` (enforced before `vec!` allocation).
3. **Vec capacity gate** — rejects when `needed > isize::MAX as u64` (dimensions that fit in `u64` but cannot be allocated).

Callers:

| Function | `max_bytes` |
|----------|-------------|
| `read_image_with_limit(max_bytes)` | config `max_image_size_bytes` |
| `read_image()` | `u64::MAX` |

`read_image_with_limit` maps size-related errors to `Ok(None)` with a log line so text/HTML capture continues when an image is skipped.

Unit tests in `clipboard.rs` cover u64 overflow, isize overflow, oversize dimensions under budget, and tiny-budget rejection.

## Copy-back handle cleanup (`set_clipboard_or_free`)

On copy-back, `set_unicode_text`, `set_html_format`, and `set_dib_image` allocate `GMEM_MOVEABLE` global memory and pass the handle to `SetClipboardData`. Win32 transfers ownership to the OS **only on success**.

`set_clipboard_or_free(format, handle, api)` centralizes the contract:

- **Success** — handle owned by the clipboard; do not free.
- **Failure** (`SetClipboardData` returns 0) — `GlobalFree(handle)` before returning `last_error(api)`.

This prevents a per-failure memory leak on the copy-back path without changing successful copy-back behavior.
