//! Finds, downloads and starts updates.
//!
//! Checking and downloading happen in the background with no special rights.
//! Installing is always the user's click and a UAC prompt. The no-prompt sensor
//! task plays no part: it stays read-only, and could not be asked to run anything.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use ureq::Agent;
use ureq::tls::{TlsConfig, TlsProvider};
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};

use crate::bridge;
use crate::manifest::{self, Manifest};
use crate::model::{Monitor, UpdateState};

const REPOSITORY: &str = "https://github.com/H3katonkheir/thermal-eye";
/// Leaves time for the network to come up after signing in.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;
/// `update=1` tells the installer to keep the user's choices and reopen the app.
const INSTALLER_ARGUMENTS: &str = "/SILENT /SUPPRESSMSGBOXES /NORESTART /update=1";

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum Request {
    Check,
    Install,
    SetAutomatic(bool),
}

struct Ready {
    manifest: Manifest,
    path: PathBuf,
}

/// The page describing the running version.
pub fn release_notes_url() -> String {
    format!("{REPOSITORY}/releases/tag/v{CURRENT_VERSION}")
}

/// Where releases come from. Overriding this is safe, and is how updates are
/// tested: every manifest must still carry a valid signature.
fn base_url() -> String {
    std::env::var("THERMAL_EYE_UPDATE_BASE").unwrap_or_else(|_| format!("{REPOSITORY}/releases/latest/download"))
}

fn agent() -> Agent {
    // The Windows TLS stack, so certificates and proxies behave like the rest of the system.
    let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).build();
    Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent(format!("thermal-eye/{CURRENT_VERSION}"))
        .build()
        .into()
}

/// The verified manifest of a newer release, if there is one.
pub fn find_update() -> Result<Option<Manifest>, String> {
    let url = format!("{}/update.json", base_url());
    let mut response = agent().get(&url).call().map_err(|e| format!("Could not reach the update server: {e}"))?;
    let mut text = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_MANIFEST_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| format!("Could not read the update details: {e}"))?;

    let manifest: Manifest = serde_json::from_str(&text).map_err(|_| "The update details are unreadable".to_owned())?;
    if !manifest.verify(manifest::PUBLIC_KEY) {
        return Err("The update's signature is not valid".to_owned());
    }
    Ok(manifest::is_newer(&manifest.version, CURRENT_VERSION).then_some(manifest))
}

fn download_folder() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    let folder = PathBuf::from(local).join("ThermalEye").join("updates");
    fs::create_dir_all(&folder).map_err(|e| format!("Could not create the download folder: {e}"))?;
    Ok(folder)
}

fn matches_hash(path: &PathBuf, expected: &str) -> bool {
    fs::read(path).is_ok_and(|bytes| manifest::sha256_hex(&bytes) == expected)
}

/// Downloads the installer named by a verified manifest and checks its hash.
pub fn download(manifest: &Manifest) -> Result<PathBuf, String> {
    let name = manifest.safe_file_name().ok_or("The update's file name is not valid")?;
    let folder = download_folder()?;
    let path = folder.join(name);
    if matches_hash(&path, &manifest.sha256) {
        return Ok(path);
    }

    // Only the release being fetched stays in the folder.
    for entry in fs::read_dir(&folder).map_err(|e| e.to_string())?.flatten() {
        let _ = fs::remove_file(entry.path());
    }

    let partial = folder.join(format!("{name}.part"));
    let url = format!("{}/{name}", base_url());
    let mut response = agent().get(&url).call().map_err(|e| format!("Could not download the update: {e}"))?;
    let mut reader = response.body_mut().as_reader().take(MAX_INSTALLER_BYTES);
    let mut file = File::create(&partial).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("The download was interrupted: {e}"))?;
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    if !matches_hash(&partial, &manifest.sha256) {
        let _ = fs::remove_file(&partial);
        return Err("The download does not match the signed release".to_owned());
    }
    fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Runs the installer, returning its exit code if the app is still alive afterwards.
///
/// The file is opened so that nothing can write to it, then checked once more:
/// the download folder is writable by any program, and this file is about to run
/// as administrator. The lock is held until the installer has finished.
pub fn install(ready_path: &PathBuf, manifest: &Manifest) -> Result<u32, String> {
    let mut locked = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(ready_path)
        .map_err(|e| format!("Could not open the update: {e}"))?;
    let mut bytes = Vec::new();
    locked.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if manifest::sha256_hex(&bytes) != manifest.sha256 {
        return Err("The downloaded update has changed".to_owned());
    }
    drop(bytes);

    // Not elevated here: the installer asks for itself, which lets it reopen the
    // widget as the signed-in user rather than as administrator.
    let process = bridge::launch(ready_path, INSTALLER_ARGUMENTS, false).map_err(|e| e.message())?;
    let mut exit_code = 1;
    unsafe {
        WaitForSingleObject(process.0, INFINITE);
        let _ = GetExitCodeProcess(process.0, &mut exit_code);
    }
    drop(locked);
    Ok(exit_code)
}

/// Starts the thread that checks for updates.
pub fn start(monitor: Arc<Mutex<Monitor>>, ctx: egui::Context, automatic: bool) -> Sender<Request> {
    let (requests, inbox) = channel();
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let set_state = |state: UpdateState| {
            monitor.lock().unwrap().update = state;
            ctx.request_repaint();
        };
        let mut automatic = automatic;
        let mut ready: Option<Ready> = None;
        let mut next_check = Instant::now() + FIRST_CHECK_DELAY;

        // Automatic checks stay quiet when they fail: being offline is not news.
        let check = |ready: &mut Option<Ready>, quiet: bool| {
            if !quiet {
                set_state(UpdateState::Checking);
            }
            let found = find_update().and_then(|found| match found {
                None => Ok(None),
                Some(manifest) => {
                    set_state(UpdateState::Downloading(manifest.version.clone()));
                    let path = download(&manifest)?;
                    Ok(Some(Ready { manifest, path }))
                }
            });
            match found {
                Ok(Some(update)) => {
                    set_state(UpdateState::Ready(update.manifest.version.clone()));
                    *ready = Some(update);
                }
                Ok(None) => set_state(UpdateState::UpToDate),
                Err(_) if quiet && ready.is_none() => set_state(UpdateState::Idle),
                Err(_) if quiet => {}
                Err(reason) => set_state(UpdateState::Failed(reason)),
            }
        };

        loop {
            let wait = next_check.saturating_duration_since(Instant::now());
            match inbox.recv_timeout(wait) {
                Ok(Request::Check) => check(&mut ready, false),
                Ok(Request::SetAutomatic(on)) => automatic = on,
                Ok(Request::Install) => {
                    let Some(update) = &ready else { continue };
                    let version = update.manifest.version.clone();
                    set_state(UpdateState::Installing(version.clone()));
                    // On success the installer closes this app, so reaching the
                    // end means it was cancelled or failed.
                    match install(&update.path, &update.manifest) {
                        Ok(_) => set_state(UpdateState::Ready(version)),
                        Err(reason) => set_state(UpdateState::Failed(reason)),
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    next_check = Instant::now() + CHECK_INTERVAL;
                    if automatic {
                        check(&mut ready, true);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    requests
}
