# Thermal Eye

A compact Windows desktop widget for system temperatures and usage. For each
component it shows the current, high and average temperature plus average usage,
with an optional history graph and per-core and per-sensor dropdowns.

## Running

```powershell
.\build.ps1
.\dist\thermal-eye.exe
```

Windows asks for permission at launch: only the small sensor helper runs as
administrator, the window itself does not. Decline, or start with `--no-elevate`,
and the widget still shows usage and GPU temperature, and offers **Grant access**.

Keep `thermal-eye.exe` and `thermal-eye-sensors.exe` in the same folder.

### Start with Windows

Turn this on under Settings, or with `thermal-eye.exe --autostart on`. It asks for
permission once, and from then on the widget opens at sign-in and at every launch
with no prompt. Turning it off (`--autostart off`) also asks once.

It sets up three things:

| What | Where |
| --- | --- |
| A startup entry for the widget | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Thermal Eye` |
| A task with no schedule, run only by the widget | Task Scheduler, `Thermal Eye Sensors` |
| A copy of the sensor helper | `C:\Program Files\ThermalEye` |

The copy exists for safety: the task runs as administrator, so it must point at a
file that ordinary programs cannot replace. If the widget is running when you turn
this off, the copy stays until the next launch, which removes it.

After rebuilding the helper, the next launch prompts once and refreshes the copy.
Turn autostart off before deleting or moving the app for good.

### Requirements

- Windows 11 (Windows 10 should work, with square corners)
- [.NET 10 runtime](https://dotnet.microsoft.com/download) or newer, for the sensor helper
- [PawnIO](https://pawnio.eu), the signed driver used for CPU, RAM and motherboard sensors

Building also needs Rust and the .NET SDK.

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
| `src/topology.rs` | Core and L3 layout from Windows |
| `sensors/` | C# helper around LibreHardwareMonitorLib |

Why a C# helper in a Rust app: reading these sensors means knowing the register
layouts of each CPU, Super I/O chip and DIMM sensor, and LibreHardwareMonitor is
the maintained source of that knowledge. It runs elevated, so it takes no input
from the UI: data only flows out of it, and it exits when the UI closes.

## Development

```powershell
cargo test
```

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
