//! Palette, fonts and small drawing helpers shared by the widget.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::Arc;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    Align2, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect,
    Shape, Stroke, StrokeKind, Theme, Visuals, pos2, vec2,
};

use crate::model::Kind;

pub const TITLE_BG: Color32 = Color32::from_rgb(0x17, 0x17, 0x23);
pub const BODY_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x34);
pub const HOVER_BG: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x3f);
pub const NESTED_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2d);
pub const BUTTON_BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x31);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x33, 0x33, 0x44);
pub const BORDER: Color32 = Color32::from_rgb(0x36, 0x36, 0x48);
pub const TRACK: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x4a);
pub const GRID: Color32 = Color32::from_rgb(0x30, 0x30, 0x43);
pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xee, 0xf5);
pub const MUTED: Color32 = Color32::from_rgb(0xa4, 0xa4, 0xb8);
pub const FAINT: Color32 = Color32::from_rgb(0x74, 0x74, 0x8c);
pub const PURPLE: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
pub const BLUE: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
pub const GREEN: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99);
pub const PINK: Color32 = Color32::from_rgb(0xf4, 0x72, 0xb6);
pub const YELLOW: Color32 = Color32::from_rgb(0xfa, 0xcc, 0x15);
pub const AMBER: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);
pub const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);

/// Colours for individually graphed sensors, cycled in selection order.
pub const SERIES: [Color32; 8] = [
    Color32::from_rgb(0x22, 0xd3, 0xee),
    Color32::from_rgb(0xfb, 0x92, 0x3c),
    Color32::from_rgb(0xa3, 0xe6, 0x35),
    Color32::from_rgb(0xe8, 0x79, 0xf9),
    Color32::from_rgb(0xf8, 0x71, 0x71),
    Color32::from_rgb(0x2d, 0xd4, 0xbf),
    Color32::from_rgb(0xc0, 0x84, 0xfc),
    Color32::from_rgb(0xfd, 0xe0, 0x47),
];

pub fn component_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Cpu => PURPLE,
        Kind::Gpu => BLUE,
        Kind::Memory => GREEN,
        Kind::Storage => PINK,
        Kind::System => YELLOW,
    }
}

/// Colour of a temperature relative to the component's limits. Normal readings
/// stay quiet, so colour on the page always means something needs attention.
pub fn heat_text(value: f32, warn: f32, critical: f32) -> Color32 {
    if value >= critical {
        RED
    } else if value >= warn {
        AMBER
    } else {
        TEXT
    }
}

pub fn heat_ring(value: f32, warn: f32, critical: f32) -> Color32 {
    if value >= critical {
        RED
    } else if value >= warn {
        AMBER
    } else {
        BLUE.lerp_to_gamma(PURPLE, (value / warn).clamp(0.0, 1.0).powi(2))
    }
}

/// Segoe Fluent Icons code points.
pub mod icon {
    pub const PIN: char = '\u{E718}';
    pub const PINNED: char = '\u{E840}';
    pub const SETTINGS: char = '\u{E713}';
    pub const RESET: char = '\u{E72C}';
    pub const GRAPH: char = '\u{E9D2}';
    pub const MINIMIZE: char = '\u{E921}';
    pub const CLOSE: char = '\u{E8BB}';
    pub const COMPACT: char = '\u{E73F}';
    pub const FULL: char = '\u{E740}';
    pub const CHEVRON_RIGHT: char = '\u{E76C}';
    pub const CHEVRON_DOWN: char = '\u{E70D}';
}

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("strong".into()))
}

pub fn title(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("title".into()))
}

pub fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

/// The chrome is dark whatever the Windows theme, so tooltips and scrollbars must be too.
pub fn install_style(ctx: &Context) {
    ctx.set_theme(Theme::Dark);
    let mut visuals = Visuals::dark();
    visuals.window_fill = TITLE_BG;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.panel_fill = BODY_BG;
    ctx.set_visuals_of(Theme::Dark, visuals);
}

/// Uses the fonts Windows ships rather than bundling any. A missing font only
/// costs that family's look: it falls back to egui's built-in face.
pub fn install_fonts(ctx: &Context) {
    const FONTS: &str = r"C:\Windows\Fonts\";
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families[&FontFamily::Proportional].clone();

    let mut add = |name: &str, files: &[&str]| {
        files.iter().find_map(|file| std::fs::read(format!("{FONTS}{file}")).ok()).map(|bytes| {
            fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
            name.to_owned()
        })
    };
    let segoe = add("segoe", &["segoeui.ttf"]);
    let semibold = add("semibold", &["seguisb.ttf", "segoeui.ttf"]);
    let georgia = add("georgia", &["georgiab.ttf", "georgia.ttf"]);
    let symbols = add("symbols", &["SegoeIcons.ttf", "segmdl2.ttf"]);

    let chain = |first: Option<String>| first.into_iter().chain(fallback.iter().cloned()).collect::<Vec<_>>();
    fonts.families.insert(FontFamily::Proportional, chain(segoe));
    fonts.families.insert(FontFamily::Name("strong".into()), chain(semibold));
    fonts.families.insert(FontFamily::Name("title".into()), chain(georgia));
    fonts.families.insert(FontFamily::Name("icons".into()), chain(symbols));
    ctx.set_fonts(fonts);
}

/// Small letter-spaced capitals, used for column headers.
pub fn caps(painter: &Painter, pos: Pos2, align: Align2, text: &str, color: Color32) {
    let mut job = LayoutJob::default();
    let format = TextFormat {
        font_id: strong(10.0),
        color,
        extra_letter_spacing: 0.9,
        ..Default::default()
    };
    job.append(text, 0.0, format);
    let galley = painter.layout_job(job);
    let rect = align.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
}

pub fn progress(painter: &Painter, rect: Rect, fraction: f32, color: Color32) {
    painter.rect_filled(rect, rect.height() / 2.0, TRACK);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let width = (rect.width() * fraction).max(rect.height());
        let fill = Rect::from_min_size(rect.min, vec2(width, rect.height()));
        painter.rect_filled(fill, rect.height() / 2.0, color);
    }
}

pub fn ring(painter: &Painter, center: Pos2, radius: f32, fraction: f32, color: Color32) {
    painter.circle_stroke(center, radius, Stroke::new(2.5, TRACK));
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.01 {
        let steps = (fraction * 48.0).ceil() as usize + 1;
        let points = (0..=steps)
            .map(|i| {
                let angle = -FRAC_PI_2 + TAU * fraction * i as f32 / steps as f32;
                center + radius * vec2(angle.cos(), angle.sin())
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(2.5, color)));
    }
}

/// A checkable pill. `on` controls the fill.
pub fn chip_painter(painter: &Painter, rect: Rect, text: &str, on: bool, hovered: bool) {
    let fill = match (on, hovered) {
        (true, _) => PURPLE.gamma_multiply(0.32),
        (false, true) => BUTTON_HOVER,
        (false, false) => BUTTON_BG,
    };
    painter.rect_filled(rect, 5.0, fill);
    if on {
        painter.rect_stroke(rect, 5.0, Stroke::new(1.0, PURPLE.gamma_multiply(0.7)), StrokeKind::Inside);
    }
    let color = if on { TEXT } else { MUTED };
    painter.text(rect.center(), Align2::CENTER_CENTER, text, strong(11.0), color);
}

pub fn point_at(rect: Rect, x: f32, y: f32) -> Pos2 {
    pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height())
}
