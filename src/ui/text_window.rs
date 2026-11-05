//! The read-only text windows: F1 help and the F3 log report. Text comes from a command, fetched on
//! a short-lived thread so the UI never waits on the session.

use super::{MAIN, scale, session, set_font};
use crate::session::Output;
use crate::wide;
use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::thread;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_SETLIMITTEXT, EM_SETMARGINS, EM_SETSEL};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const WM_TEXT: u32 = WM_APP + 10;
const ID_TEXT: i32 = 1;
const ID_RERUN: i32 = 2;
const WIDTH: i32 = 680;
const HEIGHT: i32 = 680;
const BUTTON_ROW: i32 = 44;
const BUTTON_WIDTH: i32 = 90;
const BUTTON_HEIGHT: i32 = 28;
const GAP: i32 = 8;
const TEXT_MARGIN: i32 = 8;
const FONT_POINTS: i32 = 10;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Guide = 0,
    Report = 1,
}

struct Page {
    command: &'static str,
    title: &'static str,
    waiting: &'static str,
    can_rerun: bool,
}

const PAGES: [Page; 2] = [
    Page {
        command: "show guide",
        title: "dl-rs help",
        waiting: "Loading...",
        can_rerun: false,
    },
    Page {
        command: "analyze log",
        title: "dl-rs log report",
        waiting: "Reading the game's log...",
        can_rerun: true,
    },
];

static WINDOWS: [AtomicPtr<c_void>; 2] = [AtomicPtr::new(null_mut()), AtomicPtr::new(null_mut())];
static PENDING: [Mutex<Option<String>>; 2] = [Mutex::new(None), Mutex::new(None)];
static FONT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

fn kind_of(window: HWND) -> Option<Kind> {
    if window == WINDOWS[Kind::Guide as usize].load(Ordering::Acquire) {
        Some(Kind::Guide)
    } else if window == WINDOWS[Kind::Report as usize].load(Ordering::Acquire) {
        Some(Kind::Report)
    } else {
        None
    }
}

/// Whether this top-level window is one of ours.
pub fn owns(window: HWND) -> bool {
    !window.is_null() && kind_of(window).is_some()
}

/// Shows the window, or brings it forward, and fetches its text. Only runs on a key press, so it's
/// allowed to take focus.
pub fn open(kind: Kind) {
    let existing = WINDOWS[kind as usize].load(Ordering::Acquire);
    unsafe {
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            if kind == Kind::Report {
                fetch(kind, existing);
            }
            return;
        }
        let window = create(kind);
        WINDOWS[kind as usize].store(window, Ordering::Release);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        fetch(kind, window);
    }
}

unsafe fn fetch(kind: Kind, window: HWND) {
    let page = &PAGES[kind as usize];
    unsafe {
        let text = GetDlgItem(window, ID_TEXT);
        SetWindowTextW(text, wide(page.waiting).as_ptr());
    }
    let Some(session) = session() else {
        return;
    };
    let window = window as usize;
    thread::spawn(move || {
        let text = match session.query(page.command).recv() {
            Ok(Ok(output)) => render(&output),
            Ok(Err(error)) => error,
            Err(_) => "The session has closed.".to_string(),
        };
        *PENDING[kind as usize].lock().unwrap() = Some(text);
        unsafe { PostMessageW(window as HWND, WM_TEXT, 0, 0) };
    });
}

fn render(output: &Output) -> String {
    output.render().join("\r\n")
}

fn monospace() -> HFONT {
    let existing = FONT.load(Ordering::Acquire);
    if !existing.is_null() {
        return existing;
    }
    let dpi = unsafe { GetDpiForSystem() } as i32;
    let font = unsafe {
        CreateFontW(
            -(FONT_POINTS * dpi / 72),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            (FIXED_PITCH | FF_MODERN) as u32,
            w!("Consolas"),
        )
    };
    FONT.store(font, Ordering::Release);
    font
}

