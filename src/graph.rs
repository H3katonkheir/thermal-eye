//! The history plot: temperatures and usage share a 0-100 scale, with degrees on
//! the left axis and percent on the right.

use eframe::egui::{Align2, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, pos2, vec2};

use crate::theme::{self, BORDER, FAINT, GRID, MUTED, TEXT, TITLE_BG, body, strong};

pub struct Line {
    pub label: String,
    pub color: Color32,
    /// Evenly spaced across the window, oldest first; `NaN` marks a gap.
    pub points: Vec<f32>,
    /// Position of the first point among every point the series has had, which
    /// keeps dashes fixed to the data as the graph slides.
    pub first: i64,
    pub is_usage: bool,
    pub weight: f32,
}

const LEFT_AXIS: f32 = 36.0;
const RIGHT_AXIS: f32 = 36.0;
const BOTTOM_AXIS: f32 = 18.0;
const SCALE_MAX: f32 = 100.0;
pub const TOOLTIP_LINES: usize = 12;
/// Usage is dashed over a faint solid line, so a brief spike cannot fall into a gap.
const USAGE_DASH_OPACITY: f32 = 0.85;
const USAGE_UNDER_OPACITY: f32 = 0.3;
const DASH: f32 = 5.0;
const DASH_PERIOD: f32 = 8.0;
/// Roughly how far apart the dash pattern restarts.
const DASH_CHUNK: f32 = 48.0;

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
        let count = line.points.len();
        let spacing = plot.width() / (count.max(2) - 1) as f32;
        let chunk = (DASH_CHUNK / spacing).round().max(1.0) as i64;
        // Runs of points between gaps, each point with its position in the series.
        let mut run: Vec<(i64, Pos2)> = Vec::new();
        let mut run_start = 0;
        for (index, &value) in line.points.iter().chain(std::iter::once(&f32::NAN)).enumerate() {
            if !value.is_nan() {
                if run.is_empty() {
                    run_start = index;
                }
                run.push((line.first + index as i64, to_screen(plot, index, count, value)));
                continue;
            }
            if run.len() > 1 {
                let points: Vec<Pos2> = run.iter().map(|(_, pos)| *pos).collect();
                if line.is_usage {
                    let under = Stroke::new(line.weight, line.color.gamma_multiply(USAGE_UNDER_OPACITY));
                    line_painter.add(Shape::line(points, under));
                    let stroke = Stroke::new(line.weight, line.color.gamma_multiply(USAGE_DASH_OPACITY));
                    for dash in dashes(&run, chunk, run_start == 0) {
                        line_painter.add(Shape::line(dash, stroke));
                    }
                } else {
                    line_painter.add(Shape::line(points, Stroke::new(line.weight, line.color)));
                }
            }
            run.clear();
        }
    }

    if let Some(pointer) = response.hover_pos().filter(|p| plot.expand2(vec2(0.0, 6.0)).contains(*p)) {
        hover(ui, rect, plot, pointer, lines, seconds, &format_temp);
    }
    response
}

/// Dashes along a run of points, fixed to the data so that they slide with it
/// instead of crawling along the line.
///
/// The pattern restarts at every `chunk`th point of the series, so each stretch
/// between restarts is dashed the same way for as long as it shows. A stretch cut
/// short by the left edge is measured back from its end instead, which stays put.
fn dashes(run: &[(i64, Pos2)], chunk: i64, at_left_edge: bool) -> Vec<Vec<Pos2>> {
    let mut stretches: Vec<Vec<Pos2>> = vec![Vec::new()];
    for (index, &(number, pos)) in run.iter().enumerate() {
        let current = stretches.last_mut().unwrap();
        current.push(pos);
        if number.rem_euclid(chunk) == 0 && index > 0 && index + 1 < run.len() {
            stretches.push(vec![pos]);
        }
    }
    let cut_by_edge = at_left_edge && run.first().is_some_and(|(number, _)| number.rem_euclid(chunk) != 0);
    if cut_by_edge {
        stretches[0].reverse();
    }

    let mut out = Vec::new();
    for stretch in &stretches {
        dash_stretch(stretch, &mut out);
    }
    out
}

