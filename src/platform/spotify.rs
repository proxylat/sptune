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
  let flags = chromium_flags::load_flags();
  // Validate joined form has no empty entries / double-space (uses flags_to_arg_string).
  let _ = chromium_flags::flags_to_arg_string(&flags);
  flags
}

pub fn launch(cfg: &SpotifyConfig) -> anyhow::Result<std::process::Child> {
  let bin = resolve_binary(cfg).ok_or_else(|| anyhow::anyhow!("spotify binary not found"))?;
  let args = build_args(cfg);
  let mut cmd = Command::new(bin);
  cmd.args(&args);
  let child = cmd.spawn()?;
  // Apply platform memory/trim policy if configured (single 150 MB cgroup / Job).
  // These call through to platform/linux and platform/windows which filter
  // NetworkService via is_network_service.
  #[cfg(target_os = "linux")]
  {
    if cfg.memory_limit_mb != 0 {
      crate::platform::linux::apply_memory_limits_pids(&[child.id()]);
    }
    if cfg.suspend_children || cfg.trim_working_set {
      crate::platform::linux::suspend_and_trim(child.id(), "");
    }
  }
  #[cfg(windows)]
  {
    if cfg.memory_limit_mb != 0 {
      crate::platform::windows::apply_memory_limits(&[child.id()]);
    }
    if cfg.suspend_children || cfg.trim_working_set {
      crate::platform::windows::suspend_and_trim(child.id(), "");
    }
  }
  // Keep for non-linux/windows builds so cfg fields are considered used.
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = (cfg.memory_limit_mb, cfg.suspend_children, cfg.trim_working_set);
  }
  Ok(child)
}
