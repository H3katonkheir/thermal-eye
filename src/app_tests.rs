//! Runs the real widget code headlessly, clicking it the way a user would.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    Event, Modifiers, PointerButton, RawInput, Rect, ViewportCommand, ViewportId, pos2,
};

use super::*;
use crate::topology::{L3Domain, Topology};

const SCHEMA: &[u8] = include_bytes!("../tests/fixtures/schema.json");

/// Vertical centre of a component row, by position in the list.
fn row_y(index: usize) -> f32 {
    TITLE_H + HEADER_H + COMPONENT_H * (index as f32 + 0.5)
}

/// Centre of a title bar button, counted from the right edge.
fn button(index: usize, side: f32, width: f32, bar: f32) -> (f32, f32) {
    (width - PAD - side / 2.0 - index as f32 * (side + 6.0), bar / 2.0)
}

fn full_button(index: usize) -> (f32, f32) {
    button(index, 30.0, WIDTH, TITLE_H)
}

struct Harness {
    ctx: Context,
    app: ThermalEye,
    inbox: Receiver<Command>,
    monitor: Arc<Mutex<Monitor>>,
    commands: Vec<ViewportCommand>,
    screen_height: f32,
}

impl Harness {
    fn new(config: Config) -> Self {
        let topology = Topology {
            cores: (0..16).map(|core| vec![core * 2, core * 2 + 1]).collect(),
            l3: vec![
                L3Domain { processors: (0..16).collect(), size_mb: 96 },
                L3Domain { processors: (16..32).collect(), size_mb: 32 },
            ],
        };
        let mut monitor = Monitor::new(topology);
        monitor.ingest(SCHEMA);
        let count = SCHEMA.split(|&b| b == b'"').filter(|part| *part == b"kind").count();
        let values = serde_json::json!({ "t": "values", "v": vec![45.0; count] }).to_string();
        for _ in 0..5 {
            monitor.ingest(values.as_bytes());
        }

        let ctx = Context::default();
        install_fonts(&ctx);
        ctx.run_ui(RawInput::default(), |_| {}).textures_delta.clear();
        let monitor = Arc::new(Mutex::new(monitor));
        let (sender, inbox) = channel();
        let app = ThermalEye::new(&ctx, monitor.clone(), sender, config);
        let mut harness =
            Self { ctx, app, inbox, monitor, commands: Vec::new(), screen_height: 1400.0 };
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) {
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, self.screen_height))),
            events,
            ..Default::default()
        };
        let info = input.viewports.entry(ViewportId::ROOT).or_default();
        info.monitor_size = Some(vec2(2560.0, self.screen_height));

        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| app.draw(ui));
        output.textures_delta.clear();
        if let Some(viewport) = output.viewport_output.get(&ViewportId::ROOT) {
            self.commands.extend(viewport.commands.iter().cloned());
        }
    }

    fn click(&mut self, (x, y): (f32, f32)) {
        let pos = pos2(x, y);
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![press(true)]);
        self.frame(vec![press(false)]);
        self.frame(Vec::new());
    }

    fn height(&self) -> f32 {
        self.app.requested_size.y
    }

    fn width(&self) -> f32 {
        self.app.requested_size.x
    }
}

#[test]
fn clicking_a_component_opens_and_closes_its_sensors() {
    let mut widget = Harness::new(Config::default());
    let closed = widget.height();

    widget.click((100.0, row_y(1)));
    assert_eq!(widget.app.config.expanded, ["gpu0"]);
    assert!(widget.height() > closed, "the window grows to fit the sensors");

    widget.click((100.0, row_y(1)));
    assert!(widget.app.config.expanded.is_empty());
    assert_eq!(widget.height(), closed);
}

#[test]
fn the_cpu_dropdown_lists_every_core() {
    let mut widget = Harness::new(Config { expanded: vec!["cpu".into()], ..Config::default() });
    widget.frame(Vec::new());
    let monitor = widget.monitor.clone();
    let monitor = monitor.lock().unwrap();
    let items = widget.app.items(&monitor);
    let cores = items.iter().filter(|item| matches!(item, Item::Core(_))).count();
    let dies = items.iter().filter(|item| matches!(item, Item::Sensor { die: true, .. })).count();
    assert_eq!((cores, dies), (16, 2));
}

