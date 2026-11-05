//! "Start with Windows": a per-user Run entry starts the widget at sign-in, and a
//! scheduled task lets it start the elevated helper without a UAC prompt.
//!
//! The helper does the privileged half (see `sensors/Autostart.cs`). The task runs
//! a copy in Program Files, never the one beside this exe, which unelevated
//! programs could replace.

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

fn task_exists() -> bool {
    schtasks(&["/query", "/tn", TASK])
}

pub fn run_task() -> bool {
    schtasks(&["/run", "/tn", TASK])
}

fn installed_helper(file_name: &str) -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("ProgramFiles")?).join("ThermalEye").join(file_name))
}

/// True if the task can start this version of the helper. A copy left behind by
/// an older build is not used: the next elevated start refreshes it.
pub fn task_ready(bundled: &Path) -> bool {
    let name = bundled.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let same = installed_helper(name).and_then(|path| std::fs::read(path).ok()).is_some_and(|installed| {
        std::fs::read(bundled).is_ok_and(|current| current == installed)
    });
    same && task_exists()
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

pub fn write_run_entry() -> bool {
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

pub fn delete_run_entry() {
    let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) };
}

/// Whether autostart is on. Follows the app if its folder has moved.
pub fn check_enabled() -> bool {
    let Some(entry) = run_entry() else { return false };
    if !task_exists() {
        return false;
    }
    if current_command().is_some_and(|command| command != entry) {
        write_run_entry();
    }
    true
}
