//! Encrypted local backup — move a whole setup between machines as one file.
//!
//! Everything that makes a machine "the same machine" (profiles with their
//! cookies, tabs and logins, plus proxies, VPNs, groups, extensions and
//! settings) is zipped and sealed with a password the user picks. Restoring
//! it on another device needs the file and the password — no account, no
//! server, no network.
//!
//! File layout:
//! ```text
//! b"DONUTBAK1" | salt_len: u32 LE | salt | { chunk_len: u32 LE | chunk }*
//! ```
//! Each chunk is `nonce(12B) || AES-256-GCM ciphertext` over at most
//! `CHUNK_PLAINTEXT` bytes of the zip. Chunking keeps a multi-gigabyte
//! profile set off the heap — the whole archive is never held in memory.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use crate::sync::encryption::{decrypt_bytes, derive_profile_key, encrypt_bytes, generate_salt};

const MAGIC: &[u8; 9] = b"DONUTBAK1";
const CHUNK_PLAINTEXT: usize = 4 * 1024 * 1024;

/// Data sub-directories that travel with a backup. Browser binaries, caches
/// and logs are deliberately left out: they are big, re-downloadable, and
/// none of them is user data.
const INCLUDED_DIRS: &[&str] = &[
  "profiles",
  "settings",
  "proxies",
  "vpn",
  "extensions",
  "data",
];

fn err(code: &'static str) -> String {
  serde_json::json!({ "code": code }).to_string()
}

/// Recursively add `dir` to `zip` under `prefix`, skipping anything we can't
/// read rather than failing the whole export over one locked file.
fn add_dir_to_zip<W: Write + std::io::Seek>(
  zip: &mut zip::ZipWriter<W>,
  dir: &Path,
  prefix: &str,
) -> Result<(), String> {
  let entries = match fs::read_dir(dir) {
    Ok(e) => e,
    Err(_) => return Ok(()),
  };

  for entry in entries.flatten() {
    let path = entry.path();
    let name = entry.file_name().to_string_lossy().to_string();
    let zip_path = format!("{prefix}/{name}");

    if path.is_dir() {
      add_dir_to_zip(zip, &path, &zip_path)?;
    } else if path.is_file() {
      let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => {
          log::warn!("Backup: skipping unreadable {}: {e}", path.display());
          continue;
        }
      };
      let options = zip::write::FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Deflated);
      if zip.start_file(&zip_path, options).is_err() {
        continue;
      }
      let mut buf = vec![0u8; 64 * 1024];
      loop {
        match file.read(&mut buf) {
          Ok(0) => break,
          Ok(n) => {
            if zip.write_all(&buf[..n]).is_err() {
              break;
            }
          }
          Err(_) => break,
        }
      }
    }
  }
  Ok(())
}

/// Zip the included data dirs into `temp_zip`.
fn build_archive(temp_zip: &Path) -> Result<(), String> {
  let data_dir = crate::app_dirs::data_dir();
  let file = File::create(temp_zip).map_err(|_| err("BACKUP_WRITE_FAILED"))?;
  let mut zip = zip::ZipWriter::new(BufWriter::new(file));

  for name in INCLUDED_DIRS {
    let dir = data_dir.join(name);
    if dir.is_dir() {
      add_dir_to_zip(&mut zip, &dir, name)?;
    }
  }

  zip.finish().map_err(|_| err("BACKUP_WRITE_FAILED"))?;
  Ok(())
}