#[test]
fn the_span_selector_changes_the_window() {
    let mut widget = Harness::new(Config::default());
    let mut x = PAD;
    for (_, seconds, segment) in WINDOWS {
        widget.click((x + segment / 2.0, TITLE_H + HEADER_H / 2.0));
        assert_eq!(widget.app.config.window_seconds, seconds);
        x += segment;
    }
}

#[test]
fn the_graph_button_opens_the_graph_and_dots_choose_its_lines() {
    let mut widget = Harness::new(Config { graphed: Vec::new(), ..Config::default() });
    let closed = widget.height();

    widget.click(full_button(4));
    assert!(widget.app.config.graph_open);
    assert_eq!(widget.height(), closed + GRAPH_CONTROLS_H + GRAPH_PLOT_H);

    widget.click((PAD + X_DOT, row_y(2)));
    assert_eq!(widget.app.config.graphed, ["ram"]);
    let monitor = widget.monitor.clone();
    let lines = widget.app.graph_lines(&monitor.lock().unwrap(), 60, 100);
    let labels: Vec<_> = lines.iter().map(|l| l.label.as_str()).collect();
    assert_eq!(labels, ["RAM high", "RAM usage"]);
}

#[test]
fn a_dot_does_not_also_open_the_row() {
    let config = Config { graph_open: true, graphed: Vec::new(), ..Config::default() };
    let mut widget = Harness::new(config);
    widget.click((PAD + X_DOT, row_y(0)));
    assert_eq!(widget.app.config.graphed, ["cpu"]);
    assert!(widget.app.config.expanded.is_empty());
}

#[test]
fn metric_chips_filter_the_graph() {
    let mut widget = Harness::new(Config { graph_open: true, ..Config::default() });
    let y = widget.height() - BOTTOM_PAD - GRAPH_PLOT_H - GRAPH_CONTROLS_H + 18.0;
    widget.click((PAD + 20.0, y));
    assert!(!widget.app.config.metrics.high);
    widget.click((PAD + 52.0 + 30.0, y));
    assert!(widget.app.config.metrics.avg);
    widget.click((PAD + 52.0 + 70.0 + 25.0, y));
    assert!(!widget.app.config.metrics.usage);
}

#[test]
fn compact_mode_shrinks_the_window_and_can_be_left_again() {
    let mut widget = Harness::new(Config::default());
    let full = widget.height();

    widget.click(full_button(2));
    assert!(widget.app.config.compact);
    assert_eq!(widget.width(), COMPACT_WIDTH);
    assert!(widget.height() < full);

    widget.click(button(2, 26.0, COMPACT_WIDTH, COMPACT_TITLE_H));
    assert!(!widget.app.config.compact);
    assert_eq!(widget.width(), WIDTH);
}

#[test]
fn pin_and_close_reach_the_window() {
    let mut widget = Harness::new(Config::default());
    widget.click(full_button(3));
    assert!(widget.app.config.always_on_top);
    widget.click(full_button(0));
    let sent = |wanted: &ViewportCommand| widget.commands.iter().any(|c| c == wanted);
    assert!(sent(&ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)));
    assert!(sent(&ViewportCommand::Close));
}

#[test]
fn dragging_the_title_bar_moves_the_window() {
    let mut widget = Harness::new(Config::default());
    let (start, end) = (pos2(200.0, 20.0), pos2(240.0, 24.0));
    let press = |pressed, pos| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    widget.frame(vec![Event::PointerMoved(start)]);
    widget.frame(vec![press(true, start)]);
    widget.frame(vec![Event::PointerMoved(end)]);
    widget.frame(vec![Event::PointerMoved(end)]);
    assert!(widget.commands.contains(&ViewportCommand::StartDrag));
}

#[test]
fn hovering_the_graph_is_safe_with_and_without_lines() {
    for graphed in [vec!["cpu".to_owned()], Vec::new()] {
        let mut widget = Harness::new(Config { graph_open: true, graphed, ..Config::default() });
        let y = widget.height() - BOTTOM_PAD - GRAPH_PLOT_H / 2.0;
        for x in [50.0, 300.0, WIDTH - 45.0] {
            widget.frame(vec![Event::PointerMoved(pos2(x, y))]);
        }
    }
}

#[test]
fn reset_forgets_highs() {
    let mut widget = Harness::new(Config::default());
    widget.click(full_button(5));
    let monitor = widget.monitor.lock().unwrap();
    let cpu = &monitor.components[0];
    assert_eq!(monitor.stats(cpu.high, 300).high, None);
}

