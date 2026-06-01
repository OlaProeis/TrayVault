# Storage Layer

Modules: `src/store/mod.rs` (public API + worker), `src/store/meta.rs` (metadata), `src/store/blobs.rs` (image blobs).

## Data directory

`%LOCALAPPDATA%\TrayVault\` — created on demand. Layout:

```
TrayVault/
├── entries.dat          # metadata (line-oriented, tab-separated)
├── entries.dat.tmp      # transient during atomic rewrite
├── entries.dat.bak      # backup after load failure
├── trayvault.log
├── config.toml          # settings (hand-rolled TOML subset)
└── blobs/
    └── <hex-sha256>.dib # raw BGRA pixel bytes
```

## Metadata (`entries.dat`)

- Header line: `version\t1`
- Entry lines (11 tab-separated fields): `id`, `created_at`, `kind`, `text`, `html`, `image_hash`, `image_w`, `image_h`, `source_app`, `is_pinned`, `hash_hex`
- Text fields escape `\`, tab, and newline as `\\`, `\t`, `\n`
- Atomic rewrite: write `entries.dat.tmp` → `sync_all` → `MoveFileExW` replace over `entries.dat` (no remove-then-rename gap)
- On unknown version or parse failure: move to `entries.dat.bak`, start with empty history (logged, never crashes)

### Atomic metadata replace (`write_entries_atomic`)

`src/store/meta.rs` writes the full serialized history to `entries.dat.tmp`, calls `sync_all` on the temp file, then promotes it in one step:

```text
MoveFileExW(tmp, entries.dat, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
```

- **`MOVEFILE_REPLACE_EXISTING`** — atomically overwrites an existing `entries.dat` without a separate `remove_file` + `rename` (on Windows, `fs::rename` fails when the destination exists, which previously forced remove-first and left a crash window where neither path held live metadata).
- **`MOVEFILE_WRITE_THROUGH`** — flush the move to disk before returning.
- Paths are NUL-terminated UTF-16 via `win32::wide()`; failures map to `ClipError::Win32` through `last_error("MoveFileExW")`.
- `.bak` backup-on-load-failure and blob-store behavior are unchanged; `load_entries` still ignores `.tmp`.

Tests: `atomic_write_reload_equality`, `atomic_write_overwrites_existing` (second write replaces existing `entries.dat`).

`image_pixels` are not stored in metadata; blobs are loaded on demand via `Store::read_blob`.

### In-memory image pixels

After capture, image entries hold decoded BGRA bytes in `ClipEntry::image_pixels` only until `App::on_clipboard_captured` enqueues `PersistAll` (the worker job owns a deep clone for `write_blob`). The in-memory buffer is then cleared immediately so memory does not grow with each capture and later `enqueue_persist` calls (pin, delete, cap prune) do not re-clone megabytes of pixels. UI and copy-back resolve pixels via `image_pixels` when present, otherwise `Store::read_blob` (`App::resolve_image_pixels`, history thumbnails, preview modal).

## Blob store

- Content-addressed by lowercase hex SHA-256 (`ImageRef.hash`)
- `write_blob` skips if the file already exists (dedup by hash)
- Orphan cleanup: `enqueue_prune_orphans` removes `.dib` files not referenced by current entries
- **Shared blobs:** because blobs are content-addressed, two entries with identical image content reference the same `.dib`. This is reachable with `deduplicate_global = false` (the default), where only consecutive duplicates are suppressed. `enqueue_prune_orphans` is reference-safe (it diffs against all referenced hashes). Direct deletes use `App::enqueue_removed_entry_delete` — see [`blob-reference-counting.md`](blob-reference-counting.md).

## Worker thread

Single background thread (`trayvault-store`) receives jobs via `mpsc`:

| Job | Purpose |
|-----|---------|
| `PersistAll` | Write image blobs from job snapshot `image_pixels` (present on capture enqueue only), then atomic metadata rewrite |
| `DeleteEntry` | Remove a blob when an entry is deleted and no other entry references the hash |
| `PruneOrphans` | Delete unreferenced blobs under `blobs/` |

Public API:

- `Store::load_initial()` — load history at startup, spawn worker
- `Store::enqueue_persist(&[ClipEntry])` — queue full persist after history changes
- Failures are logged only; in-memory state is retained; next change retries

## Related

- Data model: `docs/technical/clipboard-capture.md`
- Hashing / blob filenames: `docs/technical/hashing-dedup.md`
