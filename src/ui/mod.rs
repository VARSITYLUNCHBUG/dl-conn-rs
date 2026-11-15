//! The window, built on egui. This file starts it and reports anything that stops it starting, using
//! plain message boxes because there may be no working window to show them in.

mod app;
mod pages;
mod picker;
mod rows;
mod theme;

#[cfg(test)]
mod tests;

use crate::log;
use crate::session::{self, Firewall};
use crate::wfp::Engine;
use crate::wide;
use app::App;
use eframe::egui::{self, Context, ViewportCommand};
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetMessageW, GetWindowRect, IsIconic, MB_ICONERROR, MB_ICONWARNING, MSG, MessageBoxW,
    RegisterClassW, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    WM_DISPLAYCHANGE, WM_SETTINGCHANGE, WNDCLASSW, WS_OVERLAPPED,
};
use windows_sys::w;

const APP_NAME: &str = "dl-conn-rs";
const ICON: &[u8] = include_bytes!("../../assets/icon-64.rgba");
const ICON_SIZE: u32 = 64;
const WINDOW_SIZE: [f32; 2] = [720.0, 700.0];
const MIN_SIZE: [f32; 2] = [560.0, 480.0];
/// Forces the Windows software renderer, for testing and for machines with a bad graphics driver.
const RENDERER_OVERRIDE: &str = "DL_CONN_RS_RENDERER";
const EVENT_MODIFY_STATE: u32 = 2;
const SYNCHRONIZE: u32 = 0x0010_0000;

/// Shows an error when there may be no window to show it in.
fn message(text: &str) {
    let text = wide(text);
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), w!("dl-conn-rs"), MB_ICONERROR) };
}

fn warning(text: &str) {
    let text = wide(text);
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), w!("dl-conn-rs"), MB_ICONWARNING) };
}

/// Records a crash and says so. The block stays on, which is the safe side.
fn install_panic_handler() {
    std::panic::set_hook(Box::new(|info| {
        let text = format!("dl-conn-rs stopped because of an internal error: {info}");
        log::emergency(&text);
        message(&format!(
            "{text}\n\nAny block already on stays on until you pick again or restart the PC. \
             Details are in dl-conn-rs.log, in the app's data folder."
        ));
    }));
}

/// Prefers the GPU that uses least power, so a laptop's game GPU isn't woken for a small window. The
/// Windows software renderer is the last resort, for VMs and remote sessions with no usable GPU.
fn adapter_rank(device: wgpu::DeviceType, force_software: bool) -> u8 {
    match device {
        wgpu::DeviceType::Cpu if force_software => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::DiscreteGpu => 2,
        wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

fn graphics() -> WgpuConfiguration {
    let force_software =
        std::env::var_os(RENDERER_OVERRIDE).is_some_and(|value| value == "software");
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    setup.power_preference = wgpu::PowerPreference::LowPower;
    // A small window has little to store, so ask for small graphics memory blocks.
    setup.device_descriptor = Arc::new(|_adapter| wgpu::DeviceDescriptor {
        label: Some("dl-conn-rs"),
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: 8192,
            ..wgpu::Limits::default()
        },
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..wgpu::DeviceDescriptor::default()
    });
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        let chosen = adapters
            .iter()
            .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
            .min_by_key(|adapter| adapter_rank(adapter.get_info().device_type, force_software))
            .cloned()
            .ok_or_else(|| "DirectX 12 has no usable adapter on this PC".to_string())?;
        let info = chosen.get_info();
        log!("renderer: {} ({:?})", info.name, info.device_type);
        Ok(chosen)
    }));
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..WgpuConfiguration::default()
    }
}

/// Lets a later launch bring this one forward. A named event is used because the window class name
/// belongs to the windowing library.
fn listen_for_second_launch(ctx: Context) {
    let event = unsafe { CreateEventW(null(), 0, 0, w!("Global\\dl-conn-rs-show")) } as usize;
    if event == 0 {
        return;
    }
    thread::spawn(move || {
        while unsafe { WaitForSingleObject(event as _, INFINITE) } == 0 {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            ctx.request_repaint();
        }
    });
}

/// The app window's handle, from eframe, so the display listener can move it.
fn main_window(creation: &eframe::CreationContext) -> Option<usize> {
    use wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    match creation.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
        _ => None,
    }
}

/// Moves the window 1 px and back. After a game in exclusive fullscreen changes the display mode and
/// gives it back, Windows can show this window's frame up and left by the border and title bar, with
/// a white strip right and bottom. Layout and input stay right, and only a real move puts the frame
/// back: a redraw or a same-place `SetWindowPos`, even with `SWP_FRAMECHANGED`, doesn't.
/// `scripts/fullscreen-repro.ps1` reproduces it.
fn nudge(window: HWND) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if IsIconic(window) != 0 || GetWindowRect(window, &mut rect) == 0 {
            return;
        }
        let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE;
        SetWindowPos(window, null_mut(), rect.left + 1, rect.top, 0, 0, flags);
        SetWindowPos(window, null_mut(), rect.left, rect.top, 0, 0, flags);
    }
}