/// Export every profile, proxy, VPN, group, extension and setting into one
/// password-protected file at `dest`.
pub fn export_backup(dest: &Path, password: &str) -> Result<(), String> {
  if password.trim().is_empty() {
    return Err(err("BACKUP_PASSWORD_REQUIRED"));
  }

  let temp_zip = std::env::temp_dir().join(format!("donut-backup-{}.zip", std::process::id()));
  let result = (|| -> Result<(), String> {
    build_archive(&temp_zip)?;

    let salt = generate_salt();
    let key = derive_profile_key(password, &salt)?;

    let mut input = BufReader::new(File::open(&temp_zip).map_err(|_| err("BACKUP_WRITE_FAILED"))?);
    let mut out =
      BufWriter::new(File::create(dest).map_err(|_| err("BACKUP_WRITE_FAILED"))?);

    out.write_all(MAGIC).map_err(|_| err("BACKUP_WRITE_FAILED"))?;
    let salt_bytes = salt.as_bytes();
    out
      .write_all(&(salt_bytes.len() as u32).to_le_bytes())
      .map_err(|_| err("BACKUP_WRITE_FAILED"))?;
    out
      .write_all(salt_bytes)
      .map_err(|_| err("BACKUP_WRITE_FAILED"))?;

    let mut buf = vec![0u8; CHUNK_PLAINTEXT];
    loop {
      let mut filled = 0;
      // read_exact would fail on the final short chunk; fill manually.
      while filled < buf.len() {
        match input.read(&mut buf[filled..]) {
          Ok(0) => break,
          Ok(n) => filled += n,
          Err(_) => return Err(err("BACKUP_WRITE_FAILED")),
        }
      }
      if filled == 0 {
        break;
      }
      let blob = encrypt_bytes(&key, &buf[..filled])?;
      out
        .write_all(&(blob.len() as u32).to_le_bytes())
        .map_err(|_| err("BACKUP_WRITE_FAILED"))?;
      out
        .write_all(&blob)
        .map_err(|_| err("BACKUP_WRITE_FAILED"))?;
      if filled < buf.len() {
        break;
      }
    }
    out.flush().map_err(|_| err("BACKUP_WRITE_FAILED"))?;
    Ok(())
  })();

  let _ = fs::remove_file(&temp_zip);
  result
}

/// Decrypt `src` into a temporary zip. Split out so `import_backup` can keep
/// its cleanup in one place.
fn decrypt_to_zip(src: &Path, password: &str, temp_zip: &Path) -> Result<(), String> {
  let mut input = BufReader::new(File::open(src).map_err(|_| err("BACKUP_READ_FAILED"))?);

  let mut magic = [0u8; 9];
  input
    .read_exact(&mut magic)
    .map_err(|_| err("BACKUP_NOT_A_BACKUP"))?;
  if &magic != MAGIC {
    return Err(err("BACKUP_NOT_A_BACKUP"));
  }

  let mut len_buf = [0u8; 4];
  input
    .read_exact(&mut len_buf)
    .map_err(|_| err("BACKUP_NOT_A_BACKUP"))?;
  let salt_len = u32::from_le_bytes(len_buf) as usize;
  if salt_len == 0 || salt_len > 1024 {
    return Err(err("BACKUP_NOT_A_BACKUP"));
  }
  let mut salt_bytes = vec![0u8; salt_len];
  input
    .read_exact(&mut salt_bytes)
    .map_err(|_| err("BACKUP_NOT_A_BACKUP"))?;
  let salt = String::from_utf8(salt_bytes).map_err(|_| err("BACKUP_NOT_A_BACKUP"))?;

  let key = derive_profile_key(password, &salt)?;
  let mut out = BufWriter::new(File::create(temp_zip).map_err(|_| err("BACKUP_READ_FAILED"))?);

  loop {
    let mut len_buf = [0u8; 4];
    match input.read_exact(&mut len_buf) {
      Ok(()) => {}
      Err(_) => break, // clean EOF
    }
    let blob_len = u32::from_le_bytes(len_buf) as usize;
    if blob_len == 0 || blob_len > CHUNK_PLAINTEXT + 4096 {
      return Err(err("BACKUP_CORRUPT"));
    }
    let mut blob = vec![0u8; blob_len];
    input
      .read_exact(&mut blob)
      .map_err(|_| err("BACKUP_CORRUPT"))?;
    // A wrong password fails the GCM tag check on the very first chunk.
    let plain = decrypt_bytes(&key, &blob).map_err(|_| err("BACKUP_WRONG_PASSWORD"))?;
    out
      .write_all(&plain)
      .map_err(|_| err("BACKUP_READ_FAILED"))?;
  }
  out.flush().map_err(|_| err("BACKUP_READ_FAILED"))?;
  Ok(())
}

