//! The notification area icon, which shows the hottest reading as a number.
//!
//! It runs on its own thread with its own window, so it keeps updating while the
//! widget is minimised and egui is not drawing.

use std::cell::RefCell;
use std::sync::mpsc::channel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui::Color32;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    FW_BOLD, GdiFlush, HBITMAP, OUT_TT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, ICONINFO, IMAGE_ICON,
    IsIconic, LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SW_HIDE, SW_RESTORE, SW_SHOW, SendMessageW, SetForegroundWindow,
    SetTimer, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CLOSE,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::app::ThermalEye;
use crate::config::{self, Config};
use crate::model::Monitor;
use crate::theme::{TITLE_BG, heat_text};

/// Clicks on the icon arrive as this message.
const ICON_EVENT: u32 = WM_APP + 1;
/// The settings changed, so redraw now rather than at the next tick.
const REFRESH: u32 = WM_APP + 2;
/// Removes the icon and ends the thread. Sent as the app closes.
const QUIT: u32 = WM_APP + 3;
const TICK: Duration = Duration::from_secs(1);
const MENU_OPEN: usize = 1;
const MENU_EXIT: usize = 2;
/// The app icon, embedded by build.rs.
const APP_ICON_ID: u16 = 1;
const FONT: PCWSTR = w!("Segoe UI");
/// Average character widths to try, as percentages of the font size, from the
/// font's own proportions to fairly condensed.
const NARROWING: [i32; 4] = [0, 44, 40, 36];
/// Longest tooltip the shell accepts, less its terminating zero.
const TIP_LIMIT: usize = 127;

/// What the tray thread needs to know from the settings.
#[derive(Clone, Debug, PartialEq)]
struct Settings {
    enabled: bool,
    fahrenheit: bool,
    hidden: Vec<String>,
}

impl Settings {
    fn of(config: &Config) -> Self {
        Self { enabled: config.tray_icon, fahrenheit: config.fahrenheit, hidden: config.hidden.clone() }
    }
}

/// Owns the tray thread. Dropping it removes the icon.
pub struct Tray {
    window: isize,
    main_window: Option<isize>,
    settings: Arc<Mutex<Settings>>,
    /// The icon is in the notification area, so a hidden window can be found again.
    icon_shown: Arc<AtomicBool>,
}

impl Tray {
    /// `main_window` is shown again when the icon is clicked, and closed by Exit.
    pub fn start(monitor: Arc<Mutex<Monitor>>, config: &Config, main_window: Option<isize>) -> Option<Self> {
        let settings = Arc::new(Mutex::new(Settings::of(config)));
        let shared = settings.clone();
        let icon_shown = Arc::new(AtomicBool::new(false));
        let shown_flag = icon_shown.clone();
        let (ready, started) = channel();
        let spawned = std::thread::Builder::new().name("tray".into()).spawn(move || {
            let Some(window) = create_window() else {
                let _ = ready.send(None);
                return;
            };
            STATE.with(|state| {
                *state.borrow_mut() = Some(State {
                    window,
                    main_window: main_window.map(to_hwnd),
                    monitor,
                    settings: shared,
                    icon_shown: shown_flag,
                    icon: None,
                    shown: None,
                })
            });
            let _ = ready.send(Some(window.0 as isize));
            unsafe { SetTimer(Some(window), 1, TICK.as_millis() as u32, None) };
            with_state(State::refresh);
            run_messages();
        });
        if spawned.is_err() {
            return None;
        }
        let window = started.recv().ok()??;
        Some(Self { window, main_window, settings, icon_shown })
    }

    /// Hides the widget, leaving the icon as the way back. Returns false, leaving the
    /// window alone, while there is no icon to click.
    pub fn hide_window(&self) -> bool {
        let Some(main) = self.main_window.filter(|_| self.icon_shown.load(Ordering::Relaxed)) else {
            return false;
        };
        let _ = unsafe { ShowWindow(to_hwnd(main), SW_HIDE) };
        true
    }

