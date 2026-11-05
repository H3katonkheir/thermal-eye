//! The few window behaviours that need Win32 directly.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowRect, IsIconic, SW_RESTORE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetForegroundWindow, SetWindowPos, ShowWindow,
};
use windows::core::{HSTRING, w};

pub const TITLE: &str = "Thermal Eye";

/// Returns false, after raising the running window, if the app is already open.
pub fn claim_single_instance() -> bool {
    unsafe {
        let mutex = CreateMutexW(None, false, w!("Local\\ThermalEyeWidget"));
        if mutex.is_err() || GetLastError() != ERROR_ALREADY_EXISTS {
            // The handle stays open for the life of the process, holding the name.
            return true;
        }
        if let Ok(window) = FindWindowW(None, &HSTRING::from(TITLE)) {
            let _ = ShowWindow(window, SW_RESTORE);
            let _ = SetForegroundWindow(window);
        }
        false
    }
}

pub fn open_url(url: &str) {
    let url = HSTRING::from(url);
    let _ = unsafe { ShellExecuteW(None, w!("open"), &url, None, None, SW_SHOWNORMAL) };
}

fn hwnd(window: &impl HasWindowHandle) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// The raw window handle, for another thread to restore or close the window.
pub fn handle(window: &impl HasWindowHandle) -> Option<isize> {
    hwnd(window).map(|hwnd| hwnd.0 as isize)
}

// Window positions are kept in physical pixels. Logical points depend on the
// scaling of the monitor the window is on, so with mixed scaling a position saved
// on one monitor lands somewhere else when restored on another.

/// Guards against a saved position on a monitor that is no longer attached.
pub fn is_on_screen(position: [f32; 2]) -> bool {
    let corner = POINT { x: position[0] as i32 + 40, y: position[1] as i32 + 20 };
    !unsafe { MonitorFromPoint(corner, MONITOR_DEFAULTTONULL) }.is_invalid()
}

/// The window's top-left corner, or `None` while it is minimised.
pub fn position(window: &impl HasWindowHandle) -> Option<[f32; 2]> {
    let hwnd = hwnd(window)?;
    let mut rect = RECT::default();
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return None;
        }
        GetWindowRect(hwnd, &mut rect).ok()?;
    }
    Some([rect.left as f32, rect.top as f32])
}

pub fn move_to(window: &impl HasWindowHandle, position: [f32; 2]) {
    let Some(hwnd) = hwnd(window) else { return };
    let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
    let _ = unsafe { SetWindowPos(hwnd, None, position[0] as i32, position[1] as i32, 0, 0, flags) };
}

/// Asks DWM for the standard Windows 11 rounded corners on the frameless window.
pub fn round_corners(window: &impl HasWindowHandle) {
    let Some(hwnd) = hwnd(window) else { return };
    let preference = DWMWCP_ROUND;
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            size_of_val(&preference) as u32,
        )
    };
}
