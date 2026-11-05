//! dl-rs: pick one Steam relay region and block the rest, for this boot only.
//!
//! Commands are the only way to act on the app. The dropdown and the console are both thin
//! front-ends that send command lines to the session.

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*)) };
}

mod command;
mod console;
mod diag;
mod editor;
mod log;
mod sdr;
mod session;
pub mod ui;
mod wfp;

#[cfg(test)]
mod live_tests;

/// NUL-terminated UTF-16 for Win32 calls.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads UTF-16 up to the first NUL, or the end of the slice if there is none.
fn from_wide(s: &[u16]) -> String {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..len])
}
