//! Minimal file logger.
//!
//! Writes timestamped lines to `%LOCALAPPDATA%\TrayVault\trayvault.log`. This is
//! intentionally tiny and dependency-free (pure `std`): no `tracing`, no `log`.
//!
//! Initialization is best-effort and idempotent — if the log file cannot be
//! opened, logging silently becomes a no-op (plus an `eprintln!` in debug
//! builds) and never affects the running app. Logging is thread-safe so the
//! storage worker thread can log too.
//!
//! When the log file reaches [`MAX_LOG_BYTES`], it is rotated to
//! `trayvault.log.1` (single generation) before opening a fresh file.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Rotate the log when it reaches this size (5 MiB).
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// The opened log file, or `None` if logging is disabled / failed to init.
static LOGGER: OnceLock<Option<Mutex<File>>> = OnceLock::new();

/// Resolve `%LOCALAPPDATA%\TrayVault`, creating it if necessary.
///
/// Shared by the logger now and reused conceptually by storage/config later
/// (those modules own their own copy to stay decoupled).
fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(base).join("TrayVault");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// If `path` exists and is at least `max_bytes`, rotate it to `path.1`.
///
/// Best-effort: any failure is ignored so logging can fall back to plain append.
fn rotate_if_needed(path: &Path, max_bytes: u64) {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return,
    };
    if size < max_bytes {
        return;
    }

    let backup = path.with_file_name(format!(
        "{}.1",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));

    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::rename(path, &backup);
}

/// Open the log file for appending. Best-effort; returns `None` on any failure.
fn open_log() -> Option<Mutex<File>> {
    let path = data_dir()?.join("trayvault.log");
    rotate_if_needed(&path, MAX_LOG_BYTES);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    Some(Mutex::new(file))
}

/// Initialize the logger. Safe to call more than once; only the first call
/// takes effect.
pub fn init() {
    let _ = LOGGER.set(open_log());
}

/// Format a Unix-millis timestamp as `YYYY-MM-DD HH:MM:SS.mmmZ` (UTC).
///
/// Uses Howard Hinnant's civil-from-days algorithm so we avoid pulling in
/// `chrono` for a single log timestamp.
fn format_utc(ms: u128) -> String {
    let secs = (ms / 1000) as i64;
    let millis = (ms % 1000) as u32;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (tod / 3600, (tod % 3600) / 60, tod % 60);

    // civil_from_days: days since 1970-01-01 -> (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn write_line(level: &str, msg: &str) {
    let line = format!("{} [{:>5}] {}\n", format_utc(now_millis()), level, msg);

    if let Some(Some(file)) = LOGGER.get() {
        if let Ok(mut guard) = file.lock() {
            let _ = guard.write_all(line.as_bytes());
            let _ = guard.flush();
        }
    }

    #[cfg(debug_assertions)]
    {
        // Mirror to stderr during development for convenience.
        eprint!("{line}");
    }
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

#[allow(dead_code)] // used by later milestones
pub fn warn(msg: &str) {
    write_line("WARN", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_log_path(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "trayvault_log_test_{}_{suffix}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("trayvault.log")
    }

    fn cleanup(path: &Path) {
        let backup = path.with_file_name("trayvault.log.1");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn rotate_if_needed_skips_under_cap() {
        let path = temp_log_path("under");
        cleanup(&path);

        let mut f = File::create(&path).unwrap();
        f.write_all(&[b'x'; 50]).unwrap();
        drop(f);

        rotate_if_needed(&path, 100);
        assert!(path.exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 50);

        cleanup(&path);
    }

    #[test]
    fn rotate_if_needed_renames_when_over_cap() {
        let path = temp_log_path("over");
        cleanup(&path);
        let backup = path.with_file_name("trayvault.log.1");

        let mut f = File::create(&path).unwrap();
        f.write_all(&[b'x'; 150]).unwrap();
        drop(f);

        rotate_if_needed(&path, 100);
        assert!(backup.exists());
        assert_eq!(std::fs::metadata(&backup).unwrap().len(), 150);
        assert!(!path.exists());

        cleanup(&path);
    }

    #[test]
    fn rotate_if_needed_replaces_existing_backup() {
        let path = temp_log_path("backup");
        cleanup(&path);
        let backup = path.with_file_name("trayvault.log.1");

        std::fs::write(&backup, b"old").unwrap();
        let mut f = File::create(&path).unwrap();
        f.write_all(&[b'y'; 200]).unwrap();
        drop(f);

        rotate_if_needed(&path, 100);
        assert!(backup.exists());
        assert_eq!(std::fs::metadata(&backup).unwrap().len(), 200);

        cleanup(&path);
    }
}
