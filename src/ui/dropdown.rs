//! The searchable server dropdown. Typing narrows the list, and rows are grouped under region
//! headers that can't be picked. All the deciding is in `rows`; this is the Win32 wrapper.

use super::rows::{self, Row};
use super::{ID_COMBO, WM_VIEW, run_command, scale, wide_text};
use crate::session::{Server, Status};
use crate::wide;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicIsize, AtomicPtr, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{
    COMBOBOXINFO, DRAWITEMSTRUCT, GetComboBoxInfo, MEASUREITEMSTRUCT, ODS_SELECTED,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

const DROPDOWN_HEIGHT: i32 = 420;
const ROW_HEIGHT: i32 = 20;
const INDENT: i32 = 12;
/// A search result list is only this tall at most, and scrolls past it.
const MAX_SEARCH_ROWS: usize = 20;
const NO_SELECTION: usize = usize::MAX;

static COMBO: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static EDIT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static HEADER_FONT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static ITEM_FONT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// The rows in the control now. Only the UI thread touches this.
static ROWS: Mutex<Vec<Row>> = Mutex::new(Vec::new());
/// The latest relay list and status, for rebuilding the list as someone types.
static DATA: Mutex<Option<(Vec<Server>, Status)>> = Mutex::new(None);
/// The code the dropdown last showed, so closing it unchanged doesn't issue a command.
static SELECTED: Mutex<String> = Mutex::new(String::new());
/// The list holds search results, so it must not be replaced until the search ends.
static SEARCHING: AtomicBool = AtomicBool::new(false);
/// This code is changing the control, so its notifications should be ignored.
static REBUILDING: AtomicBool = AtomicBool::new(false);
/// The next close shouldn't pick what's highlighted.
static CANCELLED: AtomicBool = AtomicBool::new(false);
/// The selection is moving because of an arrow key, not a click, and which way: 1 is down the list.
static ARROW: AtomicI8 = AtomicI8::new(0);
static LAST_INDEX: AtomicIsize = AtomicIsize::new(0);

fn combo() -> HWND {
    COMBO.load(Ordering::Acquire)
}

pub fn edit() -> HWND {
    EDIT.load(Ordering::Acquire)
}

pub fn control() -> HWND {
    combo()
}

pub unsafe fn create(
    window: HWND,
    module: HMODULE,
    font: HFONT,
    bold: HFONT,
    x: i32,
    y: i32,
    width: i32,
) {
    HEADER_FONT.store(bold, Ordering::Release);
    ITEM_FONT.store(font, Ordering::Release);
    unsafe {
        let style = WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | WS_TABSTOP
            | (CBS_DROPDOWN | CBS_AUTOHSCROLL | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS) as u32;
        let handle = CreateWindowExW(
            0,
            w!("COMBOBOX"),
            null(),
            style,
            x,
            y,
            width,
            scale(DROPDOWN_HEIGHT),
            window,
            ID_COMBO as HMENU,
            module,
            null(),
        );
        COMBO.store(handle, Ordering::Release);
        EDIT.store(
            FindWindowExW(handle, null_mut(), w!("Edit"), null()),
            Ordering::Release,
        );
        SendMessageW(handle, WM_SETFONT, font as WPARAM, 0);
    }
}

pub fn measure(item: &mut MEASUREITEMSTRUCT) {
    item.itemHeight = scale(ROW_HEIGHT) as u32;
}

/// Draws one row. Headers stand apart from servers and never look selected.
pub unsafe fn draw(item: &DRAWITEMSTRUCT) {
    let Ok(index) = usize::try_from(item.itemID as i32) else {
        return;
    };
    let Some(row) = ROWS.lock().unwrap().get(index).cloned() else {
        return;
    };
    let selected = item.itemState & ODS_SELECTED != 0;
    let (fill, text, font) = if row.is_header() {
        (
            COLOR_BTNFACE,
            COLOR_BTNTEXT,
            HEADER_FONT.load(Ordering::Acquire),
        )
    } else if selected {
        (
            COLOR_HIGHLIGHT,
            COLOR_HIGHLIGHTTEXT,
            ITEM_FONT.load(Ordering::Acquire),
        )
    } else {
        (
            COLOR_WINDOW,
            COLOR_WINDOWTEXT,
            ITEM_FONT.load(Ordering::Acquire),
        )
    };
    unsafe {
        FillRect(item.hDC, &item.rcItem, GetSysColorBrush(fill));
        let previous = SelectObject(item.hDC, font as HGDIOBJ);
        SetBkMode(item.hDC, TRANSPARENT as _);
        SetTextColor(item.hDC, GetSysColor(text));
        let mut area = item.rcItem;
        area.left += scale(if row.is_header() { INDENT / 2 } else { INDENT });
        area.right -= scale(INDENT / 2);
        let label: Vec<u16> = row.label.encode_utf16().collect();
        DrawTextW(
            item.hDC,
            label.as_ptr(),
            label.len() as i32,
            &mut area,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(item.hDC, previous);
    }
}

/// An arrow key is about to move the selection.
pub fn arrow_pressed(down: bool) {
    ARROW.store(if down { 1 } else { -1 }, Ordering::Relaxed);
}

/// A key has finished being handled.
pub fn key_done() {
    ARROW.store(0, Ordering::Relaxed);
}

/// Handles a notification from the control.
pub unsafe fn notify(window: HWND, code: u32) {
    if REBUILDING.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        match code {
            CBN_EDITCHANGE => search(),
            CBN_SELCHANGE => {
                keep_off_headers();
                // A closed list commits at once, as arrowing it can't be undone by closing.
                if !is_open() {
                    send_selection();
                }
            }
            CBN_SELENDCANCEL => {
                // Clicking away from a search keeps the current pick. Closing a plain list keeps
                // the selection Windows restores, which is also how the test scripts pick.
                if SEARCHING.load(Ordering::Relaxed) {
                    CANCELLED.store(true, Ordering::Relaxed);
                }
            }
            CBN_CLOSEUP => {
                // Commit on close, not while arrowing through an open list.
                if !CANCELLED.load(Ordering::Relaxed) {
                    send_selection();
                }
                end_search();
                PostMessageW(window, WM_VIEW, 0, 0);
            }
            CBN_KILLFOCUS => {
                if SEARCHING.load(Ordering::Relaxed) {
                    CANCELLED.store(true, Ordering::Relaxed);
                }
                end_search();
                PostMessageW(window, WM_VIEW, 0, 0);
            }
            _ => {}
        }
    }
}

fn end_search() {
    SEARCHING.store(false, Ordering::Relaxed);
}

unsafe fn list() -> HWND {
    unsafe {
        let mut info: COMBOBOXINFO = zeroed();
        info.cbSize = size_of::<COMBOBOXINFO>() as u32;
        GetComboBoxInfo(combo(), &mut info);
        info.hwndList
    }
}

/// An open list keeps the height it opened with, so shrink it to what a search left.
unsafe fn fit_list(rows: usize) {
    unsafe {
        let list = list();
        let mut area: RECT = zeroed();
        GetWindowRect(list, &mut area);
        let height = scale(ROW_HEIGHT) * rows.clamp(1, MAX_SEARCH_ROWS) as i32 + 2;
        SetWindowPos(
            list,
            null_mut(),
            0,
            0,
            area.right - area.left,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

unsafe fn is_open() -> bool {
    unsafe { SendMessageW(combo(), CB_GETDROPPEDSTATE, 0, 0) != 0 }
}

unsafe fn current_index() -> Option<usize> {
    usize::try_from(unsafe { SendMessageW(combo(), CB_GETCURSEL, 0, 0) }).ok()
}

/// Moves the selection off a header: past it for an arrow key, or back for a click.
unsafe fn keep_off_headers() {
    let Some(index) = (unsafe { current_index() }) else {
        return;
    };
    let last = LAST_INDEX.load(Ordering::Relaxed);
    let target = {
        let rows = ROWS.lock().unwrap();
        match rows.get(index) {
            Some(row) if row.is_header() => match ARROW.load(Ordering::Relaxed) {
                0 => usize::try_from(last).ok(),
                way => rows::skip_header(&rows, index, way > 0),
            },
            _ => None,
        }
    };
    if let Some(target) = target {
        unsafe { SendMessageW(combo(), CB_SETCURSEL, target, 0) };
        LAST_INDEX.store(target as isize, Ordering::Relaxed);
    } else if let Ok(index) = isize::try_from(index) {
        LAST_INDEX.store(index, Ordering::Relaxed);
    }
}

/// Runs the command for a server, unless it's already selected. Returns whether it ran.
fn commit(code: &str) -> bool {
    {
        let mut selected = SELECTED.lock().unwrap();
        if *selected == code {
            return false;
        }
        selected.clear();
        selected.push_str(code);
    }
    if code.is_empty() {
        run_command("clear pick");
    } else {
        run_command(&format!("set pick {code}"));
    }
    true
}

unsafe fn send_selection() {
    let code = unsafe { current_index() }.and_then(|index| {
        ROWS.lock()
            .unwrap()
            .get(index)
            .and_then(|row| row.code.clone())
    });
    match code {
        Some(code) => {
            if !commit(&code) {
                sync_selected_with_status();
            }
        }
        None => sync_selected_with_status(),
    }
}

fn sync_selected_with_status() {
    if let Some((_, status)) = &*DATA.lock().unwrap() {
        SELECTED.lock().unwrap().clone_from(&status.pick);
    }
}

/// Fills the control. The lock isn't held across `SendMessageW`, which can re-enter the window
/// procedure, and the rows are stored first because drawing reads them.
unsafe fn fill(rows: Vec<Row>) {
    let labels: Vec<Vec<u16>> = rows.iter().map(|row| wide(&row.label)).collect();
    *ROWS.lock().unwrap() = rows;
    unsafe {
        SendMessageW(combo(), CB_RESETCONTENT, 0, 0);
        for label in &labels {
            SendMessageW(combo(), CB_ADDSTRING, 0, label.as_ptr() as LPARAM);
        }
    }
}

/// Narrows the list to what's been typed. `CB_RESETCONTENT` clears the edit box and showing the list
/// can change it, so the text and caret are put back afterwards.
unsafe fn search() {
    let Some((servers, status)) = DATA.lock().unwrap().clone() else {
        return;
    };
    REBUILDING.store(true, Ordering::Relaxed);
    SEARCHING.store(true, Ordering::Relaxed);
    CANCELLED.store(false, Ordering::Relaxed);
    unsafe {
        let typed = wide_text(combo());
        let selection = SendMessageW(combo(), CB_GETEDITSEL, 0, 0) as usize;
        let caret = selection & 0xFFFF;
        let rows = rows::build_rows(&servers, &status, &typed);
        let first = rows::first_match(&rows, &typed);
        let shown = rows.len();
        fill(rows);
        if !is_open() {
            SendMessageW(combo(), CB_SHOWDROPDOWN, 1, 0);
            // Showing the list hides the pointer until it moves.
            SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
        }
        SetWindowTextW(combo(), wide(&typed).as_ptr());
        let place = (caret | (caret << 16)) as LPARAM;
        SendMessageW(combo(), CB_SETEDITSEL, 0, place);
        fit_list(shown);
        // Set on the list directly, last: the combo's own message would overwrite the typed text,
        // and any change to that text clears the highlight.
        SendMessageW(list(), LB_SETCURSEL, first.unwrap_or(NO_SELECTION), 0);
        LAST_INDEX.store(first.map_or(-1, |index| index as isize), Ordering::Relaxed);
    }
    REBUILDING.store(false, Ordering::Relaxed);
}

/// Enter: picks the highlighted row, or the first match. Returns whether there was one.
pub unsafe fn enter(window: HWND) -> bool {
    let target = unsafe {
        let highlighted = current_index();
        let rows = ROWS.lock().unwrap();
        highlighted
            .filter(|index| rows.get(*index).is_some_and(|row| !row.is_header()))
            .or_else(|| rows.iter().position(|row| !row.is_header()))
            .and_then(|index| rows[index].code.clone())
    };
    let searching = SEARCHING.load(Ordering::Relaxed);
    let Some(code) = target else {
        return searching;
    };
    if !searching && !unsafe { is_open() } {
        return false;
    }
    commit(&code);
    unsafe { close_and_restore(window) };
    true
}

/// Esc: ends a search or closes the list without changing the pick. Returns whether it did either.
pub unsafe fn escape(window: HWND) -> bool {
    if !SEARCHING.load(Ordering::Relaxed) && !unsafe { is_open() } {
        return false;
    }
    unsafe { close_and_restore(window) };
    true
}

unsafe fn close_and_restore(window: HWND) {
    CANCELLED.store(true, Ordering::Relaxed);
    end_search();
    unsafe {
        if is_open() {
            SendMessageW(combo(), CB_SHOWDROPDOWN, 0, 0);
        }
        PostMessageW(window, WM_VIEW, 0, 0);
    }
}

/// Stores the latest data. The current pick is only adopted while nobody is using the list.
pub fn update(servers: Vec<Server>, status: Status) {
    let in_use = SEARCHING.load(Ordering::Relaxed) || unsafe { is_open() };
    if !in_use {
        SELECTED.lock().unwrap().clone_from(&status.pick);
    }
    *DATA.lock().unwrap() = Some((servers, status));
}

/// Puts the full list back and selects the current pick, unless someone is using the list.
/// Rebuilding would snap an open list shut, and `CBN_CLOSEUP` posts another refresh.
pub unsafe fn refresh() {
    unsafe {
        if SEARCHING.load(Ordering::Relaxed) || is_open() {
            return;
        }
        let Some((servers, status)) = DATA.lock().unwrap().clone() else {
            return;
        };
        CANCELLED.store(false, Ordering::Relaxed);
        let rows = rows::build_rows(&servers, &status, "");
        let selected = SELECTED.lock().unwrap().clone();
        let index = rows::position(&rows, &selected);
        REBUILDING.store(true, Ordering::Relaxed);
        if *ROWS.lock().unwrap() != rows {
            fill(rows);
        }
        SendMessageW(combo(), CB_SETCURSEL, index.unwrap_or(NO_SELECTION), 0);
        LAST_INDEX.store(index.map_or(-1, |index| index as isize), Ordering::Relaxed);
        REBUILDING.store(false, Ordering::Relaxed);
    }
}
