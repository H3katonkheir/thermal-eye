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

## Updates

Thermal Eye looks for a new version at launch and once a day, and you can check
any time under Settings. Updates download in the background, then a banner offers
**Install**. That takes one Windows permission prompt, and the widget reopens by
itself. Afterwards, **What's new** opens the release notes. Your settings, Start
with Windows and PawnIO choices are left as they were.

Each check downloads one small file from GitHub, which is the only time the app goes
online by itself. Turn **Check for updates** off under Settings to stop it.

Updates are signed: the app refuses anything without a valid signature from the
release key, refuses to install an older version, and checks the installer's hash
again immediately before running it.

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
| `src/update.rs` | Finds, downloads and starts updates |
| `src/manifest.rs` | The signed description of a release |
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

## Releasing

1. Set the new `version` in `Cargo.toml`.
2. Run `.\build.ps1`. With the signing key present, this also writes `dist\update.json`.
3. Create a GitHub release tagged `v<version>`, with the changes in its notes. The
   tag must match, because **What's new** links straight to it.
4. Attach `dist\ThermalEyeSetup-<version>.exe` and `dist\update.json` to that release.

Installed copies read `update.json` from the latest release, so attach both files
before publishing, and do not mark it as a pre-release. The repository must be
public, or friends' copies cannot reach it.

### The signing key

Updates are signed with a private key at
`%USERPROFILE%\.thermal-eye\update-signing.key`, and the matching public key is
built into the app from `assets/update-public-key.txt`.

- **Back it up** somewhere safe and offline. If it is lost, existing installs can no
  longer be updated, and everyone has to reinstall by hand.
- **Never** commit it or add it to CI. Whoever holds it can send code to every user
  as administrator.

To create a key on a new checkout: `cargo run --example release_signing -- keygen`.
This refuses to replace an existing key, because a new key cuts existing users off.

### Testing an update

`THERMAL_EYE_UPDATE_BASE` points the app at another folder of releases, such as a
local web server. Signatures are still required, so this cannot be used to bypass them.

## Licence

MIT, see [LICENSE](LICENSE). Bundled components keep their own licences, listed in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