    /// Passes on setting changes, called every frame.
    pub fn sync(&self, config: &Config) {
        let next = Settings::of(config);
        let mut settings = self.settings.lock().unwrap();
        if *settings != next {
            *settings = next;
            let _ = unsafe { PostMessageW(Some(to_hwnd(self.window)), REFRESH, WPARAM(0), LPARAM(0)) };
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // Sent rather than posted, so the icon is gone before the process ends.
        unsafe { SendMessageW(to_hwnd(self.window), QUIT, None, None) };
    }
}

fn to_hwnd(handle: isize) -> HWND {
    HWND(handle as *mut _)
}

/// The number on the icon and its colour. `None` shows the app icon.
type Face = Option<(String, Color32)>;

#[derive(Clone, Debug, PartialEq)]
struct Reading {
    face: Face,
    tip: String,
}

/// An icon handed to the shell, with what it was drawn for.
struct Drawn {
    face: Face,
    size: i32,
    handle: HICON,
}

/// The hottest current reading among the components on show, in the chosen unit.
/// Its colour follows the limits of the component it came from, as in the table.
fn reading(monitor: &Monitor, settings: &Settings) -> Reading {
    let mut hottest: Option<(f32, Color32)> = None;
    let mut tip = String::from("Thermal Eye");
    let shown = monitor.components.iter().filter(|c| !config::contains(&settings.hidden, &c.key));
    for component in shown {
        let Some(now) = monitor.latest(component.high) else { continue };
        tip.push_str(&format!("\n{} {}", component.title, ThermalEye::format_temp(settings.fahrenheit, now)));
        if hottest.is_none_or(|(high, _)| now > high) {
            hottest = Some((now, heat_text(now, component.warn, component.critical)));
        }
    }
    let face = hottest.map(|(value, color)| {
        let text = ThermalEye::format_temp(settings.fahrenheit, value);
        (text.trim_end_matches('°').to_owned(), color)
    });
    Reading { face, tip }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Skips the call if the state is already in use, as it can be while the shell
/// calls back during `Shell_NotifyIconW`.
fn with_state(action: impl FnOnce(&mut State)) {
    STATE.with(|state| {
        if let Ok(mut state) = state.try_borrow_mut()
            && let Some(state) = state.as_mut()
        {
            action(state);
        }
    });
}

struct State {
    window: HWND,
    main_window: Option<HWND>,
    monitor: Arc<Mutex<Monitor>>,
    settings: Arc<Mutex<Settings>>,
    icon: Option<Drawn>,
    /// What the shell is showing, or `None` while there is no icon.
    shown: Option<Reading>,
    icon_shown: Arc<AtomicBool>,
}

impl State {
    fn refresh(&mut self) {
        let settings = self.settings.lock().unwrap().clone();
        if !settings.enabled {
            self.remove();
            return;
        }
        let next = reading(&self.monitor.lock().unwrap(), &settings);
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let redraw = self.icon.as_ref().is_none_or(|drawn| drawn.face != next.face || drawn.size != size);
        if !redraw && self.shown.as_ref() == Some(&next) {
            return;
        }
        if redraw {
            let icon = match &next.face {
                Some((text, color)) => draw_icon(text, *color, size),
                None => app_icon(size),
            };
            let Some(handle) = icon else { return };
            if let Some(old) = self.icon.replace(Drawn { face: next.face.clone(), size, handle }) {
                let _ = unsafe { DestroyIcon(old.handle) };
            }
        }

        let mut data = self.data();
        data.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
        data.uCallbackMessage = ICON_EVENT;
        data.hIcon = self.icon.as_ref().map(|drawn| drawn.handle).unwrap_or_default();
        for (slot, unit) in data.szTip.iter_mut().zip(next.tip.encode_utf16().take(TIP_LIMIT)) {
            *slot = unit;
        }
        let action = if self.shown.is_some() { NIM_MODIFY } else { NIM_ADD };
        // Adding fails while Explorer is still starting, as at sign-in. The next
        // tick tries again.
        if unsafe { Shell_NotifyIconW(action, &data) }.as_bool() {
            self.shown = Some(next);
        } else if action == NIM_MODIFY {
            self.shown = None;
        }
        self.icon_shown.store(self.shown.is_some(), Ordering::Relaxed);
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window,
            uID: 1,
            ..Default::default()
        }
    }

    fn remove(&mut self) {
        if self.shown.take().is_some() {
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data()) };
        }
        self.icon_shown.store(false, Ordering::Relaxed);
        if let Some(drawn) = self.icon.take() {
            let _ = unsafe { DestroyIcon(drawn.handle) };
        }
    }
}

