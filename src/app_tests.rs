//! Runs the real widget code headlessly, clicking it the way a user would.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    self,
    Event, Modifiers, PointerButton, RawInput, Rect, ViewportCommand, ViewportId, pos2,
};

use super::*;
use crate::topology::{L3Domain, Topology};

const SCHEMA: &[u8] = include_bytes!("../tests/fixtures/schema.json");

/// Window widths with every column shown, as the default settings have.
const WIDTH: f32 = 654.0;
const COMPACT_WIDTH: f32 = 374.0;

/// Where the graph dots sit with the default columns.
fn dot_x() -> f32 {
    PAD + Columns::new(&Config::default()).dot()
}

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

/// Text, and the part of it left visible by its clip rect, as when a scroll area hides rows.
fn collect_text(shape: &egui::Shape, clip: Rect, into: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(text) => {
            let shown = text.visual_bounding_rect().intersect(clip);
            if shown.is_positive() {
                into.push((text.galley.text().to_owned(), shown));
            }
        }
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, clip, into)),
        _ => {}
    }
}

struct Harness {
    ctx: Context,
    app: ThermalEye,
    inbox: Receiver<Command>,
    monitor: Arc<Mutex<Monitor>>,
    commands: Vec<ViewportCommand>,
    copied: Vec<String>,
    /// Every piece of text drawn in the last frame, and where.
    drawn: Vec<(String, Rect)>,
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
            Self { ctx, app, inbox, monitor, commands: Vec::new(), copied: Vec::new(), drawn: Vec::new(), screen_height: 1400.0 };
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) {
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.screen_width(), self.screen_height))),
            events,
            ..Default::default()
        };
        let info = input.viewports.entry(ViewportId::ROOT).or_default();
        info.monitor_size = Some(vec2(2560.0, self.screen_height));

        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| app.draw(ui));
        output.textures_delta.clear();
        self.drawn.clear();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, clipped.clip_rect, &mut self.drawn);
        }
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.copied.push(text.clone());
            }
        }
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

    /// The window as last sized, or room to spare before the first frame.
    fn screen_width(&self) -> f32 {
        if self.width() > 0.0 { self.width() } else { 800.0 }
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

    widget.click((dot_x(), row_y(2)));
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
    widget.click((dot_x(), row_y(0)));
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
fn hovering_a_graph_with_many_lines_does_not_crash() {
    // More lines than the tooltip has room for in the plot's height.
    let mut graphed = vec!["cpu".to_owned(), "gpu0".to_owned(), "ram".to_owned()];
    graphed.extend((0..16).map(|core| format!("core:{core}")));
    let config = Config { graph_open: true, expanded: vec!["cpu".into()], graphed, ..Config::default() };
    let mut widget = Harness::new(config);
    let monitor = widget.monitor.clone();
    let lines = widget.app.graph_lines(&monitor.lock().unwrap(), 60, 100).len();
    assert!(lines > graph::TOOLTIP_LINES, "only {lines} lines");

    let bottom = widget.height() - BOTTOM_PAD;
    // The test data is a few seconds old, so only the right end of the plot has values.
    for y in [bottom - GRAPH_PLOT_H + 8.0, bottom - GRAPH_PLOT_H / 2.0, bottom - 30.0] {
        for x in (40..widget.width() as usize - 20).step_by(4) {
            widget.frame(vec![Event::PointerMoved(pos2(x as f32, y))]);
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
    let (y, left) = (top + SETTINGS_ROW_H * 3.5, PAD + X_NAME + 150.0);

    widget.click((left + 20.0, y));
    assert!(widget.inbox.try_recv().is_err(), "already off");

    widget.click((left + 46.0 + 20.0, y));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::SetAutostart(true))));
}

#[test]
fn a_missing_driver_offers_the_download_page() {
    let mut widget = Harness::new(Config::default());
    let full = widget.height();
    {
        let mut monitor = widget.monitor.lock().unwrap();
        monitor.link = Link::Connected { elevated: true };
        monitor.driver_missing = true;
    }
    widget.frame(Vec::new());
    assert_eq!(widget.height(), full + BANNER_H, "the notice takes a row");
    widget.click((WIDTH - PAD - 48.0, TITLE_H + BANNER_H / 2.0));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::OpenDriverPage)));
}