/// Splits a polyline into dashes, starting with a dash at its first point.
fn dash_stretch(points: &[Pos2], out: &mut Vec<Vec<Pos2>>) {
    let mut dash: Vec<Pos2> = Vec::new();
    let mut walked = 0.0;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = a.distance(b);
        let mut along = 0.0;
        while along < length {
            let phase = (walked + along) % DASH_PERIOD;
            let drawing = phase < DASH;
            let to_switch = if drawing { DASH - phase } else { DASH_PERIOD - phase };
            let step = to_switch.min(length - along).max(1e-3);
            if drawing {
                if dash.is_empty() {
                    dash.push(a.lerp(b, along / length));
                }
                dash.push(a.lerp(b, ((along + step) / length).min(1.0)));
            } else if !dash.is_empty() {
                out.push(std::mem::take(&mut dash));
            }
            along += step;
        }
        walked += length;
    }
    if dash.len() > 1 {
        out.push(dash);
    }
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
    let (row, heading_h, margin) = (16.0, 24.0, 2.0);
    // As many lines as fit inside the graph, which is shorter than TOOLTIP_LINES needs.
    let room = ((rect.height() - heading_h - 2.0 * margin) / row).floor().max(1.0) as usize;
    entries.truncate(TOOLTIP_LINES.min(room));

    let size = vec2(170.0, heading_h + row * entries.len() as f32);
    let side = if pointer.x + 12.0 + size.x < plot.right() { 12.0 } else { -12.0 - size.x };
    // Kept inside the graph; a tooltip taller than it starts at the top instead.
    let highest = rect.top() + margin;
    let lowest = (rect.bottom() - size.y - margin).max(highest);
    let top = (pointer.y - size.y / 2.0).clamp(highest, lowest);
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

#[cfg(test)]
mod tests {
    use super::*;

    const SPACING: f32 = 3.0;

    /// Points `from..=to` of a wavy series, drawn with point `from` at the left edge.
    fn window(from: i64, to: i64) -> Vec<(i64, Pos2)> {
        let value = |n: i64| 50.0 + 30.0 * ((n * 37 % 101) as f32 / 101.0 - 0.5);
        (from..=to).map(|n| (n, pos2((n - from) as f32 * SPACING, value(n)))).collect()
    }

    /// The same dash, give or take floating point error.
    fn same(a: &[Pos2], b: &[Pos2]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.distance(*b) < 0.01)
    }

    #[test]
    fn dashes_slide_with_the_data_instead_of_crawling() {
        let chunk = 16;
        for start in [100, 101, 112, 113] {
            let before = dashes(&window(start, start + 150), chunk, true);
            // One point later: the same data, one spacing further left.
            let after = dashes(&window(start + 1, start + 151), chunk, true);
            let after: Vec<Vec<Pos2>> =
                after.iter().map(|dash| dash.iter().map(|p| *p + vec2(SPACING, 0.0)).collect()).collect();
            for dash in &before {
                // The oldest stretch is redrawn once its starting point scrolls off,
                // and the dash at the newest point grows with the next one. The rest
                // must stay exactly where they were on the data.
                let (oldest, newest) = ((chunk + 1) as f32 * SPACING, 150.0 * SPACING);
                if dash.iter().all(|p| p.x > oldest + 0.01 && p.x < newest - 0.01) {
                    assert!(after.iter().any(|moved| same(dash, moved)), "start {start}: a dash moved along the line");
                }
            }
        }
    }

    #[test]
    fn dashes_cover_about_their_share_of_the_line() {
        let run = window(0, 200);
        let length: f32 = run.windows(2).map(|w| w[0].1.distance(w[1].1)).sum();
        let drawn: f32 = dashes(&run, 16, true)
            .iter()
            .map(|dash| dash.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>())
            .sum();
        let share = drawn / length;
        assert!((0.5..0.75).contains(&share), "dashes cover {share:.2} of the line");
    }

    #[test]
    fn a_short_run_still_gets_a_dash() {
        let run = [(5, pos2(0.0, 0.0)), (6, pos2(3.0, 0.0))];
        assert_eq!(dashes(&run, 16, false).len(), 1);
    }
}
