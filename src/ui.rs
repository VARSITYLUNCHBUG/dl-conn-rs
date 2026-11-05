//! Native Win32 UI: one dropdown. Every action is a command sent to the session, and everything it
//! shows comes from querying the session, so the UI holds no logic of its own.

use crate::console;
use crate::session::{self, Handle, Host, Output, Server, Status};
use crate::wfp::Engine;
use crate::wide;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::thread;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_OEM_3};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const WM_VIEW: u32 = WM_APP;
const ID_COMBO: usize = 1;
const CLIENT_WIDTH: i32 = 440;
const CLIENT_HEIGHT: i32 = 48;
const MARGIN: i32 = 12;
const DROPDOWN_HEIGHT: i32 = 420;
const NO_BLOCK_LABEL: &str = "No block - all servers";

static COMBO: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static MAIN: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static SESSION: OnceLock<Handle> = OnceLock::new();
static PENDING_VIEW: Mutex<Option<View>> = Mutex::new(None);
/// (code, label) pairs currently in the dropdown. Only the UI thread touches this.
static ITEMS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
/// The code the dropdown last showed, so closing it unchanged doesn't issue a command.
static SELECTED: Mutex<String> = Mutex::new(String::new());

/// What the dropdown and title bar should show.
#[derive(Debug, PartialEq)]
struct View {
    items: Vec<(String, String)>,
    selected: usize,
    title: String,
}

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
        unsafe { PostMessageW(MAIN.load(Ordering::Acquire), WM_CLOSE, 0, 0) };
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
        create_main(GetModuleHandleW(null()));
        let (changed, changes) = channel();
        let session = session::spawn(engine, UiHost { changed });
        console::init(session.clone());
        let _ = SESSION.set(session.clone());
        thread::spawn(move || view_loop(&session, &changes));
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            // Filtered here so the key works whichever control has focus. The dropdown would
            // otherwise treat it as type-ahead.
            if message.message == WM_KEYDOWN && message.wParam == usize::from(VK_OEM_3) {
                run_command("toggle console");
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
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
            *PENDING_VIEW.lock().unwrap() = Some(build_view(&servers, &status));
            unsafe { PostMessageW(MAIN.load(Ordering::Acquire), WM_VIEW, 0, 0) };
        }
    }
}

fn build_view(servers: &[Server], status: &Status) -> View {
    let mut items: Vec<(String, String)> = servers
        .iter()
        .map(|server| {
            let label = format!("{} - {}", server.desc, server.code);
            (server.code.clone(), label)
        })
        .collect();
    // A pick the config no longer lists stays visible, so the dropdown never misstates it.
    if !status.pick.is_empty() && status.pick_desc.is_none() {
        items.push((status.pick.clone(), status.pick.clone()));
    }
    items.sort_by_key(|(_, label)| (label.contains("(China)"), label.clone()));
    items.insert(0, (String::new(), NO_BLOCK_LABEL.to_string()));
    let selected = items
        .iter()
        .position(|(code, _)| *code == status.pick)
        .unwrap_or(0);
    View {
        items,
        selected,
        title: title(status),
    }
}

fn title(status: &Status) -> String {
    if let Some(error) = &status.error {
        return format!("dl-rs - {error}");
    }
    let offline = if status.offline { " (offline)" } else { "" };
    if status.pick.is_empty() {
        if !status.loaded && !status.offline {
            return "dl-rs - loading".to_string();
        }
        return format!("dl-rs - no block{offline}");
    }
    match &status.pick_desc {
        Some(desc) => format!("dl-rs - {desc} - {} blocked{offline}", status.blocked),
        None => format!("dl-rs - {} active{offline}", status.pick),
    }
}

fn scale(value: i32) -> i32 {
    value * unsafe { GetDpiForSystem() } as i32 / 96
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
    let Some(code) = code else {
        return;
    };
    {
        let mut selected = SELECTED.lock().unwrap();
        if *selected == code {
            return;
        }
        selected.clone_from(&code);
    }
    if code.is_empty() {
        run_command("clear pick");
    } else {
        run_command(&format!("set pick {code}"));
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
        if let Some((code, _)) = view.items.get(view.selected) {
            SELECTED.lock().unwrap().clone_from(code);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn server(code: &str, desc: &str) -> Server {
        Server {
            code: code.to_string(),
            desc: desc.to_string(),
            relays: 2,
        }
    }

    fn status() -> Status {
        Status {
            pick: String::new(),
            pick_desc: None,
            blocked: 0,
            sites_blocked: 0,
            loaded: true,
            offline: false,
            error: None,
        }
    }

    fn servers() -> Vec<Server> {
        vec![
            server("pvgm", "Alibaba Cloud Shanghai (China)"),
            server("jnb", "Johannesburg (South Africa)"),
            server("ams", "Amsterdam (Netherlands)"),
        ]
    }

    #[test]
    fn dropdown_lists_no_block_first_and_china_last() {
        let view = build_view(&servers(), &status());
        let codes: Vec<&str> = view.items.iter().map(|(code, _)| code.as_str()).collect();
        assert_eq!(codes, ["", "ams", "jnb", "pvgm"]);
        assert_eq!(view.items[0].1, NO_BLOCK_LABEL);
        assert_eq!(view.items[1].1, "Amsterdam (Netherlands) - ams");
        assert_eq!(view.selected, 0);
    }

    #[test]
    fn dropdown_selects_the_pick_and_the_title_reports_it() {
        let view = build_view(
            &servers(),
            &Status {
                pick: "jnb".to_string(),
                pick_desc: Some("Johannesburg (South Africa)".to_string()),
                blocked: 216,
                ..status()
            },
        );
        assert_eq!(view.items[view.selected].0, "jnb");
        assert_eq!(
            view.title,
            "dl-rs - Johannesburg (South Africa) - 216 blocked"
        );
    }

    #[test]
    fn a_pick_missing_from_the_list_stays_visible() {
        let view = build_view(
            &servers(),
            &Status {
                pick: "gone".to_string(),
                ..status()
            },
        );
        assert_eq!(view.items[view.selected].0, "gone");
        assert_eq!(view.title, "dl-rs - gone active");
    }

    #[test]
    fn titles_cover_every_state() {
        assert_eq!(title(&status()), "dl-rs - no block");
        assert_eq!(
            title(&Status {
                loaded: false,
                ..status()
            }),
            "dl-rs - loading"
        );
        assert_eq!(
            title(&Status {
                offline: true,
                ..status()
            }),
            "dl-rs - no block (offline)"
        );
        assert_eq!(
            title(&Status {
                error: Some("denied".to_string()),
                ..status()
            }),
            "dl-rs - denied"
        );
    }
}
