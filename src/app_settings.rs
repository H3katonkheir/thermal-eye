//! The settings panel, opened from the title bar.

use std::time::Instant;

use eframe::egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{COPIED_NOTE_TIME, PAD, SETTINGS_ROW_H, ThermalEye, X_NAME, elide, is_live, show_full_text};
use crate::bridge::Command;
use crate::config;
use crate::diagnostics;
use crate::model::{Link, Monitor, UpdateState};
use crate::theme::*;
use crate::update;

/// Unit, components, columns, start with Windows, sensor access, update checks, version, diagnostics.
const ROWS: usize = 8;
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

    pub(super) fn settings_panel(&mut self, ui: &mut Ui, width: f32, monitor: &Monitor) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, self.settings_height()), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, NESTED_BG);
        ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, BORDER));
        let left = rect.left() + PAD + X_NAME;
        let layout = Layout { top: rect.top(), left, start: left + CONTROLS_X };

        layout.label(ui, 0, "Temperature unit");
        let units = [("°C", false), ("°F", true)];
        if let Some(fahrenheit) = layout.choice(ui, 0, "unit", units, self.config.fahrenheit) {
            self.config.fahrenheit = fahrenheit;
        }

        layout.label(ui, 1, "Show components");
        let mut x = layout.start;
        for component in &monitor.components {
            let chip = 22.0 + component.title.len() as f32 * 7.0;
            let rect = layout.chip(x, 1, chip);
            let response = ui.interact(rect, ui.scope_id().with(("show", &component.key)), Sense::click());
            let shown = !config::contains(&self.config.hidden, &component.key);
            chip_painter(ui.painter(), rect, &component.title, shown, response.hovered());
            if response.clicked() {
                config::toggle(&mut self.config.hidden, &component.key);
            }
            x += chip + 6.0;
        }

        layout.label(ui, 2, "Show columns");
        let columns = &mut self.config.columns;
        let mut x = layout.start;
        for (text, shown) in [("Min", &mut columns.min), ("Max", &mut columns.max)] {
            let rect = layout.chip(x, 2, 40.0);
            let response = ui.interact(rect, ui.scope_id().with(("column", text)), Sense::click());
            chip_painter(ui.painter(), rect, text, *shown, response.hovered());
            if response.clicked() {
                *shown = !*shown;
            }
            x += 46.0;
        }
        if is_live(&self.config) {
            let note = pos2(x + 4.0, layout.y(2));
            ui.painter().text(note, Align2::LEFT_CENTER, "Not used in Live", body(11.0), FAINT);
        }

        layout.label(ui, 3, "Start with Windows");
        if let Some(enable) = layout.choice(ui, 3, "autostart", OFF_ON, monitor.autostart) {
            let _ = self.commands.send(Command::SetAutostart(enable));
        }

        layout.label(ui, 4, "Sensor access");
        let status_pos = pos2(layout.start, layout.y(4));
        ui.painter().text(status_pos, Align2::LEFT_CENTER, sensor_access(monitor), body(12.5), TEXT);

        self.update_settings(ui, &layout, monitor);
        self.diagnostics_setting(ui, &layout, monitor);
    }

    fn update_settings(&mut self, ui: &Ui, layout: &Layout, monitor: &Monitor) {
        layout.label(ui, 5, "Check for updates");
        if let Some(enable) = layout.choice(ui, 5, "updates", OFF_ON, self.config.check_updates) {
            self.config.check_updates = enable;
            let _ = self.commands.send(Command::SetAutomaticUpdates(enable));
        }

        layout.label(ui, 6, &format!("Version {}", update::CURRENT_VERSION));
        let busy = matches!(
            monitor.update,
            UpdateState::Checking | UpdateState::Downloading(_) | UpdateState::Installing(_)
        );
        let (label, command) = match &monitor.update {
            UpdateState::Ready(_) => ("Install", Command::InstallUpdate),
            _ => ("Check now", Command::CheckForUpdate),
        };
        let button = layout.chip(layout.start, 6, 86.0);
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
        let shown = ui.painter().text(pos2(x, layout.y(6)), Align2::LEFT_CENTER, &status, body(11.5), FAINT);
        if let UpdateState::Failed(reason) = &monitor.update {
            let response = ui.interact(shown, ui.scope_id().with("update-failure"), Sense::hover());
            show_full_text(ui, &response, shown, &status, reason);
        }
    }

    fn diagnostics_setting(&mut self, ui: &Ui, layout: &Layout, monitor: &Monitor) {
        layout.label(ui, 7, "Diagnostics");
        let button = layout.chip(layout.start, 7, 86.0);
        let response = ui.interact(button, ui.scope_id().with("diagnostics"), Sense::click());
        chip_painter(ui.painter(), button, "Copy details", false, response.hovered());
        if response.clicked() {
            ui.ctx().copy_text(diagnostics::report(monitor));
            self.copied_at = Some(Instant::now());
            ui.ctx().request_repaint_after(COPIED_NOTE_TIME);
        }

        let copied = self.copied_at.is_some_and(|at| at.elapsed() < COPIED_NOTE_TIME);
        let note = if copied { "Copied, ready to paste" } else { "Hardware and sensor details, for reporting a problem" };
        let note_pos = pos2(button.right() + 10.0, layout.y(7));
        ui.painter().text(note_pos, Align2::LEFT_CENTER, note, body(11.0), FAINT);
    }
}