/// Redraws after Windows changes the display or the desktop's work area, as a fullscreen game can,
/// and after a display mode change also nudges the window back into place. The window only draws on
/// a change and eframe isn't told about these. A hidden window gets these broadcasts. Each one asks
/// for a few rounds, because the desktop may still be settling.
fn refresh_on_display_change(ctx: Context, window: Option<usize>) {
    static CONTEXT: OnceLock<Context> = OnceLock::new();
    static WINDOW: AtomicUsize = AtomicUsize::new(0);
    static SCHEDULED: AtomicBool = AtomicBool::new(false);
    static MODE_CHANGED: AtomicBool = AtomicBool::new(false);
    const CLASS: *const u16 = w!("dl-conn-rs-display");

    unsafe extern "system" fn handle(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DISPLAYCHANGE {
            log!(
                "display changed to {}x{}",
                lparam & 0xFFFF,
                (lparam >> 16) & 0xFFFF
            );
            MODE_CHANGED.store(true, Ordering::SeqCst);
        }
        if matches!(message, WM_DISPLAYCHANGE | WM_SETTINGCHANGE)
            && let Some(ctx) = CONTEXT.get()
            && !SCHEDULED.swap(true, Ordering::SeqCst)
        {
            let ctx = ctx.clone();
            thread::spawn(move || {
                for wait in [0, 250, 1000, 3000] {
                    thread::sleep(Duration::from_millis(wait));
                    let app = WINDOW.load(Ordering::SeqCst);
                    if wait > 0 && app != 0 && MODE_CHANGED.load(Ordering::SeqCst) {
                        nudge(app as HWND);
                    }
                    ctx.request_repaint();
                }
                MODE_CHANGED.store(false, Ordering::SeqCst);
                SCHEDULED.store(false, Ordering::SeqCst);
            });
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    if CONTEXT.set(ctx).is_err() {
        return;
    }
    WINDOW.store(window.unwrap_or(0), Ordering::SeqCst);
    thread::spawn(|| unsafe {
        let instance = GetModuleHandleW(null());
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = Some(handle);
        class.hInstance = instance;
        class.lpszClassName = CLASS;
        if RegisterClassW(&class) == 0 {
            return;
        }
        let window = CreateWindowExW(
            0,
            CLASS,
            CLASS,
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return;
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
    });
}

fn ask_first_launch_to_show() {
    unsafe {
        // This launch came from the user, so it's allowed to hand them the foreground.
        AllowSetForegroundWindow(ASFW_ANY);
        let event = OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZE,
            0,
            w!("Global\\dl-conn-rs-show"),
        );
        if !event.is_null() {
            SetEvent(event);
        }
    }
}

fn window() -> eframe::NativeOptions {
    let icon = egui::IconData {
        rgba: ICON.to_vec(),
        width: ICON_SIZE,
        height: ICON_SIZE,
    };
    #[cfg(test)]
    let event_loop_builder: Option<eframe::EventLoopBuilderHook> = Some(Box::new(|builder| {
        // The test harness runs tests on a thread of its own.
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }));
    #[cfg(not(test))]
    let event_loop_builder = None;
    eframe::NativeOptions {
        event_loop_builder,
        viewport: egui::ViewportBuilder::default()
            .with_title("dl-conn-rs - loading")
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(MIN_SIZE)
            .with_icon(icon),
        wgpu_options: graphics(),
        ..eframe::NativeOptions::default()
    }
}

/// Opens the window and runs until it closes.
fn open_window(firewall: impl Firewall) {
    let started = eframe::run_native(
        APP_NAME,
        window(),
        Box::new(move |creation| {
            let ctx = creation.egui_ctx.clone();
            theme::install(&ctx);
            listen_for_second_launch(ctx.clone());
            refresh_on_display_change(ctx.clone(), main_window(creation));
            let app = App::start(ctx, |host| session::spawn(firewall, host));
            Ok(Box::new(app))
        }),
    );
    if let Err(error) = started {
        log!("window failed: {error}");
        warning(&format!(
            "dl-conn-rs couldn't open its window: {error}\n\nIt needs DirectX 12, on Windows 10 or later. \
             Any block already on is unchanged, and lasts until you restart the PC."
        ));
    }
}

pub fn run() {
    install_panic_handler();
    unsafe {
        // Filters are machine-wide, so two copies anywhere on this PC would fight over them.
        CreateMutexW(null(), 0, w!("Global\\dl-conn-rs"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            ask_first_launch_to_show();
            return;
        }
    }
    if let Some(folder) = crate::diag::data_dir() {
        log::to_file(folder.join("dl-conn-rs.log"));
    }
    log!("dl-conn-rs {} starting", env!("CARGO_PKG_VERSION"));
    let engine = match Engine::open() {
        Ok(engine) => engine,
        Err(error) => {
            log!("filtering platform failed: {error}");
            message(&format!(
                "dl-conn-rs couldn't open the Windows Filtering Platform: {error}\n\nCheck that the Base \
                 Filtering Engine service is running, then open dl-conn-rs again."
            ));
            return;
        }
    };
    open_window(engine);
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use crate::sdr::Blocks;
    use std::sync::Mutex;

    /// Lets a script open the real window with no admin rights and no real filters.
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
    #[ignore = "opens the real window on the main thread; run with --test-threads=1"]
    fn window_with_a_fake_firewall() {
        if let Some(folder) = crate::diag::data_dir() {
            log::to_file(folder.join("dl-conn-rs.log"));
        }
        open_window(FakeFirewall::default());
    }

    #[test]
    fn the_window_icon_matches_its_size() {
        assert_eq!(ICON.len() as u32, ICON_SIZE * ICON_SIZE * 4);
    }

    #[test]
    fn prefers_the_low_power_gpu_and_keeps_software_as_the_last_resort() {
        let rank = |device| adapter_rank(device, false);
        assert!(rank(wgpu::DeviceType::IntegratedGpu) < rank(wgpu::DeviceType::DiscreteGpu));
        assert!(rank(wgpu::DeviceType::DiscreteGpu) < rank(wgpu::DeviceType::Cpu));
        assert!(rank(wgpu::DeviceType::VirtualGpu) < rank(wgpu::DeviceType::Cpu));
    }

    #[test]
    fn software_can_be_forced() {
        assert_eq!(adapter_rank(wgpu::DeviceType::Cpu, true), 0);
    }
}
