//! The report behind "Copy details": what someone can paste when asking for help.
//!
//! It lists hardware names and readings, which is what is needed to debug a sensor,
//! and scrubs the username and PC name. Drive serial numbers are never collected.

use std::fmt::Write;

use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{PCWSTR, w};

use crate::driver;
use crate::logfile;
use crate::model::{Link, Monitor};
use crate::update;

fn windows_value(name: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 128];
    let mut size = size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| String::from_utf16_lossy(&buffer[..(size as usize / 2).saturating_sub(1)]))
}

pub fn windows_version() -> String {
    let release = windows_value(w!("DisplayVersion")).unwrap_or_default();
    let build = windows_value(w!("CurrentBuild")).unwrap_or_else(|| "unknown".into());
    format!("Windows {release} build {build}").replace("  ", " ")
}

fn is_installed() -> bool {
    let root = std::env::var_os("ProgramFiles");
    let exe = std::env::current_exe().ok();
    root.zip(exe).is_some_and(|(root, exe)| exe.starts_with(root))
}

fn link_text(link: &Link) -> String {
    match link {
        Link::Starting => "starting".into(),
        Link::AwaitingConsent => "waiting for permission".into(),
        Link::Connected { elevated: true } => "connected, full access".into(),
        Link::Connected { elevated: false } => "connected, limited (not elevated)".into(),
        Link::Lost(reason) => format!("lost: {reason}"),
    }
}

pub fn report(monitor: &Monitor) -> String {
    let mut text = String::new();
    let install = if is_installed() { "installed" } else { "portable" };
    let _ = writeln!(text, "Thermal Eye {} ({install})", update::CURRENT_VERSION);
    let _ = writeln!(text, "{}", windows_version());
    let _ = writeln!(text, "Sensors: {}, started via {}", link_text(&monitor.link), monitor.launch);
    let _ = writeln!(text, "PawnIO driver: {}", if driver::is_installed() { "installed" } else { "not installed" });
    let _ = writeln!(text, "Start with Windows: {}", if monitor.autostart { "on" } else { "off" });
    let _ = writeln!(text, "Update status: {:?}", monitor.update);

    let _ = writeln!(text, "\nSensors now:");
    for hardware in &monitor.hardware {
        let _ = writeln!(text, "[{}] {}", hardware.kind, hardware.name);
        for sensor in &hardware.sensors {
            let value = monitor.latest(sensor.series).map_or("no reading".to_owned(), |v| format!("{v:.2}"));
            let _ = writeln!(text, "  {:<12} {:<34} {value}", sensor.kind, sensor.name);
        }
    }

    let _ = writeln!(text, "\nRecent log:");
    for line in logfile::recent() {
        let _ = writeln!(text, "{line}");
    }
    logfile::scrub(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::Topology;

    const SCHEMA: &[u8] = include_bytes!("../tests/fixtures/schema.json");

    fn monitor() -> Monitor {
        let mut monitor = Monitor::new(Topology::default());
        monitor.ingest(SCHEMA);
        monitor
    }

    #[test]
    fn the_report_names_the_hardware_and_says_what_has_no_reading() {
        let report = report(&monitor());
        assert!(report.contains("AMD Ryzen 9 9950X3D"));
        assert!(report.contains("Core (Tctl/Tdie)"));
        assert!(report.contains("no reading"));
        assert!(report.contains("connected, full access"));
    }

    #[test]
    fn the_report_never_contains_the_username_or_pc_name() {
        logfile::write(format!(r"opened C:\Users\{}\thing", std::env::var("USERNAME").unwrap()));
        let report = report(&monitor()).to_lowercase();
        for variable in ["USERNAME", "COMPUTERNAME"] {
            let value = std::env::var(variable).unwrap().to_lowercase();
            assert!(!report.contains(&value), "{variable} leaked");
        }
    }

    #[test]
    fn the_windows_version_is_readable() {
        assert!(windows_version().contains("build "));
        assert!(!windows_version().contains("unknown"));
    }
}
