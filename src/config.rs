//! User preferences, stored as JSON under %APPDATA%\ThermalEye.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub high: bool,
    pub avg: bool,
    pub usage: bool,
}

impl Default for Metrics {
    fn default() -> Self {
        Self { high: true, avg: false, usage: true }
    }
}

/// The table columns that can be hidden to save room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub min: bool,
    pub max: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Self { min: true, max: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub fahrenheit: bool,
    pub always_on_top: bool,
    pub compact: bool,
    pub graph_open: bool,
    /// Span of the high, average and usage figures, and of the graph.
    pub window_seconds: u32,
    /// Components showing their sensors.
    pub expanded: Vec<String>,
    /// Components and sensors drawn on the graph.
    pub graphed: Vec<String>,
    pub metrics: Metrics,
    pub columns: Columns,
    pub hidden: Vec<String>,
    /// Look for new versions at launch and once a day.
    pub check_updates: bool,
    /// The version last run, so the first run after an update can say so.
    pub seen_version: Option<String>,
    pub position: Option<[f32; 2]>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fahrenheit: false,
            always_on_top: false,
            compact: false,
            graph_open: false,
            window_seconds: 300,
            expanded: Vec::new(),
            graphed: vec!["cpu".into(), "gpu0".into()],
            metrics: Metrics::default(),
            columns: Columns::default(),
            hidden: Vec::new(),
            check_updates: true,
            seen_version: None,
            position: None,
        }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("THERMAL_EYE_CONFIG") {
            return Some(path.into());
        }
        Some(PathBuf::from(std::env::var_os("APPDATA")?).join("ThermalEye").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let Ok(json) = serde_json::to_vec_pretty(self) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }
}

/// Adds or removes `key`, returning nothing because callers only flip state.
pub fn toggle(list: &mut Vec<String>, key: &str) {
    match list.iter().position(|item| item == key) {
        Some(index) => {
            list.remove(index);
        }
        None => list.push(key.to_owned()),
    }
}

pub fn contains(list: &[String], key: &str) -> bool {
    list.iter().any(|item| item == key)
}