#[test]
fn asking_for_admin_access_comes_before_the_driver_notice() {
    let mut widget = Harness::new(Config::default());
    {
        let mut monitor = widget.monitor.lock().unwrap();
        monitor.link = Link::Connected { elevated: false };
        monitor.driver_missing = true;
    }
    widget.frame(Vec::new());
    widget.click((WIDTH - PAD - 48.0, TITLE_H + BANNER_H / 2.0));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::Connect)));
}

#[test]
fn a_ready_update_is_offered_and_installs_on_click() {
    let mut widget = Harness::new(Config::default());
    {
        let mut monitor = widget.monitor.lock().unwrap();
        monitor.link = Link::Connected { elevated: true };
        monitor.update = UpdateState::Ready("9.9.9".into());
    }
    widget.frame(Vec::new());
    widget.click((WIDTH - PAD - 48.0, TITLE_H + BANNER_H / 2.0));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::InstallUpdate)));
}

#[test]
fn sensor_problems_outrank_the_update_notice() {
    let mut widget = Harness::new(Config::default());
    {
        let mut monitor = widget.monitor.lock().unwrap();
        monitor.link = Link::Connected { elevated: false };
        monitor.update = UpdateState::Ready("9.9.9".into());
    }
    widget.frame(Vec::new());
    widget.click((WIDTH - PAD - 48.0, TITLE_H + BANNER_H / 2.0));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::Connect)));
}

fn bare_monitor(link: Link) -> Monitor {
    let mut monitor = Monitor::new(Topology { cores: Vec::new(), l3: Vec::new() });
    monitor.link = link;
    monitor
}

#[test]
fn notices_use_the_short_wording_in_the_compact_view() {
    let mut monitor = bare_monitor(Link::Connected { elevated: false });
    let text = |monitor: &Monitor, compact| current_notice(monitor, compact).map(|(text, _)| text);
    assert_eq!(text(&monitor, true).as_deref(), Some("Some sensors need admin access"));
    assert_eq!(text(&monitor, false).as_deref(), Some("CPU, RAM and board temperatures need admin access"));

    monitor.link = Link::Connected { elevated: true };
    monitor.ingest(SCHEMA);
    monitor.link = Link::Connected { elevated: true };
    monitor.update = UpdateState::Ready("9.9.9".into());
    assert_eq!(text(&monitor, true).as_deref(), Some("Update 9.9.9 ready"));
    assert_eq!(text(&monitor, false).as_deref(), Some("Version 9.9.9 is ready to install"));
}

#[test]
fn a_healthy_link_says_it_is_reading_until_sensors_arrive() {
    let mut monitor = bare_monitor(Link::Connected { elevated: true });
    let notice = current_notice(&monitor, false);
    assert!(matches!(notice, Some((ref text, None)) if text == "Reading sensors…"));

    monitor.ingest(SCHEMA);
    monitor.link = Link::Connected { elevated: true };
    assert!(current_notice(&monitor, false).is_none());
}

#[test]
fn a_lost_helper_shows_its_reason_and_offers_to_reconnect() {
    let monitor = bare_monitor(Link::Lost("The helper stopped".into()));
    let notice = current_notice(&monitor, true);
    assert!(matches!(notice, Some((ref text, Some(("Reconnect", Command::Connect)))) if text == "The helper stopped"));
}

#[test]
fn settings_can_check_for_updates_and_turn_checks_off() {
    let mut widget = Harness::new(Config::default());
    widget.monitor.lock().unwrap().link = Link::Connected { elevated: true };
    widget.frame(Vec::new());
    widget.click(full_button(6));
    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    let left = PAD + X_NAME + 150.0;

    widget.click((left + 40.0, top + SETTINGS_ROW_H * 6.5));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::CheckForUpdate)));

    widget.click((left + 20.0, top + SETTINGS_ROW_H * 5.5));
    assert!(!widget.app.config.check_updates);
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::SetAutomaticUpdates(false))));
}

