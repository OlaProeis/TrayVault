# TrayVault Roadmap

High-level release plan for [TrayVault](README.md) (Windows clipboard history manager). Detailed product scope: [`docs/trayvault-prd.md`](docs/trayvault-prd.md). Release notes: [`CHANGELOG.md`](CHANGELOG.md).

---

## Released

### [0.1.0](https://github.com/OlaProeis/TrayVault/releases/tag/v0.1.0) — 2026-06-01

First public release: clipboard capture (text, rich text, images), search and filter chips, pin/unpin, tray + global hotkey, settings, local storage, CI/release artifacts (`.exe`, zip, MSI).

---

### [0.1.1](https://github.com/OlaProeis/TrayVault/releases/tag/v0.1.1) — 2026-06-02

Patch release: **keyboard polish, scroll/hover/paint/list performance, sharper screenshot thumbnails, blob/thumb reliability, WIC compressed blobs** (Tasks 29–33, 35–38, 40–41, 43–44, 45, 49).

Highlights: Page Up/Down scroll, TVB1 + WIC blob storage (PNG default, JPEG optional), async thumbnails, scrollbar drag crash fix, and a large paint/list perf tranche (display-indices cache, list layout cache, hover gating, wheel coalesce, binary-search culling, LRU thumb cache, and more). See [`CHANGELOG.md`](CHANGELOG.md#011).

---

## Planned: 0.2.0

Theme: **distribution and display polish** — no feature explosion.

| Task | Summary | Status |
|------|---------|--------|
| 34 | Research: compressed image blobs (WIC assessment doc) | Done |
| 39 | Async thumbnail loading (no sync `read_blob` on UI thread) | Done (0.1.1) |
| 42 | Tiered thumbnail quality while scrolling | Deferred (rolled back: smaller thumbs while scrolling hurt UX; revisit same-size / lower-res) |
| 45 | Tag and ship **v0.1.1** (perf + WIC blobs patch) | Done (0.1.1) |
| 46 | **winget** manifest (`microsoft/winget-pkgs`) | Pending |
| 47 | HiDPI / per-monitor DPI awareness | Pending |
| 48 | Copy as plain text (context menu) | Pending |

Also under consideration for 0.2.0:

- README screenshot/GIF and hotkey table refresh

---

## Deferred (post–1.0 or separate product)

| Item | Notes |
|------|--------|
| `crates.io` / `cargo install` | GitHub Releases only for now |
| Compact binary metadata format | Line-oriented `entries.dat` is fine at current caps |
| Crate extraction (pixmap, Win32 UI stack) | [`docs/technical/crate-extraction-assessment.md`](docs/technical/crate-extraction-assessment.md) |
| **CanMan** (snippet manager) | Separate product — [`docs/canman-prd.md`](docs/canman-prd.md) |

---

## Versioning

- **Semver** `0.MINOR.PATCH` while pre-1.0
- **Patch** (`0.1.x`): fixes and incremental UX/perf (0.1.1)
- **Minor** (`0.2.0`): larger perf/polish tranche (Tasks 39, 42, 34, …)
- **Major** (`1.0.0`): stable data/API story (TBD)
