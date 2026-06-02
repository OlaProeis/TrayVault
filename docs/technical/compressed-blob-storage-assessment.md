# Compressed Image Blob Storage — Research Assessment (Task 34)

**Status:** Research only — no production code or on-disk format change in this task.  
**Scope:** Replace raw BGRA `.dib` blobs with a compressed on-disk representation using Windows built-ins (zero Cargo deps).  
**Modules today:** `src/store/blobs.rs`, `src/store/mod.rs`, `src/hash.rs`, `src/win32/clipboard.rs` (`parse_dib_to_bgra`, `encode_bgra_dib`).

---

## Executive summary

| Question | Answer |
|----------|--------|
| Are today’s `.dib` files lossless? | **Yes, relative to the decoded BGRA buffer** TrayVault already uses (no re-quantization on write). They are **not** compressed. |
| How big are they? | **Exactly `width × height × 4` bytes** (no header). 1920×1080 ≈ **7.91 MiB**; 3840×2160 ≈ **31.6 MiB**. |
| PNG vs JPEG without crates? | Both need a **system codec** (WIC or GDI+). **PNG = lossless**, typically **~2–4×** smaller on UI screenshots. **JPEG = lossy**, often **~10–20×** smaller at high quality. |
| Recommended direction | **WIC** (`windowscodecs.dll`) via hand-declared COM/FFI; **default codec JPEG** (quality ~85–90) for disk size, with **optional PNG** (or config) when paste fidelity must match capture bit-for-bit. Keep **SHA-256 on decoded BGRA** (unchanged dedup semantics). |

---

## 1. Current format (baseline)

### On-disk layout

- Path: `blobs/<lowercase-hex-sha256>.dib`
- Payload: **raw top-down BGRA8** pixels only — no magic, version, width, or height in the file.
- Dimensions live in `entries.dat` (`image_w`, `image_h` on each image entry).
- `write_blob` uses `create_new` + `write_all`; **skip write if path exists** (content-addressed dedup at filesystem level).
- `read_blob` reads the entire file into a `Vec<u8>`; callers assume length = `w × h × 4`.

See [`storage.md`](storage.md), [`hashing-dedup.md`](hashing-dedup.md).

### Size math (why files feel huge)

| Resolution | Bytes (`w×h×4`) | MiB (÷ 1024²) |
|------------|-----------------|---------------|
| 1280×720 | 3,686,400 | ~3.52 |
| 1920×1080 | 8,294,400 | ~7.91 |
| 2560×1440 | 14,745,600 | ~14.06 |
| 3840×2160 | 33,177,600 | ~31.64 |

The handover example (1920×1080 ≈ 7.9 MB) is correct. Disk usage scales linearly with pixel count; alpha channel is always stored even when screenshots are fully opaque.

### Losslessness today

1. **Clipboard → BGRA:** `parse_dib_to_bgra` normalizes 32/24/8 bpp DIBs to a single top-down BGRA buffer ([`clipboard-capture.md`](clipboard-capture.md)). Palette/indexed sources are expanded to BGRA; that step can change pixels vs the original indexed representation, but it is the app’s canonical form.
2. **BGRA → disk:** `write_blob` writes bytes verbatim → **bit-identical round-trip** for that canonical buffer.
3. **Dedup hash:** `hash_image_pixels` = SHA-256 over **raw BGRA bytes** (`src/hash.rs`) — same material as stored.

So: **storage is lossless with respect to TrayVault’s decoded BGRA**, not necessarily lossless vs every possible clipboard encoding (e.g. rare compressed DIB, unsupported bit depths).

---

## 2. PNG and JPEG — “lossless” vs “good enough”

### Definitions

| Format | Compression | Visual / data fidelity | Typical role for TrayVault |
|--------|-------------|------------------------|----------------------------|
| **Raw BGRA (today)** | None | Exact canonical pixels | Huge files; trivial read path |
| **PNG** | Lossless (DEFLATE) | **Identical** decoded pixels after round-trip | Smaller disk; paste matches capture |
| **JPEG** | Lossy (DCT) | **Approximate**; artifacts on sharp text/UI edges | Much smaller disk; paste may differ slightly |

PNG and JPEG are **not** “lossless” in the same sense: only PNG (and raw BGRA) preserve the decoded buffer exactly. JPEG is **visually** good enough for many photo-like screenshots but can soften text, gradients, and 1px UI lines.

### Rough size expectations (clipboard screenshots)

Order-of-magnitude from typical desktop captures (flat UI + some imagery); measure in implementation with real histories.

| Codec | vs raw BGRA | Notes |
|-------|-------------|-------|
| PNG (WIC default filter) | **~2–4×** smaller | Best on large flat regions; worse on noisy captures |
| JPEG Q≈85–90 | **~10–20×** smaller | Strong on photos; weak on red/blue UI text |
| JPEG Q≈75 | **~15–30×** smaller | More visible artifacts on UI captures |

