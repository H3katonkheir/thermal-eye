# Developing Thermal Eye

## Building

You need Rust, the .NET 10 SDK and
[Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`).

```powershell
cargo test
.\build.ps1
```

`build.ps1` puts the app and installer in `dist`. Running `dist\thermal-eye.exe` directly
works too, but it asks for permission each launch, because the scheduled task only
serves the copy in Program Files. Start it with `--no-elevate` to skip the prompt and run
with limited sensors.

Every push is also built and tested by [GitHub Actions](../.github/workflows/build.yml),
which keeps the installer as a downloadable artifact.

## Layout

| Path | Purpose |
| --- | --- |
| `src/app.rs` | The widget: chrome, banners, table, compact view |
| `src/app_settings.rs` | The settings panel |
| `src/graph.rs` | History plot |
| `src/model.rs` | Turns the raw sensor list into components |
| `src/history.rs` | Time series and statistics |
| `src/bridge.rs` | Starts the helper, reads its pipe |
| `src/autostart.rs` | The scheduled task and Start with Windows |
| `src/driver.rs` | Checks for the PawnIO driver |
| `src/update.rs` | Finds, downloads and starts updates |
| `src/manifest.rs` | The signed description of a release |
| `src/logfile.rs` | The log, and removing personal details from it |
| `src/diagnostics.rs` | The report behind Copy details |
| `src/topology.rs` | Core and L3 layout from Windows |
| `sensors/` | C# helper around LibreHardwareMonitorLib |
| `installer/` | Inno Setup script |
| `examples/release_signing.rs` | Creates the signing key and signs releases |
| `tools/make_icons.py` | Generates the icons from `assets/logo-source.webp` |

Why a C# helper in a Rust app: reading these sensors means knowing the register layouts
of each CPU, Super I/O chip and DIMM sensor, and LibreHardwareMonitor is the maintained
source of that knowledge.

## Security rules

The helper runs as administrator without a prompt, which sets some firm limits:

- **It takes no input from the UI.** Data only flows out of it, over a pipe the UI owns,
  and it exits when the UI closes. The pipe client refuses impersonation.
- **It writes no files.** A file in a user-writable folder can be redirected, so it
  reports problems through the pipe and the UI writes the log.
- **The scheduled task only ever points into Program Files.** Registration refuses to
  run from anywhere else, so ordinary programs cannot replace what it runs.
- **It plays no part in updates.** Installing is always a click and a UAC prompt.

## Working on the UI

The UI tests run the real widget headlessly and click through it, in `src/app_tests.rs`.

To run the app without elevating each time, capture real data once from an elevated
prompt and replay it:

```powershell
.\dist\thermal-eye-sensors.exe --record capture.ndjson --seconds 300
.\dist\thermal-eye.exe --replay capture.ndjson
```

`thermal-eye-sensors.exe --dump sensors.txt` lists everything the library can see, and
the `THERMAL_EYE_CONFIG` environment variable points the app at a different settings file.

## Releasing

1. Set the new `version` in `Cargo.toml`.
2. Run `.\build.ps1`. With the signing key present, this also writes `dist\update.json`.
3. Publish it, with the changes as the release notes:

   ```powershell
   gh release create v<version> dist\ThermalEyeSetup-<version>.exe dist\update.json --title "<version>" --notes-file notes.md
   ```

Installed copies read `update.json` from the latest release, and **What's new** links to
the `v<version>` tag, so keep that name and do not mark it as a pre-release. The repository
must be public, or installed copies cannot reach it.

### The signing key

Updates are signed with a private key at `%USERPROFILE%\.thermal-eye\update-signing.key`,
and the matching public key is built into the app from `assets/update-public-key.txt`.

- **Keep a backup** somewhere safe and offline. If it is lost, existing installs can no
  longer be updated, and everyone has to reinstall by hand.
- **Never** commit it or add it to CI. Whoever holds it can send code to every user as
  administrator.

To create a key on a new checkout: `cargo run --example release_signing -- keygen`. This
refuses to replace an existing key, because a new key cuts existing users off.

### Testing an update

`THERMAL_EYE_UPDATE_BASE` points the app at another folder of releases, such as a local
web server. Signatures are still required, so this cannot be used to bypass them. The
first check happens 30 seconds after launch.
