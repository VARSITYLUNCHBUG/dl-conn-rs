//! The F2 settings form. It reads `show config`, runs `set config` for each change, and displays
//! what the session says. Validation is the session's, so errors appear here unchanged.

use super::{MAIN, scale, session, set_font, wide_text};
use crate::session::{Output, Server};
use crate::settings::{Kind, Row};
use crate::wide;
use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::thread;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const WM_LOADED: u32 = WM_APP + 20;
const WM_APPLIED: u32 = WM_APP + 21;
const ID_DEFAULTS: i32 = 1;
const ID_CLOSE: i32 = 2;
const ID_STATUS: i32 = 3;
const ID_INTRO: i32 = 4;
const ID_FIRST_FIELD: i32 = 100;
const ID_FIRST_HINT: i32 = 200;

const WIDTH: i32 = 620;
const MARGIN: i32 = 12;
const INTRO_HEIGHT: i32 = 36;
const LABEL_WIDTH: i32 = 200;
const FIELD_HEIGHT: i32 = 24;
const HINT_HEIGHT: i32 = 32;
const ROW_GAP: i32 = 6;
const BUTTON_WIDTH: i32 = 90;
const BUTTON_HEIGHT: i32 = 28;
const STATUS_HEIGHT: i32 = 36;
const LIST_HEIGHT: i32 = 300;

const INTRO: &str = "These settings control how dl-rs blocks, measures and logs. None of them \
                     lowers ping beyond choosing a route. Changes apply and save straight away.";

static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
/// The form is being filled in, so its controls' notifications aren't changes.
static FILLING: AtomicBool = AtomicBool::new(false);
/// The form is closing, so a field losing focus isn't a change.
static CLOSING: AtomicBool = AtomicBool::new(false);
static FIELDS: Mutex<Vec<Field>> = Mutex::new(Vec::new());
static LOADED: Mutex<Option<(Vec<Row>, Vec<Server>)>> = Mutex::new(None);
static APPLIED: Mutex<Option<String>> = Mutex::new(None);

/// One setting on the form. Only the UI thread touches this.
struct Field {
    key: &'static str,
    kind: Kind,
    /// As a number, because a window handle can't be stored in a static.
    control: usize,
    /// What the session says the value is.
    value: String,
    /// What each dropdown line stands for.
    choices: Vec<String>,
}

fn window() -> HWND {
    WINDOW.load(Ordering::Acquire)
}

pub fn owns(top: HWND) -> bool {
    !top.is_null() && top == window()
}

/// Shows the form, or brings it forward. Only runs on a key press, so it's allowed to take focus.
pub fn open() {
    unsafe {
        let existing = window();
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let created = create();
        WINDOW.store(created, Ordering::Release);
        CLOSING.store(false, Ordering::Relaxed);
        ShowWindow(created, SW_SHOW);
        SetForegroundWindow(created);
        load();
    }
}

/// Fetches the settings and servers on a thread, then posts them back.
fn load() {
    let Some(session) = session() else {
        return;
    };
    let target = window() as usize;
    thread::spawn(move || {
        let config = session.query("show config").recv();
        let servers = session.query("list servers").recv();
        if let (Ok(Ok(Output::Config(rows))), Ok(Ok(Output::Servers(servers)))) = (config, servers)
        {
            *LOADED.lock().unwrap() = Some((rows, servers));
            unsafe { PostMessageW(target as HWND, WM_LOADED, 0, 0) };
        }
    });
}

/// Runs a command on a thread and shows what came back.
fn run(line: String, success: Option<String>) {
    let Some(session) = session() else {
        return;
    };
    let target = window() as usize;
    thread::spawn(move || {
        let message = match session.run(&line).recv() {
            Ok(Ok(output)) => {
                success.unwrap_or_else(|| format!("Saved: {}", output.render().join(" ")))
            }
            Ok(Err(error)) => format!("Not changed: {error}"),
            Err(_) => "The session has closed.".to_string(),
        };
        *APPLIED.lock().unwrap() = Some(message);
        unsafe { PostMessageW(target as HWND, WM_APPLIED, 0, 0) };
    });
}

