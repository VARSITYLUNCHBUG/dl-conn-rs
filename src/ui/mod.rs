//! Native Win32 UI: a searchable dropdown, two ping counters, and the F1, F2 and F3 windows. Every
//! action is a command sent to the session, and everything it shows comes from querying the session,
//! so the UI holds no logic of its own.

mod dropdown;
mod rows;
mod settings_window;
mod text_window;

use crate::console;
use crate::from_wide;
use crate::session::{self, Firewall, Handle, Host, Output, Server, Status};
use crate::wfp::Engine;
use crate::wide;
use rows::View;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::thread;
use text_window::Kind;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_DOWN, VK_ESCAPE, VK_F1, VK_F2, VK_F3, VK_NEXT, VK_OEM_3, VK_PRIOR, VK_RETURN,
    VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const WM_VIEW: u32 = WM_APP;
/// The session has finished what it does on exit, so the window can close.
const WM_EXIT: u32 = WM_APP + 1;
const ID_COMBO: usize = 1;
const ID_DIRECT: usize = 2;
const ID_ROUTE: usize = 3;
const ID_KEYS: usize = 4;
const CLIENT_WIDTH: i32 = 480;
const CLIENT_HEIGHT: i32 = 122;
const MARGIN: i32 = 12;
const COUNTER_TOP: i32 = 46;
const COUNTER_HEIGHT: i32 = 20;
const KEYS_TOP: i32 = 94;
const KEYS_TEXT: &str = "F1 Help    F2 Settings    F3 Log report    ` Console";

static MAIN: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static FONT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static SESSION: OnceLock<Handle> = OnceLock::new();
static PENDING_VIEW: Mutex<Option<(View, Vec<Server>, Status)>> = Mutex::new(None);

struct UiHost {
    changed: Sender<()>,
}

impl Host for UiHost {
    fn toggle_console(&self) -> bool {
        let visible = console::toggle();
        if !visible {
            unsafe { SetForegroundWindow(MAIN.load(Ordering::Acquire)) };
        }
        visible
    }

    fn quit(&self) {
        unsafe { PostMessageW(MAIN.load(Ordering::Acquire), WM_EXIT, 0, 0) };
    }

    fn changed(&self) {
        let _ = self.changed.send(());
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
        serve(engine);
    }
}

/// Opens the windows and runs until the main one closes.
unsafe fn serve(firewall: impl Firewall) {
    unsafe {
        create_main(GetModuleHandleW(null()));
        let (changed, changes) = channel();
        let session = session::spawn(firewall, UiHost { changed });
        console::init(session.clone());
        let _ = SESSION.set(session.clone());
        thread::spawn(move || view_loop(&session, &changes));
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if message.message == WM_KEYDOWN && key(&message) {
                continue;
            }
            if settings_window::owns(GetAncestor(message.hwnd, GA_ROOT))
                && settings_window::dialog_message(&message)
            {
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
            dropdown::key_done();
        }
    }
}

/// Filtered here so keys work whichever control has focus. The dropdown would otherwise treat them
/// as type-ahead or search text. Returns whether the key was used.
unsafe fn key(message: &MSG) -> bool {
    let code = message.wParam as u16;
    unsafe {
        let top = GetAncestor(message.hwnd, GA_ROOT);
        match code {
            VK_OEM_3 => run_command("toggle console"),
            VK_F1 => text_window::open(Kind::Guide),
            VK_F2 => settings_window::open(),
            VK_F3 => text_window::open(Kind::Report),
            VK_ESCAPE => {
                if text_window::owns(top) {
                    DestroyWindow(top);
                } else if settings_window::owns(top) {
                    settings_window::close();
                } else {
                    return dropdown::escape(MAIN.load(Ordering::Acquire));
                }
            }
            VK_RETURN => {
                if message.hwnd == dropdown::edit() || message.hwnd == dropdown::control() {
                    return dropdown::enter(MAIN.load(Ordering::Acquire));
                }
                return settings_window::owns(top) && settings_window::enter(message.hwnd);
            }
            VK_DOWN | VK_NEXT => {
                dropdown::arrow_pressed(true);
                return false;
            }
            VK_UP | VK_PRIOR => {
                dropdown::arrow_pressed(false);
                return false;
            }
            _ => return false,
        }
    }
    true
}

fn session() -> Option<Handle> {
    SESSION.get().cloned()
}

fn run_command(line: &str) {
    if let Some(session) = SESSION.get() {
        // The UI reacts to the "changed" signal rather than to individual replies.
        let _ = session.run(line);
    }
}

/// Refreshes the view whenever the session reports a change. It's a separate thread so the UI
/// thread never waits on the session.
fn view_loop(session: &Handle, changes: &Receiver<()>) {
    while changes.recv().is_ok() {
        while changes.try_recv().is_ok() {}
        let servers = session.query("list servers").recv();
        let status = session.query("show status").recv();
        if let (Ok(Ok(Output::Servers(servers))), Ok(Ok(Output::Status(status)))) =
            (servers, status)
        {
            let view = rows::build_view(&servers, &status);
            *PENDING_VIEW.lock().unwrap() = Some((view, servers, status));
            unsafe { PostMessageW(MAIN.load(Ordering::Acquire), WM_VIEW, 0, 0) };
        }
    }
}

