//! The few window behaviours that need Win32 directly.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, POINT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
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

/// Guards against a saved position on a monitor that is no longer attached.
pub fn is_on_screen(position: [f32; 2]) -> bool {
    let corner = POINT { x: position[0] as i32 + 40, y: position[1] as i32 + 20 };
    !unsafe { MonitorFromPoint(corner, MONITOR_DEFAULTTONULL) }.is_invalid()
}

/// Asks DWM for the standard Windows 11 rounded corners on the frameless window.
pub fn round_corners(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
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
