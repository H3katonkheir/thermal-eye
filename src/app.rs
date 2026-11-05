//! The widget: custom window chrome, component table, graph and settings.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, ColorImage, Context, Rect, ScrollArea, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Ui, Vec2, ViewportCommand, WindowLevel, pos2, vec2,
};

use crate::bridge::Command;
use crate::config::{self, Config};
use crate::graph::{self, Line};
use crate::history::Stats;
use crate::logo;
use crate::model::{Component, Core, Extra, Link, Monitor, Row, Unit};
use crate::theme::*;

pub const WIDTH: f32 = 600.0;
pub const COMPACT_WIDTH: f32 = 336.0;
pub const INITIAL_HEIGHT: f32 = 340.0;

const PAD: f32 = 14.0;
const TITLE_H: f32 = 50.0;
const COMPACT_TITLE_H: f32 = 42.0;
const BANNER_H: f32 = 34.0;
const HEADER_H: f32 = 26.0;
const COMPONENT_H: f32 = 44.0;
const SENSOR_H: f32 = 25.0;
const CORE_HEAD_H: f32 = 20.0;
const EXTRA_LINE_H: f32 = 19.0;
const EXTRA_PAD: f32 = 12.0;
const EXTRA_COLUMNS: usize = 4;
const GRAPH_CONTROLS_H: f32 = 34.0;
const GRAPH_PLOT_H: f32 = 190.0;
const SETTINGS_ROW_H: f32 = 34.0;
const COMPACT_HEAD_H: f32 = 20.0;
const COMPACT_ROW_H: f32 = 29.0;
const BOTTOM_PAD: f32 = 10.0;
/// Space kept free around the window when it grows towards the screen height.
const SCREEN_MARGIN: f32 = 90.0;

// Column anchors, measured from the left edge of the content.
const X_NAME: f32 = 20.0;
const X_RING: f32 = 204.0;
const X_NOW: f32 = 256.0;
const X_HIGH: f32 = 310.0;
const X_AVG: f32 = 362.0;
const X_BAR_LEFT: f32 = 380.0;
const X_BAR_RIGHT: f32 = 498.0;
const X_PERCENT: f32 = 538.0;
const X_DOT: f32 = 558.0;

const WINDOWS: [(&str, u32, f32); 7] = [
    ("Live", 1, 32.0),
    ("1m", 60, 24.0),
    ("5m", 300, 24.0),
    ("15m", 900, 29.0),
    ("1h", 3600, 24.0),
    ("6h", 21600, 24.0),
    ("24h", 86400, 29.0),
];
const MIN_GRAPH_SECONDS: u32 = 60;

