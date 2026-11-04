//! Native Win32 UI: one dropdown, plus a hidden console toggled with the backtick key.

use crate::wfp::Engine;
use crate::wide;
use crate::worker::{self, Cmd};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Controls::{EM_REPLACESEL, EM_SCROLLCARET, EM_SETLIMITTEXT, EM_SETSEL};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_OEM_3};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::PCWSTR;
use windows_sys::w;

const WM_VIEW: u32 = WM_APP;
const WM_LOG: u32 = WM_APP + 1;
const ID_COMBO: usize = 1;
const LOG_CAPACITY: usize = 1 << 20;
const LOG_TRIM_AT: i32 = 900_000;
const CONSOLE_TEXT: COLORREF = 0x00C8_E6C8;
const CLIENT_WIDTH: i32 = 440;
const CLIENT_HEIGHT: i32 = 48;
const MARGIN: i32 = 12;
const DROPDOWN_HEIGHT: i32 = 420;

static COMBO: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static CONSOLE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static CONSOLE_EDIT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static MAIN: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
static PENDING_VIEW: Mutex<Option<View>> = Mutex::new(None);
static PENDING_LOG: Mutex<String> = Mutex::new(String::new());
/// (code, label) pairs currently in the dropdown. Only the UI thread touches this.
static ITEMS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// What the dropdown and title bar should show.
pub struct View {
    pub items: Vec<(String, String)>,
    pub selected: usize,
    pub title: String,
}

/// Hands a new view to the UI thread. Only the latest one matters, so it overwrites.
pub fn publish(view: View) {
    *PENDING_VIEW.lock().unwrap() = Some(view);
    unsafe { PostMessageW(MAIN.load(Ordering::Acquire), WM_VIEW, 0, 0) };
}

/// Queues a timestamped line for the console, which keeps it even while hidden.
pub fn log_line(line: String) {
    let time = unsafe {
        let mut time: SYSTEMTIME = zeroed();
        GetLocalTime(&mut time);
        time
    };
    let stamped = format!(
        "{:02}:{:02}:{:02}.{:03} {line}\r\n",
        time.wHour, time.wMinute, time.wSecond, time.wMilliseconds
    );
    PENDING_LOG.lock().unwrap().push_str(&stamped);
    let console = CONSOLE.load(Ordering::Acquire);
    if !console.is_null() {
        unsafe { PostMessageW(console, WM_LOG, 0, 0) };
    }
}

