# Crate Extraction Assessment

Pros and cons for splitting parts of TrayVault into standalone crates. This is a planning document only — TrayVault remains a single crate today.

See also: [`pixmap-rasterizer.md`](pixmap-rasterizer.md), [`rendering.md`](rendering.md).

---

## Summary

| Candidate | Extractability today | External demand | Effort to extract |
|-----------|---------------------|-----------------|-------------------|
| `pixmap.rs` | High | Low–moderate | Low |
| `glyph_raster.rs` | Moderate | Moderate (Windows niche) | Moderate |
| `text.rs` (`GlyphCache` + blend) | Moderate | Low–moderate | Moderate |
| Minimal UI stack (widgets + theme + text + glyph) | Low | Moderate (Windows niche) | High |
| Win32 FFI (`ffi.rs` + wrappers) | Moderate | Low | High (maintenance) |
| SHA-256 core (`hash.rs`) | Moderate | Low | Low |
| Config parser (`config.rs`) | Low | Very low | Low (low value) |
| Storage (`store/`) | Low | Very low | High |
| Logger (`log.rs`) | Low | Very low | Low (low value) |
| Clipboard monitor | Low | Low | High |

**Recommendation:** Stay single-crate for shipping. If experimenting, extract `pixmap` first (smallest risk). Treat the Windows UI stack as a separate initiative after v0.1.

---

## 1. `pixmap.rs` — micro RGBA rasterizer

**~270 lines · pure `std` · zero deps · tested**

Hand-rolled RGBA8 buffer: `fill_rect`, `blit`, `blit_scaled`, `scale_bilinear_rgba` (thumbnails/preview), `scale_nearest_rgba` (legacy). Replaced `tiny-skia` for TrayVault’s immediate-mode UI.

### Pros

- Already isolated: no `App`, `ClipEntry`, Win32, or config types.
- Zero third-party dependencies — matches TrayVault’s philosophy and is the main selling point.
- Small, tested API with unit tests in-module.
- Deliberately mimics the former tiny-skia `Pixmap` / `Color` shape — easy migration story for tiny-skia drop-ins.
- Lowest extraction cost: move file, add `Cargo.toml`, publish.

### Cons

- Very limited feature set: no paths, strokes, gradients, anti-aliasing, or PNG decode.
- Nearest-neighbor scaling only — fine for thumbnails, not a general graphics library.
- Many teams inline similar code or use `tiny-skia`, `raqote`, `image`, etc.
- Small crates (~300 lines) rarely get sustained crates.io traction without a broader story.
- Competing with mature crates on features; only wins on size and zero deps.

### Best audience

Developers who want “~200 lines, zero deps, fill + blit + nearest scale” and refuse transitive dependencies.

---

## 2. `glyph_raster.rs` — GDI glyph rasterizer (fontdue replacement)

**~657 lines · Windows-only · zero Rust deps · Task 16**

`rasterize_glyph(ch, size_px) -> RasterizedGlyph` using bundled Roboto (`AddFontMemResourceEx`), GDI measurement (`GetGlyphOutlineW`), and offscreen DIB rasterization. Segoe UI fallback; hand-stamped U+2026 ellipsis.

### Pros

- Clean public API shaped like fontdue output (`advance_width`, `left`, `top`, alpha8 bitmap).
- Completes “zero Rust deps” text story on Windows — uses OS shaper for full Unicode (CJK, emoji), not Latin-only.
- Separated from layout/cache: `text.rs` only calls `rasterize_glyph`.
- Baseline tests compare metrics to captured fontdue values — drop-in migration narrative.
- More compelling extraction pitch than pixmap alone for the Windows utility niche.

### Cons

