//! Runs the sensor helper and feeds its stream into the [`Monitor`].
//!
//! The UI stays an ordinary process. Only the helper is elevated, it is started
//! through the normal UAC prompt, and data flows one way: helper to UI, over a
//! pipe the UI owns. Closing the UI breaks the pipe, which ends the helper.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, HANDLE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND, ReadFile,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetExitCodeProcess, INFINITE, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{Error, PCWSTR, w};

use crate::autostart;
use crate::model::{Link, Monitor};

const HELPER: &str = "thermal-eye-sensors.exe";
const CONNECT_TIMEOUT_MS: u32 = 30_000;
/// The task needs no one to answer a prompt, so silence means it is not working.
const TASK_CONNECT_TIMEOUT_MS: u32 = 10_000;

pub enum Command {
    /// Start, or restart, the helper with full hardware access.
    Connect,
    /// Turn "Start with Windows" on or off.
    SetAutostart(bool),
}

#[derive(Clone, Copy, PartialEq)]
enum Launch {
    /// Through the scheduled task: elevated, with no prompt.
    Task,
    /// Through the UAC prompt.
    Elevated,
    /// As an ordinary process. GPU and usage figures still work.
    Plain,
}

enum Ended {
    /// The user declined the UAC prompt.
    Declined,
    /// The user asked for a fresh connection.
    Restart,
    /// The scheduled task did not produce a helper.
    TaskFailed,
    Failed(String),
}

#[derive(Clone)]
struct Shared {
    monitor: Arc<Mutex<Monitor>>,
    ctx: egui::Context,
    commands: Sender<Command>,
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut Monitor)) {
        change(&mut self.monitor.lock().unwrap());
        self.ctx.request_repaint();
    }
}

/// Starts the background thread that keeps the helper running.
pub fn start(monitor: Arc<Mutex<Monitor>>, ctx: egui::Context, elevate: bool) -> Sender<Command> {
    let (commands, inbox) = channel();
    let shared = Shared { monitor, ctx, commands: commands.clone() };
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let enabled = autostart::check_enabled();
        shared.update(|monitor| monitor.autostart.enabled = enabled);

        let mut elevate = elevate;
        let mut try_task = true;
        loop {
            let task_ready = || helper_path().is_some_and(|helper| autostart::task_ready(&helper));
            let launch = match (elevate, try_task) {
                (false, _) => Launch::Plain,
                (true, true) if task_ready() => Launch::Task,
                (true, _) => Launch::Elevated,
            };
            let link = if launch == Launch::Elevated { Link::AwaitingConsent } else { Link::Starting };
            shared.update(|monitor| monitor.link = link);

            match session(&shared, &inbox, launch) {
                Ended::Restart => (elevate, try_task) = (true, true),
                Ended::Declined => elevate = false,
                Ended::TaskFailed => try_task = false,
                Ended::Failed(reason) => {
                    shared.update(|monitor| monitor.link = Link::Lost(reason));
                    loop {
                        match inbox.recv() {
                            Ok(Command::Connect) => break,
                            Ok(Command::SetAutostart(on)) => change_autostart(&shared, on),
                            Err(_) => return,
                        }
                    }
                    (elevate, try_task) = (true, true);
                }
            }
        }
    });
    commands
}

/// The outcome of asking Windows to change the autostart setup.
pub enum Change {
    Done,
    Declined,
    Failed,
}

/// Runs the elevated half, then the per-user half. Blocks while the prompt is up.
pub fn apply_autostart(enable: bool) -> Change {
    let Some(helper) = helper_path() else { return Change::Failed };
    let argument = if enable { "--install-autostart" } else { "--remove-autostart" };
    let process = match launch_helper(&helper, argument, true) {
        Ok(process) => process,
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Change::Declined,
        Err(_) => return Change::Failed,
    };
    let mut exit_code = 1;
    unsafe {
        WaitForSingleObject(process.0, INFINITE);
        let _ = GetExitCodeProcess(process.0, &mut exit_code);
    }
    if exit_code != 0 {
        return Change::Failed;
    }
    if !enable {
        autostart::delete_run_entry();
    } else if !autostart::write_run_entry() {
        return Change::Failed;
    }
    Change::Done
}

/// On a thread of its own: the UAC prompt must not hold up the sensor stream.
fn change_autostart(shared: &Shared, enable: bool) {
    let shared = shared.clone();
    shared.update(|monitor| monitor.autostart.busy = true);
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let done = matches!(apply_autostart(enable), Change::Done);
        let mut limited = false;
        shared.update(|monitor| {
            monitor.autostart.busy = false;
            if done {
                monitor.autostart.enabled = enable;
            }
            limited = monitor.link == Link::Connected { elevated: false };
        });
        // The task can now give a limited session full access with no prompt.
        if done && enable && limited {
            let _ = shared.commands.send(Command::Connect);
        }
    });
}

/// Replays a stream captured with `thermal-eye-sensors --record`. Everything is
/// loaded at once so graphs are full, then the capture loops in real time.
pub fn replay(path: PathBuf, monitor: Arc<Mutex<Monitor>>, ctx: egui::Context) -> Sender<Command> {
    let (commands, _inbox) = channel();
    std::thread::spawn(move || {
        let Ok(data) = std::fs::read(path) else {
            monitor.lock().unwrap().link = Link::Lost("Could not read the capture".into());
            return;
        };
        let lines: Vec<&[u8]> = data.split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
        {
            let mut monitor = monitor.lock().unwrap();
            lines.iter().for_each(|line| monitor.ingest(line));
        }
        for line in lines.iter().skip(1).cycle() {
            std::thread::sleep(Duration::from_secs(1));
            monitor.lock().unwrap().ingest(line);
            ctx.request_repaint();
        }
    });
    commands
}

fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let installed = dir.join(HELPER);
    if installed.exists() {
        return Some(installed);
    }
    // Running from target/{debug,release} during development.
    let dev = dir.parent()?.parent()?.join("sensors/bin/Release/net10.0-windows").join(HELPER);
    dev.exists().then_some(dev)
}

fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn launch_helper(path: &Path, parameters: &str, elevate: bool) -> Result<Handle, Error> {
    let file = wide(path);
    let parameters = wide(parameters);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: if elevate { w!("runas") } else { w!("open") },
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info)? };
    Ok(Handle(info.hProcess))
}

fn session(shared: &Shared, inbox: &Receiver<Command>, launch: Launch) -> Ended {
    let Some(helper) = helper_path() else {
        return Ended::Failed(format!("{HELPER} is missing from the app folder"));
    };

    let short_name = if launch == Launch::Task {
        autostart::PIPE.to_owned()
    } else {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        format!("thermal-eye-{}-{stamp}", std::process::id())
    };
    let full_name = wide(format!(r"\\.\pipe\{short_name}"));

    let pipe = unsafe {
        CreateNamedPipeW(
            PCWSTR(full_name.as_ptr()),
            PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            1 << 20,
            0,
            None,
        )
    };
    if pipe.is_invalid() {
        return match launch {
            Launch::Task => Ended::TaskFailed,
            _ => Ended::Failed("Could not create the sensor pipe".into()),
        };
    }
    let pipe = Handle(pipe);

    let process = if launch == Launch::Task {
        if !autostart::run_task() {
            return Ended::TaskFailed;
        }
        None
    } else {
        match launch_helper(&helper, &format!("--pipe {short_name}"), launch == Launch::Elevated) {
            Ok(process) => Some(process),
            Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Ended::Declined,
            Err(e) => return Ended::Failed(format!("Could not start the sensor helper: {}", e.message())),
        }
    };

    let Ok(event) = (unsafe { CreateEventW(None, true, false, None) }) else {
        return Ended::Failed("Could not create a wait event".into());
    };
    let event = Handle(event);
    let mut overlapped = OVERLAPPED { hEvent: event.0, ..Default::default() };

    match unsafe { ConnectNamedPipe(pipe.0, Some(&mut overlapped)) } {
        Ok(()) => {}
        Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
            let wait = unsafe {
                match &process {
                    Some(process) => WaitForMultipleObjects(&[event.0, process.0], false, CONNECT_TIMEOUT_MS),
                    None => WaitForSingleObject(event.0, TASK_CONNECT_TIMEOUT_MS),
                }
            };
            if wait != WAIT_OBJECT_0 {
                let mut ignored = 0;
                unsafe {
                    let _ = CancelIoEx(pipe.0, Some(&overlapped));
                    let _ = GetOverlappedResult(pipe.0, &overlapped, &mut ignored, true);
                }
                return match launch {
                    Launch::Task => Ended::TaskFailed,
                    _ => Ended::Failed("The sensor helper did not connect".into()),
                };
            }
        }
        Err(e) => return Ended::Failed(format!("Pipe error: {}", e.message())),
    }

    read_stream(shared, inbox, &pipe, &event, &mut overlapped)
}

fn read_stream(
    shared: &Shared,
    inbox: &Receiver<Command>,
    pipe: &Handle,
    event: &Handle,
    overlapped: &mut OVERLAPPED,
) -> Ended {
    const LOST: &str = "The sensor helper stopped";
    let mut chunk = vec![0u8; 64 * 1024];
    let mut line = Vec::with_capacity(64 * 1024);

    loop {
        let started = unsafe { ReadFile(pipe.0, Some(&mut chunk), None, Some(overlapped)) };
        if let Err(e) = started {
            let code = e.code();
            if code != ERROR_IO_PENDING.to_hresult() && code != ERROR_MORE_DATA.to_hresult() {
                return Ended::Failed(LOST.into());
            }
        }

        // Wait in short slices so a reconnect request is not stuck behind a read.
        loop {
            let wait = unsafe { WaitForSingleObject(event.0, 200) };
            if wait != WAIT_TIMEOUT {
                break;
            }
            match inbox.try_recv() {
                Err(TryRecvError::Empty) => {}
                Ok(Command::SetAutostart(on)) => change_autostart(shared, on),
                Ok(Command::Connect) | Err(TryRecvError::Disconnected) => {
                    let mut ignored = 0;
                    unsafe {
                        let _ = CancelIoEx(pipe.0, Some(overlapped));
                        let _ = GetOverlappedResult(pipe.0, overlapped, &mut ignored, true);
                    }
                    return Ended::Restart;
                }
            }
        }

        let mut received = 0u32;
        if let Err(e) = unsafe { GetOverlappedResult(pipe.0, overlapped, &mut received, false) } {
            if e.code() != ERROR_MORE_DATA.to_hresult() {
                return Ended::Failed(LOST.into());
            }
        }

        for &byte in &chunk[..received as usize] {
            if byte == b'\n' {
                shared.update(|monitor| monitor.ingest(&line));
                line.clear();
            } else {
                line.push(byte);
            }
        }
    }
}
