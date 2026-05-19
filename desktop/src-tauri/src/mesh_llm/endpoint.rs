//! Iroh endpoint keypair: persisted per Sprout install.
//!
//! The user's Nostr identity (`identity.key`) is separate from the iroh
//! endpoint identity. The Nostr key signs kind:31990 offers and the NIP-98
//! admission bearer; the iroh key proves possession of the iroh `EndpointId`
//! during QUIC handshake. The kind:31990 event's Nostr signature binds the
//! two identities together — anyone who trusts the Nostr pubkey can trust
//! the advertised endpoint id, because nobody else could have signed that
//! offer.
//!
//! We deliberately do **not** derive the iroh key from the Nostr key:
//!
//! - It would couple key rotation: rotating the Nostr key would silently
//!   change the iroh endpoint id, breaking active offers.
//! - It would force a particular HKDF over the Nostr seckey, picking a new
//!   custody convention nobody else implements.
//! - The iroh key is generated once, never leaves the desktop, and is
//!   already inside the same Tauri sandbox as `identity.key`. Two files,
//!   one trust boundary.

use std::path::{Path, PathBuf};

use iroh_base::SecretKey;
use tauri::{AppHandle, Manager};

const KEY_FILENAME: &str = "mesh_iroh.key";

/// Errors loading or creating the iroh endpoint keypair.
#[derive(Debug, thiserror::Error)]
pub enum EndpointKeyError {
    /// Couldn't determine the Tauri app data dir.
    #[error("app data dir: {0}")]
    AppDataDir(String),
    /// Filesystem I/O failure.
    #[error("filesystem: {0}")]
    Io(String),
    /// On-disk key file exists but is malformed.
    #[error("malformed key file: {0}")]
    MalformedKeyFile(String),
}

/// Resolve the iroh endpoint key file path under the Tauri app data dir.
fn key_path(app: &AppHandle) -> Result<PathBuf, EndpointKeyError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| EndpointKeyError::AppDataDir(e.to_string()))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| EndpointKeyError::Io(e.to_string()))?;
    Ok(data_dir.join(KEY_FILENAME))
}

/// Load the persisted iroh endpoint keypair, generating + saving one on
/// first run. Mirrors the pattern in [`crate::app_state::resolve_persisted_identity`].
///
/// File format: 32 raw secret-key bytes encoded as lower-case hex on a single
/// line. Matches what `iroh_base::SecretKey`'s `FromStr` accepts.
pub fn load_or_create_endpoint_key(app: &AppHandle) -> Result<SecretKey, EndpointKeyError> {
    let path = key_path(app)?;

    if path.exists() {
        match load_key_file(&path) {
            Ok(k) => return Ok(k),
            Err(e) => {
                // Quarantine corrupt files so we never overwrite a usable
                // backup — same pattern as `identity.key`.
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let bad = path.with_extension(format!("bad.{ts}"));
                let _ = std::fs::rename(&path, &bad);
                eprintln!(
                    "sprout-desktop: corrupt mesh_iroh.key ({e}), quarantined to {}",
                    bad.display(),
                );
            }
        }
    }

    let key = SecretKey::generate();
    save_key_file(&path, &key)?;
    eprintln!(
        "sprout-desktop: generated and saved mesh iroh endpoint pubkey {}",
        key.public(),
    );
    Ok(key)
}

fn load_key_file(path: &Path) -> Result<SecretKey, EndpointKeyError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| EndpointKeyError::Io(e.to_string()))?;
    let trimmed = content.trim();
    trimmed
        .parse::<SecretKey>()
        .map_err(|e| EndpointKeyError::MalformedKeyFile(e.to_string()))
}

fn save_key_file(path: &Path, key: &SecretKey) -> Result<(), EndpointKeyError> {
    let bytes = key.to_bytes();
    let hex = hex::encode(bytes);
    // Atomic write: write to a temp file in the same dir, fsync, rename.
    let dir = path
        .parent()
        .ok_or_else(|| EndpointKeyError::Io("key path has no parent".to_string()))?;
    let tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| EndpointKeyError::Io(format!("temp file: {e}")))?;
    std::fs::write(tmp.path(), hex.as_bytes())
        .map_err(|e| EndpointKeyError::Io(format!("write temp: {e}")))?;
    tmp.persist(path)
        .map_err(|e| EndpointKeyError::Io(format!("rename temp: {e}")))?;
    // No fsync of the directory here — matches the existing identity.key path.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Round-trip a generated key through the save/load functions.
    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mesh_iroh.key");
        let original = SecretKey::generate();
        save_key_file(&path, &original).expect("save");
        let loaded = load_key_file(&path).expect("load");
        assert_eq!(original.to_bytes(), loaded.to_bytes());
    }

    /// A truncated/corrupted file is rejected with `MalformedKeyFile`.
    #[test]
    fn corrupt_file_returns_malformed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mesh_iroh.key");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"not a valid hex key").unwrap();
        drop(f);
        let err = load_key_file(&path).expect_err("should fail");
        match err {
            EndpointKeyError::MalformedKeyFile(_) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
