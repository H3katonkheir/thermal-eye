#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod bridge;
mod config;
mod diagnostics;
mod driver;
mod graph;
mod history;
mod logfile;
mod logo;
mod manifest;
mod model;
mod theme;
mod topology;
mod update;
mod window;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::ViewportBuilder;

use app::{INITIAL_HEIGHT, ThermalEye};
use config::Config;
use model::Monitor;
use topology::Topology;

fn main() -> eframe::Result {
    if let Some(enable) = autostart_request() {
        std::process::exit(if autostart::set(enable) { 0 } else { 1 });
    }

    let replay = replay_path();
    if replay.is_none() && !window::claim_single_instance() {
        return Ok(());
    }

    logfile::init();
    logfile::write(format!("Thermal Eye {} on {}", update::CURRENT_VERSION, diagnostics::windows_version()));

    let config = Config::load();
    let icon = logo::decode(logo::ICON);
    let width = app::window_width(&config);

    let mut viewport = ViewportBuilder::default()
        .with_title(window::TITLE)
        .with_icon(Arc::new(icon))
        .with_decorations(false)
        .with_resizable(false)
        .with_inner_size([width, INITIAL_HEIGHT]);
    if config.always_on_top {
        viewport = viewport.with_always_on_top();
    }
    let check_updates = config.check_updates;
    let position = config.position.filter(|p| window::is_on_screen(*p));

    let options = eframe::NativeOptions {
        viewport,
        centered: position.is_none(),
        ..Default::default()
    };

    eframe::run_native(
        window::TITLE,
        options,
        Box::new(move |creation| {
            theme::install_fonts(&creation.egui_ctx);
            theme::install_style(&creation.egui_ctx);
            window::round_corners(creation);
            if let Some(position) = position {
                window::move_to(creation, position);
            }

            let ctx = creation.egui_ctx.clone();
            let monitor = Arc::new(Mutex::new(Monitor::new(Topology::read())));
            let commands = match replay {
                Some(path) => bridge::replay(path, monitor.clone(), ctx.clone()),
                None => bridge::start(monitor.clone(), ctx.clone(), !flag("--no-elevate"), check_updates),
            };
            Ok(Box::new(ThermalEye::new(&ctx, monitor, commands, config)))
        }),
    )
}

/// `--autostart on|off` changes the setting without opening the window.
fn autostart_request() -> Option<bool> {
    let mut args = std::env::args_os().skip(1);
    args.find(|arg| arg == "--autostart")?;
    Some(args.next()? == "on")
}

fn flag(name: &str) -> bool {
    std::env::args_os().skip(1).any(|arg| arg == name)
}

/// `--replay <file>` shows a capture made with `thermal-eye-sensors --record`.
fn replay_path() -> Option<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    args.find(|arg| arg == "--replay")?;
    args.next().map(PathBuf::from)
}
