# Session Handover

## Environment
- **Project:** TrayVault — Windows-only, pure-Rust, from-scratch clipboard manager.
- **Tech Stack:** Rust 2021 (MSRV 1.85), `x86_64-pc-windows-msvc`, hand-declared Win32 FFI. **Zero third-party deps** (hand-rolled pixmap + Win32 GDI text).
- **Context file:** Always read `ai-context.md` first (note: border section there may be stale vs `docs/technical/window-gdi.md`).
- **Branch:** main

## Core Handover Rules
- **SCOPE:** Investigate and fix the **white/light border glitch** during window **move** and **resize**. Do not start unrelated tasks unless the user asks.
- **VERIFY:** `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` must pass clean.
- **User report (latest):** After the recent border fix attempt, behavior **got worse** — white borders **glitch prominently while moving** the window (see screenshot in chat: ~1–2px bright line on outer right/bottom edge, outside dark UI).

---

## Current Task: Borderless window — white border on move/resize (BLOCKED / NEEDS REVIEW)

| Field | Value |
|-------|-------|
| **Priority** | high (visual polish, near release) |
| **Status** | Regression after attempted fix — **revert or replace approach** |
| **Repro** | Intermittent before; more visible **during title-bar drag (move)** after latest changes. Also reported on resize (harder to repro). |
| **Window style** | `WS_POPUP \| WS_THICKFRAME`, custom 28px title bar, `WM_NCHITTEST` edge grips + `HTCAPTION` drag |

### Symptom

- Thin **white or light-gray** band along the **outer** edge of the window (often right + bottom), outside the dark client UI.
- Sometimes thicker; user screenshot shows ~1–2px high-contrast line during move.
- Looks like DWM/NC chrome, unpainted gap, or desync between outer window rect and GDI-presented client — **not** an in-app widget border.

### Architecture context (read first)

- Single-threaded `WndProc` in `src/win32/window.rs`; UI rasterized to DIB BGRA, `StretchDIBits` in `src/win32/gdi.rs`.
- Borderless/DWM handling documented in `docs/technical/window-gdi.md` (Borderless frame section).
- **Do not** use `SendMessageW(SC_MOVE)` from input while `RefCell` borrows are active (re-enters `WndProc` → panic).

---

## What existed BEFORE the recent fix attempt (baseline)

These mitigations were already in place for **gray** NC flash on drag / tray focus (partially helped; white edge still reported intermittently):

| Mechanism | Location | Behavior |
|-----------|----------|----------|
| `hbrBackground = 0` | `register_class` | No system brush; rely on full client paint |
| `WM_NCCALCSIZE` | `on_nc_calc_size` | When `wParam != 0`: `rgrc[0] = rgrc[1]`, return `0` (client = proposed window rect) |
| `WM_NCACTIVATE` → `1` | `dispatch` | Skip default NC activation paint |
| `WM_ERASEBKGND` → `1` | `dispatch` | Skip erase; paint in `WM_PAINT` |
| `apply_dwm_borderless` | create/show/activate | `DWMNCRP_DISABLED`, `DWMWCP_DONOTROUND`, **`DwmExtendFrameIntoClientArea` with margins `-1` on all sides**, `DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE` |
| Class style | `WNDCLASSW` | **`CS_HREDRAW \| CS_VREDRAW`** enabled |
| Resize | `WM_SIZE` | Resize DIB + `InvalidateRect` only (no synchronous present during drag) |

---

## What we tried in the recent session (CURRENT CODE — regression)

Goal: fix intermittent white border by syncing GDI with DWM during modal move/resize (research: Avalonia/Qt/Handmade Network — DWM glass desync + white flash when frame extended without dark mode).

### Changes made

| Change | Files | Intent |
|--------|-------|--------|
| **`DwmExtendFrameIntoClientArea` margins `{-1,-1,-1,-1}` → `{0,0,0,0}`** | `window.rs` | Avoid full glass extension desync during live resize |
| **`DWMWA_USE_IMMERSIVE_DARK_MODE = 1`** | `window.rs`, `ffi.rs` | Dark NC/chrome (Win10 1809+) |
| **`DWMWA_BORDER_COLOR = 0x001E1E1E`** (was `DWMWA_COLOR_NONE`) | `window.rs`, `ffi.rs` | Dark Win11 visible border |
| **Removed `CS_HREDRAW \| CS_VREDRAW`** (class `style = 0`) | `window.rs` | Avoid partial invalidation stripes |
| **`WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE`** | `window.rs` | `in_size_move` flag; re-apply DWM on enter/exit |
| **`WM_MOVE` during modal loop** | `window.rs` | `apply_dwm_borderless` + **`present_client()`** + **`sync_repaint_frame()`** on every move message |
| **`WM_SIZE` during modal loop** | `window.rs` | After resize: **`fill_solid(0x1E,0x1E,0x1E)`** + **`present_client()`** + **`sync_repaint_frame()`** |
| **`present_client()` via `GetDC`** | `gdi.rs` `with_dc`, `window.rs` | Paint outside `WM_PAINT` (BeginPaint invalid outside paint cycle) |
| **`sync_repaint_frame`**: `RedrawWindow(RDW_INVALIDATE \| RDW_UPDATENOW \| RDW_ALLCHILDREN \| RDW_FRAME)` + `UpdateWindow` | `window.rs`, `ffi.rs` | Force client + **NC/frame** repaint |
| Docs updated | `docs/technical/window-gdi.md`, `docs/technical/win32-ffi.md` | Describes new approach |