fn create_window() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("ThermalEyeTray"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // A hidden top-level window rather than a message-only one, because only
        // top-level windows hear that the taskbar was recreated.
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.lpszClassName,
            w!("Thermal Eye tray"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

fn run_messages() {
    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            DispatchMessageW(&message);
        }
    }
}

/// Explorer sends this to every top-level window when it restarts, and the
/// icons have to be added again.
fn taskbar_created() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER | REFRESH => with_state(State::refresh),
        ICON_EVENT => match lparam.0 as u32 {
            WM_LBUTTONUP => open_main_window(),
            WM_RBUTTONUP => show_menu(window),
            _ => {}
        },
        QUIT => {
            with_state(State::remove);
            unsafe {
                let _ = DestroyWindow(window);
                PostQuitMessage(0);
            }
        }
        _ if message == taskbar_created() => with_state(|state| {
            state.shown = None;
            state.icon_shown.store(false, Ordering::Relaxed);
            state.refresh();
        }),
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
    LRESULT(0)
}

fn main_window() -> Option<HWND> {
    STATE.with(|state| state.borrow().as_ref().and_then(|state| state.main_window))
}

fn open_main_window() {
    let Some(main) = main_window() else { return };
    unsafe {
        let _ = ShowWindow(main, if IsIconic(main).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(main);
    }
}

fn show_menu(window: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("Open Thermal Eye"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("Exit"));
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        // Without this the menu stays open when the user clicks elsewhere.
        let _ = SetForegroundWindow(window);
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
        let chosen = TrackPopupMenu(menu, flags, cursor.x, cursor.y, None, window, None).0 as usize;
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match chosen {
            MENU_OPEN => open_main_window(),
            // Closed the normal way, so the settings are saved.
            MENU_EXIT => {
                if let Some(main) = main_window() {
                    let _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            _ => {}
        }
    }
}

fn app_icon(size: i32) -> Option<HICON> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let id = PCWSTR(APP_ICON_ID as usize as *const u16);
        let image = LoadImageW(Some(instance.into()), id, IMAGE_ICON, size, size, LR_DEFAULTCOLOR).ok()?;
        Some(HICON(image.0))
    }
}

fn draw_icon(text: &str, color: Color32, size: i32) -> Option<HICON> {
    icon_from_pixels(&tile(text, color, size)?, size)
}

