# Compressed Image Blob Storage (WIC / TVB1)

Modules: `src/store/blobs.rs`, `src/win32/wic.rs`, `src/config.rs`, `src/ui/settings.rs`. Research baseline: [`compressed-blob-storage-assessment.md`](compressed-blob-storage-assessment.md).

## On-disk format

Filename stays `blobs/<hex-sha256>.dib` (content-addressed by decoded BGRA hash). New writes use a **`TVB1` container header** followed by WIC-encoded bytes:

```
magic "TVB1" (4) | version u8 (1) | codec_id u8 | reserved u16 | WIC payload
```

| `codec_id` | Meaning |
|------------|---------|
| `0` | Legacy — entire file is raw top-down BGRA (no header) |
| `1` | PNG (WIC container) |
| `2` | JPEG (WIC container) |

Dimensions are **not** stored in the blob file; they remain in `entries.dat` (`image_w`, `image_h`). `read_blob` validates decoded length against `w × h × 4` when dimensions are known.

## Read path (unchanged API contract)

`Store::read_blob` / `read_blob_at` always return **decoded top-down BGRA** `Vec<u8>`:

1. Read file bytes.
2. If magic is `TVB1` → WIC decode via `win32::wic::decode_to_bgra`.
3. Else → treat as legacy raw BGRA (pre-0.2.0 installs).

Call sites (thumbnails, preview, copy-back) require no format awareness.

## Write path

- `write_blob` encodes with the **current config codec** on first write (`skip-if-exists` unchanged).
- **WIC fallback:** if `encode_bgra` fails (e.g. `CreateEncoder` / codec not registered on the OS), the blob is written as **legacy raw top-down BGRA** (no `TVB1` header) and a warning is logged. Captures still persist and thumbnails load via the existing legacy read path.
- Atomic replace: write `*.dib.tmp`, then `rename` over the final path.
- Hash / dedup: **SHA-256 on decoded BGRA** (`hash_image_pixels`); filename = `hash_to_hex(digest)`.

## Config

| Key | Default | Values |
|-----|---------|--------|
| `image_blob_codec` | `"png"` | `"png"` \| `"jpeg"` |
| `jpeg_quality` | `90` | 1–100 (JPEG only; clamped on save) |

Changing codec affects **new writes only**; existing blobs remain readable via dual-read. `max_image_size_mb` is still a **decoded-pixel budget at capture** only.

## WIC / COM (`src/win32/wic.rs`)

Hand-declared COM vtables and GUIDs for `windowscodecs.dll` (zero Cargo deps). Public helpers:

- `wic_codecs_available()` — probe whether PNG encoder can be created (used by tests on stripped Windows installs).
- `ensure_com_initialized()` — `CoInitializeEx(COINIT_APARTMENTTHREADED)` per worker thread (idempotent; accepts `RPC_E_CHANGED_MODE`).
- `encode_bgra(width, height, pixels, codec, jpeg_quality)` → PNG or JPEG bytes.
- `decode_to_bgra(payload, width, height)` → BGRA bytes.

## Threading

| Thread | COM |
|--------|-----|
| `trayvault-store` | Init at worker start; **encode** during `PersistAll` |
| `trayvault-thumb` | Init at worker start; **decode** inside `read_blob_at` |
| UI thread | No COM for blob I/O |

## Settings UI

PNG / JPEG codec chips (same pattern as theme row). JPEG quality numeric field visible only when JPEG is selected. See [`settings-panel.md`](settings-panel.md).

## Related

- [`storage.md`](storage.md) — worker jobs, metadata, blob lifecycle
- [`blob-reference-counting.md`](blob-reference-counting.md) — shared `.dib` delete rules unchanged
- [`hashing-dedup.md`](hashing-dedup.md) — hash still on decoded BGRA

## Out of scope (v1)

- Background migration of legacy raw blobs to PNG/JPEG.
- Re-encoding existing blobs when the user switches codec in Settings.