unsafe fn register(module: HMODULE) {
    unsafe {
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(form_proc);
        class.hInstance = module;
        class.lpszClassName = w!("dl-rs-settings");
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.hbrBackground = (COLOR_BTNFACE + 1) as usize as HBRUSH;
        // Registering twice fails harmlessly.
        RegisterClassW(&class);
    }
}

fn row_height() -> i32 {
    FIELD_HEIGHT + HINT_HEIGHT + ROW_GAP
}

fn row_top(index: i32) -> i32 {
    MARGIN + INTRO_HEIGHT + MARGIN + index * row_height()
}

unsafe fn child(
    parent: HWND,
    class: *const u16,
    text: *const u16,
    style: u32,
    id: i32,
    place: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        let handle = CreateWindowExW(
            0,
            class,
            text,
            WS_CHILD | WS_VISIBLE | style,
            scale(place.0),
            scale(place.1),
            scale(place.2),
            scale(place.3),
            parent,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        set_font(handle);
        handle
    }
}

unsafe fn create() -> HWND {
    let rows = crate::settings::Settings::default().rows().len() as i32;
    let bottom = row_top(rows) + STATUS_HEIGHT + BUTTON_HEIGHT + MARGIN * 2;
    unsafe {
        let module = GetModuleHandleW(null());
        register(module);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scale(WIDTH),
            bottom: scale(bottom),
        };
        AdjustWindowRect(&mut frame, style, 0);
        let form = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("dl-rs-settings"),
            w!("dl-rs settings"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            frame.right - frame.left,
            frame.bottom - frame.top,
            MAIN.load(Ordering::Acquire),
            null_mut(),
            module,
            null(),
        );
        let inner = WIDTH - 2 * MARGIN;
        child(
            form,
            w!("STATIC"),
            wide(INTRO).as_ptr(),
            0,
            ID_INTRO,
            (MARGIN, MARGIN, inner, INTRO_HEIGHT),
        );
        let top = row_top(rows);
        child(
            form,
            w!("STATIC"),
            null(),
            0,
            ID_STATUS,
            (MARGIN, top, inner, STATUS_HEIGHT),
        );
        let buttons = top + STATUS_HEIGHT + MARGIN;
        child(
            form,
            w!("BUTTON"),
            w!("Defaults"),
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            ID_DEFAULTS,
            (MARGIN, buttons, BUTTON_WIDTH, BUTTON_HEIGHT),
        );
        child(
            form,
            w!("BUTTON"),
            w!("Close"),
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            ID_CLOSE,
            (
                WIDTH - MARGIN - BUTTON_WIDTH,
                buttons,
                BUTTON_WIDTH,
                BUTTON_HEIGHT,
            ),
        );
        form
    }
}

/// Builds the rows the first time, then only updates their values.
unsafe fn show(rows: &[Row], servers: &[Server]) {
    let form = window();
    FILLING.store(true, Ordering::Relaxed);
    let first_time = FIELDS.lock().unwrap().is_empty();
    unsafe {
        if first_time {
            let focus = build_rows(form, rows);
            SetFocus(focus);
        }
        for (index, row) in rows.iter().enumerate() {
            fill_field(index, row, servers);
        }
    }
    FILLING.store(false, Ordering::Relaxed);
}

