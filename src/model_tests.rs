use super::*;
use crate::topology::L3Domain;

/// The sensor list reported by the machine this was built on.
const SCHEMA: &[u8] = include_bytes!("../tests/fixtures/schema.json");

const TCTL: &str = "/amdcpu/0/temperature/2";
const CCD1: &str = "/amdcpu/0/temperature/3";
const CCD2: &str = "/amdcpu/0/temperature/4";

/// 16 cores with two threads each, on one 96 MB and one 32 MB L3 domain.
fn topology() -> Topology {
    Topology {
        cores: (0..16).map(|core| vec![core * 2, core * 2 + 1]).collect(),
        l3: vec![
            L3Domain { processors: (0..16).collect(), size_mb: 96 },
            L3Domain { processors: (16..32).collect(), size_mb: 32 },
        ],
    }
}

fn sensor_ids() -> Vec<String> {
    let schema: serde_json::Value = serde_json::from_slice(SCHEMA).unwrap();
    schema["sensors"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_owned()).collect()
}

fn monitor() -> Monitor {
    let mut monitor = Monitor::new(topology());
    monitor.ingest(SCHEMA);
    monitor
}

/// Sends one sample with the given sensors set and the rest unavailable.
fn sample(monitor: &mut Monitor, readings: &[(&str, f32)]) {
    let ids = sensor_ids();
    let mut values = vec![None; ids.len()];
    for (id, value) in readings {
        let index = ids.iter().position(|s| s == id).unwrap_or_else(|| panic!("no sensor {id}"));
        values[index] = Some(*value);
    }
    let line = serde_json::json!({ "t": "values", "v": values }).to_string();
    monitor.ingest(line.as_bytes());
}

fn component<'m>(monitor: &'m Monitor, key: &str) -> &'m Component {
    monitor.components.iter().find(|c| c.key == key).unwrap()
}

#[test]
fn groups_hardware_into_components_and_skips_the_sensorless_igpu() {
    let monitor = monitor();
    let keys: Vec<_> = monitor.components.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["cpu", "gpu0", "ram", "storage", "system"]);
    assert_eq!(component(&monitor, "cpu").subtitle, "Ryzen 9 9950X3D");
    assert_eq!(component(&monitor, "gpu0").subtitle, "RX 9070 XT");
    assert_eq!(component(&monitor, "gpu0").title, "GPU");
}

#[test]
fn cores_sit_on_their_die_and_the_larger_cache_is_marked() {
    let monitor = monitor();
    let dies = &component(&monitor, "cpu").dies;
    assert_eq!(dies.len(), 2);
    assert_eq!(dies[0].cores.len(), 8);
    assert_eq!(dies[1].cores[0].label, "Core 9");
    assert_eq!(dies[0].note.as_deref(), Some("96 MB L3 · V-Cache"));
    assert_eq!(dies[1].note.as_deref(), Some("32 MB L3"));
}

#[test]
fn core_usage_averages_its_two_threads() {
    let mut monitor = monitor();
    // Logical processors 0 and 1 share physical core 1.
    sample(&mut monitor, &[("/amdcpu/0/load/2", 10.0), ("/amdcpu/0/load/3", 30.0)]);
    let core = &component(&monitor, "cpu").dies[0].cores[0];
    assert_eq!(monitor.latest(core.usage), Some(20.0));
}

#[test]
fn tctl_raises_the_cpu_high_but_is_not_averaged_with_the_dies() {
    let mut monitor = monitor();
    sample(&mut monitor, &[(TCTL, 70.0), (CCD1, 40.0), (CCD2, 50.0)]);
    let cpu = component(&monitor, "cpu");
    assert_eq!(monitor.latest(cpu.high), Some(70.0));
    assert_eq!(monitor.latest(cpu.avg), Some(45.0));
}

#[test]
fn a_zero_reading_means_no_data_not_zero_degrees() {
    let mut monitor = monitor();
    sample(&mut monitor, &[(TCTL, 0.0)]);
    let cpu = component(&monitor, "cpu");
    assert_eq!(monitor.latest(cpu.high), None);
    assert!(!monitor.history.series(cpu.rows[0].temp.unwrap()).has_data());
}

#[test]
fn system_lists_the_cpu_socket_but_leaves_it_out_of_its_figures() {
    let mut monitor = monitor();
    sample(&mut monitor, &[
        ("/lpc/it8696e/0/temperature/2", 99.0),
        ("/lpc/it8696e/0/temperature/4", 50.0),
        ("/lpc/it8696e/0/temperature/0", 30.0),
    ]);
    let system = component(&monitor, "system");
    assert_eq!(monitor.latest(system.high), Some(50.0));
    assert_eq!(monitor.latest(system.avg), Some(40.0));
    let labels: Vec<_> = system.rows.iter().map(|r| r.label.as_str()).collect();
    assert!(labels.contains(&"CPU socket") && labels.contains(&"VRM MOS"));
}

#[test]
fn ram_shows_only_dimm_readings_and_adopts_the_limits_they_report() {
    let mut monitor = monitor();
    assert_eq!(component(&monitor, "ram").rows.len(), 2);
    sample(&mut monitor, &[
        ("/memory/dimm/1/temperature/3", 60.0),
        ("/memory/dimm/1/temperature/5", 90.0),
    ]);
    let ram = component(&monitor, "ram");
    assert_eq!((ram.warn, ram.critical), (60.0, 90.0));
}

#[test]
fn storage_ignores_threshold_sensors_and_averages_drive_activity() {
    let mut monitor = monitor();
    sample(&mut monitor, &[("/nvme/3/load/53", 40.0), ("/hdd/1/load/53", 20.0)]);
    let storage = component(&monitor, "storage");
    assert!(!storage.rows.iter().any(|r| r.key.contains("temperature/10")));
    assert_eq!(monitor.latest(storage.usage.unwrap()), Some(30.0));
}

#[test]
fn a_problem_reported_by_the_helper_reaches_the_log() {
    let mut monitor = monitor();
    monitor.ingest(br#"{"t":"log","message":"IOException: driver went away"}"#);
    assert!(crate::logfile::recent().iter().any(|line| line.ends_with("Helper: IOException: driver went away")));
    assert!(monitor.components.len() == 5, "the stream carries on");
}

#[test]
fn history_survives_the_sensor_list_being_announced_again() {
    let mut monitor = monitor();
    sample(&mut monitor, &[(CCD1, 40.0)]);
    monitor.ingest(SCHEMA);
    sample(&mut monitor, &[(CCD1, 60.0)]);
    let high = component(&monitor, "cpu").high;
    assert_eq!(monitor.stats(high, 60).high, Some(60.0));
    assert_eq!(monitor.stats(high, 60).avg, Some(50.0));
}
