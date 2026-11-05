//! The debug console: a real Windows console where the log scrolls above a command prompt.
//!
//! One thread does all console I/O, waiting on key presses and new log text together. That means
//! no locks between typing and logging, and a mouse selection, which freezes console writes, only
//! pauses this thread while the log buffers.

use crate::editor::{Action, Editor, Key};
use crate::log;
use crate::session::Handle;
use crate::wide;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use windows_sys::Win32::Foundation::{HANDLE, HWND, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForMultipleObjects};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB,
    VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const PROMPT: &str = "dl-rs> ";
const ERASE_LINE: &str = "\r\x1b[2K";
const CLEAR_SCREEN: &str = "\x1b[2J\x1b[3J\x1b[H";
const CTRL_C_CHAR: u16 = 3;
const WRITE_CHUNK: usize = 8192;
const KEYS_AT_ONCE: usize = 64;

static SESSION: OnceLock<Handle> = OnceLock::new();
static OPENED: AtomicBool = AtomicBool::new(false);

/// Gives the console the session its prompt talks to. Nothing opens until the first toggle.
pub fn init(session: Handle) {
    let _ = SESSION.set(session);
}

/// Shows or hides the console, creating it the first time. Returns whether it's now visible.
pub fn toggle() -> bool {
    unsafe {
        if !OPENED.swap(true, Ordering::AcqRel) {
            return open();
        }
        let window = GetConsoleWindow();
        if window.is_null() {
            return false;
        }
        let show = IsWindowVisible(window) == 0;
        // Async, because the window belongs to another process and we shouldn't wait on it.
        ShowWindowAsync(window, if show { SW_SHOW } else { SW_HIDE });
        if show {
            SetForegroundWindow(window);
        }
        show
    }
}

unsafe fn open() -> bool {
    unsafe {
        if AllocConsole() == 0 {
            log!("console: could not open one");
            return false;
        }
        SetConsoleTitleW(w!("dl-rs console"));
        SetConsoleCtrlHandler(Some(on_console_signal), 1);
        protect_from_closing(GetConsoleWindow());
        let (input, output) = (
            GetStdHandle(STD_INPUT_HANDLE) as usize,
            GetStdHandle(STD_OUTPUT_HANDLE) as usize,
        );
        thread::spawn(move || io_loop(input as HANDLE, output as HANDLE));
        SetForegroundWindow(GetConsoleWindow());
        true
    }
}

/// Windows ends the whole process when a console closes, so its X button has to go.
unsafe fn protect_from_closing(window: HWND) {
    unsafe {
        DeleteMenu(GetSystemMenu(window, 0), SC_CLOSE, MF_BYCOMMAND);
    }
}

/// Ctrl+Break would otherwise end the process. Closing can't be stopped, so it stays default.
unsafe extern "system" fn on_console_signal(signal: u32) -> windows_sys::core::BOOL {
    i32::from(signal == CTRL_C_EVENT || signal == CTRL_BREAK_EVENT)
}

fn io_loop(input: HANDLE, output: HANDLE) {
    let mut editor = Editor::default();
    unsafe {
        SetConsoleMode(
            output,
            ENABLE_PROCESSED_OUTPUT
                | ENABLE_WRAP_AT_EOL_OUTPUT
                | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        );
        // Raw keys, so Ctrl+C and Tab arrive as input. Quick edit keeps mouse selection and copy.
        SetConsoleMode(
            input,
            ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS | ENABLE_QUICK_EDIT_MODE,
        );
        let log_ready = CreateEventW(null(), 0, 0, null());
        if log_ready.is_null() {
            return;
        }
        log::set_wake(log_ready);
        let waits = [input, log_ready];
        loop {
            match WaitForMultipleObjects(2, waits.as_ptr(), 0, INFINITE) {
                WAIT_OBJECT_0 => read_keys(input, output, &mut editor),
                signalled if signalled == WAIT_OBJECT_0 + 1 => flush_log(output, &editor),
                _ => return,
            }
        }
    }
}

unsafe fn read_keys(input: HANDLE, output: HANDLE, editor: &mut Editor) {
    unsafe {
        let mut records: [INPUT_RECORD; KEYS_AT_ONCE] = zeroed();
        let mut count = 0;
        if ReadConsoleInputW(input, records.as_mut_ptr(), KEYS_AT_ONCE as u32, &mut count) == 0 {
            return;
        }
        for record in &records[..count as usize] {
            if u32::from(record.EventType) != KEY_EVENT {
                continue;
            }
            let key = record.Event.KeyEvent;
            if key.bKeyDown == 0 {
                continue;
            }
            for _ in 0..key.wRepeatCount {
                let action = press(editor, &key, capacity(output));
                perform(action, output, editor);
            }
        }
    }
}

fn press(editor: &mut Editor, key: &KEY_EVENT_RECORD, capacity: usize) -> Action {
    let text = unsafe { key.uChar.UnicodeChar };
    let mapped = match key.wVirtualKeyCode {
        VK_TAB => {
            let servers = SESSION.get().map(Handle::servers).unwrap_or_default();
            return editor.tab(&servers);
        }
        VK_RETURN => Key::Enter,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_ESCAPE => Key::Cancel,
        _ if text == CTRL_C_CHAR => Key::Cancel,
        _ => match char::from_u32(u32::from(text)) {
            Some(c) if text != 0 => Key::Char(c),
            _ => return Action::None,
        },
    };
    editor.key(mapped, capacity)
}

fn perform(action: Action, output: HANDLE, editor: &Editor) {
    match action {
        Action::None => {}
        Action::Redraw => write(output, &input_line(editor)),
        Action::Submit(line) => {
            write(output, &input_line(editor));
            run(&line);
        }
        Action::Toggle => run("toggle console"),
        Action::Help(line) => crate::command::help_for(&line)
            .into_iter()
            .for_each(log::line),
        Action::ShowCandidates(candidates) => {
            let width = candidates.iter().map(|c| c.text.len()).max().unwrap_or(0);
            for candidate in candidates {
                log!("  {:<width$}  {}", candidate.text, candidate.note);
            }
        }
    }
}

fn run(line: &str) {
    if let Some(session) = SESSION.get() {
        // The result reaches the log, so the reply isn't needed here.
        let _ = session.run(line);
    }
}

/// Prints new log text above the prompt, leaving whatever's being typed intact.
fn flush_log(output: HANDLE, editor: &Editor) {
    let (clear, text) = log::drain();
    let mut screen = String::from(ERASE_LINE);
    if clear {
        screen.push_str(CLEAR_SCREEN);
    }
    screen.push_str(&text);
    screen.push_str(&input_line(editor));
    write(output, &screen);
}

/// Erases the current line and redraws the prompt, input and cursor.
fn input_line(editor: &Editor) -> String {
    format!(
        "{ERASE_LINE}{PROMPT}{}\x1b[{}G",
        editor.line(),
        PROMPT.len() + editor.cursor() + 1
    )
}

/// How much input fits on one console line, which keeps redraws simple.
fn capacity(output: HANDLE) -> usize {
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { zeroed() };
    if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
        return 0;
    }
    let width = usize::try_from(info.srWindow.Right - info.srWindow.Left + 1).unwrap_or(0);
    width.saturating_sub(PROMPT.len() + 1)
}

fn write(output: HANDLE, text: &str) {
    let mut units = wide(text);
    units.pop();
    for chunk in units.chunks(WRITE_CHUNK) {
        unsafe {
            WriteConsoleW(
                output,
                chunk.as_ptr().cast(),
                chunk.len() as u32,
                null_mut(),
                null(),
            );
        }
    }
}
