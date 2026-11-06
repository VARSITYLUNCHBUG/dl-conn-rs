//! The app's log: a capped buffer that anything can write to and the console page drains, plus a
//! capped file, so there's a record after a crash or a support question.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::mem::zeroed;
use std::path::PathBuf;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// Enough history to replay when the console first opens, without growing forever if it never does.
const CAPACITY: usize = 1 << 20;
/// The file is replaced once it reaches this, keeping one old copy, so it can't grow forever.
const FILE_CAPACITY: u64 = 1 << 20;
const OLD_SUFFIX: &str = ".old";

struct Pending {
    text: String,
    clear: bool,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    text: String::new(),
    clear: false,
});
type Wake = Box<dyn Fn() + Send + Sync>;
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);
static FILE: Mutex<Option<LogFile>> = Mutex::new(None);

struct LogFile {
    path: PathBuf,
    file: File,
    written: u64,
}

impl LogFile {
    fn open(path: PathBuf) -> std::io::Result<Self> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            written,
        })
    }

    /// Failing to write must never affect the app, so errors are ignored.
    fn write(&mut self, text: &str) {
        if self.written + text.len() as u64 > FILE_CAPACITY {
            let mut old = self.path.clone().into_os_string();
            old.push(OLD_SUFFIX);
            let _ = fs::rename(&self.path, old);
            if let Ok(fresh) = Self::open(self.path.clone()) {
                *self = fresh;
            }
        }
        if self.file.write_all(text.as_bytes()).is_ok() {
            self.written += text.len() as u64;
        }
    }
}

/// Starts keeping the log in this file as well.
pub fn to_file(path: PathBuf) {
    if let Ok(file) = LogFile::open(path) {
        *FILE.lock().unwrap() = Some(file);
    }
}

/// Writes to the file only, without waiting on anything a panicking thread might hold.
pub fn emergency(text: &str) {
    if let Ok(mut file) = FILE.try_lock()
        && let Some(file) = file.as_mut()
    {
        file.write(&format!("{text}\r\n"));
    }
}

/// Adds a timestamped line.
pub fn line(text: String) {
    let time = unsafe {
        let mut time: SYSTEMTIME = zeroed();
        GetLocalTime(&mut time);
        time
    };
    let stamped = format!(
        "{:02}:{:02}:{:02}.{:03} {text}\r\n",
        time.wHour, time.wMinute, time.wSecond, time.wMilliseconds
    );
    if let Some(file) = FILE.lock().unwrap().as_mut() {
        file.write(&stamped);
    }
    {
        let mut pending = PENDING.lock().unwrap();
        pending.text.push_str(&stamped);
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

/// Sets what's called whenever there is something to drain.
pub fn set_wake(callback: Wake) {
    *WAKE.lock().unwrap() = Some(callback);
    wake();
}

fn wake() {
    if let Some(callback) = WAKE.lock().unwrap().as_ref() {
        callback();
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
    fn the_file_is_capped_and_keeps_one_old_copy() {
        let folder = std::env::temp_dir().join(format!("dl-rs-log-{}", std::process::id()));
        let path = folder.join("nested").join("dl-rs.log");
        let mut file = LogFile::open(path.clone()).unwrap();
        file.write("first\r\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\r\n");
        file.written = FILE_CAPACITY;
        file.write("second\r\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\r\n");
        let mut old = path.clone().into_os_string();
        old.push(OLD_SUFFIX);
        assert_eq!(fs::read_to_string(old).unwrap(), "first\r\n");
        drop(file);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn trim_leaves_small_buffers_alone() {
        let mut text = "short\r\n".to_string();
        trim(&mut text);
        assert_eq!(text, "short\r\n");
    }
}
