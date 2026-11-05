//! A small log for working out what went wrong on someone else's PC.
//!
//! Only this process writes it, never the elevated helper: a file in a user-writable
//! folder can be redirected, which would let an admin process overwrite anything.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 512 * 1024;
const RECENT_LINES: usize = 60;

struct Log {
    file: Option<File>,
    recent: VecDeque<String>,
}

static LOG: Mutex<Log> = Mutex::new(Log { file: None, recent: VecDeque::new() });

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("ThermalEye").join("thermal-eye.log"))
}

/// Opens the log, keeping one older file so it cannot grow without limit.
pub fn init() {
    let Some(path) = path() else { return };
    if let Some(folder) = path.parent() {
        let _ = fs::create_dir_all(folder);
    }
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = fs::rename(&path, path.with_extension("old.log"));
    }
    LOG.lock().unwrap().file = OpenOptions::new().create(true).append(true).open(path).ok();

    std::panic::set_hook(Box::new(|info| {
        write(format!("Crash: {info}"));
    }));
}

/// Removes what could identify the person, for anything that may be shared.
pub fn scrub(text: &str) -> String {
    let mut clean = text.to_owned();
    for variable in ["USERPROFILE", "COMPUTERNAME", "USERNAME"] {
        let value = std::env::var(variable).unwrap_or_default();
        if value.len() >= 2 {
            clean = replace_ignoring_case(&clean, &value, &format!("<{}>", variable.to_lowercase()));
        }
    }
    clean
}

fn replace_ignoring_case(text: &str, target: &str, replacement: &str) -> String {
    let (lower_text, lower_target) = (text.to_ascii_lowercase(), target.to_ascii_lowercase());
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(found) = lower_text[position..].find(&lower_target) {
        result.push_str(&text[position..position + found]);
        result.push_str(replacement);
        position += found + target.len();
    }
    result.push_str(&text[position..]);
    result
}

pub fn write(message: impl AsRef<str>) {
    let time = unsafe { GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        scrub(message.as_ref())
    );

    // A panic while the lock is held must not stop the crash being recorded.
    let mut log = LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(file) = &mut log.file {
        let _ = writeln!(file, "{line}");
    }
    if log.recent.len() == RECENT_LINES {
        log.recent.pop_front();
    }
    log.recent.push_back(line);
}

pub fn recent() -> Vec<String> {
    LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).recent.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_ignores_case_and_keeps_the_rest_of_the_text() {
        assert_eq!(replace_ignoring_case(r"C:\Users\Sam\x and sam", "sam", "<user>"), r"C:\Users\<user>\x and <user>");
        assert_eq!(replace_ignoring_case("nothing here", "sam", "<user>"), "nothing here");
    }

    #[test]
    fn the_real_username_never_survives() {
        let name = std::env::var("USERNAME").expect("set on Windows");
        let cleaned = scrub(&format!("failed for {name} in C:\\Users\\{name}\\AppData"));
        assert!(!cleaned.to_lowercase().contains(&name.to_lowercase()), "{cleaned}");
    }

    #[test]
    fn recent_lines_are_kept_in_order_and_capped() {
        for index in 0..RECENT_LINES + 5 {
            write(format!("entry {index}"));
        }
        let recent = recent();
        assert!(recent.len() <= RECENT_LINES);
        assert!(recent.last().unwrap().ends_with(&format!("entry {}", RECENT_LINES + 4)));
    }
}