/// Restore a backup over the current data dir. Existing entries with the same
/// name are replaced; anything not in the backup is left alone.
pub fn import_backup(src: &Path, password: &str) -> Result<(), String> {
  if password.trim().is_empty() {
    return Err(err("BACKUP_PASSWORD_REQUIRED"));
  }

  let temp_zip =
    std::env::temp_dir().join(format!("donut-restore-{}.zip", std::process::id()));

  let result = (|| -> Result<(), String> {
    decrypt_to_zip(src, password, &temp_zip)?;

    let data_dir = crate::app_dirs::data_dir();
    let file = File::open(&temp_zip).map_err(|_| err("BACKUP_READ_FAILED"))?;
    let mut archive =
      zip::ZipArchive::new(BufReader::new(file)).map_err(|_| err("BACKUP_CORRUPT"))?;

    for i in 0..archive.len() {
      let mut entry = archive.by_index(i).map_err(|_| err("BACKUP_CORRUPT"))?;
      // `enclosed_name` rejects absolute paths and `..` traversal, so a
      // tampered archive can't write outside the data dir.
      let Some(rel) = entry.enclosed_name() else {
        continue;
      };
      let Some(top) = rel.components().next() else {
        continue;
      };
      let top_name = top.as_os_str().to_string_lossy().to_string();
      if !INCLUDED_DIRS.contains(&top_name.as_str()) {
        continue;
      }

      let dest: PathBuf = data_dir.join(&rel);
      if entry.is_dir() {
        let _ = fs::create_dir_all(&dest);
        continue;
      }
      if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|_| err("BACKUP_READ_FAILED"))?;
      }
      let mut out = File::create(&dest).map_err(|_| err("BACKUP_READ_FAILED"))?;
      std::io::copy(&mut entry, &mut out).map_err(|_| err("BACKUP_READ_FAILED"))?;
    }
    Ok(())
  })();

  let _ = fs::remove_file(&temp_zip);
  result
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_roundtrip_preserves_bytes() {
    let tmp = std::env::temp_dir().join(format!("donut-bak-test-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let zip_path = tmp.join("payload.zip");
    let sealed = tmp.join("sealed.donutbak");
    let restored = tmp.join("restored.zip");

    // Larger than one chunk so the chunk loop is exercised.
    let payload: Vec<u8> = (0..(CHUNK_PLAINTEXT + 1234)).map(|i| (i % 251) as u8).collect();
    fs::write(&zip_path, &payload).unwrap();

    let salt = generate_salt();
    let key = derive_profile_key("hunter2", &salt).unwrap();

    // Seal exactly like export_backup does.
    {
      let mut input = BufReader::new(File::open(&zip_path).unwrap());
      let mut out = BufWriter::new(File::create(&sealed).unwrap());
      out.write_all(MAGIC).unwrap();
      out
        .write_all(&(salt.as_bytes().len() as u32).to_le_bytes())
        .unwrap();
      out.write_all(salt.as_bytes()).unwrap();
      let mut buf = vec![0u8; CHUNK_PLAINTEXT];
      loop {
        let mut filled = 0;
        while filled < buf.len() {
          match input.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => panic!("read failed"),
          }
        }
        if filled == 0 {
          break;
        }
        let blob = encrypt_bytes(&key, &buf[..filled]).unwrap();
        out.write_all(&(blob.len() as u32).to_le_bytes()).unwrap();
        out.write_all(&blob).unwrap();
        if filled < buf.len() {
          break;
        }
      }
      out.flush().unwrap();
    }

    decrypt_to_zip(&sealed, "hunter2", &restored).unwrap();
    assert_eq!(fs::read(&restored).unwrap(), payload);

    let wrong = decrypt_to_zip(&sealed, "wrong-password", &restored);
    assert!(wrong.is_err(), "wrong password must not decrypt");

    let _ = fs::remove_dir_all(&tmp);
  }

  #[test]
  fn test_rejects_foreign_file() {
    let tmp = std::env::temp_dir().join(format!("donut-bak-foreign-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let junk = tmp.join("junk.bin");
    let out = tmp.join("out.zip");
    fs::write(&junk, b"this is not a backup file at all").unwrap();

    let res = decrypt_to_zip(&junk, "pw", &out);
    assert!(res.is_err());

    let _ = fs::remove_dir_all(&tmp);
  }
}