#[test]
fn nothing_can_be_clicked_while_an_update_is_in_progress() {
    let mut widget = Harness::new(Config::default());
    widget.monitor.lock().unwrap().update = UpdateState::Downloading("9.9.9".into());
    widget.frame(Vec::new());
    widget.click(full_button(6));
    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    widget.click((PAD + X_NAME + 190.0, top + SETTINGS_ROW_H * 6.5));
    assert!(widget.inbox.try_recv().is_err());
}

/// A widget that last ran `seen` and is otherwise healthy.
fn widget_after(seen: Option<&str>) -> Harness {
    let config = Config { seen_version: seen.map(str::to_owned), ..Config::default() };
    let mut widget = Harness::new(config);
    widget.monitor.lock().unwrap().link = Link::Connected { elevated: true };
    widget.frame(Vec::new());
    widget
}

const BANNER_Y: f32 = TITLE_H + BANNER_H / 2.0;
const WHATS_NEW_X: f32 = WIDTH - PAD - 20.0 - 6.0 - 48.0;
const DISMISS_X: f32 = WIDTH - PAD - 10.0;

#[test]
fn a_fresh_install_announces_nothing_and_records_the_version() {
    let widget = widget_after(None);
    assert!(!widget.app.just_updated);
    assert_eq!(widget.app.config.seen_version.as_deref(), Some(update::CURRENT_VERSION));
}

#[test]
fn a_new_install_writes_its_version_to_disk() {
    // Otherwise the first update would look like a fresh install and say nothing.
    let widget = widget_after(None);
    assert_ne!(widget.app.saved, widget.app.config, "the version is waiting to be saved");
}

#[test]
fn the_same_version_announces_nothing() {
    assert!(!widget_after(Some(update::CURRENT_VERSION)).app.just_updated);
}

#[test]
fn whats_new_opens_the_release_notes_and_clears_the_notice() {
    let mut widget = widget_after(Some("0.0.1"));
    assert!(widget.app.just_updated);
    let with_banner = widget.height();

    widget.click((WHATS_NEW_X, BANNER_Y));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::OpenReleaseNotes)));
    assert!(!widget.app.just_updated);
    assert_eq!(widget.app.config.seen_version.as_deref(), Some(update::CURRENT_VERSION));
    assert_eq!(widget.height(), with_banner - BANNER_H, "the window shrinks back");
}

#[test]
fn dismissing_the_notice_opens_nothing_and_it_stays_gone() {
    let mut widget = widget_after(Some("0.0.1"));
    widget.click((DISMISS_X, BANNER_Y));
    assert!(widget.inbox.try_recv().is_err());
    assert!(!widget.app.just_updated);

    let saved = widget.app.config.seen_version.clone();
    assert!(!widget_after(saved.as_deref()).app.just_updated);
}

#[test]
fn the_notice_waits_while_a_sensor_problem_is_showing() {
    let mut widget = widget_after(Some("0.0.1"));
    widget.monitor.lock().unwrap().link = Link::Connected { elevated: false };
    widget.frame(Vec::new());
    widget.click((WIDTH - PAD - 48.0, BANNER_Y));
    assert!(matches!(widget.inbox.try_recv(), Ok(Command::Connect)));
    assert!(widget.app.just_updated, "still waiting to be seen");
}

#[test]
fn release_notes_point_at_the_running_version() {
    assert!(update::release_notes_url().ends_with(&format!("/releases/tag/v{}", update::CURRENT_VERSION)));
}

#[test]
fn copy_details_puts_the_report_on_the_clipboard() {
    let mut widget = Harness::new(Config::default());
    widget.monitor.lock().unwrap().link = Link::Connected { elevated: true };
    widget.frame(Vec::new());
    widget.click(full_button(6));
    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    widget.click((PAD + X_NAME + 150.0 + 40.0, top + SETTINGS_ROW_H * 7.5));
    assert_eq!(widget.copied.len(), 1);
    assert!(widget.copied[0].contains("AMD Ryzen 9 9950X3D"));
}

