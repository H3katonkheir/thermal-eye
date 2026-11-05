//! The app's images, generated from the source logo by `tools/make_icons.py`.

use eframe::egui::IconData;

/// The logo on a transparent background, for the title bar.
pub const MARK: &[u8] = include_bytes!("../assets/logo.png");

/// The logo on a dark tile, which stays legible on light taskbars.
pub const ICON: &[u8] = include_bytes!("../assets/icon.png");

pub fn decode(png: &[u8]) -> IconData {
    eframe::icon_data::from_png_bytes(png).expect("bundled image is a valid PNG")
}
