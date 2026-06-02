# Session Handover

## Environment
- **Project:** TrayVault — Windows-only, pure-Rust, from-scratch clipboard manager.
- **Tech Stack:** Rust 2021 (MSRV 1.85), `x86_64-pc-windows-msvc`, hand-declared Win32 FFI. **Zero third-party deps** (hand-rolled pixmap + Win32 GDI text).
- **Context file:** Always read `ai-context.md` first.
- **Branch:** main
- **Release target:** 0.2.0

## Core Handover Rules
- **SCOPE:** HiDPI / per-monitor DPI awareness (Task 47). Do not start unrelated tasks unless the user asks.
- **VERIFY:** After any `src/` changes, run `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`.
- **Constraint:** Zero Cargo deps; hand-declared Win32 FFI only (same pattern as existing modules).

---

## Current Task: HiDPI — per-monitor DPI awareness (Task 47)

| Field | Value |
|-------|-------|
| **ID** | 47 |
| **Priority** | high |
| **Complexity** | ~8 |
| **Depends on** | Tasks 2 (window), 7 (UI render stack) |
| **Status** | pending |

### Description

Make fonts, layout metrics, thumbnails, and pointer hit-testing correct on mixed-DPI setups (e.g. 125–200% laptop + 100% external). TrayVault is not yet DPI-aware; GDI/UI may appear soft or misaligned on scaled displays.

### Implementation scope

1. Call `SetProcessDpiAwarenessContext` (Per Monitor V2) at startup; document in `window-gdi.md`.
2. Handle `WM_DPICHANGED` — resize client, invalidate layout caches (`thumb_cache` width bucket, list layout, scratch buffer).
3. Scale logical layout constants (padding, font sizes, thumb caps) using `GetDpiForWindow` or per-monitor factor; avoid double-scaling GDI text if using device pixels for `TextOutW`.
4. Re-test: title bar drag, search caret, history card hit-test, scrollbar, settings overlay.

### Out of scope

- Full fractional scaling redesign.
- Dirty-rectangle partial repaint (defer).

### Test strategy

- Manual: 125% and 150% display scaling — UI sharp, no clipped text, hover/copy on cards accurate.
- Manual: move window between monitors with different scale factors.
- `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`.

### Key files

| File | Purpose |
|------|---------|
| `src/main.rs` | Early startup — set DPI awareness before window create |
| `src/win32/window.rs` | `WM_DPICHANGED`, client resize, `GetDpiForWindow` |
| `src/win32/ffi.rs` | DPI-related APIs and constants |
| `src/ui/mod.rs`, `history.rs`, `render.rs` | Layout metrics, cache invalidation on DPI change |
| `src/ui/thumb_cache.rs` | Thumb width bucket invalidation |
| `docs/technical/window-gdi.md` | Document DPI behavior |
| `docs/trayvault-prd.md` | Open question #2 (display scaling) |

### References

- PRD open question #2 (display scaling)
- Product counsel on mixed-DPI UX