- **Windows-only** — cannot run glyph tests on Linux CI without Windows runners.
- Hardcoded bundled Roboto + Segoe UI fallback — not configurable without API work.
- Process-wide `OnceLock<Mutex<UiFontRasterizer>>` — assumes main-thread UI paint; awkward for off-thread rasterization.
- Depends on hand-rolled `ffi.rs` and `ClipError`, not a standalone Win32 surface.
- ~657 lines of owned GDI edge-case handling (empty ink, `GGO_BITMAP` fallback chain, ellipsis workaround).
- General Rust ecosystem uses `fontdue`, `rusttype`, or `swash` for cross-platform TTF.

### Best audience

Authors of small Win32 tray/utility apps who hand-roll FFI and want fontdue-like glyphs without any crates.

---

## 3. `text.rs` — glyph cache, blend, measure, caret

**~480 lines · depends on `pixmap` + `glyph_raster`**

`GlyphCache`, alpha blending into RGBA, `measure`, `caret_index_from_x`, `truncate_to_width`. App helpers like `format_relative_time` are TrayVault-specific.

### Pros

- Platform-agnostic layout layer sitting on top of a raster backend — good trait/crate boundary (`rasterize_glyph` injectable in theory).
- Reusable with any alpha8 glyph source, not only GDI.
- Caret hit-testing and ellipsis truncation are non-trivial; others building immediate-mode UIs might want this.
- Pairs naturally with `pixmap` as a minimal text stack.

### Cons

- Not useful alone — requires a glyph backend (`glyph_raster` or fontdue-like API).
- `format_relative_time` and date helpers are app-domain, not library-generic.
- Blending assumes `Color` from pixmap — couples to that crate unless traits are introduced.
- `GlyphCache` clones cached glyphs on hit — fine for TrayVault scale, may need tuning for a library.

### Best audience

Same niche as (1)+(2): immediate-mode CPU UI on a fixed RGBA buffer with cached per-glyph rasterization.

---

## 4. Minimal immediate-mode UI stack

**~1.5–2k lines · widgets + theme + text + glyph_raster (+ optional GDI present glue)**

`widgets.rs` (~476 lines), `theme.rs` (~150 lines), plus `text`, `glyph_raster`, and optionally `gdi` / parts of `window` for “draw to HWND.”

### Pros

- Coherent product story: “Build a ~2 MB Windows utility with zero Rust dependencies.”
- Immediate-mode + CPU pixmap + GDI present is a real gap (similar spirit to Nuklear, but Rust-native).
- Hard problems already solved: mouse-up semantics, sticky headers, caret in search field, virtualized list patterns.
- Wins on **size and dependency count**, not on features vs egui/iced/slint.

### Cons

- **Windows-only** and **GDI-coupled** — sharply limits audience.
- `UiState`, `render.rs`, and views (`history`, `settings`, `search`, etc.) are deeply tied to `App`, `ClipEntry`, `Config`, `Store`.
- `theme.rs` reads Windows registry for system light/dark; `widgets` includes TrayVault context-menu labels.
- High extraction effort: push app types up, design generic `UiContext`, pluggable fonts, optional system theme injection.
- Competing UI frameworks dominate mindshare; needs clear docs and examples to attract users.
- Ongoing maintenance burden across Win32, GDI, and widget API stability.

### Best audience

Rust developers building tray-first Windows tools who want full control and zero transitive deps.

---

## 5. Win32 FFI subset (`ffi.rs` + safe wrappers)

**~817+ lines in `ffi.rs` · hand-declared bindings by DLL**

Curated Win32 surface: only APIs TrayVault needs. Pattern documented in `win32-ffi.md`.

### Pros

- Organized by concern; establishes a reusable **pattern** for minimal FFI.
- Avoids pulling `windows` / `windows-sys` and their transitive weight — aligns with project goals.
- Could serve as a starter template for “tray app Win32 surface.”

### Cons

- **Low external demand** — `windows-sys` is the standard; most teams use it.
- High **maintenance cost** if published: every consumer depends on you keeping bindings correct and complete for their use case.
- Not a general Win32 binding crate — intentionally incomplete.
- Safe wrappers (`last_error`, `wide`) are tied to `ClipError`.
- Duplicates ecosystem effort without matching `windows-rs` coverage or tooling.

