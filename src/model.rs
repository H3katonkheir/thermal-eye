//! Turns the helper's flat sensor list into the components shown in the widget.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::history::{History, SeriesId, Stats};
use crate::topology::Topology;

/// Temperatures outside this range are disconnected inputs or failed reads
/// (without driver access the CPU reports a literal 0).
const PLAUSIBLE_TEMP: std::ops::RangeInclusive<f32> = 1.0..=150.0;

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Message {
    Schema { admin: bool, hardware: Vec<HardwareDef>, sensors: Vec<SensorDef> },
    Values { v: Vec<Option<f32>> },
}

#[derive(Deserialize)]
struct HardwareDef {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct SensorDef {
    id: String,
    hw: String,
    name: String,
    kind: String,
}

impl SensorDef {
    fn is(&self, kind: &str, name: &str) -> bool {
        self.kind == kind && self.name == name
    }
}

/// A sensor line under a component: a temperature, a usage figure, or both.
#[derive(Clone, Debug)]
pub struct Row {
    /// Stable identity, used to remember graph selections.
    pub key: String,
    pub label: String,
    pub temp: Option<SeriesId>,
    pub usage: Option<SeriesId>,
}

#[derive(Clone, Debug)]
pub struct Core {
    pub key: String,
    pub label: String,
    pub usage: SeriesId,
    pub clock: Option<SeriesId>,
    pub power: Option<SeriesId>,
}

/// A CPU die: its temperature row and the cores on it.
#[derive(Clone, Debug)]
pub struct Die {
    pub row: Row,
    pub note: Option<String>,
    pub cores: Vec<Core>,
}

#[derive(Clone, Copy, Debug)]
pub enum Unit {
    Megahertz,
    Watts,
    Rpm,
}

#[derive(Clone, Debug)]
pub enum Extra {
    Reading { label: String, series: SeriesId, unit: Unit },
    /// Used and total, with the factor that converts them to gigabytes.
    Capacity { label: String, used: SeriesId, total: SeriesId, to_gb: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Cpu,
    Gpu,
    Memory,
    Storage,
    System,
}

#[derive(Clone, Debug)]
pub struct Component {
    pub key: String,
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    /// Hottest sensor at each sample.
    pub high: SeriesId,
    /// Mean of the component's sensors at each sample.
    pub avg: SeriesId,
    pub usage: Option<SeriesId>,
    pub warn: f32,
    pub critical: f32,
    pub rows: Vec<Row>,
    pub dies: Vec<Die>,
    pub extras: Vec<Extra>,
}

impl Component {
    pub fn has_details(&self) -> bool {
        !self.rows.is_empty() || !self.dies.is_empty() || !self.extras.is_empty()
    }
}

/// Progress of the updater. Versions are the release being offered.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading(String),
    Ready(String),
    Installing(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Starting,
    AwaitingConsent,
    Connected { elevated: bool },
    Lost(String),
}

enum Rule {
    Max(Vec<SeriesId>),
    Mean(Vec<SeriesId>),
    Sum(Vec<SeriesId>),
}

struct Derived {
    out: SeriesId,
    rule: Rule,
}

impl Derived {
    fn compute(&self, history: &History) -> f32 {
        let inputs = match &self.rule {
            Rule::Max(ids) | Rule::Mean(ids) | Rule::Sum(ids) => ids,
        };
        let values = inputs.iter().map(|&id| history.pending(id)).filter(|v| !v.is_nan());
        match self.rule {
            Rule::Max(_) => values.fold(f32::NAN, f32::max),
            Rule::Sum(_) => values.fold(f32::NAN, |a, v| if a.is_nan() { v } else { a + v }),
            Rule::Mean(_) => {
                let (sum, count) = values.fold((0.0, 0), |(s, n), v| (s + v, n + 1));
                if count == 0 { f32::NAN } else { sum / count as f32 }
            }
        }
    }
}

struct Binding {
    series: SeriesId,
    is_temperature: bool,
}

/// Sensors that report the limits a component should be judged against.
struct Limits {
    component: usize,
    warn: SeriesId,
    critical: SeriesId,
}

pub struct Monitor {
    pub link: Link,
    /// "Start with Windows" is on.
    pub autostart: bool,
    /// The PawnIO driver is not installed, so most temperatures are unavailable.
    pub driver_missing: bool,
    pub update: UpdateState,
    pub components: Vec<Component>,
    pub history: History,
    topology: Topology,
    bindings: Vec<Binding>,
    derived: Vec<Derived>,
    limits: Vec<Limits>,
}

impl Monitor {
    pub fn new(topology: Topology) -> Self {
        Self {
            link: Link::Starting,
            autostart: false,
            driver_missing: false,
            update: UpdateState::Idle,
            components: Vec::new(),
            history: History::default(),
            topology,
            bindings: Vec::new(),
            derived: Vec::new(),
            limits: Vec::new(),
        }
    }

