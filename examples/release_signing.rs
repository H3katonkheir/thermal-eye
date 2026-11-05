//! Maintainer tool for signing releases. Not part of the app.
//!
//!     cargo run --example release_signing -- keygen
//!     cargo run --example release_signing -- sign <installer> <version> <output>
//!
//! The private key lives in the user profile, outside the repository. Anyone who
//! has it can push code to every user as administrator, and if it is lost, users
//! can no longer update automatically and must reinstall by hand. Back it up
//! somewhere safe and offline, and never put it in the repo or in CI secrets.

#[path = "../src/manifest.rs"]
#[allow(dead_code)]
mod manifest;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ed25519_dalek::SigningKey;
use manifest::Manifest;

const PUBLIC_KEY_FILE: &str = "assets/update-public-key.txt";

fn key_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE").ok_or("USERPROFILE is not set")?;
    Ok(PathBuf::from(home).join(".thermal-eye").join("update-signing.key"))
}

fn keygen() -> Result<(), String> {
    let path = key_path()?;
    if path.exists() {
        return Err(format!("{} already exists. Replacing it would cut existing users off from updates.", path.display()));
    }
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| e.to_string())?;

    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, manifest::to_hex(&secret)).map_err(|e| e.to_string())?;
    let public = SigningKey::from_bytes(&secret).verifying_key();
    std::fs::write(PUBLIC_KEY_FILE, manifest::to_hex(public.as_bytes())).map_err(|e| e.to_string())?;

    println!("Private key: {}", path.display());
    println!("Public key:  {PUBLIC_KEY_FILE}");
    println!("Back up the private key. It cannot be recovered.");
    Ok(())
}

fn sign(installer: &str, version: &str, output: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(key_path()?).map_err(|e| format!("No signing key: {e}"))?;
    let secret = manifest::from_hex::<32>(text.trim()).ok_or("The signing key is malformed")?;
    let bytes = std::fs::read(installer).map_err(|e| format!("{installer}: {e}"))?;
    let file = Path::new(installer).file_name().and_then(|n| n.to_str()).ok_or("Bad installer path")?;

    let signed = Manifest::sign(version, file, &manifest::sha256_hex(&bytes), &secret);
    if signed.safe_file_name().is_none() {
        return Err(format!("{file} is not a name the app will accept"));
    }

    // Read the public key at run time: a build made before keygen has an old copy.
    let public = std::fs::read_to_string(PUBLIC_KEY_FILE).map_err(|e| e.to_string())?;
    if !signed.verify(&public) {
        return Err(format!("{PUBLIC_KEY_FILE} does not match the private key, so users would reject this release"));
    }

    let json = serde_json::to_string_pretty(&signed).map_err(|e| e.to_string())?;
    std::fs::write(output, json).map_err(|e| e.to_string())?;
    println!("Signed {file} as version {version}");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["keygen"] => keygen(),
        ["sign", installer, version, output] => sign(installer, version, output),
        _ => Err("usage: release_signing keygen | sign <installer> <version> <output>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
