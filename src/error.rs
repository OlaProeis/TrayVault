//! The crate-wide error type.
//!
//! Every fallible operation in TrayVault — especially the hand-rolled Win32
//! FFI wrappers — returns [`Result<T>`], never panicking on an OS failure. The
//! Win32 surface in particular must translate `GetLastError` codes into
//! [`ClipError::Win32`] so the message loop can log and recover instead of
//! crashing.

use std::fmt;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ClipError>;

/// All failure modes TrayVault cares about.
///
/// Several variants are constructed only by later milestones (registry,
/// hotkey, config), so the enum is allowed to carry not-yet-used variants
/// during early development.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ClipError {
    /// A Win32 API call failed. `code` is the value returned by `GetLastError`
    /// captured immediately after the failing call.
    Win32 { api: &'static str, code: u32 },

    /// A Windows registry operation failed. `code` is a Win32 error code
    /// (registry APIs return the code directly rather than via `GetLastError`).
    Registry { op: &'static str, code: u32 },

    /// The requested global hotkey could not be registered (already taken).
    HotkeyConflict { hotkey: String },

    /// Configuration parsing or validation failed.
    Config(String),

    /// A filesystem / IO error (storage, logging, config persistence).
    Io(std::io::Error),

    /// Any other contextual failure.
    Other(String),
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClipError::Win32 { api, code } => {
                write!(f, "Win32 call `{api}` failed (error {code}, 0x{code:08X})")
            }
            ClipError::Registry { op, code } => {
                write!(f, "registry op `{op}` failed (error {code}, 0x{code:08X})")
            }
            ClipError::HotkeyConflict { hotkey } => {
                write!(
                    f,
                    "global hotkey `{hotkey}` is already registered by another application"
                )
            }
            ClipError::Config(msg) => write!(f, "config error: {msg}"),
            ClipError::Io(err) => write!(f, "io error: {err}"),
            ClipError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ClipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClipError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ClipError {
    fn from(err: std::io::Error) -> Self {
        ClipError::Io(err)
    }
}