    /// Applies one line of the helper's stream.
    pub fn ingest(&mut self, line: &[u8]) {
        match serde_json::from_slice(line) {
            Ok(Message::Schema { admin, hardware, sensors }) => {
                self.link = Link::Connected { elevated: admin };
                self.apply_schema(&hardware, &sensors);
            }
            Ok(Message::Values { v }) => self.apply_values(&v),
            Err(_) => {}
        }
    }

    fn apply_schema(&mut self, hardware: &[HardwareDef], sensors: &[SensorDef]) {
        let mut builder = Builder {
            history: &mut self.history,
            topology: &self.topology,
            hardware,
            sensors,
            derived: Vec::new(),
            components: Vec::new(),
            limits: Vec::new(),
        };
        builder.cpu();
        builder.gpus();
        builder.memory();
        builder.storage();
        builder.system();
        let (components, derived, limits) = (builder.components, builder.derived, builder.limits);
        self.components = components;
        self.derived = derived;
        self.limits = limits;
        self.bindings = sensors
            .iter()
            .map(|sensor| Binding {
                series: self.history.id(&sensor.id),
                is_temperature: sensor.kind == "Temperature",
            })
            .collect();
    }

    fn apply_values(&mut self, values: &[Option<f32>]) {
        for (binding, value) in self.bindings.iter().zip(values) {
            let Some(value) = value else { continue };
            if !binding.is_temperature || PLAUSIBLE_TEMP.contains(value) {
                self.history.set(binding.series, *value);
            }
        }
        for rule in &self.derived {
            self.history.set(rule.out, rule.compute(&self.history));
        }
        for limit in &self.limits {
            let component = &mut self.components[limit.component];
            let warn = self.history.pending(limit.warn);
            let critical = self.history.pending(limit.critical);
            if !warn.is_nan() && !critical.is_nan() {
                component.warn = warn;
                component.critical = critical;
            }
        }
        self.history.commit();
    }

    pub fn stats(&self, series: SeriesId, seconds: u32) -> Stats {
        self.history.series(series).stats(seconds)
    }

    pub fn latest(&self, series: SeriesId) -> Option<f32> {
        self.history.series(series).latest()
    }
}

struct Builder<'a> {
    history: &'a mut History,
    topology: &'a Topology,
    hardware: &'a [HardwareDef],
    sensors: &'a [SensorDef],
    derived: Vec<Derived>,
    components: Vec<Component>,
    limits: Vec<Limits>,
}

impl<'a> Builder<'a> {
    fn of(&self, hardware: &HardwareDef) -> Vec<&'a SensorDef> {
        self.sensors.iter().filter(|s| s.hw == hardware.id).collect()
    }

    fn find(&mut self, hardware: &HardwareDef, kind: &str, name: &str) -> Option<SeriesId> {
        let sensor = self.of(hardware).into_iter().find(|s| s.is(kind, name))?;
        Some(self.history.id(&sensor.id))
    }

    fn derive(&mut self, key: &str, rule: Rule) -> SeriesId {
        let out = self.history.id(key);
        self.derived.push(Derived { out, rule });
        out
    }

