//! Starting without prompts. Two independent pieces:
//!
//! - A scheduled task, registered by the installer, lets the widget start the
//!   elevated helper with no UAC prompt. It only applies to an installed copy:
//!   a helper outside Program Files could be replaced by any program.
//! - "Start with Windows" is a per-user Run entry, which needs no elevation.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::core::{PCWSTR, w};

/// The pipe name the task passes to the helper. Tasks have fixed arguments.
pub const PIPE: &str = "thermal-eye-sensors";

const TASK: &str = "Thermal Eye Sensors";
const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("Thermal Eye");

fn schtasks(args: &[&str]) -> bool {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Command::new(PathBuf::from(root).join(r"System32\schtasks.exe"))
        .args(args)
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn run_task() -> bool {
    schtasks(&["/run", "/tn", TASK])
}

/// True if the task can be used to start `helper`.
pub fn task_ready(helper: &Path) -> bool {
    let installed = std::env::var_os("ProgramFiles").is_some_and(|root| helper.starts_with(root));
    installed && schtasks(&["/query", "/tn", TASK])
}

fn run_entry() -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then(|| {
        let length = (size as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buffer[..length])
    })
}

fn current_command() -> Option<String> {
    Some(format!("\"{}\"", std::env::current_exe().ok()?.display()))
}

fn write_run_entry() -> bool {
    let Some(command) = current_command() else { return false };
    let text: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(text.as_ptr().cast()),
            (text.len() * 2) as u32,
        )
    };
    status == ERROR_SUCCESS
}

/// Turns "Start with Windows" on or off, returning whether that worked.
pub fn set(enable: bool) -> bool {
    if enable {
        return write_run_entry();
    }
    let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) };
    run_entry().is_none()
}

/// Whether "Start with Windows" is on. This only reads: another copy of the app
/// starting up must not take over the entry from the installed one.
pub fn is_enabled() -> bool {
    run_entry().is_some()
}
