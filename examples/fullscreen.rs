//! Takes the screen in exclusive fullscreen for a few seconds and gives it back, the way a game does,
//! to reproduce what that does to other windows. Not part of the app.
//!
//! `dxgi` uses a DirectX 11 swap chain with the old Discard effect, which is what Deadlock's log
//! reports. `display` only changes the display mode and back, with no DirectX, to tell the two apart.
//! The screen goes black while it runs. Esc ends it early.
//!
//!   fullscreen [--mode dxgi|display] [--secs 5] [--size <width>x<height>] [--hz <refresh rate>]

use std::mem::{size_of, transmute, zeroed};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BLACK_BRUSH, CDS_FULLSCREEN, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL,
    DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS,
    ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW, GetStockObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics, MSG,
    PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SetForegroundWindow, SetProcessDPIAware, TranslateMessage, WM_KEYDOWN, WM_QUIT, WNDCLASSW,
    WS_POPUP, WS_VISIBLE,
};
use windows_sys::w;

const MAX_SECS: u64 = 30;
const VK_ESCAPE: usize = 0x1B;

/// The few DirectX 11 and DXGI pieces needed. windows-sys doesn't carry Direct3D, so they're
/// declared here, and COM methods are called through their vtable slots.
mod dx {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Rational {
        pub numerator: u32,
        pub denominator: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct ModeDesc {
        pub width: u32,
        pub height: u32,
        pub refresh_rate: Rational,
        pub format: u32,
        pub scanline_ordering: u32,
        pub scaling: u32,
    }

    #[repr(C)]
    pub struct SampleDesc {
        pub count: u32,
        pub quality: u32,
    }

    #[repr(C)]
    pub struct SwapChainDesc {
        pub buffer_desc: ModeDesc,
        pub sample_desc: SampleDesc,
        pub buffer_usage: u32,
        pub buffer_count: u32,
        pub output_window: *mut c_void,
        pub windowed: i32,
        pub swap_effect: u32,
        pub flags: u32,
    }

    pub const FORMAT_R8G8B8A8_UNORM: u32 = 28;
    pub const USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
    pub const SWAP_EFFECT_DISCARD: u32 = 0;
    pub const FLAG_ALLOW_MODE_SWITCH: u32 = 2;
    pub const DRIVER_TYPE_HARDWARE: u32 = 1;
    pub const SDK_VERSION: u32 = 7;

    // IUnknown, then IDXGIObject, IDXGIDeviceSubObject and IDXGISwapChain, in declaration order.
    pub const RELEASE: usize = 2;
    pub const PRESENT: usize = 8;
    pub const SET_FULLSCREEN_STATE: usize = 10;
    pub const GET_FULLSCREEN_STATE: usize = 11;
    pub const RESIZE_BUFFERS: usize = 13;
    pub const RESIZE_TARGET: usize = 14;

    pub type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    pub type Present = unsafe extern "system" fn(*mut c_void, u32, u32) -> i32;
    pub type SetFullscreenState = unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32;
    pub type GetFullscreenState =
        unsafe extern "system" fn(*mut c_void, *mut i32, *mut *mut c_void) -> i32;
    pub type ResizeBuffers = unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, u32) -> i32;
    pub type ResizeTarget = unsafe extern "system" fn(*mut c_void, *const ModeDesc) -> i32;

    #[link(name = "d3d11", kind = "raw-dylib")]
    unsafe extern "system" {
        #[allow(clippy::too_many_arguments)]
        pub fn D3D11CreateDeviceAndSwapChain(
            adapter: *mut c_void,
            driver_type: u32,
            software: *mut c_void,
            flags: u32,
            feature_levels: *const u32,
            feature_level_count: u32,
            sdk_version: u32,
            desc: *const SwapChainDesc,
            swap_chain: *mut *mut c_void,
            device: *mut *mut c_void,
            feature_level: *mut u32,
            context: *mut *mut c_void,
        ) -> i32;
    }

    /// The function in a COM object's vtable slot.
    pub unsafe fn method(object: *mut c_void, slot: usize) -> *const c_void {
        unsafe { *(*(object as *const *const *const c_void)).add(slot) }
    }