unsafe fn register(module: HMODULE) {
    unsafe {
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(text_proc);
        class.hInstance = module;
        class.lpszClassName = w!("dl-rs-text");
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.hbrBackground = (COLOR_BTNFACE + 1) as usize as HBRUSH;
        // Registering twice fails harmlessly.
        RegisterClassW(&class);
    }
}

unsafe fn create(kind: Kind) -> HWND {
    let page = &PAGES[kind as usize];
    unsafe {
        let module = GetModuleHandleW(null());
        register(module);
        let window = CreateWindowExW(
            0,
            w!("dl-rs-text"),
            wide(page.title).as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(WIDTH),
            scale(HEIGHT),
            MAIN.load(Ordering::Acquire),
            null_mut(),
            module,
            null(),
        );
        let style = WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | WS_TABSTOP
            | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32;
        let text = CreateWindowExW(
            0,
            w!("EDIT"),
            null(),
            style,
            0,
            0,
            0,
            0,
            window,
            ID_TEXT as usize as HMENU,
            module,
            null(),
        );
        SendMessageW(text, WM_SETFONT, monospace() as WPARAM, 0);
        SendMessageW(text, EM_SETLIMITTEXT, 0, 0);
        let margin = scale(TEXT_MARGIN) as isize;
        SendMessageW(
            text,
            EM_SETMARGINS,
            (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
            margin | (margin << 16),
        );
        if page.can_rerun {
            let button = CreateWindowExW(
                0,
                w!("BUTTON"),
                w!("Re-run"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                0,
                0,
                0,
                window,
                ID_RERUN as usize as HMENU,
                module,
                null(),
            );
            set_font(button);
        }
        let mut client: RECT = zeroed();
        GetClientRect(window, &mut client);
        layout(window, client.right, client.bottom);
        SetFocus(text);
        window
    }
}

unsafe fn layout(window: HWND, width: i32, height: i32) {
    unsafe {
        let button = GetDlgItem(window, ID_RERUN);
        let row = if button.is_null() {
            0
        } else {
            scale(BUTTON_ROW)
        };
        MoveWindow(
            GetDlgItem(window, ID_TEXT),
            0,
            0,
            width,
            (height - row).max(0),
            1,
        );
        if !button.is_null() {
            let gap = scale(GAP);
            MoveWindow(
                button,
                width - scale(BUTTON_WIDTH) - gap,
                height - row + gap,
                scale(BUTTON_WIDTH),
                scale(BUTTON_HEIGHT),
                1,
            );
        }
    }
}

unsafe extern "system" fn text_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_SIZE => {
                let width = (lparam & 0xFFFF) as i32;
                let height = ((lparam >> 16) & 0xFFFF) as i32;
                layout(window, width, height);
                0
            }
            WM_SETFOCUS => {
                SetFocus(GetDlgItem(window, ID_TEXT));
                0
            }
            WM_CTLCOLORSTATIC => {
                // A read-only edit would otherwise be grey, which is harder to read.
                let device = wparam as HDC;
                SetBkColor(device, GetSysColor(COLOR_WINDOW));
                SetTextColor(device, GetSysColor(COLOR_WINDOWTEXT));
                GetSysColorBrush(COLOR_WINDOW) as LRESULT
            }
            WM_COMMAND if (wparam & 0xFFFF) as i32 == ID_RERUN => {
                if let Some(kind) = kind_of(window) {
                    fetch(kind, window);
                }
                0
            }
            WM_TEXT => {
                if let Some(kind) = kind_of(window)
                    && let Some(text) = PENDING[kind as usize].lock().unwrap().take()
                {
                    let edit = GetDlgItem(window, ID_TEXT);
                    SetWindowTextW(edit, wide(&text).as_ptr());
                    // Nothing selected, and the top of the text in view.
                    SendMessageW(edit, EM_SETSEL, 0, 0);
                }
                0
            }
            WM_DESTROY => {
                if let Some(kind) = kind_of(window) {
                    WINDOWS[kind as usize].store(null_mut(), Ordering::Release);
                }
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}