### Outcome

- **User feedback: worse** — white border **glitches while moving** (more aggressive than before).
- Likely culprits to investigate:
  1. **`present_client()` on every `WM_MOVE`** — full UI re-render + GDI blit at high frequency during drag; may fight DWM composition / leave frame artifacts.
  2. **`RedrawWindow(..., RDW_FRAME)`** on every move — explicitly repaints non-client; may **cause** white NC flash rather than suppress it.
  3. **`apply_dwm_borderless` called repeatedly** during move (DWM attr + `DwmExtendFrameIntoClientArea` every `WM_MOVE`) — expensive; may reset DWM state mid-drag.
  4. **Margin change `-1` → `0`** — may have removed the fix that hid the 1px band, while adding move-loop paint made glitches visible.
  5. **DWM/GDI fundamental desync** (Avalonia issue #8316) — no perfect sync without different presentation path (e.g. no `WS_THICKFRAME`, or different DWM strategy).

### Suggested first steps for Opus

1. **Reproduce** on user's Win10 build `10.0.19045`: drag title bar slowly/quickly; resize corners; tray show + focus.
2. **Try surgical revert** (one at a time, manual test after each):
   - A) Remove **`WM_MOVE`** handler body entirely (keep `WM_ENTERSIZEMOVE`/`EXITSIZEMOVE` only).
   - B) Remove **`present_client` / `sync_repaint_frame`** from `WM_SIZE` during `in_size_move` — back to invalidate-only.
   - C) Restore **`DwmExtendFrameIntoClientArea(-1)`** margins; keep or drop immersive dark mode / border color separately.
   - D) Restore **`CS_HREDRAW \| CS_VREDRAW`**.
   - E) Restore **`DWMWA_COLOR_NONE`** instead of dark `DWMWA_BORDER_COLOR`.
3. **Alternative directions** (if reverts restore “sometimes” bug but not worse):
   - Only `apply_dwm_borderless` on `WM_EXITSIZEMOVE` + `WM_ACTIVATE`, never per-frame during move.
   - `WM_NCCALCSIZE` when `wParam == FALSE` — verify是否需要 handling.
   - Per-monitor DPI awareness (`SetProcessDpiAwareness`) — 1px gap at non-integer scale.
   - Sample apps: borderless + `WS_THICKFRAME` without `DwmExtendFrameIntoClientArea` at all when `DWMNCRP_DISABLED`.
   - Compare outer `GetWindowRect` vs `GetClientRect` during move (log delta) — detect NC gap.

### Key files

| File | Purpose |
|------|---------|
| `src/win32/window.rs` | **All border logic**: `apply_dwm_borderless`, `on_nc_calc_size`, `WM_ENTERSIZEMOVE`/`MOVE`/`SIZE`/`EXITSIZEMOVE`, `present_client`, `sync_repaint_frame` |
| `src/win32/gdi.rs` | `with_paint` / `with_dc`, DIB resize, `present_internal` |
| `src/win32/ffi.rs` | DWM constants (`DWMWA_*`, `DWM_BORDER_DARK_GRAY`), `RedrawWindow`, `WM_*` move/size messages |
| `docs/technical/window-gdi.md` | Borderless frame section (reflects attempted fix — treat as hypothesis, not gospel) |
| `src/ui/titlebar.rs` | `HTCAPTION` hit regions for drag |

### Test strategy

- Manual: move + resize + tray focus; light/dark theme; screenshot corners.
- Automated: existing unit tests only (no window integration test); `cargo test` must stay green.
- **Do not** mark border task done until user confirms move + resize look clean.

### Model selection

**High reasoning** — Win32/DWM interaction, regression bisection, and trade-offs between prior `-1` margins vs new move-loop painting. Prefer incremental revert over more aggressive DWM hacks until baseline is restored.

---

## Deferred (not in scope unless user asks)

- **Task 31** (preview pixmap cache) — appears implemented (`UiState::preview_cache`, `src/ui/preview.rs`); verify status in Task Master separately.
- Tasks 32+ per `current-handover-prompt` history / Task Master.