    fn temp_row(&mut self, sensor: &SensorDef, label: String) -> Row {
        Row {
            key: format!("t:{}", sensor.id),
            label,
            temp: Some(self.history.id(&sensor.id)),
            usage: None,
        }
    }

    fn reading(&mut self, hw: &HardwareDef, kind: &str, name: &str, label: &str, unit: Unit) -> Option<Extra> {
        let series = self.find(hw, kind, name)?;
        Some(Extra::Reading { label: label.into(), series, unit })
    }

    /// CPU temperatures come from Tctl and the CCD dies. Tctl is the control
    /// value the fans follow, not a die reading, so it counts towards the high
    /// but only stands in for the average when there are no die sensors.
    fn cpu(&mut self) {
        let Some(hw) = self.hardware.iter().find(|h| h.kind == "Cpu") else { return };

        let mut rows = Vec::new();
        let mut die_temps: BTreeMap<usize, Row> = BTreeMap::new();
        for sensor in self.of(hw).into_iter().filter(|s| s.kind == "Temperature") {
            if is_summary(&sensor.name) {
                continue;
            }
            if let Some(die) = ccd_number(&sensor.name) {
                let row = self.temp_row(sensor, format!("CCD{die}"));
                die_temps.insert(die, row);
            } else {
                let label = if sensor.name.contains("Tctl") { "Tctl".into() } else { sensor.name.clone() };
                rows.push(self.temp_row(sensor, label));
            }
        }

        let dies = self.cpu_dies(hw, &die_temps);
        let die_series: Vec<_> = die_temps.values().filter_map(|row| row.temp).collect();
        let other_series: Vec<_> = rows.iter().filter_map(|row| row.temp).collect();
        let hot = [die_series.as_slice(), &other_series].concat();
        let mean = if die_series.is_empty() { other_series } else { die_series };

        let high = self.derive("cpu/high", Rule::Max(hot));
        let avg = self.derive("cpu/avg", Rule::Mean(mean));
        let usage = self.find(hw, "Load", "CPU Total");
        let extras = [
            self.reading(hw, "Power", "Package", "Package", Unit::Watts),
            self.reading(hw, "Clock", "Cores (Average Effective)", "Avg clock", Unit::Megahertz),
        ];
        self.components.push(Component {
            key: "cpu".into(),
            kind: Kind::Cpu,
            title: "CPU".into(),
            subtitle: short_cpu_name(&hw.name),
            high,
            avg,
            usage,
            warn: 85.0,
            critical: 95.0,
            rows,
            dies,
            extras: extras.into_iter().flatten().collect(),
        });
    }

    fn cpu_dies(&mut self, hw: &HardwareDef, temps: &BTreeMap<usize, Row>) -> Vec<Die> {
        let threads = self.cpu_threads(hw);

        // Group physical cores by L3 domain; one group if the layout is unknown.
        let mut groups: BTreeMap<usize, Vec<(usize, Vec<SeriesId>)>> = BTreeMap::new();
        for (core, series) in threads {
            let processor = self.topology.cores.get(core).and_then(|c| c.first().copied());
            let group = processor.and_then(|p| self.topology.l3_of(p)).unwrap_or(0);
            groups.entry(group).or_default().push((core, series));
        }

        let largest = self.topology.l3.iter().map(|d| d.size_mb).max().unwrap_or(0);
        let smallest = self.topology.l3.iter().map(|d| d.size_mb).min().unwrap_or(0);

        let mut dies = Vec::new();
        for (group, members) in groups {
            let number = group + 1;
            let cores = members.iter().map(|(core, series)| self.core(hw, *core, series.clone())).collect();
            let all: Vec<SeriesId> = members.iter().flat_map(|(_, s)| s.iter().copied()).collect();
            let usage = self.derive(&format!("die/{number}/usage"), Rule::Mean(all));

            let mut row = temps.get(&number).cloned().unwrap_or(Row {
                key: format!("die:{number}"),
                label: format!("CCD{number}"),
                temp: None,
                usage: None,
            });
            row.usage = Some(usage);

            let size_mb = self.topology.l3.get(group).map_or(0, |d| d.size_mb);
            let note = (size_mb > 0).then(|| {
                if size_mb == largest && largest > smallest {
                    format!("{size_mb} MB L3 · V-Cache")
                } else {
                    format!("{size_mb} MB L3")
                }
            });
            dies.push(Die { row, note, cores });
        }
        dies
    }