fn scale(value: i32) -> i32 {
    value * unsafe { GetDpiForSystem() } as i32 / 96
}

/// Gives a control the same system font as everything else.
unsafe fn set_font(control: HWND) {
    unsafe {
        SendMessageW(
            control,
            WM_SETFONT,
            FONT.load(Ordering::Acquire) as WPARAM,
            1,
        );
    }
}

/// A window's text.
unsafe fn wide_text(window: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(window);
        let mut buffer = vec![0u16; usize::try_from(length).unwrap_or(0) + 1];
        GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
        from_wide(&buffer)
    }
}

unsafe fn label(window: HWND, module: HMODULE, id: usize, top: i32, text: &str) {
    unsafe {
        let handle = CreateWindowExW(
            0,
            w!("STATIC"),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE,
            scale(MARGIN),
            scale(top),
            scale(CLIENT_WIDTH - 2 * MARGIN),
            scale(COUNTER_HEIGHT),
            window,
            id as HMENU,
            module,
            null(),
        );
        set_font(handle);
    }
}

unsafe fn create_main(module: HMODULE) {
    unsafe {
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(main_proc);
        class.hInstance = module;
        class.lpszClassName = w!("dl-rs");
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.hbrBackground = (COLOR_BTNFACE + 1) as usize as HBRUSH;
        RegisterClassW(&class);
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
        let mut metrics: NONCLIENTMETRICSW = zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast(),
            0,
        );
        let font = CreateFontIndirectW(&metrics.lfMessageFont);
        FONT.store(font, Ordering::Release);
        let mut bold = metrics.lfMessageFont;
        bold.lfWeight = FW_BOLD as i32;
        let bold = CreateFontIndirectW(&bold);
        dropdown::create(
            window,
            module,
            font,
            bold,
            scale(MARGIN),
            scale(MARGIN),
            scale(CLIENT_WIDTH - 2 * MARGIN),
        );
        label(
            window,
            module,
            ID_DIRECT,
            COUNTER_TOP,
            "Direct: pick a server",
        );
        label(
            window,
            module,
            ID_ROUTE,
            COUNTER_TOP + COUNTER_HEIGHT,
            "ExitLag route: not running",
        );
        label(window, module, ID_KEYS, KEYS_TOP, KEYS_TEXT);
        ShowWindow(window, SW_SHOW);
        SetFocus(dropdown::control());
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
            WM_MEASUREITEM if wparam == ID_COMBO => {
                dropdown::measure(&mut *(lparam as *mut MEASUREITEMSTRUCT));
                1
            }
            WM_DRAWITEM if wparam == ID_COMBO => {
                dropdown::draw(&*(lparam as *const DRAWITEMSTRUCT));
                1
            }
            WM_COMMAND if wparam & 0xFFFF == ID_COMBO => {
                dropdown::notify(window, (wparam >> 16) as u32 & 0xFFFF);
                0
            }
            WM_VIEW => {
                apply_view(window);
                0
            }
            WM_CLOSE => {
                // Closing is a command, so a setting can act on it before the window goes.
                if SESSION.get().is_some() {
                    run_command("quit");
                } else {
                    DestroyWindow(window);
                }
                0
            }
            WM_EXIT => {
                DestroyWindow(window);
                0
            }
            WM_DESTROY => {
                // WFP filters are left in place unless clear_on_exit removed them: they last until
                // reboot.
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// Updates a control's text only when it changed, so counters don't flicker.
unsafe fn set_text(control: HWND, text: &str) {
    unsafe {
        if wide_text(control) != text {
            SetWindowTextW(control, wide(text).as_ptr());
        }
    }
}

unsafe fn apply_view(window: HWND) {
    unsafe {
        // The lock isn't held across SendMessage, which can re-enter main_proc.
        let pending = PENDING_VIEW.lock().unwrap().take();
        if let Some((view, servers, status)) = pending {
            dropdown::update(servers, status);
            set_text(window, &view.title);
            set_text(GetDlgItem(window, ID_DIRECT as i32), &view.direct);
            set_text(GetDlgItem(window, ID_ROUTE as i32), &view.route);
        }
        dropdown::refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdr::Blocks;

    /// Lets a script drive the real windows with no admin rights and no real filters.
    #[derive(Default)]
    struct FakeFirewall(Mutex<Option<String>>);

    impl Firewall for FakeFirewall {
        fn stamp(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn apply(&self, stamp: &str, _blocks: &Blocks) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(stamp.to_string());
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
        fn watch_drops(&self, _on: bool) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    #[ignore = "opens real windows; run by scripts/ui-test.ps1"]
    fn windows_with_a_fake_firewall() {
        unsafe { serve(FakeFirewall::default()) };
    }
}
