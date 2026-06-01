//! Persistence layer: metadata file, blob store, and background worker thread.
//!
//! Disk IO runs on a single worker thread via `mpsc`; the UI thread enqueues jobs
//! after history changes. Failures are logged only — in-memory state is retained.

#![allow(dead_code)] // public API for Tasks 6–7

mod blobs;
mod meta;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use crate::hash::hash_to_hex;
use crate::log;
use crate::models::ClipEntry;

pub use meta::LoadResult;

/// Jobs processed by the storage worker thread.
enum Job {
    PersistAll(Vec<ClipEntry>),
    DeleteEntry {
        entry_id: u64,
        image_hash: Option<String>,
    },
    PruneOrphans(Vec<ClipEntry>),
    /// Ack when the worker is idle (used during shutdown).
    Ping(std::sync::mpsc::Sender<()>),
}

/// Handle for enqueueing persistence jobs from the UI thread.
pub struct Store {
    data_dir: PathBuf,
    job_tx: Option<Sender<Job>>,
    worker: Option<JoinHandle<()>>,
}

impl Store {
    /// Resolve `%LOCALAPPDATA%\TrayVault`, creating it if missing.
    pub fn data_dir() -> Option<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA")?;
        let dir = PathBuf::from(base).join("TrayVault");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// Load persisted history at startup and spawn the worker thread.
    pub fn load_initial() -> (LoadResult, Self) {
        let data_dir =
            Self::data_dir().unwrap_or_else(|| std::env::temp_dir().join("trayvault-fallback"));
        let loaded = meta::load_entries(&data_dir);
        let store = Self::open_with_dir(data_dir);
        (loaded, store)
    }

    fn open_with_dir(data_dir: PathBuf) -> Self {
        let (job_tx, job_rx) = mpsc::channel();
        let worker_dir = data_dir.clone();
        let worker = thread::Builder::new()
            .name("trayvault-store".into())
            .spawn(move || worker_loop(worker_dir, job_rx))
            .expect("spawn storage worker");

        Self {
            data_dir,
            job_tx: Some(job_tx),
            worker: Some(worker),
        }
    }

    /// Queue a full metadata rewrite and blob writes for in-memory image pixels.
    pub fn enqueue_persist(&self, entries: &[ClipEntry]) {
        self.send_job(Job::PersistAll(entries.to_vec()));
    }

    /// Queue deletion of an entry's image blob (metadata rewrite is separate).
    pub fn enqueue_delete(&self, entry_id: u64, image_hash: Option<String>) {
        self.send_job(Job::DeleteEntry {
            entry_id,
            image_hash,
        });
    }

    /// Queue orphan blob cleanup against the current entry set.
    pub fn enqueue_prune_orphans(&self, entries: &[ClipEntry]) {
        self.send_job(Job::PruneOrphans(entries.to_vec()));
    }

    /// Load blob pixels on demand (safe to call from any thread).
    pub fn read_blob(&self, hash: &str) -> Option<Vec<u8>> {
        blobs::read_blob(&self.data_dir, hash)
    }

    fn send_job(&self, job: Job) {
        if let Some(tx) = &self.job_tx {
            if tx.send(job).is_err() {
                log::error("storage worker channel closed; job dropped");
            }
        }
    }

    /// Block until queued jobs finish (call before process exit).
    pub fn flush(&self, entries: &[ClipEntry]) {
        self.enqueue_persist(entries);
        let (tx, rx) = mpsc::channel();
        self.send_job(Job::Ping(tx));
        if rx.recv().is_err() {
            log::warn("storage worker did not ack flush ping");
        }
    }

    /// Drop the job channel and join the worker thread.
    pub fn join(&mut self) {
        drop(self.job_tx.take());
        if let Some(worker) = self.worker.take() {
            if let Err(err) = worker.join() {
                log::error(&format!("storage worker join failed: {err:?}"));
            }
        }
    }
}