    fn core(&mut self, hw: &HardwareDef, core: usize, threads: Vec<SeriesId>) -> Core {
        let n = core + 1;
        let name = format!("Core #{n}");
        let effective = format!("{name} (Effective)");
        let clock = self.find(hw, "Clock", &effective).or_else(|| self.find(hw, "Clock", &name));
        let power = self.find(hw, "Power", &format!("{name} (SMU)")).or_else(|| self.find(hw, "Power", &name));
        Core {
            key: format!("core:{n}"),
            label: format!("Core {n}"),
            usage: self.derive(&format!("core/{n}/usage"), Rule::Mean(threads)),
            clock,
            power,
        }
    }

    /// Per-thread load sensors, grouped by physical core. The library names them
    /// either `CPU Core #c Thread #t` or, on this generation, `CPU Core #<logical>`.
    fn cpu_threads(&mut self, hw: &HardwareDef) -> BTreeMap<usize, Vec<SeriesId>> {
        let mut threads: BTreeMap<usize, Vec<SeriesId>> = BTreeMap::new();
        for sensor in self.of(hw).into_iter().filter(|s| s.kind == "Load") {
            let Some(rest) = sensor.name.strip_prefix("CPU Core #") else { continue };
            let Some(number) = leading_number(rest) else { continue };
            let core = if rest.contains("Thread") {
                number - 1
            } else {
                self.topology.core_of(number - 1).unwrap_or(number - 1)
            };
            threads.entry(core).or_default().push(self.history.id(&sensor.id));
        }
        threads
    }

    fn gpus(&mut self) {
        let gpus: Vec<_> = self
            .hardware
            .iter()
            .filter(|h| h.kind.starts_with("Gpu"))
            .filter(|h| self.of(h).iter().any(|s| s.kind == "Temperature" || s.is("Load", "GPU Core")))
            .collect();

        for (index, hw) in gpus.iter().enumerate() {
            let sensors = self.of(hw);
            let mut rows = Vec::new();
            for sensor in sensors.iter().filter(|s| s.kind == "Temperature") {
                let label = sensor.name.strip_prefix("GPU ").unwrap_or(&sensor.name).to_owned();
                rows.push(self.temp_row(sensor, label));
            }
            let temps: Vec<_> = rows.iter().filter_map(|row| row.temp).collect();
            let key = format!("gpu{index}");
            let high = self.derive(&format!("{key}/high"), Rule::Max(temps.clone()));
            let avg = self.derive(&format!("{key}/avg"), Rule::Mean(temps));
            let usage = self.find(hw, "Load", "GPU Core").or_else(|| self.find(hw, "Load", "D3D 3D"));

            let mut extras = Vec::new();
            if let Some(power) = sensors.iter().find(|s| s.kind == "Power") {
                extras.extend(self.reading(hw, "Power", &power.name, "Power", Unit::Watts));
            }
            extras.extend(self.reading(hw, "Clock", "GPU Core", "Clock", Unit::Megahertz));
            if let (Some(used), Some(total)) = (
                self.find(hw, "SmallData", "GPU Memory Used"),
                self.find(hw, "SmallData", "GPU Memory Total"),
            ) {
                extras.push(Extra::Capacity { label: "VRAM".into(), used, total, to_gb: 1.0 / 1024.0 });
            }
            extras.extend(self.reading(hw, "Fan", "GPU Fan", "Fan", Unit::Rpm));

            self.components.push(Component {
                key,
                kind: Kind::Gpu,
                title: if gpus.len() > 1 { format!("GPU {}", index + 1) } else { "GPU".into() },
                subtitle: hw.name.replace("AMD Radeon ", "").replace("NVIDIA GeForce ", ""),
                high,
                avg,
                usage,
                warn: 90.0,
                critical: 105.0,
                rows,
                dies: Vec::new(),
                extras,
            });
        }
    }