**Without introducing Rust crates**, PNG/JPEG encode/decode implies **WIC or GDI+** — there is no practical “easiest” hand-rolled JPEG/PNG at acceptable engineering cost (see §8).

### Practical recommendation

- **Default on disk:** WIC **JPEG** at high quality (e.g. 85–90) if primary goal is **disk and I/O** (aligns with PRD pain: multi‑MB blobs).
- **Configurable lossless mode:** WIC **PNG** for users who care about **bit-identical paste** and dedup stability under re-encode (see §4).
- **Hybrid (future):** store JPEG + optional PNG sidecar only for pinned items — likely overkill for v1; document as open question.

---

## 3. WIC vs GDI+

Both ship on Windows 10+ (TrayVault’s target). Both require **COM** on threads that call them. Neither is linked from Rust today.

### WIC — `windowscodecs.dll`

| Aspect | Notes |
|--------|--------|
| **Purpose** | Imaging codecs and pixel format conversion (decode/encode, resize, metadata) |
| **Typical flow** | `CoInitializeEx` → `CoCreateInstance(CLSID_WICImagingFactory)` → `IWICImagingFactory` → `IWICBitmap` / encoder/decoder → `IWICFormatConverter` (to/from 32bpp BGRA) |
| **Formats** | PNG, JPEG, BMP, TIFF, GIF, ICO, etc. via built-in codecs |
| **FFI surface** | COM interfaces (`IUnknown` + vtables), many GUIDs, `WINCODEC_ERR_*` — **Medium–Large** hand binding |
| **Fit for TrayVault** | **Strong** — encode on storage worker, decode on storage or thumb worker; no dependency on GDI drawing |

### GDI+ — `gdiplus.dll`

| Aspect | Notes |
|--------|--------|
| **Purpose** | 2D drawing + `Gdiplus::Bitmap` load/save |
| **Typical flow** | `GdiplusStartup` → `Bitmap::FromScan0` / `FromStream` → `Save` with encoder CLSID (JPEG/PNG) |
| **FFI surface** | C++-ish API (names mangled / flat exports depending on entry point), `GdiplusStartupInput`, status enums — **Medium** |
| **Fit for TrayVault** | **Weaker** — pulls in a second graphics stack alongside existing GDI DIB path; codec path is less direct than WIC |

### COM initialization vs message loop

TrayVault today: **single STA UI thread** (Win32 message loop), **plain `std::thread` workers** (storage, thumb loader) — **no COM usage yet**.

| Thread | WIC/GDI+ requirement |
|--------|---------------------|
| Main (WndProc) | Avoid heavy codec work on UI thread; if ever needed, `CoInitializeEx(NULL, COINIT_APARTMENTTHREADED)` once per thread |
| `trayvault-store` | **`CoInitializeEx` on worker start**, `CoUninitialize` on exit — natural place for **encode on persist** |
| `trayvault-thumb` | Same for **decode + scale** if blobs are compressed (decode before `build_thumbnail_pixmap`) |

**Risk:** calling COM codecs from multiple threads without per-thread init → `RPC_E_CHANGED_MODE` / `CO_E_NOTINITIALIZED`. Rule: **one init per thread that touches WIC**, never assume the main thread initialized workers.

**Complexity (rough):**

| Approach | Effort | Maintainability |
|----------|--------|-----------------|
| WIC encode/decode module | **L** (new `src/win32/wic.rs` + COM helpers) | Best long-term for blobs only |
| GDI+ save/load | **M** | Acceptable but redundant with GDI DIB |
| WIC on UI thread only | **S** (small API) | **Bad** — blocks paint/copy-back |

**Recommendation:** **WIC over GDI+** for blob compression.

---

## 4. Proposed on-disk format and migration

### Container (recommended)

Replace extension `.dib` with a small header + codec payload, e.g. extension **`.tvblob`** or keep `.dib` with magic for backward compatibility.

```
Offset  Size  Field
0       4     magic "TVB1" (0x54 0x56 0x42 0x31)
4       1     version (1)
5       1     codec_id (0=raw BGRA, 1=PNG, 2=JPEG, …)
6       2     reserved / flags
8       4     payload_len (optional; can use file_size - header_len)
12      …     codec bitstream (PNG/JPEG file bytes from WIC encoder)
```

**Width/height:** keep in `entries.dat` only (already there). Optional: store in header for validation (`payload decode size` vs `w×h×4`).

**Raw legacy:** `codec_id = 0` and payload length = `file_size - 12` **or** detect legacy by **absence of magic** (see dual-read).

### Migration strategies