unsafe fn build_rows(form: HWND, rows: &[Row]) -> HWND {
    let mut fields = Vec::new();
    let mut first: HWND = null_mut();
    for (index, row) in rows.iter().enumerate() {
        let number = index as i32;
        let top = row_top(number);
        let field_left = MARGIN + LABEL_WIDTH;
        let field_width = WIDTH - MARGIN - field_left;
        unsafe {
            child(
                form,
                w!("STATIC"),
                wide(row.title).as_ptr(),
                0,
                ID_FIRST_HINT + 100 + number,
                (MARGIN, top + 4, LABEL_WIDTH - MARGIN, FIELD_HEIGHT),
            );
            let id = ID_FIRST_FIELD + number;
            let control = match row.kind {
                Kind::Toggle | Kind::Server => child(
                    form,
                    w!("COMBOBOX"),
                    null(),
                    WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
                    id,
                    (field_left, top, field_width, LIST_HEIGHT),
                ),
                Kind::Number | Kind::Path => {
                    let handle = CreateWindowExW(
                        WS_EX_CLIENTEDGE,
                        w!("EDIT"),
                        null(),
                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                        scale(field_left),
                        scale(top),
                        scale(field_width),
                        scale(FIELD_HEIGHT),
                        form,
                        id as usize as HMENU,
                        GetModuleHandleW(null()),
                        null(),
                    );
                    set_font(handle);
                    handle
                }
            };
            if first.is_null() {
                first = control;
            }
            let hint = format!("{} Allowed: {}.", row.about, row.allowed);
            child(
                form,
                w!("STATIC"),
                wide(&hint).as_ptr(),
                0,
                ID_FIRST_HINT + number,
                (
                    MARGIN,
                    top + FIELD_HEIGHT + 2,
                    WIDTH - 2 * MARGIN,
                    HINT_HEIGHT,
                ),
            );
            fields.push(Field {
                key: row.key,
                kind: row.kind,
                control: control as usize,
                value: row.value.clone(),
                choices: Vec::new(),
            });
        }
    }
    *FIELDS.lock().unwrap() = fields;
    first
}

/// The lines a dropdown shows and the value each one stands for.
fn choices(kind: Kind, value: &str, servers: &[Server]) -> Vec<(String, String)> {
    let mut lines = match kind {
        Kind::Toggle => vec![
            ("on".to_string(), "on".to_string()),
            ("off".to_string(), "off".to_string()),
        ],
        Kind::Server => std::iter::once(("none".to_string(), "none - strict".to_string()))
            .chain(servers.iter().map(|server| {
                (
                    server.code.clone(),
                    format!("{} - {}", server.code, server.desc),
                )
            }))
            .collect(),
        Kind::Number | Kind::Path => Vec::new(),
    };
    // A saved backup the relay list no longer has stays visible rather than reading as none.
    if matches!(kind, Kind::Server) && !lines.iter().any(|(code, _)| code == value) {
        lines.push((value.to_string(), value.to_string()));
    }
    lines
}