#[test]
fn settings_switch_units_and_hide_components() {
    let mut widget = Harness::new(Config::default());
    assert_eq!(widget.app.temp(Some(100.0)), "100°");

    widget.click(full_button(6));
    assert!(widget.app.settings_open);
    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    let left = PAD + X_NAME + 150.0;
    widget.click((left + 46.0 + 20.0, top + SETTINGS_ROW_H / 2.0));
    assert!(widget.app.config.fahrenheit);
    assert_eq!(widget.app.temp(Some(100.0)), "212°");

    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    widget.click((left + 10.0, top + SETTINGS_ROW_H * 1.5));
    assert_eq!(widget.app.config.hidden, ["cpu"]);
    let monitor = widget.monitor.clone();
    assert_eq!(widget.app.visible(&monitor.lock().unwrap()).len(), 4);
}

#[test]
fn start_with_windows_asks_only_when_the_setting_changes() {
    let mut widget = Harness::new(Config::default());
    widget.click(full_button(6));
    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    let (y, left) = (top + SETTINGS_ROW_H * 2.5, PAD + X_NAME + 150.0);

    widget.click((left + 20.0, y));
    assert!(widget.inbox.try_recv().is_err(), "already off");

    widget.click((left + 46.0 + 20.0, y));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::SetAutostart(true))));

    widget.monitor.lock().unwrap().autostart.busy = true;
    widget.frame(Vec::new());
    widget.click((left + 46.0 + 20.0, y));
    assert!(widget.inbox.try_recv().is_err(), "no second request while one is pending");
}

#[test]
fn limited_access_offers_a_way_to_elevate() {
    let mut widget = Harness::new(Config::default());
    widget.monitor.lock().unwrap().link = Link::Connected { elevated: false };
    widget.frame(Vec::new());
    widget.click((WIDTH - PAD - 48.0, TITLE_H + BANNER_H / 2.0));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::Connect)));
}

#[test]
fn a_tall_window_is_capped_to_the_screen_and_scrolls() {
    let all: Vec<String> = ["cpu", "gpu0", "ram", "storage", "system"].map(String::from).to_vec();
    let mut widget = Harness::new(Config { expanded: all, graph_open: true, ..Config::default() });
    widget.screen_height = 800.0;
    widget.frame(Vec::new());
    assert!(widget.height() <= 800.0 - SCREEN_MARGIN);
}

#[test]
fn temperatures_use_a_plain_degree_sign() {
    let widget = Harness::new(Config::default());
    assert_eq!(widget.app.temp(Some(52.4)).as_bytes(), [b'5', b'2', 0xC2, 0xB0]);
    assert_eq!(widget.app.temp(None).as_bytes(), [0xE2, 0x80, 0x94]);
}

#[test]
fn bundled_images_decode_with_the_expected_shape() {
    let mark = logo::decode(logo::MARK);
    assert!(mark.width > mark.height, "the title bar mark is wide");
    assert!(mark.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0), "the mark has no background");

    let icon = logo::decode(logo::ICON);
    assert_eq!((icon.width, icon.height), (256, 256));
}

#[test]
fn elide_keeps_short_text_and_marks_cuts() {
    assert_eq!(elide("short", 10), "short");
    assert_eq!(elide("a very long drive name", 8), "a very…");
}

#[test]
fn frequencies_switch_units_at_a_gigahertz() {
    assert_eq!(frequency(Some(5520.0)), "5.52 GHz");
    assert_eq!(frequency(Some(138.4)), "138 MHz");
    assert_eq!(frequency(None), "—");
}

#[test]
fn stopped_fans_are_hidden_but_a_gpu_in_zero_rpm_mode_is_not() {
    let widget = Harness::new(Config::default());
    let mut system = widget.monitor.lock().unwrap().components[0].clone();

    let mut monitor = Monitor::new(Topology::default());
    let id = monitor.history.id("fan");
    monitor.history.set(id, 0.0);
    monitor.history.commit();
    let reading = |label: &str| Extra::Reading { label: label.into(), series: id, unit: Unit::Rpm };
    system.extras = vec![reading("Sys fan 3"), reading("Fan")];
    assert_eq!(extra_values(&monitor, &system), [("Fan".to_owned(), "0 RPM".to_owned())]);
}

