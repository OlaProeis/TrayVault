//! Background thumbnail loader: disk `read_blob` + bilinear scale off the UI thread.
//!
//! Results arrive on an `mpsc` reply channel; the worker posts [`WM_THUMB_READY`] so the
//! message loop can drain replies, insert into [`ThumbCache`], and repaint.

use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::log;
use crate::store::read_blob_at;
use crate::ui::pixmap::Pixmap;
use crate::ui::thumb_cache::build_thumbnail_pixmap;
use crate::win32::ffi::{PostMessageW, HWND};
use crate::win32::wic;
use crate::win32::window::WM_THUMB_READY;

/// Request sent to the thumb-loader worker.
#[derive(Clone, Debug)]
pub struct ThumbLoadRequest {
    pub entry_id: u64,
    pub hash: String,
    pub img_w: u32,
    pub img_h: u32,
    pub dst_w: u32,
    pub dst_h: u32,
    pub generation: u64,
}

/// Completed thumbnail from the worker (may be dropped when stale).
#[derive(Debug)]
pub struct ThumbLoadReply {
    pub entry_id: u64,
    pub dst_w: u32,
    pub dst_h: u32,
    pub generation: u64,
    /// `None` when the load failed so the UI can retry on the next paint.
    pub pixmap: Option<Arc<Pixmap>>,
}

enum Job {
    Load(ThumbLoadRequest),
}

/// Single worker thread for async list thumbnails.
pub struct ThumbLoader {
    request_tx: Option<Sender<Job>>,
    reply_rx: Receiver<ThumbLoadReply>,
    notify_hwnd: Arc<AtomicPtr<std::ffi::c_void>>,
    worker: Option<JoinHandle<()>>,
}

impl ThumbLoader {
    pub fn new(data_dir: PathBuf) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let notify_hwnd = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let worker_notify = Arc::clone(&notify_hwnd);
        let worker_dir = data_dir;
        let worker = thread::Builder::new()
            .name("trayvault-thumb".into())
            .spawn(move || worker_loop(worker_dir, request_rx, reply_tx, worker_notify))
            .expect("spawn thumb loader worker");

        Self {
            request_tx: Some(request_tx),
            reply_rx,
            notify_hwnd,
            worker: Some(worker),
        }
    }

    /// HWND to wake via `PostMessageW` when a load completes (main thread only).
    pub fn set_notify_hwnd(&self, hwnd: HWND) {
        self.notify_hwnd
            .store(hwnd as *mut std::ffi::c_void, Ordering::SeqCst);
    }

    pub fn request(&self, req: ThumbLoadRequest) {
        if let Some(tx) = &self.request_tx {
            if tx.send(Job::Load(req)).is_err() {
                log::warn("thumb loader channel closed; request dropped");
            }
        }
    }

    /// Drain all completed loads queued since the last call.
    pub fn drain_replies(&self) -> Vec<ThumbLoadReply> {
        let mut replies = Vec::new();
        while let Ok(reply) = self.reply_rx.try_recv() {
            replies.push(reply);
        }
        replies
    }

    pub fn join(&mut self) {
        drop(self.request_tx.take());
        if let Some(worker) = self.worker.take() {
            if let Err(err) = worker.join() {
                log::error(&format!("thumb loader join failed: {err:?}"));
            }
        }
    }
}

fn worker_loop(
    data_dir: PathBuf,
    rx: Receiver<Job>,
    reply_tx: Sender<ThumbLoadReply>,
    notify_hwnd: Arc<AtomicPtr<std::ffi::c_void>>,
) {
    if let Err(err) = wic::ensure_com_initialized() {
        log::error(&format!("thumb loader COM init failed: {err}"));
    }

    for job in rx {
        let Job::Load(req) = job;
        let pixmap = read_blob_at(&data_dir, &req.hash, req.img_w, req.img_h).and_then(|pixels| {
            build_thumbnail_pixmap(&pixels, req.img_w, req.img_h, req.dst_w, req.dst_h)
        });
        let reply = ThumbLoadReply {
            entry_id: req.entry_id,
            dst_w: req.dst_w,
            dst_h: req.dst_h,
            generation: req.generation,
            pixmap: pixmap.map(Arc::new),
        };
        if reply_tx.send(reply).is_err() {
            break;
        }
        notify_thumb_ready(notify_hwnd.as_ref());
    }
}

fn notify_thumb_ready(notify_hwnd: &AtomicPtr<std::ffi::c_void>) {
    let hwnd = notify_hwnd.load(Ordering::SeqCst) as HWND;
    if hwnd != 0 {
        // SAFETY: `hwnd` is the live main window set on the UI thread.
        unsafe {
            let _ = PostMessageW(hwnd, WM_THUMB_READY, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{hash_image_pixels, hash_to_hex};
    use std::fs;
    use std::time::Duration;

    fn temp_data_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("trayvault-thumb-{prefix}-{}", std::process::id()))
    }

    fn wait_for<F: FnMut() -> bool>(mut predicate: F) {
        for _ in 0..200 {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("condition not met within timeout");
    }

    #[test]
    fn async_load_delivers_scaled_pixmap_without_ui_thread_read() {
        let dir = temp_data_dir("async");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let pixels = vec![10u8, 20, 30, 255];
        let digest = hash_image_pixels(&pixels);
        let hash = hash_to_hex(digest);
        fs::create_dir_all(dir.join("blobs")).expect("mkdir blobs");
        fs::write(dir.join("blobs").join(format!("{hash}.dib")), &pixels).expect("write raw");

        let mut loader = ThumbLoader::new(dir.clone());
        loader.request(ThumbLoadRequest {
            entry_id: 42,
            hash,
            img_w: 1,
            img_h: 1,
            dst_w: 1,
            dst_h: 1,
            generation: 0,
        });

        let mut replies = Vec::new();
        wait_for(|| {
            replies.extend(loader.drain_replies());
            !replies.is_empty()
        });
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].entry_id, 42);
        let pixmap = replies[0].pixmap.as_ref().expect("thumb pixmap");
        assert_eq!(pixmap.width(), 1);
        assert_eq!(pixmap.height(), 1);

        loader.join();
        let _ = fs::remove_dir_all(&dir);
    }
}