    fn memory(&mut self) {
        let Some(total_hw) = self.hardware.iter().find(|h| h.id == "/ram") else { return };

        let mut rows = Vec::new();
        let mut limits = None;
        let dimms: Vec<_> = self.hardware.iter().filter(|h| h.id.starts_with("/memory/dimm")).collect();
        for hw in &dimms {
            for sensor in self.of(hw).into_iter().filter(|s| s.kind == "Temperature" && s.name.starts_with("DIMM")) {
                rows.push(self.temp_row(sensor, sensor.name.replace('#', "")));
            }
            if limits.is_none() {
                let warn = self.find(hw, "Temperature", "Thermal Sensor High Limit");
                let critical = self.find(hw, "Temperature", "Thermal Sensor Critical High Limit");
                limits = warn.zip(critical);
            }
        }

        let temps: Vec<_> = rows.iter().filter_map(|row| row.temp).collect();
        let high = self.derive("ram/high", Rule::Max(temps.clone()));
        let avg = self.derive("ram/avg", Rule::Mean(temps));
        let usage = self.find(total_hw, "Load", "Memory");

        let mut extras = Vec::new();
        if let (Some(used), Some(free)) = (
            self.find(total_hw, "Data", "Memory Used"),
            self.find(total_hw, "Data", "Memory Available"),
        ) {
            let total = self.derive("ram/total", Rule::Sum(vec![used, free]));
            extras.push(Extra::Capacity { label: "In use".into(), used, total, to_gb: 1.0 });
        }

        let subtitle = match dimms.first() {
            Some(hw) => format!("{} × {}", dimms.len(), short_dimm_name(&hw.name)),
            None => "System memory".into(),
        };
        if let Some((warn, critical)) = limits {
            self.limits.push(Limits { component: self.components.len(), warn, critical });
        }
        self.components.push(Component {
            key: "ram".into(),
            kind: Kind::Memory,
            title: "RAM".into(),
            subtitle,
            high,
            avg,
            usage,
            warn: 55.0,
            critical: 85.0,
            rows,
            dies: Vec::new(),
            extras,
        });
    }

    fn storage(&mut self) {
        let drives: Vec<_> = self.hardware.iter().filter(|h| h.kind == "Storage").collect();
        if drives.is_empty() {
            return;
        }

        let mut rows = Vec::new();
        let mut activity = Vec::new();
        for hw in &drives {
            let name = hw.name.trim();
            let drive_usage = self.find(hw, "Load", "Total Activity");
            activity.extend(drive_usage);
            let temps = self
                .of(hw)
                .into_iter()
                .filter(|s| s.kind == "Temperature" && !s.name.contains("Warning") && !s.name.contains("Critical"));
            for (index, sensor) in temps.enumerate() {
                let label = if index == 0 { name.to_owned() } else { format!("{name} · sensor {}", index + 1) };
                let mut row = self.temp_row(sensor, label);
                row.usage = if index == 0 { drive_usage } else { None };
                rows.push(row);
            }
        }

        let temps: Vec<_> = rows.iter().filter_map(|row| row.temp).collect();
        let high = self.derive("storage/high", Rule::Max(temps.clone()));
        let avg = self.derive("storage/avg", Rule::Mean(temps));
        let usage = (!activity.is_empty()).then(|| self.derive("storage/usage", Rule::Mean(activity)));
        self.components.push(Component {
            key: "storage".into(),
            kind: Kind::Storage,
            title: "Storage".into(),
            subtitle: format!("{} drives", drives.len()),
            high,
            avg,
            usage,
            warn: 60.0,
            critical: 75.0,
            rows,
            dies: Vec::new(),
            extras: Vec::new(),
        });
    }

