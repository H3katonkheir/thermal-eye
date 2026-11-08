use super::*;
use crate::topology::Topology;

const SCHEMA: &[u8] = include_bytes!("../tests/fixtures/schema.json");
const CPU_TCTL: &str = "/amdcpu/0/temperature/2";
const GPU_HOT_SPOT: &str = "/gpu-amd/5/temperature/7";
const DRIVE: &str = "/nvme/3/temperature/0";

fn settings() -> Settings {
    Settings { enabled: true, fahrenheit: false, hidden: Vec::new(), hide_taskbar_button: true }
}

/// A monitor that has had one sample, with only the given sensors reading.
fn monitor(readings: &[(&str, f32)]) -> Monitor {
    let mut monitor = Monitor::new(Topology { cores: Vec::new(), l3: Vec::new() });
    monitor.ingest(SCHEMA);
    let schema: serde_json::Value = serde_json::from_slice(SCHEMA).unwrap();
    let ids: Vec<&str> = schema["sensors"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    let mut values = vec![None; ids.len()];
    for (id, value) in readings {
        values[ids.iter().position(|s| s == id).unwrap()] = Some(*value);
    }
    let line = serde_json::json!({ "t": "values", "v": values }).to_string();
    monitor.ingest(line.as_bytes());
    monitor
}

#[test]
fn shows_the_hottest_component_in_its_colour() {
    let monitor = monitor(&[(CPU_TCTL, 61.4), (GPU_HOT_SPOT, 72.6), (DRIVE, 40.0)]);
    let shown = reading(&monitor, &settings());
    let gpu = monitor.components.iter().find(|c| c.key == "gpu0").unwrap();
    assert_eq!(shown.face, Some(("73".to_owned(), heat_text(72.6, gpu.warn, gpu.critical))));
    assert_eq!(shown.tip, "Thermal Eye\nCPU 61°\nGPU 73°\nStorage 40°");
}

#[test]
fn leaves_out_hidden_components_and_follows_the_unit() {
    let monitor = monitor(&[(CPU_TCTL, 61.4), (GPU_HOT_SPOT, 72.6)]);
    let settings = Settings { fahrenheit: true, hidden: vec!["gpu0".into()], ..settings() };
    let shown = reading(&monitor, &settings);
    assert_eq!(shown.face.unwrap().0, "143");
    assert_eq!(shown.tip, "Thermal Eye\nCPU 143°");
}

#[test]
fn falls_back_to_the_app_icon_without_readings() {
    let shown = reading(&monitor(&[]), &settings());
    assert_eq!(shown, Reading { face: None, tip: "Thermal Eye".into() });
}

#[test]
fn the_number_fills_the_tile_without_touching_its_edges() {
    for size in [16, 20, 24, 32] {
        for text in ["7", "45", "100"] {
            let pixels = tile(text, TEXT_COLOR, size).unwrap();
            assert_eq!(pixels.len(), (size * size * 4) as usize);
            let alpha = |x: i32, y: i32| pixels[((y * size + x) * 4 + 3) as usize];
            assert_eq!(alpha(0, 0), 0, "corners are rounded off");
            assert_eq!(alpha(size / 2, 0), 255, "the tile is opaque");

            // Text pixels are the ones lighter than the tile.
            let lit: Vec<(i32, i32)> = (0..size * size)
                .map(|i| (i % size, i / size))
                .filter(|&(x, y)| pixels[((y * size + x) * 4 + 1) as usize] > 0x60)
                .collect();
            let left = lit.iter().map(|p| p.0).min().unwrap();
            let right = lit.iter().map(|p| p.0).max().unwrap();
            let top = lit.iter().map(|p| p.1).min().unwrap();
            let bottom = lit.iter().map(|p| p.1).max().unwrap();
            assert!(left >= 1 && top >= 1 && right < size - 1 && bottom < size - 1, "{text} at {size}px");
            if text.len() < 3 {
                assert!(bottom - top >= size / 2, "{text} at {size}px is readable");
            }
            assert!((left - (size - 1 - right)).abs() <= 1, "{text} at {size}px is centred");
        }
    }
}

#[test]
fn draws_an_icon() {
    let icon = draw_icon("45", TEXT_COLOR, 24).unwrap();
    unsafe { DestroyIcon(icon).unwrap() };
}

const TEXT_COLOR: Color32 = crate::theme::TEXT;

/// Writes the tiles to `THERMAL_EYE_TILES` as raw BGRA, for looking at them.
#[test]
#[ignore]
fn dump_tiles() {
    let dir = std::path::PathBuf::from(std::env::var_os("THERMAL_EYE_TILES").unwrap());
    for size in [16, 24, 32] {
        for (text, color) in [("45", TEXT_COLOR), ("88", crate::theme::AMBER), ("100", crate::theme::RED)] {
            std::fs::write(dir.join(format!("{text}-{size}.bgra")), tile(text, color, size).unwrap()).unwrap();
        }
    }
}
