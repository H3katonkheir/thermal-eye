//! The history plot: temperatures and usage share a 0-100 scale, with degrees on
//! the left axis and percent on the right.

use eframe::egui::{Align2, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, pos2, vec2};

use crate::theme::{self, BORDER, FAINT, GRID, MUTED, TEXT, TITLE_BG, body, strong};

pub struct Line {
    pub label: String,
    pub color: Color32,
    /// Evenly spaced across the window, oldest first; `NaN` marks a gap.
    pub points: Vec<f32>,
    pub is_usage: bool,
    pub weight: f32,
}

const LEFT_AXIS: f32 = 36.0;
const RIGHT_AXIS: f32 = 36.0;
const BOTTOM_AXIS: f32 = 18.0;
const SCALE_MAX: f32 = 100.0;
const TOOLTIP_LINES: usize = 12;
const USAGE_OPACITY: f32 = 0.5;

fn to_screen(plot: Rect, index: usize, count: usize, value: f32) -> Pos2 {
    let x = index as f32 / (count.max(2) - 1) as f32;
    let y = 1.0 - value.clamp(0.0, SCALE_MAX) / SCALE_MAX;
    theme::point_at(plot, x, y)
}

fn age_label(seconds: f32) -> String {
    let seconds = seconds.round() as u32;
    match seconds {
        0 => "now".into(),
        1..=119 => format!("{seconds}s"),
        120..=7199 => format!("{}m", seconds / 60),
        _ => format!("{}h", seconds / 3600),
    }
}

pub fn draw(ui: &mut Ui, rect: Rect, lines: &[Line], seconds: u32, format_temp: impl Fn(f32) -> String) -> Response {
    let response = ui.allocate_rect(rect, Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    let plot = Rect::from_min_max(
        pos2(rect.left() + LEFT_AXIS, rect.top() + 6.0),
        pos2(rect.right() - RIGHT_AXIS, rect.bottom() - BOTTOM_AXIS),
    );

    for step in 0..=4 {
        let value = step as f32 * 25.0;
        let y = to_screen(plot, 0, 2, value).y;
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, GRID));
        let left = pos2(plot.left() - 7.0, y);
        painter.text(left, Align2::RIGHT_CENTER, format_temp(value), body(10.5), FAINT);
        let right = pos2(plot.right() + 7.0, y);
        painter.text(right, Align2::LEFT_CENTER, format!("{value:.0}%"), body(10.5), FAINT);
    }

    for step in 0..=4 {
        let fraction = step as f32 / 4.0;
        let x = plot.left() + plot.width() * fraction;
        let align = match step {
            0 => Align2::LEFT_BOTTOM,
            4 => Align2::RIGHT_BOTTOM,
            _ => Align2::CENTER_BOTTOM,
        };
        let label = age_label(seconds as f32 * (1.0 - fraction));
        painter.text(pos2(x, rect.bottom()), align, label, body(10.5), FAINT);
    }

    if lines.is_empty() {
        let hint = "Use the dots beside each row to choose what to plot";
        painter.text(plot.center(), Align2::CENTER_CENTER, hint, body(12.0), FAINT);
        return response;
    }

    let line_painter = painter.with_clip_rect(plot.expand(2.0));
    for line in lines {
        // Fainter rather than dashed: a dash pattern is laid along the line, so it
        // would crawl over every peak each time the graph slides.
        let color = if line.is_usage { line.color.gamma_multiply(USAGE_OPACITY) } else { line.color };
        let stroke = Stroke::new(line.weight, color);
        let mut run: Vec<Pos2> = Vec::new();
        for (index, &value) in line.points.iter().chain(std::iter::once(&f32::NAN)).enumerate() {
            if value.is_nan() {
                if run.len() > 1 {
                    line_painter.add(Shape::line(std::mem::take(&mut run), stroke));
                }
                run.clear();
            } else {
                run.push(to_screen(plot, index, line.points.len(), value));
            }
        }
    }

    if let Some(pointer) = response.hover_pos().filter(|p| plot.expand2(vec2(0.0, 6.0)).contains(*p)) {
        hover(ui, rect, plot, pointer, lines, seconds, &format_temp);
    }
    response
}

fn hover(ui: &Ui, rect: Rect, plot: Rect, pointer: Pos2, lines: &[Line], seconds: u32, format_temp: &impl Fn(f32) -> String) {
    let fraction = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let painter = ui.painter().with_clip_rect(rect);
    painter.line_segment(
        [pos2(pointer.x, plot.top()), pos2(pointer.x, plot.bottom())],
        Stroke::new(1.0, MUTED.gamma_multiply(0.6)),
    );

    let mut entries = Vec::new();
    for line in lines {
        let last = line.points.len().saturating_sub(1);
        let index = (fraction * last as f32).round() as usize;
        let Some(&value) = line.points.get(index).filter(|v| !v.is_nan()) else { continue };
        painter.circle_filled(to_screen(plot, index, line.points.len(), value), 3.0, line.color);
        let text = if line.is_usage { format!("{value:.0}%") } else { format_temp(value) };
        entries.push((line.color, line.label.as_str(), text));
    }
    if entries.is_empty() {
        return;
    }
    entries.truncate(TOOLTIP_LINES);

    let row = 16.0;
    let size = vec2(170.0, 24.0 + row * entries.len() as f32);
    let side = if pointer.x + 12.0 + size.x < plot.right() { 12.0 } else { -12.0 - size.x };
    let top = (pointer.y - size.y / 2.0).clamp(rect.top() + 2.0, (rect.bottom() - size.y - 2.0).max(rect.top()));
    let panel = Rect::from_min_size(pos2(pointer.x + side, top), size);

    painter.rect_filled(panel, 6.0, TITLE_BG.gamma_multiply(0.96));
    painter.rect_stroke(panel, 6.0, Stroke::new(1.0, BORDER), eframe::egui::StrokeKind::Inside);
    let age = age_label(seconds as f32 * (1.0 - fraction));
    let heading = if age == "now" { age } else { format!("{age} ago") };
    painter.text(panel.left_top() + vec2(9.0, 6.0), Align2::LEFT_TOP, heading, body(10.5), FAINT);
    for (index, (color, label, value)) in entries.iter().enumerate() {
        let y = panel.top() + 28.0 + row * index as f32;
        painter.circle_filled(pos2(panel.left() + 12.0, y), 3.0, *color);
        painter.text(pos2(panel.left() + 21.0, y), Align2::LEFT_CENTER, label, body(11.5), MUTED);
        painter.text(pos2(panel.right() - 9.0, y), Align2::RIGHT_CENTER, value, strong(11.5), TEXT);
    }
}