enum Item<'a> {
    Component(&'a Component),
    Sensor { component: &'a Component, row: &'a Row, note: Option<&'a str>, die: bool },
    CoreHead,
    Core(&'a Core),
    Extras(Vec<(String, String)>),
}

impl Item<'_> {
    fn height(&self) -> f32 {
        match self {
            Item::Component(_) => COMPONENT_H,
            Item::Sensor { .. } | Item::Core(_) => SENSOR_H,
            Item::CoreHead => CORE_HEAD_H,
            Item::Extras(values) => values.len().div_ceil(EXTRA_COLUMNS) as f32 * EXTRA_LINE_H + EXTRA_PAD,
        }
    }
}

pub struct ThermalEye {
    monitor: Arc<Mutex<Monitor>>,
    commands: Sender<Command>,
    config: Config,
    saved: Config,
    last_save: Instant,
    logo: TextureHandle,
    settings_open: bool,
    requested_size: Vec2,
}

impl ThermalEye {
    pub fn new(ctx: &Context, monitor: Arc<Mutex<Monitor>>, commands: Sender<Command>, config: Config) -> Self {
        let (size, pixels) = logo::pixels();
        let image = ColorImage::from_rgba_unmultiplied([size, size], &pixels);
        Self {
            monitor,
            commands,
            saved: config.clone(),
            config,
            last_save: Instant::now(),
            logo: ctx.load_texture("logo", image, TextureOptions::LINEAR),
            settings_open: false,
            requested_size: Vec2::ZERO,
        }
    }

    fn format_temp(fahrenheit: bool, celsius: f32) -> String {
        let value = if fahrenheit { celsius * 9.0 / 5.0 + 32.0 } else { celsius };
        format!("{value:.0}°")
    }

    fn temp(&self, value: Option<f32>) -> String {
        value.map_or("—".into(), |v| Self::format_temp(self.config.fahrenheit, v))
    }

    fn is_graphed(&self, key: &str) -> bool {
        config::contains(&self.config.graphed, key)
    }

    /// Sensors keep a stable colour for as long as they stay selected.
    fn series_color(&self, key: &str) -> Color32 {
        let position = self.config.graphed.iter().position(|k| k == key).unwrap_or(0);
        SERIES[position % SERIES.len()]
    }

    fn visible<'m>(&self, monitor: &'m Monitor) -> Vec<&'m Component> {
        let hidden = &self.config.hidden;
        monitor.components.iter().filter(|c| !config::contains(hidden, &c.key)).collect()
    }

    fn items<'m>(&self, monitor: &'m Monitor) -> Vec<Item<'m>> {
        let has_data = |row: &Row| {
            let usable = |id: Option<usize>| id.is_some_and(|id| monitor.history.series(id).has_data());
            usable(row.temp) || usable(row.usage)
        };

        let mut items = Vec::new();
        for component in self.visible(monitor) {
            items.push(Item::Component(component));
            if !config::contains(&self.config.expanded, &component.key) {
                continue;
            }
            for row in component.rows.iter().filter(|r| has_data(r)) {
                items.push(Item::Sensor { component, row, note: None, die: false });
            }
            for die in &component.dies {
                let note = die.note.as_deref();
                items.push(Item::Sensor { component, row: &die.row, note, die: true });
                items.push(Item::CoreHead);
                items.extend(die.cores.iter().map(|core| Item::Core(core)));
            }
            let extras = extra_values(monitor, component);
            if !extras.is_empty() {
                items.push(Item::Extras(extras));
            }
        }
        items
    }

    fn graph_lines(&self, monitor: &Monitor, seconds: u32, columns: usize) -> Vec<Line> {
        let metrics = &self.config.metrics;
        let series = |id: usize, peak: bool| monitor.history.series(id).points(seconds, columns, peak);
        let mut lines = Vec::new();
        let row_lines = |lines: &mut Vec<Line>, title: &str, row: &Row| {
            if !self.is_graphed(&row.key) {
                return;
            }
            let color = self.series_color(&row.key);
            if let Some(id) = row.temp {
                let label = format!("{title} {}", row.label);
                lines.push(Line { label, color, points: series(id, false), is_usage: false, weight: 1.6 });
            }
            if let (Some(id), true) = (row.usage, metrics.usage) {
                let label = format!("{} usage", row.label);
                lines.push(Line { label, color, points: series(id, false), is_usage: true, weight: 1.3 });
            }
        };

        for component in self.visible(monitor) {
            let title = &component.title;
            if self.is_graphed(&component.key) {
                let color = component_color(component.kind);
                if metrics.high {
                    let points = series(component.high, true);
                    lines.push(Line { label: format!("{title} high"), color, points, is_usage: false, weight: 2.0 });
                }
                if metrics.avg {
                    let points = series(component.avg, false);
                    let color = color.gamma_multiply(0.7);
                    lines.push(Line { label: format!("{title} avg"), color, points, is_usage: false, weight: 1.4 });
                }
                if let (Some(id), true) = (component.usage, metrics.usage) {
                    let points = series(id, false);
                    lines.push(Line { label: format!("{title} usage"), color, points, is_usage: true, weight: 1.4 });
                }
            }
            for row in &component.rows {
                row_lines(&mut lines, title, row);
            }
            for die in &component.dies {
                row_lines(&mut lines, title, &die.row);
                for core in die.cores.iter().filter(|c| self.is_graphed(&c.key)) {
                    lines.push(Line {
                        label: format!("{} usage", core.label),
                        color: self.series_color(&core.key),
                        points: series(core.usage, false),
                        is_usage: true,
                        weight: 1.3,
                    });
                }
            }
        }
        lines
    }

    fn icon_button(&self, ui: &mut Ui, rect: Rect, glyph: char, active: bool, tip: &str) -> bool {
        let response = ui.interact(rect, ui.scope_id().with(("button", glyph)), Sense::click()).on_hover_text(tip);
        let fill = match (active, response.hovered()) {
            (true, _) => PURPLE.gamma_multiply(0.32),
            (false, true) => BUTTON_HOVER,
            (false, false) => BUTTON_BG,
        };
        let edge = if active { PURPLE.gamma_multiply(0.7) } else { BORDER };
        let painter = ui.painter();
        painter.rect_filled(rect, 6.0, fill);
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, edge), StrokeKind::Inside);
        let color = if active || response.hovered() { TEXT } else { MUTED };
        painter.text(rect.center(), Align2::CENTER_CENTER, glyph.to_string(), icons(13.0), color);
        response.clicked()
    }

    fn title_bar(&mut self, ui: &mut Ui, width: f32, reset: &mut bool) {
        let compact = self.config.compact;
        let height = if compact { COMPACT_TITLE_H } else { TITLE_H };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, TITLE_BG);
        let mark = if compact { 22.0 } else { 26.0 };
        let mark_rect = Rect::from_center_size(pos2(rect.left() + PAD + mark / 2.0, rect.center().y), Vec2::splat(mark));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(self.logo.id(), mark_rect, uv, Color32::WHITE);
        let text_pos = pos2(mark_rect.right() + 10.0, rect.center().y);
        let size = if compact { 16.0 } else { 19.0 };
        painter.text(text_pos, Align2::LEFT_CENTER, "Thermal Eye", title(size), TEXT);

        let side = if compact { 26.0 } else { 30.0 };
        let gap = 6.0;
        let mut right = rect.right() - PAD;
        let mut next = || {
            let button = Rect::from_min_size(pos2(right - side, rect.center().y - side / 2.0), Vec2::splat(side));
            right -= side + gap;
            button
        };

        let (close, minimize) = (next(), next());
        if self.icon_button(ui, close, icon::CLOSE, false, "Close") {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        if self.icon_button(ui, minimize, icon::MINIMIZE, false, "Minimise") {
            ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
        }

        let size_button = next();
        let (glyph, tip) = if compact { (icon::FULL, "Full view") } else { (icon::COMPACT, "Compact view") };
        if self.icon_button(ui, size_button, glyph, false, tip) {
            self.config.compact = !compact;
        }

        let pin = next();
        let on_top = self.config.always_on_top;
        let glyph = if on_top { icon::PINNED } else { icon::PIN };
        if self.icon_button(ui, pin, glyph, on_top, "Keep on top") {
            self.config.always_on_top = !on_top;
            let level = if on_top { WindowLevel::Normal } else { WindowLevel::AlwaysOnTop };
            ui.ctx().send_viewport_cmd(ViewportCommand::WindowLevel(level));
        }

        if compact {
            return;
        }
        let (graph, refresh, settings) = (next(), next(), next());
        if self.icon_button(ui, graph, icon::GRAPH, self.config.graph_open, "Graph") {
            self.config.graph_open = !self.config.graph_open;
        }
        if self.icon_button(ui, refresh, icon::RESET, false, "Reset highs, averages and graph") {
            *reset = true;
        }
        if self.icon_button(ui, settings, icon::SETTINGS, self.settings_open, "Settings") {
            self.settings_open = !self.settings_open;
        }
    }

    fn banner(&self, ui: &mut Ui, width: f32, message: &str, action: Option<&str>) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, BANNER_H), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, NESTED_BG);
        let left = pos2(rect.left() + PAD, rect.center().y);
        painter.text(left, Align2::LEFT_CENTER, message, body(12.0), AMBER);
        if let Some(label) = action {
            let button = Rect::from_min_size(pos2(rect.right() - PAD - 96.0, rect.top() + 6.0), vec2(96.0, 22.0));
            let response = ui.interact(button, ui.scope_id().with("banner"), Sense::click());
            chip_painter(ui.painter(), button, label, false, response.hovered());
            if response.clicked() {
                let _ = self.commands.send(Command::Connect);
            }
        }
    }

    fn table_header(&mut self, ui: &mut Ui, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
        let left = rect.left() + PAD;
        let y = rect.center().y;

        let mut x = left;
        for (label, seconds, segment) in WINDOWS {
            let segment_rect = Rect::from_min_size(pos2(x, y - 9.0), vec2(segment, 18.0));
            let response = ui.interact(segment_rect, ui.scope_id().with(("window", seconds)), Sense::click());
            let selected = self.config.window_seconds == seconds;
            let painter = ui.painter();
            if selected {
                painter.rect_filled(segment_rect, 4.0, TRACK);
            } else if response.hovered() {
                painter.rect_filled(segment_rect, 4.0, HOVER_BG);
            }
            let color = if selected { TEXT } else { FAINT };
            painter.text(segment_rect.center(), Align2::CENTER_CENTER, label, strong(10.5), color);
            if response.clicked() {
                self.config.window_seconds = seconds;
            }
            x += segment;
        }

        let painter = ui.painter();
        let at = |offset: f32| pos2(left + offset, y);
        caps(painter, at(X_NOW), Align2::RIGHT_CENTER, "NOW", FAINT);
        caps(painter, at(X_HIGH), Align2::RIGHT_CENTER, "HIGH", FAINT);
        caps(painter, at(X_AVG), Align2::RIGHT_CENTER, "AVG", FAINT);
        caps(painter, at(X_BAR_LEFT), Align2::LEFT_CENTER, "USAGE", FAINT);
        let span = ui.interact(
            Rect::from_min_max(pos2(left + X_RING, rect.top()), pos2(left + X_PERCENT, rect.bottom())),
            ui.scope_id().with("column-help"),
            Sense::hover(),
        );
        span.on_hover_text(
            "NOW: hottest sensor right now\nHIGH: hottest any sensor got in the selected span\n\
             AVG: mean of the component's sensors over the span\nUSAGE: mean usage over the span",
        );
    }

    /// Bar and percentage for a usage series; a dash when there is none.
    fn usage_cell(&self, ui: &Ui, rect: Rect, value: Option<f32>, bar_height: f32, color: Color32) {
        let left = rect.left() + PAD;
        let y = rect.center().y;
        let painter = ui.painter();
        let Some(value) = value else {
            painter.text(pos2(left + X_PERCENT, y), Align2::RIGHT_CENTER, "—", body(12.5), FAINT);
            return;
        };
        let bar = Rect::from_min_max(
            pos2(left + X_BAR_LEFT, y - bar_height / 2.0),
            pos2(left + X_BAR_RIGHT, y + bar_height / 2.0),
        );
        progress(painter, bar, value / 100.0, color);
        let text = format!("{value:.0}%");
        painter.text(pos2(left + X_PERCENT, y), Align2::RIGHT_CENTER, text, strong(12.5), TEXT);
    }

    fn graph_dot(&mut self, ui: &mut Ui, rect: Rect, key: &str, color: Color32) {
        if !self.config.graph_open {
            return;
        }
        let center = pos2(rect.left() + PAD + X_DOT, rect.center().y);
        let hit = Rect::from_center_size(center, Vec2::splat(20.0));
        let response = ui.interact(hit, ui.scope_id().with(("dot", key)), Sense::click()).on_hover_text("Show on graph");
        let selected = self.is_graphed(key);
        let painter = ui.painter();
        if selected {
            painter.circle_filled(center, 5.0, color);
        } else {
            let edge = if response.hovered() { MUTED } else { FAINT };
            painter.circle_stroke(center, 4.5, Stroke::new(1.3, edge));
        }
        if response.clicked() {
            config::toggle(&mut self.config.graphed, key);
        }
    }

    fn component_row(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor, component: &Component) {
        let seconds = self.config.window_seconds;
        let (rect, response) = ui.allocate_exact_size(vec2(width, COMPONENT_H), Sense::click());
        let expandable = component.has_details();
        if response.hovered() && expandable {
            ui.painter().rect_filled(rect, 0.0, HOVER_BG);
        }
        if response.clicked() && expandable {
            config::toggle(&mut self.config.expanded, &component.key);
        }

        let left = rect.left() + PAD;
        let y = rect.center().y;
        let painter = ui.painter();
        if expandable {
            let open = config::contains(&self.config.expanded, &component.key);
            let glyph = if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT };
            painter.text(pos2(left + 7.0, y), Align2::CENTER_CENTER, glyph.to_string(), icons(9.0), FAINT);
        }
        painter.text(pos2(left + X_NAME, y - 8.0), Align2::LEFT_CENTER, &component.title, strong(14.0), TEXT);
        let subtitle = elide(&component.subtitle, 30);
        painter.text(pos2(left + X_NAME, y + 9.0), Align2::LEFT_CENTER, subtitle, body(11.0), FAINT);

        let high = monitor.stats(component.high, seconds);
        let avg = monitor.stats(component.avg, seconds);
        let (warn, critical) = (component.warn, component.critical);
        if let Some(now) = high.now {
            let fraction = now / critical;
            ring(painter, pos2(left + X_RING, y), 9.0, fraction, heat_ring(now, warn, critical));
        }
        let now_color = high.now.map_or(FAINT, |v| heat_text(v, warn, critical));
        painter.text(pos2(left + X_NOW, y), Align2::RIGHT_CENTER, self.temp(high.now), strong(16.0), now_color);
        let high_color = high.high.map_or(FAINT, |v| heat_text(v, warn, critical));
        painter.text(pos2(left + X_HIGH, y), Align2::RIGHT_CENTER, self.temp(high.high), strong(13.5), high_color);
        painter.text(pos2(left + X_AVG, y), Align2::RIGHT_CENTER, self.temp(avg.avg), body(13.5), MUTED);

        let usage = component.usage.and_then(|id| monitor.stats(id, seconds).avg);
        self.usage_cell(ui, rect, usage, 8.0, PURPLE);
        self.graph_dot(ui, rect, &component.key, component_color(component.kind));
    }

    fn sensor_row(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor, component: &Component, row: &Row, note: Option<&str>, die: bool) {
        let seconds = self.config.window_seconds;
        let (rect, _) = ui.allocate_exact_size(vec2(width, SENSOR_H), Sense::hover());
        let left = rect.left() + PAD;
        let y = rect.center().y;
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, NESTED_BG);

        let font = if die { strong(12.5) } else { body(12.5) };
        let color = if die { TEXT } else { MUTED };
        let label = painter.text(pos2(left + X_NAME, y), Align2::LEFT_CENTER, elide(&row.label, 28), font, color);
        if let Some(note) = note {
            let x = label.right() + 8.0;
            painter.text(pos2(x, y), Align2::LEFT_CENTER, note, body(11.0), FAINT);
        }

        let stats = row.temp.map(|id| monitor.stats(id, seconds)).unwrap_or(Stats::default());
        let (warn, critical) = (component.warn, component.critical);
        let tint = |value: Option<f32>| value.map_or(FAINT, |v| heat_text(v, warn, critical));
        painter.text(pos2(left + X_NOW, y), Align2::RIGHT_CENTER, self.temp(stats.now), strong(12.5), tint(stats.now));
        painter.text(pos2(left + X_HIGH, y), Align2::RIGHT_CENTER, self.temp(stats.high), body(12.5), tint(stats.high));
        painter.text(pos2(left + X_AVG, y), Align2::RIGHT_CENTER, self.temp(stats.avg), body(12.5), MUTED);

        if let Some(usage) = row.usage {
            let value = monitor.stats(usage, seconds).avg;
            self.usage_cell(ui, rect, value, 5.0, PURPLE.gamma_multiply(0.8));
        }
        let color = self.series_color(&row.key);
        self.graph_dot(ui, rect, &row.key, color);
    }

    fn core_head(&self, ui: &mut Ui, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, CORE_HEAD_H), Sense::hover());
        let left = rect.left() + PAD;
        let y = rect.center().y + 2.0;
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, NESTED_BG);
        let at = |offset: f32| pos2(left + offset, y);
        caps(painter, at(X_NAME + 12.0), Align2::LEFT_CENTER, "CORE", FAINT);
        caps(painter, at(X_NOW), Align2::RIGHT_CENTER, "CLOCK", FAINT);
        caps(painter, at(X_HIGH), Align2::RIGHT_CENTER, "PEAK", FAINT);
        caps(painter, at(X_AVG), Align2::RIGHT_CENTER, "POWER", FAINT);
    }

    fn core_row(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor, core: &Core) {
        let seconds = self.config.window_seconds;
        let (rect, _) = ui.allocate_exact_size(vec2(width, SENSOR_H), Sense::hover());
        let left = rect.left() + PAD;
        let y = rect.center().y;
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, NESTED_BG);
        painter.text(pos2(left + X_NAME + 12.0, y), Align2::LEFT_CENTER, &core.label, body(12.5), MUTED);

        let clock = core.clock.map(|id| monitor.stats(id, seconds)).unwrap_or(Stats::default());
        let power = core.power.and_then(|id| monitor.latest(id));
        painter.text(pos2(left + X_NOW, y), Align2::RIGHT_CENTER, frequency(clock.now), body(12.0), TEXT);
        painter.text(pos2(left + X_HIGH, y), Align2::RIGHT_CENTER, frequency(clock.high), body(12.0), MUTED);
        let power = power.map_or("—".to_owned(), |w| format!("{w:.1} W"));
        painter.text(pos2(left + X_AVG, y), Align2::RIGHT_CENTER, power, body(12.0), MUTED);

        let usage = monitor.stats(core.usage, seconds).avg;
        self.usage_cell(ui, rect, usage, 5.0, PURPLE.gamma_multiply(0.8));
        let color = self.series_color(&core.key);
        self.graph_dot(ui, rect, &core.key, color);
    }

    fn extras_row(&self, ui: &mut Ui, width: f32, values: &[(String, String)], height: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, NESTED_BG);
        let left = rect.left() + PAD + X_NAME;
        let column = (rect.right() - PAD - left) / EXTRA_COLUMNS as f32;
        for (index, (label, value)) in values.iter().enumerate() {
            let x = left + column * (index % EXTRA_COLUMNS) as f32;
            let y = rect.top() + 3.0 + EXTRA_LINE_H * (index / EXTRA_COLUMNS) as f32 + EXTRA_LINE_H / 2.0;
            let text = painter.text(pos2(x, y), Align2::LEFT_CENTER, label, body(11.5), FAINT);
            painter.text(pos2(text.right() + 6.0, y), Align2::LEFT_CENTER, value, strong(11.5), MUTED);
        }
    }

    fn graph_panel(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor) {
        let (controls, _) = ui.allocate_exact_size(vec2(width, GRAPH_CONTROLS_H), Sense::hover());
        ui.painter().line_segment(
            [pos2(controls.left() + PAD, controls.top()), pos2(controls.right() - PAD, controls.top())],
            Stroke::new(1.0, BORDER),
        );

        let mut x = controls.left() + PAD;
        let metrics = &mut self.config.metrics;
        for (label, value, chip_width) in [
            ("High", &mut metrics.high, 46.0),
            ("Average", &mut metrics.avg, 64.0),
            ("Usage", &mut metrics.usage, 54.0),
        ] {
            let rect = Rect::from_min_size(pos2(x, controls.top() + 8.0), vec2(chip_width, 21.0));
            let response = ui.interact(rect, ui.scope_id().with(("metric", label)), Sense::click());
            chip_painter(ui.painter(), rect, label, *value, response.hovered());
            if response.clicked() {
                *value = !*value;
            }
            x += chip_width + 6.0;
        }
        let note = pos2(controls.right() - PAD, controls.top() + 18.5);
        ui.painter().text(note, Align2::RIGHT_CENTER, "Solid: temperature    Dashed: usage", body(11.0), FAINT);

        let (rect, _) = ui.allocate_exact_size(vec2(width, GRAPH_PLOT_H), Sense::hover());
        let rect = rect.shrink2(vec2(PAD - 8.0, 0.0));
        let seconds = self.config.window_seconds.max(MIN_GRAPH_SECONDS);
        let lines = self.graph_lines(monitor, seconds, rect.width() as usize);
        let fahrenheit = self.config.fahrenheit;
        graph::draw(ui, rect, &lines, seconds, |c| Self::format_temp(fahrenheit, c));
    }

    fn settings_height(&self) -> f32 {
        SETTINGS_ROW_H * 4.0
    }

    fn settings_panel(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, self.settings_height()), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, NESTED_BG);
        ui.painter().line_segment(
            [pos2(rect.left(), rect.top()), pos2(rect.right(), rect.top())],
            Stroke::new(1.0, BORDER),
        );
        let left = rect.left() + PAD + X_NAME;
        let row = |index: f32| rect.top() + SETTINGS_ROW_H * (index + 0.5);
        let label = |ui: &Ui, index: f32, text: &str| {
            ui.painter().text(pos2(left, row(index)), Align2::LEFT_CENTER, text, body(12.5), MUTED);
        };
        let chip_rect = |x: f32, index: f32, chip: f32| Rect::from_min_size(pos2(x, row(index) - 11.0), vec2(chip, 22.0));
        let start = left + 150.0;

        label(ui, 0.0, "Temperature unit");
        for (index, (text, fahrenheit)) in [("°C", false), ("°F", true)].into_iter().enumerate() {
            let rect = chip_rect(start + index as f32 * 46.0, 0.0, 40.0);
            let response = ui.interact(rect, ui.scope_id().with(("unit", text)), Sense::click());
            chip_painter(ui.painter(), rect, text, self.config.fahrenheit == fahrenheit, response.hovered());
            if response.clicked() {
                self.config.fahrenheit = fahrenheit;
            }
        }

        label(ui, 1.0, "Show components");
        let mut x = start;
        for component in &monitor.components {
            let chip = 22.0 + component.title.len() as f32 * 7.0;
            let rect = chip_rect(x, 1.0, chip);
            let response = ui.interact(rect, ui.scope_id().with(("show", &component.key)), Sense::click());
            let shown = !config::contains(&self.config.hidden, &component.key);
            chip_painter(ui.painter(), rect, &component.title, shown, response.hovered());
            if response.clicked() {
                config::toggle(&mut self.config.hidden, &component.key);
            }
            x += chip + 6.0;
        }

        self.autostart_setting(ui, monitor, rect, start);

        label(ui, 3.0, "Sensor access");
        let status = match &monitor.link {
            Link::Connected { elevated: true } => "Full access",
            Link::Connected { elevated: false } => "Limited: not running as administrator",
            Link::Starting => "Starting",
            Link::AwaitingConsent => "Waiting for permission",
            Link::Lost(_) => "Disconnected",
        };
        ui.painter().text(pos2(start, row(3.0)), Align2::LEFT_CENTER, status, body(12.5), TEXT);
    }

    fn autostart_setting(&mut self, ui: &mut Ui, monitor: &Monitor, panel: Rect, start: f32) {
        let y = panel.top() + SETTINGS_ROW_H * 2.5;
        let left = panel.left() + PAD + X_NAME;
        ui.painter().text(pos2(left, y), Align2::LEFT_CENTER, "Start with Windows", body(12.5), MUTED);

        let setting = monitor.autostart;
        if setting.busy {
            let text = "Waiting for the Windows permission prompt…";
            ui.painter().text(pos2(start, y), Align2::LEFT_CENTER, text, body(12.5), TEXT);
            return;
        }
        for (index, (text, enable)) in [("Off", false), ("On", true)].into_iter().enumerate() {
            let rect = Rect::from_min_size(pos2(start + index as f32 * 46.0, y - 11.0), vec2(40.0, 22.0));
            let response = ui.interact(rect, ui.scope_id().with(("autostart", text)), Sense::click());
            chip_painter(ui.painter(), rect, text, setting.enabled == enable, response.hovered());
            if response.clicked() && setting.enabled != enable {
                let _ = self.commands.send(Command::SetAutostart(enable));
            }
        }
        let hint = pos2(start + 98.0, y);
        ui.painter().text(hint, Align2::LEFT_CENTER, "Also skips the prompt at launch", body(11.0), FAINT);
    }

    fn compact_table(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor) {
        const X_NOW: f32 = 112.0;
        const X_HIGH: f32 = 150.0;
        const X_AVG: f32 = 188.0;
        const X_BAR: f32 = 200.0;

        let seconds = self.config.window_seconds;
        let (head, _) = ui.allocate_exact_size(vec2(width, COMPACT_HEAD_H), Sense::hover());
        let left = head.left() + PAD;
        let painter = ui.painter();
        let y = head.center().y + 1.0;
        caps(painter, pos2(left + X_NOW, y), Align2::RIGHT_CENTER, "NOW", FAINT);
        caps(painter, pos2(left + X_HIGH, y), Align2::RIGHT_CENTER, "HIGH", FAINT);
        caps(painter, pos2(left + X_AVG, y), Align2::RIGHT_CENTER, "AVG", FAINT);
        caps(painter, pos2(left + X_BAR, y), Align2::LEFT_CENTER, "USAGE", FAINT);

        for component in self.visible(monitor) {
            let (rect, _) = ui.allocate_exact_size(vec2(width, COMPACT_ROW_H), Sense::hover());
            let y = rect.center().y;
            let painter = ui.painter();
            let (warn, critical) = (component.warn, component.critical);
            let high = monitor.stats(component.high, seconds);
            let avg = monitor.stats(component.avg, seconds).avg;
            let tint = |value: Option<f32>| value.map_or(FAINT, |v| heat_text(v, warn, critical));

            let name = component.title.to_uppercase();
            painter.text(pos2(left, y), Align2::LEFT_CENTER, name, strong(11.5), MUTED);
            painter.text(pos2(left + X_NOW, y), Align2::RIGHT_CENTER, self.temp(high.now), strong(14.0), tint(high.now));
            painter.text(pos2(left + X_HIGH, y), Align2::RIGHT_CENTER, self.temp(high.high), strong(12.5), tint(high.high));
            painter.text(pos2(left + X_AVG, y), Align2::RIGHT_CENTER, self.temp(avg), body(12.5), MUTED);

            let bar = Rect::from_min_max(pos2(left + X_BAR, y - 8.0), pos2(rect.right() - PAD, y + 8.0));
            match component.usage.and_then(|id| monitor.stats(id, seconds).avg) {
                Some(usage) => {
                    painter.rect_filled(bar, 4.0, TRACK);
                    let fill = bar.width() * (usage / 100.0).clamp(0.0, 1.0);
                    if fill >= 1.0 {
                        let fill_rect = Rect::from_min_size(bar.min, vec2(fill.max(8.0), bar.height()));
                        painter.rect_filled(fill_rect, 4.0, PURPLE);
                    }
                    let text = format!("{usage:.0}%");
                    painter.text(bar.center(), Align2::CENTER_CENTER, text, strong(11.0), TEXT);
                }
                None => {
                    painter.text(bar.center(), Align2::CENTER_CENTER, "—", body(12.0), FAINT);
                }
            }
        }
    }

    fn resize_to(&mut self, ctx: &Context, size: Vec2) {
        if size != self.requested_size {
            self.requested_size = size;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
    }

    /// Remembers the window position and writes settings at most every few seconds.
    fn persist(&mut self, ctx: &Context) {
        let (position, minimized) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().minimized));
        if let (Some(rect), false) = (position, minimized.unwrap_or(false)) {
            if rect.min.x > -10_000.0 {
                self.config.position = Some([rect.min.x, rect.min.y]);
            }
        }
        if self.config != self.saved && self.last_save.elapsed() > Duration::from_secs(3) {
            self.save();
        }
    }

    fn save(&mut self) {
        self.config.save();
        self.saved = self.config.clone();
        self.last_save = Instant::now();
    }
}

