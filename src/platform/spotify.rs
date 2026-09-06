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
  let bin_dir = bin.parent().map(|p| p.to_path_buf());
  let mut cmd = Command::new(bin);
  cmd.args(&args);
  // Detach from sptune's TTY so Chromium/Spotify stderr (e.g. resource_bundle.cc
  // NOTREACHED "Unable to load bitmap") never corrupts the TUI alternate screen.
  // Also set cwd to the binary's directory so resource pak lookup is stable.
  use std::process::Stdio;
  cmd.stdin(Stdio::null());
  cmd.stdout(Stdio::null());
  cmd.stderr(Stdio::null());
  if let Some(dir) = bin_dir {
    if dir.is_dir() {
      cmd.current_dir(dir);
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
  }
  let child = cmd.spawn()?;
  // Spotify is multi-process (Chromium). Its renderer/utility children appear
  // a few seconds after the browser process starts, so defer handling.
  let root = child.id();
  let suspend = cfg.suspend_children;
  let trim = cfg.trim_working_set;
  let mem_limit = cfg.memory_limit_mb;
  if suspend || trim || mem_limit != 0 {
    std::thread::spawn(move || {
      std::thread::sleep(std::time::Duration::from_millis(2500));
      #[cfg(target_os = "linux")]
      {
        if mem_limit != 0 {
          let kids = crate::platform::linux::descendants_of(root);
          if !kids.is_empty() {
            crate::platform::linux::apply_memory_limits(&kids);
          } else {
            crate::platform::linux::apply_memory_limits_pids(&[root]);
          }
        }
        if suspend || trim {
          crate::platform::linux::suspend_children_of(root, suspend, trim);
        }
      }
      #[cfg(windows)]
      {
        if mem_limit != 0 {
          crate::platform::windows::apply_memory_limits(&[root]);
        }
        if suspend || trim {
          crate::platform::windows::suspend_children_of(root, suspend, trim);
        }
      }
    });
  }
  // Keep for non-linux/windows builds so cfg fields are considered used.
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = (cfg.memory_limit_mb, cfg.suspend_children, cfg.trim_working_set);
  }
  Ok(child)
}
