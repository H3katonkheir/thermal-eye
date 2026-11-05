# Third-party notices

Thermal Eye itself is under the MIT licence (see `LICENSE`). It ships with the
components below, each under its own licence.

## Sensor helper

These are compiled into `thermal-eye-sensors.exe`, unmodified. The MPL-2.0 ones
are covered by the Mozilla Public License 2.0, <https://mozilla.org/MPL/2.0/>, and
their source code is available at the links shown.

| Component | Licence | Source |
| --- | --- | --- |
| LibreHardwareMonitorLib | MPL-2.0 | <https://github.com/LibreHardwareMonitor/LibreHardwareMonitor> |
| DiskInfoToolkit | MPL-2.0 | <https://github.com/Blacktempel/DiskInfoToolkit> |
| RAMSPDToolkit-NDD | MPL-2.0 | <https://github.com/Blacktempel/RAMSPDToolkit> |
| BlackSharp.Core | MPL-2.0 | <https://github.com/Blacktempel/BlackSharp> |
| HidSharp | Apache-2.0 | <https://software.seekye.com/hidsharp> |
| Mono.Posix.NETStandard | MIT | <https://github.com/mono/mono> |
| .NET libraries (System.Management and others) | MIT | <https://github.com/dotnet/runtime> |

## Widget

`thermal-eye.exe` is built from Rust crates under permissive licences, mainly MIT
and Apache-2.0, with some BSD, ISC, Zlib and Unicode-3.0. The full list is in
`Cargo.lock`, and each crate's licence is published with it on <https://crates.io>.
The main ones are egui and eframe (MIT or Apache-2.0) and windows-rs (MIT or
Apache-2.0).

The egui default fonts are also compiled in, under the SIL Open Font License 1.1
and the Ubuntu Font Licence 1.0.

## Not included

The PawnIO driver and the .NET runtime are installed separately and keep their own
licences. The Windows fonts the widget uses are loaded from the system at runtime.