    pub unsafe fn release(object: *mut c_void) {
        if !object.is_null() {
            unsafe {
                let release: Release = std::mem::transmute(method(object, RELEASE));
                release(object);
            }
        }
    }
}

unsafe extern "system" fn handle(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_KEYDOWN && wparam == VK_ESCAPE {
        unsafe { PostQuitMessage(0) };
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// A black window over the whole primary screen, in front.
fn cover_screen() -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(handle);
        class.hInstance = instance;
        class.hbrBackground = GetStockObject(BLACK_BRUSH);
        class.lpszClassName = w!("dl-conn-rs-fullscreen-test");
        RegisterClassW(&class);
        let window = CreateWindowExW(
            0,
            class.lpszClassName,
            w!("fullscreen test"),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            GetSystemMetrics(SM_CXSCREEN),
            GetSystemMetrics(SM_CYSCREEN),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return Err("couldn't create the window".into());
        }
        SetForegroundWindow(window);
        Ok(window)
    }
}

/// Handles window messages until the time is up or Esc is pressed, calling `frame` between them.
fn run_for(secs: u64, mut frame: impl FnMut()) {
    let until = Instant::now() + Duration::from_secs(secs);
    let mut message: MSG = unsafe { zeroed() };
    while Instant::now() < until {
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == WM_QUIT {
                return;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        frame();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn dxgi(secs: u64, size: Option<(u32, u32)>, hz: Option<u32>) -> Result<(), String> {
    let window = cover_screen()?;
    let screen = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let (width, height) = size.unwrap_or((screen.0 as u32, screen.1 as u32));
    let mode = dx::ModeDesc {
        width,
        height,
        refresh_rate: dx::Rational {
            numerator: hz.unwrap_or(0),
            denominator: u32::from(hz.is_some()),
        },
        format: dx::FORMAT_R8G8B8A8_UNORM,
        scanline_ordering: 0,
        scaling: 0,
    };
    let desc = dx::SwapChainDesc {
        buffer_desc: mode,
        sample_desc: dx::SampleDesc {
            count: 1,
            quality: 0,
        },
        buffer_usage: dx::USAGE_RENDER_TARGET_OUTPUT,
        buffer_count: 1,
        output_window: window,
        windowed: 1,
        swap_effect: dx::SWAP_EFFECT_DISCARD,
        flags: dx::FLAG_ALLOW_MODE_SWITCH,
    };
    let mut swap_chain = null_mut();
    let mut device = null_mut();
    let mut context = null_mut();
    let mut level = 0;
    let created = unsafe {
        dx::D3D11CreateDeviceAndSwapChain(
            null_mut(),
            dx::DRIVER_TYPE_HARDWARE,
            null_mut(),
            0,
            null(),
            0,
            dx::SDK_VERSION,
            &desc,
            &mut swap_chain,
            &mut device,
            &mut level,
            &mut context,
        )
    };
    if created < 0 {
        unsafe { DestroyWindow(window) };
        return Err(format!(
            "D3D11CreateDeviceAndSwapChain failed: {created:#010x}"
        ));
    }
    unsafe {
        let set_fullscreen: dx::SetFullscreenState =
            transmute(dx::method(swap_chain, dx::SET_FULLSCREEN_STATE));
        let get_fullscreen: dx::GetFullscreenState =
            transmute(dx::method(swap_chain, dx::GET_FULLSCREEN_STATE));
        let resize_buffers: dx::ResizeBuffers =
            transmute(dx::method(swap_chain, dx::RESIZE_BUFFERS));
        let resize_target: dx::ResizeTarget = transmute(dx::method(swap_chain, dx::RESIZE_TARGET));
        let present: dx::Present = transmute(dx::method(swap_chain, dx::PRESENT));

        let entered = set_fullscreen(swap_chain, 1, null_mut());
        println!("SetFullscreenState(true): {entered:#010x}");
        if size.is_some() || hz.is_some() {
            println!("ResizeTarget: {:#010x}", resize_target(swap_chain, &mode));
        }
        println!(
            "ResizeBuffers: {:#010x}",
            resize_buffers(swap_chain, 0, 0, 0, 0, dx::FLAG_ALLOW_MODE_SWITCH)
        );
        let mut checked = false;
        let started = Instant::now();
        run_for(secs, || {
            present(swap_chain, 1, 0);
            if !checked && started.elapsed() > Duration::from_secs(1) {
                checked = true;
                let mut fullscreen = 0;
                get_fullscreen(swap_chain, &mut fullscreen, null_mut());
                println!("exclusive fullscreen after 1 s: {}", fullscreen != 0);
            }
        });
        // Whether anything, such as another window moving, took exclusive fullscreen away early.
        let mut fullscreen = 0;
        get_fullscreen(swap_chain, &mut fullscreen, null_mut());
        println!("exclusive fullscreen at the end: {}", fullscreen != 0);
        println!(
            "SetFullscreenState(false): {:#010x}",
            set_fullscreen(swap_chain, 0, null_mut())
        );
        dx::release(context);
        dx::release(swap_chain);
        dx::release(device);
        DestroyWindow(window);
    }
    Ok(())
}

fn display_mode(index: ENUM_DISPLAY_SETTINGS_MODE) -> Option<DEVMODEW> {
    let mut mode: DEVMODEW = unsafe { zeroed() };
    mode.dmSize = size_of::<DEVMODEW>() as u16;
    (unsafe { EnumDisplaySettingsW(null(), index, &mut mode) } != 0).then_some(mode)
}

fn display(secs: u64, size: Option<(u32, u32)>, hz: Option<u32>) -> Result<(), String> {
    let current = display_mode(ENUM_CURRENT_SETTINGS).ok_or("can't read the display mode")?;
    let same_size = |mode: &DEVMODEW| {
        mode.dmPelsWidth == current.dmPelsWidth && mode.dmPelsHeight == current.dmPelsHeight
    };
    // Without --hz, any other refresh rate at the same size, so the mode really changes.
    let rate = match (hz, size) {
        (Some(rate), _) => rate,
        (None, Some(_)) => current.dmDisplayFrequency,
        (None, None) => (0..)
            .map_while(display_mode)
            .filter(|mode| same_size(mode) && mode.dmDisplayFrequency != current.dmDisplayFrequency)
            .map(|mode| mode.dmDisplayFrequency)
            .min_by_key(|rate| rate.abs_diff(current.dmDisplayFrequency))
            .ok_or("no other refresh rate at this size; pass --hz")?,
    };
    let (width, height) = size.unwrap_or((current.dmPelsWidth, current.dmPelsHeight));
    println!(
        "display: {}x{} at {} Hz, switching to {width}x{height} at {rate} Hz",
        current.dmPelsWidth, current.dmPelsHeight, current.dmDisplayFrequency
    );
    let mut target = current;
    target.dmPelsWidth = width;
    target.dmPelsHeight = height;
    target.dmDisplayFrequency = rate;
    target.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    let window = cover_screen()?;
    let changed =
        unsafe { ChangeDisplaySettingsExW(null(), &target, null_mut(), CDS_FULLSCREEN, null()) };
    println!("ChangeDisplaySettingsEx: {changed} (0 is success)");
    if changed == DISP_CHANGE_SUCCESSFUL {
        run_for(secs, || {});
    }
    let restored = unsafe { ChangeDisplaySettingsExW(null(), null(), null_mut(), 0, null()) };
    println!("restore: {restored} (0 is success)");
    unsafe { DestroyWindow(window) };
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .map(String::as_str)
    };
    let secs = value("--secs")
        .map_or(Ok(5), str::parse)
        .map_err(|_| "bad --secs")?;
    if secs > MAX_SECS {
        return Err(format!("--secs is at most {MAX_SECS}"));
    }
    let hz = value("--hz")
        .map(str::parse)
        .transpose()
        .map_err(|_| "bad --hz")?;
    let size = value("--size")
        .map(|text| {
            let (width, height) = text.split_once('x')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })
        .map(|size| size.ok_or("bad --size: use <width>x<height>"))
        .transpose()?;
    // The window must cover the screen in real pixels.
    unsafe { SetProcessDPIAware() };
    match value("--mode").unwrap_or("dxgi") {
        "dxgi" => dxgi(secs, size, hz),
        "display" => display(secs, size, hz),
        other => Err(format!("unknown --mode {other}: use dxgi or display")),
    }
}
