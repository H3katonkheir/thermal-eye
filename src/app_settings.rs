//! The settings panel, opened from the title bar.

use std::time::Instant;

use eframe::egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{
    COPIED_NOTE_TIME, COOL_HELP, HEADER_H, HOT_HELP, MAX_HELP, MIN_HELP, PAD, SETTINGS_ROW_H, ThermalEye, X_NAME, elide, is_live,
    show_full_text,
};
use crate::bridge::Command;
use crate::config;
use crate::diagnostics;
use crate::model::{Link, Monitor, UpdateState};
use crate::theme::*;
use crate::update;

// The panel's rows, top to bottom.
pub(super) const ACCESS_ROW: usize = 0;
pub(super) const UNIT_ROW: usize = 1;
pub(super) const COMPONENTS_ROW: usize = 2;
pub(super) const COLUMNS_ROW: usize = 3;
pub(super) const AUTOSTART_ROW: usize = 4;
pub(super) const TRAY_ROW: usize = 5;
pub(super) const TASKBAR_ROW: usize = 6;
pub(super) const CLOSE_ROW: usize = 7;
pub(super) const UPDATES_ROW: usize = 8;
pub(super) const VERSION_ROW: usize = 9;
pub(super) const DIAGNOSTICS_ROW: usize = 10;
const ROWS: usize = 11;
/// Distance from the labels to the controls.
const CONTROLS_X: f32 = 150.0;
const OFF_ON: [(&str, bool); 2] = [("Off", false), ("On", true)];

/// Where things go in the panel, one setting per row.
struct Layout {
    top: f32,
    left: f32,
    /// Left edge of the controls.
    start: f32,
}

impl Layout {
    fn y(&self, row: usize) -> f32 {
        self.top + SETTINGS_ROW_H * (row as f32 + 0.5)
    }

    fn label(&self, ui: &Ui, row: usize, text: &str) {
        ui.painter().text(pos2(self.left, self.y(row)), Align2::LEFT_CENTER, text, body(12.5), MUTED);
    }

    fn chip(&self, x: f32, row: usize, width: f32) -> Rect {
        Rect::from_min_size(pos2(x, self.y(row) - 11.0), vec2(width, 22.0))
    }

    /// Two chips with the current option lit. Returns the other option when it is clicked.
    fn choice<T: Copy + PartialEq>(&self, ui: &Ui, row: usize, id: &str, options: [(&str, T); 2], current: T) -> Option<T> {
        let mut picked = None;
        for (index, (text, value)) in options.into_iter().enumerate() {
            let rect = self.chip(self.start + index as f32 * 46.0, row, 40.0);
            let response = ui.interact(rect, ui.scope_id().with((id, text)), Sense::click());
            chip_painter(ui.painter(), rect, text, current == value, response.hovered());
            if response.clicked() && current != value {
                picked = Some(value);
            }
        }
        picked
    }
}

fn sensor_access(monitor: &Monitor) -> &'static str {
    match &monitor.link {
        Link::Connected { elevated: true } if monitor.driver_missing => "Limited: PawnIO driver not installed",
        Link::Connected { elevated: true } => "Full access",
        Link::Connected { elevated: false } => "Limited: not running as administrator",
        Link::Starting => "Starting",
        Link::AwaitingConsent => "Waiting for permission",
        Link::Lost(_) => "Disconnected",
    }
}

impl ThermalEye {
    pub(super) fn settings_height(&self) -> f32 {
        SETTINGS_ROW_H * ROWS as f32
    }

    /// Heading of the settings view, with the way back to the table.
    pub(super) fn settings_header(&mut self, ui: &mut Ui, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
        let y = rect.center().y;
        caps(ui.painter(), pos2(rect.left() + PAD + X_NAME, y), Align2::LEFT_CENTER, "SETTINGS", FAINT);
        let done = Rect::from_min_size(pos2(rect.right() - PAD - 64.0, y - 10.0), vec2(64.0, 20.0));
        let response = ui.interact(done, ui.scope_id().with("settings-done"), Sense::click());
        chip_painter(ui.painter(), done, "Done", false, response.hovered());
        if response.clicked() {
            self.settings_open = false;
        }
    }