impl eframe::App for ThermalEye {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BODY_BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
    }
}

impl ThermalEye {
    fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let shared = self.monitor.clone();
        let mut monitor = shared.lock().unwrap();
        let mut reset = false;

        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.painter().rect_filled(ui.max_rect(), 0.0, BODY_BG);

        let compact = self.config.compact;
        let width = if compact { COMPACT_WIDTH } else { WIDTH };

        let notice: Option<(String, Option<&str>)> = match &monitor.link {
            Link::Connected { elevated: true } if monitor.components.is_empty() => {
                Some(("Reading sensors…".into(), None))
            }
            Link::Connected { elevated: true } => None,
            Link::Connected { elevated: false } => {
                Some(("CPU, RAM and board temperatures need admin access".into(), Some("Grant access")))
            }
            Link::Starting => Some(("Starting sensors…".into(), None)),
            Link::AwaitingConsent => Some(("Waiting for the Windows permission prompt…".into(), None)),
            Link::Lost(reason) => Some((reason.clone(), Some("Reconnect"))),
        };

        let items = self.items(&monitor);
        let title_h = if compact { COMPACT_TITLE_H } else { TITLE_H };
        let banner_h = if notice.is_some() { BANNER_H } else { 0.0 };
        let mut fixed = title_h + banner_h + BOTTOM_PAD;
        let mut table_h = 0.0;
        if compact {
            let rows = self.visible(&monitor).len() as f32;
            fixed += COMPACT_HEAD_H + rows * COMPACT_ROW_H;
        } else {
            fixed += HEADER_H;
            if self.config.graph_open {
                fixed += GRAPH_CONTROLS_H + GRAPH_PLOT_H;
            }
            if self.settings_open {
                fixed += self.settings_height();
            }
            table_h = items.iter().map(Item::height).sum();
        }
        let screen = ctx.input(|i| i.viewport().monitor_size).map_or(1000.0, |s| s.y);
        let room = (screen - SCREEN_MARGIN - fixed).max(COMPONENT_H * 2.0);
        let visible_table_h = table_h.min(room);

