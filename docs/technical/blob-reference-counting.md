# Shared Blob Reference Counting

Modules: `src/app.rs` (delete/prune paths), `src/store/` (blob I/O). Storage layout: [`storage.md`](storage.md).

## Problem

Image blobs are content-addressed: `blobs/<hex-sha256>.dib`. `write_blob` skips writes when the file already exists. With the default config (`deduplicate_global = false`), only **consecutive** duplicates are suppressed at capture — so the same image copied twice with text in between yields two `ClipEntry` rows sharing one blob file.

Deleting or cap-pruning one entry must not remove a blob still referenced by another entry.

## Fix

`App` gates blob deletion through `enqueue_removed_entry_delete`:

1. Remove entry from `self.entries` (or via `prune_to_cap` first).
2. If the removed entry had an image, call `blob_still_referenced(&hash)` on **remaining** entries.
3. `store.enqueue_delete(entry_id, Some(hash))` only when no survivor references that hash; otherwise `enqueue_delete(entry_id, None)` (metadata persist is separate).

Used in:

- `delete_entry`
- Cap prune in `on_clipboard_captured`
- Cap reduction in `set_max_entries`

## Backstop

`enqueue_prune_orphans` diffs on-disk blobs against `meta::referenced_blob_hashes(&entries)` and is already reference-safe. Direct `DeleteEntry` jobs must follow the same rule so shared blobs are not removed prematurely.

## Manual regression

With “Deduplicate globally” **off**: copy an image, copy text, copy the same image again, delete the newer entry — the older entry should still show its thumbnail (not “Image unavailable”).