    pub(super) fn settings_panel(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, self.settings_height()), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, NESTED_BG);
        ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, BORDER));
        let left = rect.left() + PAD + X_NAME;
        let layout = Layout { top: rect.top(), left, start: left + CONTROLS_X };

        layout.label(ui, UNIT_ROW, "Temperature unit");
        let units = [("°C", false), ("°F", true)];
        if let Some(fahrenheit) = layout.choice(ui, UNIT_ROW, "unit", units, self.config.fahrenheit) {
            self.config.fahrenheit = fahrenheit;
        }

        layout.label(ui, COMPONENTS_ROW, "Show components");
        let mut x = layout.start;
        for component in &monitor.components {
            let chip = 22.0 + component.title.len() as f32 * 7.0;
            let rect = layout.chip(x, COMPONENTS_ROW, chip);
            let response = ui.interact(rect, ui.scope_id().with(("show", &component.key)), Sense::click());
            let shown = !config::contains(&self.config.hidden, &component.key);
            chip_painter(ui.painter(), rect, &component.title, shown, response.hovered());
            if response.clicked() {
                config::toggle(&mut self.config.hidden, &component.key);
            }
            x += chip + 6.0;
        }

        layout.label(ui, COLUMNS_ROW, "Show columns");
        let live = is_live(&self.config);
        let columns = &mut self.config.columns;
        let chips = [
            ("Hot", &mut columns.hot, HOT_HELP, false),
            ("Max", &mut columns.max, MAX_HELP, true),
            ("Cool", &mut columns.cool, COOL_HELP, false),
            ("Min", &mut columns.min, MIN_HELP, true),
        ];
        let mut x = layout.start;
        for (text, shown, help, needs_span) in chips {
            let rect = layout.chip(x, COLUMNS_ROW, 40.0);
            let mut tip = format!("{}: {help}", text.to_uppercase());
            if needs_span {
                tip.push_str("\nHidden in Live, where it would repeat ");
                tip.push_str(if text == "Max" { "HOT" } else { "COOL" });
            }
            let response = ui.interact(rect, ui.scope_id().with(("column", text)), Sense::click()).on_hover_text(tip);
            chip_painter(ui.painter(), rect, text, *shown, response.hovered());
            if response.clicked() {
                *shown = !*shown;
            }
            x += 46.0;
        }
        if live {
            let note = pos2(x + 4.0, layout.y(COLUMNS_ROW));
            ui.painter().text(note, Align2::LEFT_CENTER, "Live hides Max, Min", body(11.0), FAINT);
        }

        layout.label(ui, AUTOSTART_ROW, "Start with Windows");
        if let Some(enable) = layout.choice(ui, AUTOSTART_ROW, "autostart", OFF_ON, monitor.autostart) {
            let _ = self.commands.send(Command::SetAutostart(enable));
        }

        layout.label(ui, TRAY_ROW, "Temperature in tray");
        if let Some(enable) = layout.choice(ui, TRAY_ROW, "tray", OFF_ON, self.config.tray_icon) {
            self.config.tray_icon = enable;
        }
        // New icons start out in the overflow, so say where to look.
        let note = pos2(layout.start + 2.0 * 46.0 + 4.0, layout.y(TRAY_ROW));
        ui.painter().text(note, Align2::LEFT_CENTER, "Windows may hide it under ^ at first", body(11.0), FAINT);

        layout.label(ui, TASKBAR_ROW, "Hide taskbar button");
        let current = self.config.hide_taskbar_button;
        if let Some(enable) = layout.choice(ui, TASKBAR_ROW, "hide-taskbar", OFF_ON, current) {
            self.config.hide_taskbar_button = enable;
        }
        let note = if self.config.tray_icon { "Open it from the tray icon instead" } else { "Needs the temperature in tray" };
        let note_pos = pos2(layout.start + 2.0 * 46.0 + 4.0, layout.y(TASKBAR_ROW));
        ui.painter().text(note_pos, Align2::LEFT_CENTER, note, body(11.0), FAINT);

        layout.label(ui, CLOSE_ROW, "Close to tray");
        if let Some(enable) = layout.choice(ui, CLOSE_ROW, "close-to-tray", OFF_ON, self.config.close_to_tray) {
            self.config.close_to_tray = enable;
        }
        let note = if self.config.tray_icon { "Exit from the tray icon's menu" } else { "Needs the temperature in tray" };
        let note_pos = pos2(layout.start + 2.0 * 46.0 + 4.0, layout.y(CLOSE_ROW));
        ui.painter().text(note_pos, Align2::LEFT_CENTER, note, body(11.0), FAINT);

        layout.label(ui, ACCESS_ROW, "Sensor access");
        let status_pos = pos2(layout.start, layout.y(ACCESS_ROW));
        ui.painter().text(status_pos, Align2::LEFT_CENTER, sensor_access(monitor), body(12.5), TEXT);

        self.update_settings(ui, &layout, monitor);
        self.diagnostics_setting(ui, &layout, monitor);
    }

    fn update_settings(&mut self, ui: &Ui, layout: &Layout, monitor: &Monitor) {
        layout.label(ui, UPDATES_ROW, "Check for updates");
        if let Some(enable) = layout.choice(ui, UPDATES_ROW, "updates", OFF_ON, self.config.check_updates) {
            self.config.check_updates = enable;
            let _ = self.commands.send(Command::SetAutomaticUpdates(enable));
        }

        layout.label(ui, VERSION_ROW, &format!("Version {}", update::CURRENT_VERSION));
        let busy = matches!(
            monitor.update,
            UpdateState::Checking | UpdateState::Downloading(_) | UpdateState::Installing(_)
        );
        let (label, command) = match &monitor.update {
            UpdateState::Ready(_) => ("Install", Command::InstallUpdate),
            _ => ("Check now", Command::CheckForUpdate),
        };
        let button = layout.chip(layout.start, VERSION_ROW, 86.0);
        if !busy {
            let response = ui.interact(button, ui.scope_id().with("check-update"), Sense::click());
            chip_painter(ui.painter(), button, label, false, response.hovered());
            if response.clicked() {
                let _ = self.commands.send(command);
            }
        }

        let status = match &monitor.update {
            UpdateState::Idle => String::new(),
            UpdateState::Checking => "Checking…".to_owned(),
            UpdateState::UpToDate => "Up to date".to_owned(),
            UpdateState::Downloading(version) => format!("Downloading {version}…"),
            UpdateState::Ready(version) => format!("{version} is ready"),
            UpdateState::Installing(version) => format!("Installing {version}…"),
            UpdateState::Failed(reason) => elide(reason, 44),
        };
        let x = if busy { layout.start } else { button.right() + 10.0 };
        let shown = ui.painter().text(pos2(x, layout.y(VERSION_ROW)), Align2::LEFT_CENTER, &status, body(11.5), FAINT);
        if let UpdateState::Failed(reason) = &monitor.update {
            let response = ui.interact(shown, ui.scope_id().with("update-failure"), Sense::hover());
            show_full_text(ui, &response, shown, &status, reason);
        }
    }

    fn diagnostics_setting(&mut self, ui: &Ui, layout: &Layout, monitor: &Monitor) {
        layout.label(ui, DIAGNOSTICS_ROW, "Diagnostics");
        let button = layout.chip(layout.start, DIAGNOSTICS_ROW, 86.0);
        let response = ui.interact(button, ui.scope_id().with("diagnostics"), Sense::click());
        chip_painter(ui.painter(), button, "Copy details", false, response.hovered());
        if response.clicked() {
            ui.ctx().copy_text(diagnostics::report(monitor));
            self.copied_at = Some(Instant::now());
            ui.ctx().request_repaint_after(COPIED_NOTE_TIME);
        }

        let copied = self.copied_at.is_some_and(|at| at.elapsed() < COPIED_NOTE_TIME);
        let note = if copied { "Copied, ready to paste" } else { "Hardware and sensor details, for reporting a problem" };
        let note_pos = pos2(button.right() + 10.0, layout.y(DIAGNOSTICS_ROW));
        ui.painter().text(note_pos, Align2::LEFT_CENTER, note, body(11.0), FAINT);
    }
}