        self.title_bar(ui, width, &mut reset);
        if let Some((message, action)) = &notice {
            self.banner(ui, width, message, *action);
        }

        if compact {
            self.compact_table(ui, width, &monitor);
        } else {
            self.table_header(ui, width);
            ScrollArea::vertical()
                .max_height(visible_table_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    for item in &items {
                        match item {
                            Item::Component(component) => self.component_row(ui, width, &monitor, component),
                            Item::Sensor { component, row, note, die } => {
                                self.sensor_row(ui, width, &monitor, component, row, *note, *die)
                            }
                            Item::CoreHead => self.core_head(ui, width),
                            Item::Core(core) => self.core_row(ui, width, &monitor, core),
                            Item::Extras(values) => self.extras_row(ui, width, values, item.height()),
                        }
                    }
                });
            if self.settings_open {
                self.settings_panel(ui, width, &monitor);
            }
            if self.config.graph_open {
                self.graph_panel(ui, width, &monitor);
            }
        }
        drop(items);

        self.resize_to(&ctx, vec2(width, (fixed + visible_table_h).ceil()));
        self.persist(&ctx);
        if reset {
            monitor.history.clear();
        }
        ctx.request_repaint_after(Duration::from_secs(2));
    }

}

fn frequency(megahertz: Option<f32>) -> String {
    match megahertz {
        None => "—".into(),
        Some(mhz) if mhz >= 1000.0 => format!("{:.2} GHz", mhz / 1000.0),
        Some(mhz) => format!("{mhz:.0} MHz"),
    }
}

fn elide(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit - 1).collect();
    format!("{}…", kept.trim_end())
}

/// Label and formatted value of each extra reading that currently has one.
/// Fan headers reading 0 RPM are skipped: most have nothing plugged in.
fn extra_values(monitor: &Monitor, component: &Component) -> Vec<(String, String)> {
    component
        .extras
        .iter()
        .filter_map(|extra| match extra {
            Extra::Reading { label, series, unit } => {
                let value = monitor.latest(*series)?;
                let text = match unit {
                    Unit::Megahertz => frequency(Some(value)),
                    Unit::Watts => format!("{value:.0} W"),
                    Unit::Rpm if value < 1.0 && label != "Fan" => return None,
                    Unit::Rpm => format!("{value:.0} RPM"),
                };
                Some((label.clone(), text))
            }
            Extra::Capacity { label, used, total, to_gb } => {
                let (used, total) = (monitor.latest(*used)?, monitor.latest(*total)?);
                Some((label.clone(), format!("{:.1} / {:.1} GB", used * to_gb, total * to_gb)))
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