| Strategy | Pros | Cons |
|----------|------|------|
| **Dual-read fallback** | Old installs work immediately | Two code paths until migration completes |
| **Lazy re-encode on read** | No big-bang disk walk | First open after upgrade pays encode; needs atomic replace |
| **Background migration job** | Amortize CPU | Must not delete old blob until new blob + metadata safe |
| **Write-new-only** | Simplest | Old blobs remain large until touched |

**Suggested v1:** **Dual-read** in `read_blob` / `write_blob`:

1. If magic `TVB1` → decode via WIC to BGRA (return `Vec<u8>` same as today’s API).
2. Else → treat entire file as **legacy raw BGRA** (current behavior).
3. **Encode on write** (new captures and optional `PersistAll` migration pass).
4. **Do not change** `ImageRef.hash` or filename scheme when only compression changes — filename stays `hash(decoded BGRA).hex` (see §5).

**Orphan prune / extension:** `prune_orphaned_blobs` today strips `.dib` suffix — generalize to `.tvblob` or any registered blob extension.

---

## 5. Dedup and content-addressed storage

### Keep hashing decoded BGRA (recommended)

- **Continue:** `hash_image_pixels(pixels)` and `hash_to_hex` → `blobs/<hex>.<ext>`.
- **Rationale:** Dedup semantics stay “same clipboard image → same hash,” independent of JPEG quality settings or PNG encoder version. `write_blob` skip-if-exists remains valid.
- **Entry `hash` field** in `entries.dat` unchanged.

### If hashing compressed bytes instead (not recommended)

- Same visual capture could produce **different JPEG files** (encoder updates, quality settings) → **different paths** → worse dedup, duplicated disk.
- Would require hashing normalized compressed stream or fixed encoder params — fragile.

### Lossy storage + dedup edge cases

- Two entries with **identical decoded BGRA** share one blob (today’s shared-blob story in [`blob-reference-counting.md`](blob-reference-counting.md)) — still true.
- Two captures that **look** the same but decode to **different** BGRA (e.g. different crop, color profile, 24 vs 32 bpp path) → different hashes — unchanged.
- **JPEG does not** break hash identity for identical decoded pixels; it only affects **fidelity on paste** (decode → BGRA → `encode_bgra_dib`).

### `write_blob` skip-if-exists

After compression, skip-if-exists still works: first writer encodes and writes; subsequent entries with same hash skip re-encode. Optional optimization: if legacy raw exists and policy is JPEG, **replace in background** (careful with ref counts).

---

## 6. Read-path performance

### Consumers of `read_blob` / `read_blob_at`

| Path | Thread | Work today | With compression |
|------|--------|------------|------------------|
| `ThumbLoader` worker | `trayvault-thumb` | Read file + BGRA→RGBA + bilinear scale | **+ WIC decode** (CPU, alloc `w×h×4`) |
| Preview modal (`preview.rs`) | UI (on miss) | Read + convert + scale | **+ decode on UI thread** — prefer moving to worker or caching decoded BGRA briefly |
| Copy-back (`resolve_image_pixels` → `encode_bgra_dib`) | Often UI | Read full BGRA | **+ decode** before DIB build |
| Storage persist | `trayvault-store` | Write raw | **+ encode** (good fit) |

### Interaction with caches

- **`ThumbCache`:** Keys `(entry_id, dst_w, dst_h)` on **scaled RGBA pixmap** — unchanged. Cold miss cost rises by **one decode per hash** until cached ([`thumbnail-cache.md`](thumbnail-cache.md), [`async-thumbnail-loading.md`](async-thumbnail-loading.md)).
- **`preview_cache`:** Single-slot scaled cache — first open pays decode; acceptable if decode not on every `WM_MOUSEMOVE` (already cached after first build).
- **Async thumb loader:** Already off UI thread — **decode fits naturally** before `build_thumbnail_pixmap`. Consider **decode+scale** in one worker pass to avoid holding full BGRA on UI.

### Performance expectations

- **JPEG decode** 1080p: typically sub‑ms to low‑ms on modern CPUs — small vs bilinear scale to 800×520.
- **Disk I/O:** Smaller files help more than decode hurts for large screenshots.
- **Worst case:** 4K JPEG → full BGRA alloc (~32 MiB) per cold read — same memory class as today’s raw read; monitor with existing `max_image_size_mb` at capture.

---

## 7. Config — `max_image_size_mb`

**Today:** Applied at **capture** as `max_image_size_bytes` on **decoded** `width × height × 4` ([`clipboard-hardening.md`](clipboard-hardening.md), `CaptureConfig`).

**Recommendation:**