unsafe fn fill_field(index: usize, row: &Row, servers: &[Server]) {
    let (control, kind) = {
        let mut fields = FIELDS.lock().unwrap();
        let Some(field) = fields.get_mut(index) else {
            return;
        };
        field.value.clone_from(&row.value);
        (field.control as HWND, field.kind)
    };
    unsafe {
        match kind {
            Kind::Number | Kind::Path => {
                SetWindowTextW(control, wide(&row.value).as_ptr());
            }
            Kind::Toggle | Kind::Server => {
                let lines = choices(kind, &row.value, servers);
                SendMessageW(control, CB_RESETCONTENT, 0, 0);
                for (_, label) in &lines {
                    SendMessageW(control, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
                }
                let at = lines.iter().position(|(value, _)| *value == row.value);
                SendMessageW(control, CB_SETCURSEL, at.unwrap_or(0), 0);
                if let Some(field) = FIELDS.lock().unwrap().get_mut(index) {
                    field.choices = lines.into_iter().map(|(value, _)| value).collect();
                }
            }
        }
    }
}

/// Runs `set config` if a field differs from what the session has.
unsafe fn apply(index: usize) {
    if FILLING.load(Ordering::Relaxed) || CLOSING.load(Ordering::Relaxed) {
        return;
    }
    let change = {
        let fields = FIELDS.lock().unwrap();
        fields.get(index).map(|field| {
            (
                field.key,
                field.kind,
                field.control as HWND,
                field.value.clone(),
                field.choices.clone(),
            )
        })
    };
    let Some((key, kind, control, current, choices)) = change else {
        return;
    };
    let value = unsafe {
        match kind {
            Kind::Number | Kind::Path => wide_text(control).trim().to_string(),
            Kind::Toggle | Kind::Server => {
                let at = SendMessageW(control, CB_GETCURSEL, 0, 0);
                let Some(value) = usize::try_from(at).ok().and_then(|at| choices.get(at)) else {
                    return;
                };
                value.clone()
            }
        }
    };
    if value != current {
        run(format!("set config {key} {value}"), None);
    }
}

fn field_index(control: HWND) -> Option<usize> {
    FIELDS
        .lock()
        .unwrap()
        .iter()
        .position(|field| field.control == control as usize)
}

/// Enter in an edit box: applies it. Returns whether the key was used.
pub unsafe fn enter(control: HWND) -> bool {
    let Some(index) = field_index(control) else {
        return false;
    };
    unsafe { apply(index) };
    true
}

/// Esc: closes the form, dropping a half-typed value.
pub unsafe fn close() {
    CLOSING.store(true, Ordering::Relaxed);
    unsafe { DestroyWindow(window()) };
}

/// Tab and Shift+Tab between fields. Returns whether the message was used.
pub unsafe fn dialog_message(message: &MSG) -> bool {
    let form = window();
    unsafe { !form.is_null() && IsDialogMessageW(form, message) != 0 }
}

unsafe extern "system" fn form_proc(
    form: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as i32;
                let code = (wparam >> 16) as u32 & 0xFFFF;
                match id {
                    ID_DEFAULTS if code == BN_CLICKED => {
                        run(
                            "clear config".to_string(),
                            Some("All settings are back to their defaults.".to_string()),
                        );
                    }
                    ID_CLOSE if code == BN_CLICKED => {
                        // Moving focus applies a value still being typed.
                        SetFocus(form);
                        close();
                    }
                    _ => {
                        if let Ok(index) = usize::try_from(id - ID_FIRST_FIELD)
                            && index < FIELDS.lock().unwrap().len()
                            && (code == EN_KILLFOCUS || code == CBN_SELCHANGE)
                        {
                            apply(index);
                        }
                    }
                }
                0
            }
            WM_CLOSE => {
                SetFocus(form);
                close();
                0
            }
            WM_CTLCOLORSTATIC => {
                let device = wparam as HDC;
                SetBkColor(device, GetSysColor(COLOR_BTNFACE));
                let control = lparam as HWND;
                let id = GetDlgCtrlID(control);
                let hint = (ID_FIRST_HINT..ID_FIRST_HINT + 100).contains(&id);
                let color = if hint { COLOR_GRAYTEXT } else { COLOR_BTNTEXT };
                SetTextColor(device, GetSysColor(color));
                GetSysColorBrush(COLOR_BTNFACE) as LRESULT
            }
            WM_LOADED => {
                let loaded = LOADED.lock().unwrap().take();
                if let Some((rows, servers)) = loaded {
                    show(&rows, &servers);
                }
                0
            }
            WM_APPLIED => {
                let text = APPLIED.lock().unwrap().take();
                if let Some(text) = text {
                    SetWindowTextW(GetDlgItem(form, ID_STATUS), wide(&text).as_ptr());
                }
                // Puts a rejected value back, and picks up what a change really did.
                load();
                0
            }
            WM_DESTROY => {
                WINDOW.store(null_mut(), Ordering::Release);
                FIELDS.lock().unwrap().clear();
                0
            }
            _ => DefWindowProcW(form, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn servers() -> Vec<Server> {
        vec![Server {
            code: "maa2".to_string(),
            desc: "Chennai".to_string(),
            relays: 2,
        }]
    }

    #[test]
    fn a_backup_dropdown_offers_none_and_every_server() {
        let lines = choices(Kind::Server, "none", &servers());
        assert_eq!(lines[0].0, "none");
        assert_eq!(lines[1], ("maa2".to_string(), "maa2 - Chennai".to_string()));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn a_saved_backup_missing_from_the_list_stays_visible() {
        let lines = choices(Kind::Server, "gone", &servers());
        assert_eq!(lines.last().unwrap().0, "gone");
    }

    #[test]
    fn a_toggle_offers_on_and_off() {
        let lines = choices(Kind::Toggle, "on", &[]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "on");
    }
}