impl Harness {
    fn shows(&self, text: &str) -> bool {
        self.drawn.iter().any(|(drawn, _)| drawn == text)
    }

    /// Rests the pointer somewhere, with tooltips set to appear at once.
    fn hover(&mut self, (x, y): (f32, f32)) {
        self.ctx.global_style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        for _ in 0..4 {
            self.frame(vec![Event::PointerMoved(pos2(x, y))]);
        }
    }
}

const LIVE: u32 = 1;
const RAM_SUBTITLE: &str = "2 × A-DATA AX5U6000C3032G-BLABK";

#[test]
fn hovering_shortened_text_shows_all_of_it() {
    let mut widget = Harness::new(Config::default());
    widget.frame(Vec::new());
    assert!(!widget.shows(RAM_SUBTITLE), "shortened until hovered");

    widget.hover((PAD + X_NAME + 40.0, row_y(2) + 9.0));
    assert!(widget.shows(RAM_SUBTITLE));
}

#[test]
fn the_tooltip_only_appears_over_the_text_itself() {
    let mut widget = Harness::new(Config::default());
    widget.hover((PAD + X_NOW, row_y(2)));
    assert!(!widget.shows(RAM_SUBTITLE));
}

#[test]
fn hovering_text_that_fits_adds_nothing() {
    let mut widget = Harness::new(Config::default());
    widget.hover((PAD + X_NAME + 20.0, row_y(1) + 9.0));
    assert_eq!(widget.drawn.iter().filter(|(text, _)| text == "RX 9070 XT").count(), 1);
}

#[test]
fn a_shortened_sensor_name_also_shows_in_full() {
    let mut widget = Harness::new(Config { expanded: vec!["storage".into()], ..Config::default() });
    widget.monitor.lock().unwrap().components[3].rows[1].label = "A drive with a really very long model name".into();
    widget.frame(Vec::new());
    assert!(!widget.shows("A drive with a really very long model name"));

    let rows_above = TITLE_H + HEADER_H + COMPONENT_H * 4.0;
    widget.hover((PAD + X_NAME + 30.0, rows_above + SENSOR_H * 1.5));
    assert!(widget.shows("A drive with a really very long model name"));
}

#[test]
fn live_shows_now_and_avg_but_not_min_or_max() {
    let mut widget = Harness::new(Config { window_seconds: LIVE, ..Config::default() });
    widget.frame(Vec::new());
    assert!(widget.shows("NOW") && widget.shows("AVG") && widget.shows("USAGE"));
    assert!(!widget.shows("MIN") && !widget.shows("MAX") && !widget.shows("TEMP"));
}

#[test]
fn other_spans_keep_all_four_columns() {
    let mut widget = Harness::new(Config::default());
    widget.frame(Vec::new());
    assert!(widget.shows("NOW") && widget.shows("MIN") && widget.shows("MAX") && widget.shows("AVG"));
}

#[test]
fn live_applies_to_the_compact_view_too() {
    let mut widget = Harness::new(Config { window_seconds: LIVE, compact: true, ..Config::default() });
    widget.frame(Vec::new());
    assert!(widget.shows("NOW") && widget.shows("AVG"));
    assert!(!widget.shows("MIN") && !widget.shows("MAX") && !widget.shows("TEMP"));
}

