//! The signed description of a release.
//!
//! Updates run as administrator, so being served over HTTPS is not enough: anyone
//! able to publish to the repository could otherwise reach every user. A manifest
//! only counts if it carries a valid signature from a key that never leaves the
//! developer's machine. It names the version and the installer's hash, so one
//! signature covers the whole release and cannot be reused for another version.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The public half of the release signing key.
pub const PUBLIC_KEY: &str = include_str!("../assets/update-public-key.txt");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub signature: String,
}

impl Manifest {
    fn signed_text(version: &str, file: &str, sha256: &str) -> String {
        format!("thermal-eye-update\n{version}\n{file}\n{sha256}")
    }

    /// Used by the release tool, which shares this file.
    #[allow(dead_code)]
    pub fn sign(version: &str, file: &str, sha256: &str, key: &[u8; 32]) -> Self {
        let text = Self::signed_text(version, file, sha256);
        let signature = SigningKey::from_bytes(key).sign(text.as_bytes());
        Self {
            version: version.to_owned(),
            file: file.to_owned(),
            sha256: sha256.to_owned(),
            signature: to_hex(&signature.to_bytes()),
        }
    }

    pub fn verify(&self, public_key: &str) -> bool {
        let key = from_hex::<32>(public_key.trim()).and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok());
        let signature = from_hex::<64>(&self.signature).map(|bytes| Signature::from_bytes(&bytes));
        let (Some(key), Some(signature)) = (key, signature) else { return false };
        let text = Self::signed_text(&self.version, &self.file, &self.sha256);
        key.verify_strict(text.as_bytes(), &signature).is_ok()
    }

    /// The installer's name, if it is a plain file name. It becomes part of a URL
    /// and a local path, so anything that could point elsewhere is refused.
    pub fn safe_file_name(&self) -> Option<&str> {
        let plain = self.file.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        (plain && !self.file.starts_with('.') && self.file.ends_with(".exe")).then_some(self.file.as_str())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(Sha256::digest(bytes).as_slice())
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn parse_version(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.split('.').map(|part| part.parse::<u32>().ok());
    let version = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(version)
}

/// True only for a clearly higher version, so a signed old release can never be
/// used to move someone backwards onto a version with known problems.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];
    const OTHER_KEY: [u8; 32] = [9; 32];

    fn public_key(secret: &[u8; 32]) -> String {
        to_hex(SigningKey::from_bytes(secret).verifying_key().as_bytes())
    }

    fn manifest() -> Manifest {
        Manifest::sign("1.2.3", "ThermalEyeSetup-1.2.3.exe", &sha256_hex(b"installer"), &KEY)
    }

    #[test]
    fn a_signed_manifest_verifies() {
        assert!(manifest().verify(&public_key(&KEY)));
    }

    #[test]
    fn a_signature_from_any_other_key_is_refused() {
        assert!(!manifest().verify(&public_key(&OTHER_KEY)));
    }

    #[test]
    fn changing_any_signed_field_breaks_the_signature() {
        let key = public_key(&KEY);
        let tampered = [
            Manifest { version: "9.9.9".into(), ..manifest() },
            Manifest { file: "evil.exe".into(), ..manifest() },
            Manifest { sha256: sha256_hex(b"malware"), ..manifest() },
        ];
        for manifest in tampered {
            assert!(!manifest.verify(&key), "{manifest:?}");
        }
    }

    #[test]
    fn a_malformed_signature_or_key_is_refused_without_panicking() {
        let key = public_key(&KEY);
        assert!(!Manifest { signature: "not hex".into(), ..manifest() }.verify(&key));
        assert!(!Manifest { signature: String::new(), ..manifest() }.verify(&key));
        assert!(!manifest().verify("short"));
        assert!(!manifest().verify(""));
    }

    #[test]
    fn the_built_in_public_key_is_usable() {
        let bytes = from_hex::<32>(PUBLIC_KEY.trim()).expect("assets/update-public-key.txt holds a hex key");
        assert!(VerifyingKey::from_bytes(&bytes).is_ok());
    }

    #[test]
    fn only_a_higher_version_counts_as_newer() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"), "compared as numbers, not text");
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn unreadable_versions_are_never_newer() {
        for candidate in ["", "1", "1.2", "1.2.3.4", "1.2.x", "v1.2.3", "1.2.3-beta"] {
            assert!(!is_newer(candidate, "0.1.0"), "{candidate}");
        }
    }

    #[test]
    fn file_names_cannot_leave_the_download_folder() {
        let named = |file: &str| Manifest { file: file.into(), ..manifest() };
        assert_eq!(manifest().safe_file_name(), Some("ThermalEyeSetup-1.2.3.exe"));
        for file in [r"..\evil.exe", "../evil.exe", r"C:\evil.exe", "a/b.exe", ".exe", "setup.bat", "a b.exe"] {
            assert_eq!(named(file).safe_file_name(), None, "{file}");
        }
    }

    #[test]
    fn hex_round_trips_and_rejects_wrong_lengths() {
        assert_eq!(from_hex::<2>(&to_hex(&[0xab, 0x01])), Some([0xab, 0x01]));
        assert_eq!(from_hex::<2>("abc"), None);
        assert_eq!(from_hex::<2>("zzzz"), None);
    }
}
