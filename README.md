# Thermal Eye

A compact Windows desktop widget for system temperatures and usage. For each
component it shows the current, high and average temperature plus average usage,
with an optional history graph and per-core and per-sensor dropdowns.

## Installing

Run `ThermalEyeSetup-<version>.exe`. It asks for permission once, and nothing asks
again after that, including at sign-in. There is nothing else to install first:
.NET is bundled, and setup offers to fetch the PawnIO driver if it is missing.

The installer is not code-signed yet, so Windows may warn about an unrecognised
app. Choose **More info**, then **Run anyway**.

### Requirements

- Windows 11, 64-bit (Windows 10 should work, with square corners)
- [PawnIO](https://pawnio.eu), the signed driver used for CPU, RAM and motherboard
  sensors. Without it the widget still shows GPU temperature, drive temperatures and
  usage, and offers the download.

**FACEIT:** its anti-cheat blocks PawnIO, so anyone who plays on FACEIT should leave
that option unticked. Uninstalling Thermal Eye leaves PawnIO in place, since other
apps may use it. Remove it from Windows' installed apps list if you no longer need it.

### What gets installed

| What | Where |
| --- | --- |
| The app | `C:\Program Files\ThermalEye` |
| A task with no schedule, run only by the widget | Task Scheduler, `Thermal Eye Sensors` |
| A startup entry, if **Start with Windows** is on | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Thermal Eye` |

Only the small sensor helper runs as administrator, the window itself does not. The
task is what lets it start without a prompt, and that is why the app always goes in
Program Files: an elevated task must not point at a file ordinary programs could
replace. **Start with Windows** can be changed at any time under Settings.

## What the columns mean

| Column | Meaning |
| --- | --- |
| **Now** | The hottest of the component's sensors right now |
| **High** | The hottest any of its sensors got during the selected span |
| **Avg** | The mean of its sensors over the selected span |
| **Usage** | Mean usage over the selected span |

The span selector (Live to 24h) drives these figures and the graph together.
History is kept in memory: one sample per second for an hour, then one-minute
buckets for a day. Peaks survive the bucketing. Nothing is written to disk except
settings, in `%APPDATA%\ThermalEye\settings.json`.

Temperatures turn amber or red past each component's limits. RAM uses the limits
its own sensors report; the others use fixed values in `src/model.rs`.

### Component notes

- **CPU**: High includes Tctl, the value fans follow. Avg uses only the CCD die
  sensors, because Tctl reads well above them and would skew the mean.
- **Cores**: the Ryzen sensors available here report temperature per CCD, not per
  core, so temperature sits on the CCD row and core rows show clock, peak, power
  and usage. Clock is the effective clock, which is why idle cores read in MHz.
  Core usage is the mean of its two threads, so one busy thread reads about 50%.
- **System**: the CPU socket sensor is listed but left out of the figures, or the
  row would just repeat the CPU. Labels for X870 AORUS boards are in `board_label`
  in `src/model.rs`, taken from the table the sensor library uses for sibling boards.
- **Fans** reading 0 RPM are hidden, since most are empty headers. A GPU at 0 RPM is
  shown, because that is its normal idle mode.

## Layout

| Path | Purpose |
| --- | --- |
| `src/app.rs` | The widget: chrome, table, settings, compact view |
| `src/graph.rs` | History plot |
| `src/model.rs` | Turns the raw sensor list into components |
| `src/history.rs` | Time series and statistics |
| `src/bridge.rs` | Starts the helper, reads its pipe |
| `src/autostart.rs` | The scheduled task and Start with Windows |
| `src/driver.rs` | Checks for the PawnIO driver |
| `src/topology.rs` | Core and L3 layout from Windows |
| `sensors/` | C# helper around LibreHardwareMonitorLib |
| `installer/` | Inno Setup script |

Why a C# helper in a Rust app: reading these sensors means knowing the register
layouts of each CPU, Super I/O chip and DIMM sensor, and LibreHardwareMonitor is
the maintained source of that knowledge. It runs elevated, so it takes no input
from the UI: data only flows out of it, and it exits when the UI closes.

## Development

Building needs Rust, the .NET SDK and
[Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`).

```powershell
cargo test
.uild.ps1
```

`build.ps1` puts the app and installer in `dist`. Running `dist	hermal-eye.exe`
directly works too, but it asks for permission each launch, because the scheduled
task only serves the copy in Program Files. Start it with `--no-elevate` to skip the
prompt and run with limited sensors.

The UI tests run the real widget headlessly and click through it.

To work on the UI without elevating each run, capture real data once from an
elevated prompt and replay it:

```powershell
.\dist\thermal-eye-sensors.exe --record capture.ndjson --seconds 300
.\dist\thermal-eye.exe --replay capture.ndjson
```

`thermal-eye-sensors.exe --dump sensors.txt` lists everything the library can see.

The icons in `assets/` are generated from `assets/logo-source.webp` by
`tools/make_icons.py`.

## Licence

MIT, see [LICENSE](LICENSE). Bundled components keep their own licences, listed in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
