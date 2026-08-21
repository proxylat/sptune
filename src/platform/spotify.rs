use crate::{platform::chromium_flags, user_config::SpotifyConfig};
use std::path::PathBuf;
use std::process::Command;

pub fn resolve_binary(cfg: &SpotifyConfig) -> Option<PathBuf> {
  if !cfg.binary_path.trim().is_empty() {
    let p = PathBuf::from(cfg.binary_path.trim());
    if p.is_file() {
      return Some(p);
    }
    return None;
  }
  #[cfg(windows)]
  {
    if let Ok(appdata) = std::env::var("APPDATA") {
      let p = PathBuf::from(appdata).join("Spotify").join("Spotify.exe");
      if p.is_file() {
        return Some(p);
      }
    }
  }
  #[cfg(not(windows))]
  {
    let p = PathBuf::from("/usr/bin/spotify");
    if p.is_file() {
      return Some(p);
    }
  }
  None
}

// Launcher never hard-codes flags; it delegates to chromium_flags presets
// (lean/safe/audioOnly) via load_flags(). User yml overrides const entirely.
pub fn build_args(cfg: &SpotifyConfig) -> Vec<String> {
  if !cfg.use_chromium_flags {
    return vec![];
  }
  chromium_flags::load_flags()
}

pub fn launch(cfg: &SpotifyConfig) -> anyhow::Result<std::process::Child> {
  let bin = resolve_binary(cfg).ok_or_else(|| anyhow::anyhow!("spotify binary not found"))?;
  let args = build_args(cfg);
  let mut cmd = Command::new(bin);
  cmd.args(&args);
  Ok(cmd.spawn()?)
}
