//! The app's log: a capped buffer that anything can write to and the console drains.

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, Ordering};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::SetEvent;

/// Enough history to replay when the console first opens, without growing forever if it never does.
const CAPACITY: usize = 1 << 20;

struct Pending {
    text: String,
    clear: bool,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    text: String::new(),
    clear: false,
});
static WAKE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// Adds a timestamped line.
pub fn line(text: String) {
    let time = unsafe {
        let mut time: SYSTEMTIME = zeroed();
        GetLocalTime(&mut time);
        time
    };
    {
        let mut pending = PENDING.lock().unwrap();
        pending.text.push_str(&format!(
            "{:02}:{:02}:{:02}.{:03} {text}\r\n",
            time.wHour, time.wMinute, time.wSecond, time.wMilliseconds
        ));
        trim(&mut pending.text);
    }
    wake();
}

/// Discards pending lines and asks the console to clear its screen.
pub fn clear() {
    {
        let mut pending = PENDING.lock().unwrap();
        pending.text.clear();
        pending.clear = true;
    }
    wake();
}

/// Takes everything pending: whether to clear the screen first, then the text to print.
pub fn drain() -> (bool, String) {
    let mut pending = PENDING.lock().unwrap();
    (
        std::mem::take(&mut pending.clear),
        std::mem::take(&mut pending.text),
    )
}

/// Sets the event signalled whenever there is something to drain.
pub fn set_wake(event: *mut c_void) {
    WAKE.store(event, Ordering::Release);
    wake();
}

fn wake() {
    let event = WAKE.load(Ordering::Acquire);
    if !event.is_null() {
        unsafe { SetEvent(event) };
    }
}

/// Drops the oldest half, on a line boundary, once the buffer is over capacity.
fn trim(text: &mut String) {
    if text.len() <= CAPACITY {
        return;
    }
    let from = text.len() / 2;
    let cut = text[from..].find('\n').map_or(from, |at| from + at + 1);
    text.drain(..cut);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_whole_recent_lines() {
        let mut text = "old line\r\n".repeat(CAPACITY / 10 + 1);
        text.push_str("newest\r\n");
        trim(&mut text);
        assert!(text.len() <= CAPACITY);
        assert!(text.starts_with("old line"));
        assert!(text.ends_with("newest\r\n"));
    }

    #[test]
    fn trim_leaves_small_buffers_alone() {
        let mut text = "short\r\n".to_string();
        trim(&mut text);
        assert_eq!(text, "short\r\n");
    }
}