#[test]
fn live_drops_the_redundant_peak_clock_but_keeps_power() {
    let config = Config { window_seconds: LIVE, expanded: vec!["cpu".into()], ..Config::default() };
    let mut widget = Harness::new(config);
    widget.frame(Vec::new());
    assert!(widget.shows("CLOCK") && widget.shows("POWER"));
    assert!(!widget.shows("LOW") && !widget.shows("PEAK"));
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


#[test]
fn the_default_widths_show_every_column() {
    assert_eq!(window_width(&Config::default()), WIDTH);
    assert_eq!(window_width(&Config { compact: true, ..Config::default() }), COMPACT_WIDTH);
}

#[test]
fn min_shows_how_low_the_hottest_sensor_got() {
    let widget = Harness::new(Config::default());
    let mut monitor = widget.monitor.lock().unwrap();
    let cpu = monitor.components[0].high;
    for value in [52.0, 38.0, 61.0] {
        monitor.history.set(cpu, value);
        monitor.history.commit();
    }
    let stats = monitor.stats(cpu, 300);
    assert_eq!((stats.low, stats.high, stats.now), (Some(38.0), Some(61.0), Some(61.0)));
}

#[test]
fn the_core_dropdown_shows_the_lowest_clock_under_min() {
    let mut widget = Harness::new(Config { expanded: vec!["cpu".into()], ..Config::default() });
    widget.frame(Vec::new());
    assert!(widget.shows("CLOCK") && widget.shows("LOW") && widget.shows("PEAK") && widget.shows("POWER"));
}

/// Clicks the Min or Max chip in the settings panel.
fn toggle_column(widget: &mut Harness, index: usize) {
    let top = widget.height() - BOTTOM_PAD - widget.app.settings_height();
    widget.click((PAD + X_NAME + 150.0 + index as f32 * 46.0 + 20.0, top + SETTINGS_ROW_H * 2.5));
}

#[test]
fn hiding_min_and_max_narrows_the_window() {
    let mut widget = Harness::new(Config::default());
    widget.click(full_button(6));
    assert_eq!(widget.width(), WIDTH);

    toggle_column(&mut widget, 0);
    assert!(!widget.app.config.columns.min);
    assert!(!widget.shows("MIN") && widget.shows("MAX"));
    assert_eq!(widget.width(), WIDTH - TEMP_STEP);

    toggle_column(&mut widget, 1);
    assert!(!widget.app.config.columns.max);
    assert!(!widget.shows("MIN") && !widget.shows("MAX"));
    assert_eq!(widget.width(), WIDTH - 2.0 * TEMP_STEP);

    toggle_column(&mut widget, 0);
    assert!(widget.shows("MIN") && !widget.shows("MAX"));
    assert_eq!(widget.width(), WIDTH - TEMP_STEP);
}

#[test]
fn hidden_columns_also_narrow_the_compact_view() {
    let columns = |min, max| config::Columns { min, max };
    let width = |columns| window_width(&Config { compact: true, columns, ..Config::default() });
    assert_eq!(width(columns(false, true)), COMPACT_WIDTH - COMPACT_STEP);
    assert_eq!(width(columns(true, false)), COMPACT_WIDTH - COMPACT_STEP);
    assert_eq!(width(columns(false, false)), COMPACT_WIDTH - 2.0 * COMPACT_STEP);

    let mut widget = Harness::new(Config { compact: true, columns: columns(false, true), ..Config::default() });
    assert_eq!(widget.width(), COMPACT_WIDTH - COMPACT_STEP);
    widget.frame(Vec::new());
    assert!(!widget.shows("MIN") && widget.shows("MAX"));
}

#[test]
fn live_takes_no_room_for_min_and_max() {
    let hidden = config::Columns { min: false, max: false };
    for compact in [false, true] {
        let live = Config { compact, window_seconds: LIVE, ..Config::default() };
        let narrow = Config { compact, columns: hidden.clone(), ..Config::default() };
        assert_eq!(window_width(&live), window_width(&narrow));
    }
}

#[test]
fn the_column_help_lists_only_the_columns_shown() {
    let config = Config { columns: config::Columns { min: false, max: true }, ..Config::default() };
    let widget = Harness::new(config);
    let help = widget.app.column_help(&widget.app.columns());
    assert!(help.contains("MAX:") && !help.contains("MIN:"));
}

/// Every text drawn fits the window and none overlaps another.
fn assert_tidy(widget: &Harness, case: &str) {
    let width = widget.width();
    let drawn: Vec<_> = widget.drawn.iter().filter(|(text, _)| !text.trim().is_empty()).collect();
    for (text, rect) in &drawn {
        assert!(rect.left() >= PAD / 2.0 && rect.right() <= width - PAD / 2.0, "{case}: {text:?} reaches the edge of a {width} wide window");
    }
    for (index, (a, a_rect)) in drawn.iter().enumerate() {
        for (b, b_rect) in &drawn[index + 1..] {
            assert!(!a_rect.intersects(*b_rect), "{case}: {a:?} overlaps {b:?}");
        }
    }
}

/// A named state for the monitor, to see the notice it brings up.
type Situation = (&'static str, fn(&mut Monitor));

#[test]
fn every_layout_fits_its_text() {
    let notices: [Situation; 4] = [
        ("healthy", |m| m.link = Link::Connected { elevated: true }),
        ("no admin", |m| m.link = Link::Connected { elevated: false }),
        ("no driver", |m| {
            m.link = Link::Connected { elevated: true };
            m.driver_missing = true;
        }),
        ("update", |m| {
            m.link = Link::Connected { elevated: true };
            m.update = UpdateState::Ready("10.10.10".into());
        }),
    ];
    for compact in [false, true] {
        for (min, max) in [(true, true), (true, false), (false, true), (false, false)] {
            for window_seconds in [LIVE, 300] {
                for (notice, set_up) in notices {
                    for seen in [None, Some("0.0.1")] {
                        let config = Config {
                            compact,
                            columns: config::Columns { min, max },
                            window_seconds,
                            expanded: vec!["cpu".into(), "system".into()],
                            seen_version: seen.map(str::to_owned),
                            ..Config::default()
                        };
                        let mut widget = Harness::new(config);
                        set_up(&mut widget.monitor.lock().unwrap());
                        widget.app.settings_open = !compact;
                        widget.frame(Vec::new());
                        widget.frame(Vec::new());
                        let case = format!("compact {compact}, min {min}, max {max}, {window_seconds}s, {notice}, seen {seen:?}");
                        assert_tidy(&widget, &case);
                    }
                }
            }
        }
    }
}

#[test]
fn the_compact_title_clears_the_buttons_at_every_width() {
    for (min, max) in [(true, true), (true, false), (false, false)] {
        let config = Config { compact: true, columns: config::Columns { min, max }, ..Config::default() };
        let mut widget = Harness::new(config);
        widget.frame(Vec::new());
        let title = widget.drawn.iter().find(|(text, _)| text == "Thermal Eye").unwrap().1;
        let (side, gap, buttons) = (26.0, 6.0, 4.0);
        let first_button = widget.width() - PAD - buttons * side - (buttons - 1.0) * gap;
        assert!(title.right() + gap <= first_button, "min {min}, max {max}: the title runs under the buttons");
    }
}

/// Centre of the heat ring on a component row.
fn ring_at(index: usize) -> (f32, f32) {
    (PAD + X_RING, row_y(index))
}

#[test]
fn hovering_a_ring_shows_its_warning_and_danger_levels() {
    let mut widget = Harness::new(Config::default());
    let (warn, critical) = {
        let monitor = widget.monitor.lock().unwrap();
        (monitor.components[0].warn, monitor.components[0].critical)
    };
    let expected = widget.app.limits_help(warn, critical);
    assert_eq!(expected, format!("Warning from {warn:.0}°\nDanger from {critical:.0}°"));

    widget.hover(ring_at(0));
    assert!(widget.shows(&expected));
}

#[test]
fn ring_levels_follow_the_temperature_unit() {
    let mut widget = Harness::new(Config { fahrenheit: true, ..Config::default() });
    assert_eq!(widget.app.limits_help(85.0, 95.0), "Warning from 185°\nDanger from 203°");
    widget.hover(ring_at(0));
    assert!(widget.drawn.iter().any(|(text, _)| text.starts_with("Warning from") && text.contains("°")));
}

#[test]
fn the_ring_tooltip_only_appears_over_the_ring() {
    let mut widget = Harness::new(Config::default());
    widget.hover((PAD + X_NOW + 30.0, row_y(0)));
    assert!(!widget.drawn.iter().any(|(text, _)| text.starts_with("Warning from")));
}

#[test]
fn clicking_the_ring_still_opens_the_row() {
    let mut widget = Harness::new(Config::default());
    widget.click(ring_at(0));
    assert_eq!(widget.app.config.expanded, ["cpu"]);
}
