# Hashing and Deduplication

Modules: `src/hash.rs` (core), integrated in `src/win32/clipboard.rs` (capture pipeline).

## SHA-256 (`hash.rs`)

Hand-rolled FIPS 180-4 implementation in safe Rust (no crypto crates):

- **`Sha256`** — streaming API: `new`, `update`, `finalize`.
- **`sha256(bytes)`** — one-shot digest → `[u8; 32]`.
- **`hash_to_hex`** — lowercase hex string for blob filenames and `ImageRef.hash`.

Known test vectors: empty input, `"abc"`, multi-block message.

## Content normalization

| Kind | Hashed material |
|------|-----------------|
| Text | UTF-8 bytes after line-ending normalization (`\r\n`/`\r` → `\n`) and per-line trailing whitespace trim |
| RichText | Normalized HTML fragment when present; otherwise normalized plain text |
| Image | Raw BGRA pixel bytes from `parse_dib_to_bgra` |

Each captured `ClipEntry` stores the digest in `hash`. Image entries also set `ImageRef.hash` to the hex form.

## Deduplication (`clipboard.rs`)

After `build_entry`, before `ClipHistory::push`:

1. **Consecutive (always):** drop when the new entry’s hash matches the most recent entry **and** `kind` matches.
2. **Global (`deduplicate_global`, default `false`):** drop when the hash matches **any** entry in history.

`CaptureConfig::deduplicate_global` is a placeholder until Task 6 wires real config.

## Related

- Data model: `docs/technical/clipboard-capture.md`
- Blob storage by hash: `docs/technical/storage.md`