#[cfg(test)]
impl Store {
    pub fn open_for_test(data_dir: PathBuf) -> Self {
        Self::open_with_dir(data_dir)
    }
}

fn worker_loop(data_dir: PathBuf, rx: mpsc::Receiver<Job>) {
    for job in rx {
        match job {
            Job::PersistAll(entries) => {
                if let Err(err) = persist_all(&data_dir, &entries) {
                    log::error(&format!("persist failed: {err}"));
                }
            }
            Job::DeleteEntry {
                entry_id,
                image_hash,
            } => {
                if let Some(hash) = image_hash {
                    if let Err(err) = blobs::delete_blob(&data_dir, &hash) {
                        log::warn(&format!(
                            "blob delete failed for entry {entry_id} ({hash}): {err}"
                        ));
                    }
                }
            }
            Job::PruneOrphans(entries) => {
                let referenced = meta::referenced_blob_hashes(&entries);
                if let Err(err) = blobs::prune_orphaned_blobs(&data_dir, &referenced) {
                    log::warn(&format!("orphan blob prune failed: {err}"));
                }
            }
            Job::Ping(tx) => {
                let _ = tx.send(());
            }
        }
    }
}

fn persist_all(data_dir: &Path, entries: &[ClipEntry]) -> crate::error::Result<()> {
    for entry in entries {
        if let (Some(image), Some(pixels)) = (&entry.image, entry.image_pixels.as_ref()) {
            let expected = hash_to_hex(entry.hash);
            if image.hash != expected {
                log::warn(&format!(
                    "entry {} image hash mismatch (meta={}, entry={expected})",
                    entry.id, image.hash
                ));
            }
            blobs::write_blob(data_dir, &image.hash, pixels)?;
        }
    }
    meta::write_entries_atomic(data_dir, entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{hash_text, hash_to_hex};
    use crate::models::{EntryKind, ImageRef};
    use std::fs;
    use std::time::Duration;

    fn temp_data_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("trayvault-store-{prefix}-{}", std::process::id()))
    }

    fn wait_for<F: Fn() -> bool>(predicate: F) {
        for _ in 0..100 {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("condition not met within timeout");
    }

    #[test]
    fn integration_persist_and_reload() {
        let dir = temp_data_dir("integration");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let pixels = vec![10u8, 20, 30, 40];
        let digest = crate::hash::hash_image_pixels(&pixels);
        let entry = ClipEntry {
            id: 0,
            created_at: 1_700_000_000_000,
            kind: EntryKind::Image,
            text: None,
            html: None,
            image: Some(ImageRef {
                hash: hash_to_hex(digest),
                width: 1,
                height: 1,
            }),
            image_pixels: Some(pixels.clone()),
            source_app: None,
            is_pinned: false,
            hash: digest,
        };

        let store = Store::open_for_test(dir.clone());
        store.enqueue_persist(std::slice::from_ref(&entry));
        wait_for(|| meta::entries_path(&dir).exists());

        let loaded = meta::load_entries(&dir);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].kind, EntryKind::Image);
        assert_eq!(loaded.entries[0].image, entry.image);
        assert_eq!(blobs::read_blob(&dir, &hash_to_hex(digest)), Some(pixels));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn integration_text_persist_reload() {
        let dir = temp_data_dir("text");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let entry = ClipEntry {
            id: 0,
            created_at: 42,
            kind: EntryKind::Text,
            text: Some("saved".into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: Some("app.exe".into()),
            is_pinned: true,
            hash: hash_text("saved"),
        };

        let store = Store::open_for_test(dir.clone());
        store.enqueue_persist(std::slice::from_ref(&entry));
        wait_for(|| meta::entries_path(&dir).exists());

        let loaded = meta::load_entries(&dir);
        assert_eq!(loaded.entries, vec![entry]);

        let _ = fs::remove_dir_all(&dir);
    }
}