pub fn run() {
    unsafe {
        // Two copies would fight over the same filters, so the second just focuses the first.
        CreateMutexW(null(), 0, w!("Local\\dl-rs"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let existing = FindWindowW(w!("dl-rs"), null());
            if !existing.is_null() {
                ShowWindow(existing, SW_RESTORE);
                SetForegroundWindow(existing);
            }
            return;
        }
        let engine = match Engine::open() {
            Ok(engine) => engine,
            Err(error) => {
                let text = wide(&format!("Windows Filtering Platform: {error}"));
                MessageBoxW(null_mut(), text.as_ptr(), w!("dl-rs"), MB_ICONERROR);
                return;
            }
        };
        let module = GetModuleHandleW(null());
        register_class(module, w!("dl-rs"), main_proc);
        register_class(module, w!("dl-rs-console"), console_proc);
        create_console(module);
        create_main(module);
        let _ = TX.set(worker::spawn(engine));
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            // Filtered here so the key works whichever control has focus. The dropdown would
            // otherwise treat it as type-ahead.
            if message.message == WM_KEYDOWN && message.wParam == usize::from(VK_OEM_3) {
                toggle_console();
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

type WindowProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

unsafe fn register_class(module: HMODULE, name: PCWSTR, proc: WindowProc) {
    unsafe {
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(proc);
        class.hInstance = module;
        class.lpszClassName = name;
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.hbrBackground = (COLOR_BTNFACE + 1) as usize as HBRUSH;
        RegisterClassW(&class);
    }
}

fn scale(value: i32) -> i32 {
    value * unsafe { GetDpiForSystem() } as i32 / 96
}

unsafe fn create_main(module: HMODULE) {
    unsafe {
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scale(CLIENT_WIDTH),
            bottom: scale(CLIENT_HEIGHT),
        };
        AdjustWindowRect(&mut frame, style, 0);
        let window = CreateWindowExW(
            0,
            w!("dl-rs"),
            w!("dl-rs - loading"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            frame.right - frame.left,
            frame.bottom - frame.top,
            null_mut(),
            null_mut(),
            module,
            null(),
        );
        MAIN.store(window, Ordering::Release);
        let combo = CreateWindowExW(
            0,
            w!("COMBOBOX"),
            null(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
            scale(MARGIN),
            scale(MARGIN),
            scale(CLIENT_WIDTH - 2 * MARGIN),
            scale(DROPDOWN_HEIGHT),
            window,
            ID_COMBO as HMENU,
            module,
            null(),
        );
        COMBO.store(combo, Ordering::Release);
        let mut metrics: NONCLIENTMETRICSW = zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast(),
            0,
        );
        let font = CreateFontIndirectW(&metrics.lfMessageFont);
        SendMessageW(combo, WM_SETFONT, font as WPARAM, 0);
        ShowWindow(window, SW_SHOW);
        SetFocus(combo);
    }
}

unsafe fn create_console(module: HMODULE) {
    unsafe {
        let window = CreateWindowExW(
            0,
            w!("dl-rs-console"),
            w!("dl-rs console"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(860),
            scale(520),
            null_mut(),
            null_mut(),
            module,
            null(),
        );
        let edit = CreateWindowExW(
            0,
            w!("EDIT"),
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
            0,
            0,
            0,
            0,
            window,
            null_mut(),
            module,
            null(),
        );
        let mut face: LOGFONTW = zeroed();
        face.lfHeight = -scale(14);
        let name = wide("Consolas");
        face.lfFaceName[..name.len()].copy_from_slice(&name);
        SendMessageW(edit, WM_SETFONT, CreateFontIndirectW(&face) as WPARAM, 0);
        SendMessageW(edit, EM_SETLIMITTEXT, LOG_CAPACITY, 0);
        CONSOLE_EDIT.store(edit, Ordering::Release);
        CONSOLE.store(window, Ordering::Release);
    }
}

fn toggle_console() {
    let console = CONSOLE.load(Ordering::Acquire);
    let show = unsafe { IsWindowVisible(console) } == 0;
    unsafe {
        ShowWindow(console, if show { SW_SHOW } else { SW_HIDE });
        if show {
            SetForegroundWindow(console);
        }
    }
    send(Cmd::Console(show));
}

fn send(cmd: Cmd) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(cmd);
    }
}

unsafe extern "system" fn main_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_COMMAND if wparam & 0xFFFF == ID_COMBO => {
                let code = (wparam >> 16) as u32 & 0xFFFF;
                let combo = COMBO.load(Ordering::Acquire);
                let open = SendMessageW(combo, CB_GETDROPPEDSTATE, 0, 0) != 0;
                // Commit on close, not while arrowing through an open list.
                if code == CBN_CLOSEUP || (code == CBN_SELCHANGE && !open) {
                    send_selection();
                }
                if code == CBN_CLOSEUP {
                    PostMessageW(window, WM_VIEW, 0, 0);
                }
                0
            }
            WM_VIEW => {
                apply_view(window);
                0
            }
            WM_DESTROY => {
                // WFP filters are deliberately left in place: they last until reboot.
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

unsafe fn send_selection() {
    let combo = COMBO.load(Ordering::Acquire);
    let index = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) };
    let code = usize::try_from(index)
        .ok()
        .and_then(|i| ITEMS.lock().unwrap().get(i).map(|(code, _)| code.clone()));
    if let Some(code) = code {
        send(Cmd::Pick(code));
    }
}

unsafe fn apply_view(window: HWND) {
    let combo = COMBO.load(Ordering::Acquire);
    unsafe {
        // Rebuilding would snap an open list shut; CBN_CLOSEUP re-posts WM_VIEW.
        if SendMessageW(combo, CB_GETDROPPEDSTATE, 0, 0) != 0 {
            return;
        }
        let Some(view) = PENDING_VIEW.lock().unwrap().take() else {
            return;
        };
        // The lock isn't held across SendMessage, which can re-enter main_proc.
        let changed = *ITEMS.lock().unwrap() != view.items;
        if changed {
            SendMessageW(combo, CB_RESETCONTENT, 0, 0);
            for (_, label) in &view.items {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
            }
            *ITEMS.lock().unwrap() = view.items;
        }
        SendMessageW(combo, CB_SETCURSEL, view.selected, 0);
        SetWindowTextW(window, wide(&view.title).as_ptr());
    }
}

unsafe extern "system" fn console_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_LOG => {
                flush_log();
                0
            }
            WM_SIZE => {
                let (width, height) = ((lparam & 0xFFFF) as i32, ((lparam >> 16) & 0xFFFF) as i32);
                MoveWindow(CONSOLE_EDIT.load(Ordering::Acquire), 0, 0, width, height, 1);
                0
            }
            // Read-only edit controls paint through this message.
            WM_CTLCOLORSTATIC => {
                let device = wparam as HDC;
                SetTextColor(device, CONSOLE_TEXT);
                SetBkColor(device, 0);
                GetStockObject(BLACK_BRUSH) as LRESULT
            }
            // Closing only hides it, since the app has to keep running.
            WM_CLOSE => {
                toggle_console();
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

unsafe fn flush_log() {
    let text = std::mem::take(&mut *PENDING_LOG.lock().unwrap());
    if text.is_empty() {
        return;
    }
    let edit = CONSOLE_EDIT.load(Ordering::Acquire);
    unsafe {
        if GetWindowTextLengthW(edit) > LOG_TRIM_AT {
            SendMessageW(edit, EM_SETSEL, 0, (LOG_TRIM_AT / 2) as LPARAM);
            SendMessageW(edit, EM_REPLACESEL, 0, w!("") as LPARAM);
        }
        let end = GetWindowTextLengthW(edit) as usize;
        SendMessageW(edit, EM_SETSEL, end, end as LPARAM);
        SendMessageW(edit, EM_REPLACESEL, 0, wide(&text).as_ptr() as LPARAM);
        SendMessageW(edit, EM_SCROLLCARET, 0, 0);
    }
}