    /// Motherboard sensors. The sensor on the CPU socket is listed but kept out of
    /// the figures, otherwise the System row would just repeat the CPU row.
    fn system(&mut self) {
        let chips: Vec<_> = self.hardware.iter().filter(|h| h.kind == "SuperIO").collect();
        if chips.is_empty() {
            return;
        }
        let board = self.hardware.iter().find(|h| h.kind == "Motherboard").map_or("", |h| h.name.as_str());

        let mut rows = Vec::new();
        let mut extras = Vec::new();
        let mut counted = Vec::new();
        for hw in &chips {
            for sensor in self.of(hw) {
                let label = board_label(board, &sensor.id).unwrap_or_else(|| sensor.name.replace('#', ""));
                match sensor.kind.as_str() {
                    "Temperature" => {
                        let row = self.temp_row(sensor, label.clone());
                        if !label.starts_with("CPU") {
                            counted.extend(row.temp);
                        }
                        rows.push(row);
                    }
                    "Fan" => {
                        let series = self.history.id(&sensor.id);
                        extras.push(Extra::Reading { label, series, unit: Unit::Rpm });
                    }
                    _ => {}
                }
            }
        }

        let high = self.derive("system/high", Rule::Max(counted.clone()));
        let avg = self.derive("system/avg", Rule::Mean(counted));
        self.components.push(Component {
            key: "system".into(),
            kind: Kind::System,
            title: "System".into(),
            subtitle: board.replace("Gigabyte ", ""),
            high,
            avg,
            usage: None,
            warn: 70.0,
            critical: 90.0,
            rows,
            dies: Vec::new(),
            extras,
        });
    }
}

fn is_summary(name: &str) -> bool {
    matches!(name, "CCDs Max (Tdie)" | "CCDs Average (Tdie)" | "Core Max" | "Core Average")
}

fn ccd_number(name: &str) -> Option<usize> {
    leading_number(name.strip_prefix("CCD")?)
}

fn leading_number(text: &str) -> Option<usize> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    text[..end].parse().ok().filter(|&n| n > 0)
}

fn short_cpu_name(name: &str) -> String {
    name.replace("AMD ", "").replace("Intel ", "").replace(" Processor", "")
}

/// "A-DATA Technology - AX5U6000C3032G-BLABK (#1)" becomes "A-DATA AX5U6000C3032G-BLABK".
fn short_dimm_name(name: &str) -> String {
    let name = name.split(" (#").next().unwrap_or(name);
    name.replace(" Technology", "").replace(" - ", " ")
}

/// The library has no label table for this exact board, but the X870/X870E AORUS
/// boards it does know all share one, and the readings here line up with it
/// (the third input tracks Tctl to within a degree).
fn board_label(board: &str, sensor: &str) -> Option<String> {
    if !(board.contains("X870") && board.contains("AORUS")) {
        return None;
    }
    let label = match sensor.strip_prefix("/lpc/")? {
        "it8696e/0/temperature/0" => "System 1",
        "it8696e/0/temperature/1" => "Chipset",
        "it8696e/0/temperature/2" => "CPU socket",
        "it8696e/0/temperature/3" => "PCIe x16",
        "it8696e/0/temperature/4" => "VRM MOS",
        "it8696e/0/temperature/5" => "EC",
        "it87952e/0/temperature/0" => "PCIe x4",
        "it87952e/0/temperature/1" => "External 2",
        "it87952e/0/temperature/2" => "System 2",
        "it8696e/0/fan/0" => "CPU fan",
        "it8696e/0/fan/1" => "Sys fan 1",
        "it8696e/0/fan/2" => "Sys fan 2",
        "it8696e/0/fan/3" => "Sys fan 3",
        "it8696e/0/fan/4" => "CPU opt fan",
        "it87952e/0/fan/0" => "Sys fan 5",
        "it87952e/0/fan/1" => "Sys fan 6",
        "it87952e/0/fan/2" => "Sys fan 4",
        _ => return None,
    };
    Some(label.to_owned())
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
