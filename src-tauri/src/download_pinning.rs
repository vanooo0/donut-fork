//! Trust-on-first-use pinning for downloaded browser archives.
//!
//! The browser build is fetched from a URL named by a small JSON manifest on
//! a remote server, and upstream ships no checksum for it — so whoever
//! controls that manifest can point this machine at any file and it will be
//! unpacked and executed. There is no published hash to compare against, so
//! the strongest check available locally is: remember the SHA-256 of the
//! archive the first time a given browser version is fetched, and refuse it
//! later if the bytes ever change.
//!
//! That does not help if the very first download is already hostile, so it is
//! paired with `ensure_https`, which rejects a manifest that tries to
//! downgrade the transfer to plaintext HTTP.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PinStore {
  /// `"<browser>/<version>/<filename>"` -> lowercase hex SHA-256.
  #[serde(default)]
  pins: HashMap<String, String>,
}

fn pins_path() -> PathBuf {
  crate::app_dirs::settings_dir().join("download_pins.json")
}

fn pin_key(browser: &str, version: &str, filename: &str) -> String {
  format!("{browser}/{version}/{filename}")
}

fn load_store() -> PinStore {
  std::fs::read_to_string(pins_path())
    .ok()
    .and_then(|raw| serde_json::from_str(&raw).ok())
    .unwrap_or_default()
}

fn save_store(store: &PinStore) -> Result<(), String> {
  let path = pins_path();
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create settings dir: {e}"))?;
  }
  let json = serde_json::to_string_pretty(store).map_err(|e| format!("Failed to serialize: {e}"))?;
  std::fs::write(&path, json).map_err(|e| format!("Failed to write download pins: {e}"))
}

/// Reject a download URL that isn't HTTPS. A hijacked manifest could
/// otherwise ask for plaintext HTTP and let anyone on the network path swap
/// the browser build in transit.
pub fn ensure_https(url: &str) -> Result<(), String> {
  let lower = url.trim().to_ascii_lowercase();
  if lower.starts_with("https://") {
    return Ok(());
  }
  Err(
    serde_json::json!({
      "code": "DOWNLOAD_INSECURE_URL",
      "params": { "url": url }
    })
    .to_string(),
  )
}

/// SHA-256 of a file, streamed so a ~1 GB archive never lands in memory.
pub fn hash_file(path: &Path) -> Result<String, String> {
  let file = std::fs::File::open(path).map_err(|e| format!("Failed to open download: {e}"))?;
  let mut reader = std::io::BufReader::new(file);
  let mut hasher = Sha256::new();
  let mut buf = vec![0u8; 1024 * 1024];
  loop {
    let n = reader
      .read(&mut buf)
      .map_err(|e| format!("Failed to read download: {e}"))?;
    if n == 0 {
      break;
    }
    hasher.update(&buf[..n]);
  }
  Ok(format!("{:x}", hasher.finalize()))
}

/// Outcome of checking a freshly downloaded archive against the pin store.
pub enum PinCheck {
  /// First time this version was seen; the hash is now recorded.
  Pinned(String),
  /// Hash matches what was recorded earlier.
  Matched(String),
}

/// Compare a downloaded archive against its recorded hash, recording it on
/// first sight. `Err` means the bytes changed for a version already seen —
/// the caller must delete the file and refuse to install it.
pub fn verify_or_pin(
  browser: &str,
  version: &str,
  filename: &str,
  archive_path: &Path,
) -> Result<PinCheck, String> {
  let actual = hash_file(archive_path)?;
  let key = pin_key(browser, version, filename);
  let mut store = load_store();

  match store.pins.get(&key) {
    Some(expected) if expected.eq_ignore_ascii_case(&actual) => Ok(PinCheck::Matched(actual)),
    Some(expected) => Err(
      serde_json::json!({
        "code": "DOWNLOAD_HASH_MISMATCH",
        "params": {
          "browser": browser,
          "version": version,
          "expected": expected.clone(),
          "actual": actual
        }
      })
      .to_string(),
    ),
    None => {
      store.pins.insert(key, actual.clone());
      save_store(&store)?;
      Ok(PinCheck::Pinned(actual))
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_ensure_https_rejects_plaintext_and_tricks() {
    assert!(ensure_https("https://example.com/a.zip").is_ok());
    assert!(ensure_https("HTTPS://EXAMPLE.COM/a.zip").is_ok());
    assert!(ensure_https("http://example.com/a.zip").is_err());
    assert!(ensure_https("  http://example.com/a.zip").is_err());
    assert!(ensure_https("file:///etc/passwd").is_err());
    // A host that merely starts with "https" must not pass.
    assert!(ensure_https("http://https.example.com/a.zip").is_err());
  }

  #[test]
  fn test_hash_file_is_stable_and_content_dependent() {
    let dir = std::env::temp_dir().join(format!("donut-pin-hash-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    std::fs::write(&a, b"hello world").unwrap();
    std::fs::write(&b, b"hello worle").unwrap();

    let ha = hash_file(&a).unwrap();
    assert_eq!(ha, hash_file(&a).unwrap(), "same bytes must hash the same");
    assert_ne!(ha, hash_file(&b).unwrap(), "different bytes must differ");
    // Known SHA-256 of "hello world".
    assert_eq!(
      ha,
      "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );

    let _ = std::fs::remove_dir_all(&dir);
  }
}