- **Keep** `max_image_size_mb` as a **decoded-pixel budget only** (canonical safety cap before allocation).
- **Do not** use compressed file size as the primary gate (a 400 KB JPEG might decode to 32 MiB).
- Optional future: `blob_codec = jpeg | png | raw` and `jpeg_quality` in `config.toml` — document in implementation task, not required for assessment.

---

## 8. Copy-back path

Flow unchanged logically:

1. `resolve_image_pixels` → BGRA `Vec<u8>` (from memory or **decode blob**).
2. `encode_bgra_dib(width, height, pixels)` → `CF_DIB` ([`clipboard-capture.md`](clipboard-capture.md)).
3. `mark_own_write()` + `SetClipboardData`.

**PNG / raw TVB1:** Decode → BGRA identical to today → **no regression** for paste.

**JPEG:** Decode → BGRA is **lossy** vs original capture; paste into Photoshop etc. may differ slightly from first capture. Acceptable if product positions JPEG as default; offer PNG for lossless.

**Tests to add in implementation:** round-trip encode/decode → `encode_bgra_dib` → `parse_dib_to_bgra` byte equality for PNG; bounded tolerance or strict equality for JPEG depending on policy.

---

## 9. Alternatives ruled in / out

| Option | Verdict | Reason |
|--------|---------|--------|
| **WIC + JPEG/PNG** | **In — recommended** | System codecs, zero Cargo deps, matches goals |
| **GDI+ save/load** | **Fallback** | Works but overlaps GDI; less focused |
| **24-bpp BGR on disk** | **Optional later** | ~25% smaller, lossless if alpha unused; still large vs JPEG |
| **BMP RLE** | **Out** | Poor ratio on screenshots |
| **Hand-rolled DEFLATE/PNG** | **Out** | Large effort; violates “simplest path” |
| **Hand-rolled JPEG** | **Out** | Impractical |
| **Store original `CF_DIB` bytes** | **Out** | Variable layout, still need decode for UI; rarely smaller than BGRA for 32 bpp |
| **NTFS compression on `blobs/`** | **Out** | Not under app control; complicates support |
| **Keep raw forever** | **Status quo** | Simplest code; disk/RAM pressure remains |

---

## 10. Open questions and risks

| Risk | Mitigation |
|------|------------|
| COM on worker threads | Init/uninit per worker; document in `app-lifecycle.md` when implemented |
| WIC FFI drift / HRESULT handling | Central `win32::com` + `ClipError` mapping; focused tests with golden PNG/JPEG round-trip |
| UI-thread decode on preview/copy | Route decode to storage/thumb worker or cache decoded BGRA short-term |
| Migration partial failure | Write `*.tmp` blob → rename; keep legacy file until new blob verified |
| JPEG quality vs UX | Default Q 85–90; settings exposure |
| Encoder non-determinism | Irrelevant if hash stays on BGRA; same pixels → same blob path |
| `prune_orphaned_blobs` suffix filter | Extend for new extension(s) |
| Memory peak at 4K | Keep capture-time `max_image_size_mb`; consider lowering default if JPEG default |

---

## 11. Recommended next implementation task (scope sketch)

**Not part of Task 34** — for a follow-up task:

1. Add `src/win32/wic.rs` (COM init helpers, factory, encode BGRA→JPEG/PNG, decode→BGRA).
2. Extend `src/store/blobs.rs`: header write/read, legacy raw detection, `read_blob` always returns BGRA `Vec`.
3. Encode in `persist_all` / `write_blob` on storage worker (COM initialized in `worker_loop`).
4. Decode in `read_blob_at` **or** defer decode to callers — prefer **single decode point** inside `blobs::read_blob` so `ThumbLoader` and `resolve_image_pixels` stay unchanged.
5. Optional: background migration + metrics log (bytes saved).
6. Tests: round-trip WIC; legacy raw file still reads; shared-blob delete unchanged.
7. Docs: update [`storage.md`](storage.md), [`clipboard-capture.md`](clipboard-capture.md) when format ships.

**Default product choice:** WIC **JPEG** on disk, **SHA-256 on decoded BGRA**, dual-read legacy raw `.dib`, encode on new writes.

---

## 12. Direct answers (handover questions)

**Is PNG the best easiest choice without dependencies?**  
PNG is the best **lossless** system-codec choice; **JPEG** is easiest for **large wins** on screenshot-heavy histories. “Easiest” overall is still **WIC** (one factory, both codecs) — not hand-rolling either format.

**Are PNG/JPEG “really” lossless?**  
**PNG: yes** (for your BGRA buffer). **JPEG: no** — lossy by design; often **good enough** for visual recall in a clipboard manager, not for pixel-perfect workflows.

**Is our format completely lossless? Are files really that big?**  
**Yes** — lossless vs canonical BGRA; **yes** — size is exactly `4×w×h` bytes with no compression, so multi‑MB screenshots are expected and measured correctly in the handover.