### Best audience

Internal TrayVault use only, unless willing to maintain a documented “minimal Win32 for tray apps” with narrow scope.

---

## 6. SHA-256 core (`hash.rs`)

**~544 lines · FIPS 180-4 in safe Rust · dedup helpers tied to `ClipEntry`**

### Pros

- Core SHA-256 implementation is generic and educational.
- Zero deps — consistent with project philosophy.
- Could split: pure `sha256` module + app-specific `hash_entry(ClipEntry)` in TrayVault.

### Cons

- **Low external demand** — `sha2`, `ring`, etc. are standard and audited.
- Dedup/normalization logic is clipboard-domain-specific.
- Security-sensitive code benefits from community review; a solo 500-line impl is a harder sell.
- Extraction value is mostly “learning / zero deps,” not practical adoption.

### Best audience

Learning projects or extreme minimalists avoiding all crypto crates (with understood audit tradeoffs).

---

## 7. Config parser (`config.rs`)

**~504 lines · minimal TOML subset · top-level keys only**

### Pros

- Zero deps; simple `key = value` subset without full TOML crate.
- Self-contained load/save with migration hooks.

### Cons

- **Too TrayVault-specific** — struct fields, defaults, paths, and migrations are app-defined.
- Not a general TOML library — no tables, arrays, or spec compliance.
- `toml` / `serde` are the obvious choices for anyone needing real config.
- Very low reuse value outside this app.

### Best audience

None as a standalone crate; keep internal.

---

## 8. Storage (`store/`)

**Line-oriented `entries.dat` + content-addressed blobs in `blobs/`**

### Pros

- Clean worker-thread design; atomic tmp+rename; content-addressed images.
- Documented pattern for simple persistence without SQLite.

### Cons

- **Application data format** — not a generic storage library.
- Tied to `ClipEntry`, SHA-256 blob naming, DIB blobs, TrayVault data dir layout.
- No schema versioning story aimed at external consumers.
- High effort to generalize; low payoff.

### Best audience

None as a standalone crate; the pattern is documentation value only (`storage.md`).

---

## 9. Logger (`log.rs`)

**~110 lines · file logger to `%LOCALAPPDATA%\TrayVault\`**

### Pros

- Tiny, thread-safe, pure `std`.
- Howard Hinnant-style UTC timestamp without `chrono`.

### Cons

- Hardcoded TrayVault paths and filename.
- `tracing` / `log` ecosystem dominates Rust logging.
- Trivial to copy-paste; not worth a crate.

### Best audience

None; keep internal.

---

## 10. Clipboard monitor (`win32/clipboard.rs`)

### Pros

- Useful **pattern** for `WM_CLIPBOARDUPDATE`, format enumeration, sensitive-content skip.
- Integrates with TrayVault’s capture pipeline.

### Cons

- Tightly coupled to app flow, formats, dedup, and storage enqueue.
- Windows-only; format handling is product-specific.
- Not a generic “clipboard crate” without major API design.

### Best audience

Reference implementation in docs only (`clipboard-capture.md`).

---

## Suggested extraction order (if pursued)

1. **`pixmap`** — validate publishing workflow, almost no refactor.
2. **`glyph_raster`** — peel relevant `ffi` + `RasterizedGlyph`; Windows-only crate.
3. **`text` (GlyphCache + blend)** — optional layer on (1)+(2).
4. **Full UI stack** — only after deliberate API design and decoupling from `App` / `UiState`.

Do not prioritize extracting hash, config, logger, storage, or clipboard as crates — internal value only.

---

## Natural workspace layout (future)

```text
micro-pixmap/          # pixmap.rs
win32-glyph-raster/    # glyph_raster.rs + subset of ffi
imui-text/             # GlyphCache, blend, measure (optional)
TrayVault/             # app, views, store, config, win32 shell
```

Until then, the current single-crate module boundaries (`pixmap` → `widgets` → views; `glyph_raster` → `text`) are sufficient for shipping TrayVault.
