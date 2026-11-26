//! Lightweight append-only error/warning log file (`~/.config/sptune/sptune.log`).
//! Rotates once to `sptune.log.old` when exceeding 1 MiB.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_LOG_BYTES: u64 = 1_048_576;

pub fn log_path() -> Option<PathBuf> {
  dirs::home_dir().map(|h| h.join(".config").join("sptune").join("sptune.log"))
}

pub fn write_line_to(path: &Path, level: &str, msg: &str) {
  if let Some(dir) = path.parent() {
    let _ = fs::create_dir_all(dir);
  }
  if let Ok(meta) = fs::metadata(path) {
    if meta.len() >= MAX_LOG_BYTES {
      let mut old = path.to_path_buf();
      old.set_extension("log.old");
      let _ = fs::rename(path, old);
    }
  }
  let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
  let clean = msg.replace('\n', " | ");
  if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
    let _ = writeln!(f, "{ts} [{level}] {clean}");
  }
}

pub fn error(msg: &str) {
  if let Some(path) = log_path() {
    write_line_to(&path, "ERROR", msg);
  }
}

pub fn warn(msg: &str) {
  if let Some(path) = log_path() {
    write_line_to(&path, "WARN", msg);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn writes_timestamped_lines_and_rotates() {
    let dir = std::env::temp_dir().join(format!("sptune-log-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let log = dir.join("sptune.log");
    write_line_to(&log, "ERROR", "first\nline");
    let contents = fs::read_to_string(&log).unwrap();
    assert!(contents.contains("[ERROR] first | line"));

    // Force rotation when exceeding MAX_LOG_BYTES.
    let big = vec![b'x'; MAX_LOG_BYTES as usize + 16];
    fs::write(&log, &big).unwrap();
    write_line_to(&log, "WARN", "after rotate");
    assert!(dir.join("sptune.log.old").exists());
    let fresh = fs::read_to_string(&log).unwrap();
    assert!(fresh.contains("[WARN] after rotate"));
    let _ = fs::remove_dir_all(&dir);
  }
}