/// The number, as large as fits, centred on a rounded dark tile. The tile keeps
/// it legible on light and dark taskbars alike. Pixels are BGRA, top row first.
fn tile(text: &str, color: Color32, size: i32) -> Option<Vec<u8>> {
    let text: Vec<u16> = text.encode_utf16().collect();
    let pad = (size / 12).max(1);
    let room = size - 2 * pad;
    // Full height would squash two digits into posts; this keeps them in proportion.
    let tallest = size * 3 / 4;
    // Drawn on a larger canvas first, to measure the ink without clipping it.
    let canvas = Canvas { width: size * 3, height: size * 2, origin: size / 2 };
    let mut em = size * 3 / 2;
    let (coverage, ink) = 'fit: loop {
        // Narrower digits before smaller ones, as height matters more at this size.
        for squeeze in NARROWING {
            let coverage = canvas.text(&text, em, em * squeeze / 100)?;
            let ink = canvas.ink(&coverage);
            let fits = ink.is_none_or(|[left, top, right, bottom]| right - left <= room && bottom - top <= tallest);
            if fits || em <= 6 {
                break 'fit (coverage, ink?);
            }
        }
        em -= 1;
    };

    let [left, top, right, bottom] = ink;
    let shift_x = (size - (right - left)) / 2 - left;
    let shift_y = (size - (bottom - top)) / 2 - top;
    let radius = size as f32 * 0.22;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = (x - shift_x, y - shift_y);
            let ink = if (0..canvas.width).contains(&cx) && (0..canvas.height).contains(&cy) {
                coverage[(cy * canvas.width + cx) as usize] as f32 / 255.0
            } else {
                0.0
            };
            let tile = rounded_square(x as f32 + 0.5, y as f32 + 0.5, size as f32, radius);
            let mix = |back: u8, front: u8| (back as f32 + (front as f32 - back as f32) * ink).round() as u8;
            let alpha = tile.max(ink);
            pixels.extend([
                mix(TITLE_BG.b(), color.b()),
                mix(TITLE_BG.g(), color.g()),
                mix(TITLE_BG.r(), color.r()),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
    Some(pixels)
}

/// Coverage of a rounded square at a pixel centre, softened over one pixel.
fn rounded_square(x: f32, y: f32, size: f32, radius: f32) -> f32 {
    let half = size / 2.0;
    let qx = (x - half).abs() - (half - radius);
    let qy = (y - half).abs() - (half - radius);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    let distance = outside + qx.max(qy).min(0.0) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// An off-screen bitmap for measuring and drawing text in grey levels.
struct Canvas {
    width: i32,
    height: i32,
    /// Where the text starts, leaving room for overhang to the left and above.
    origin: i32,
}

impl Canvas {
    /// One coverage byte per pixel, top row first.
    /// A `width` of zero keeps the font's own proportions.
    fn text(&self, text: &[u16], em: i32, width: i32) -> Option<Vec<u8>> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let (bitmap, bits) = dib(self.width, self.height)?;
            let old_bitmap = SelectObject(dc, bitmap.into());
            let font = CreateFontW(
                -em,
                width,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                // Grey-level smoothing: ClearType's colour fringes would show on the tile.
                ANTIALIASED_QUALITY,
                0,
                FONT,
            );
            let old_font = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00ff_ffff));
            let _ = TextOutW(dc, self.origin, self.origin / 2, text);
            let _ = GdiFlush();
            let length = (self.width * self.height * 4) as usize;
            let (pixels, _) = std::slice::from_raw_parts(bits, length).as_chunks::<4>();
            let coverage = pixels.iter().map(|pixel| pixel[1]).collect();
            SelectObject(dc, old_font);
            SelectObject(dc, old_bitmap);
            let _ = DeleteObject(font.into());
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            Some(coverage)
        }
    }

    /// Left, top, right and bottom of the drawn pixels, the last two exclusive.
    fn ink(&self, coverage: &[u8]) -> Option<[i32; 4]> {
        let mut bounds: Option<[i32; 4]> = None;
        for (index, _) in coverage.iter().enumerate().filter(|(_, value)| **value > 0) {
            let (x, y) = (index as i32 % self.width, index as i32 / self.width);
            let b = bounds.get_or_insert([x, y, x + 1, y + 1]);
            *b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
        }
        bounds
    }
}

/// A 32-bit top-down bitmap and its pixels, which start out black.
unsafe fn dib(width: i32, height: i32) -> Option<(HBITMAP, *mut u8)> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        let _ = unsafe { DeleteObject(bitmap.into()) };
        return None;
    }
    Some((bitmap, bits.cast()))
}

fn icon_from_pixels(pixels: &[u8], size: i32) -> Option<HICON> {
    unsafe {
        let (color, bits) = dib(size, size)?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits, pixels.len());
        // The alpha channel does the masking; this mask only has to exist. Rows of a
        // one-bit bitmap are padded to 16 bits.
        let mask_bits = vec![0u8; ((size + 15) / 16 * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;
