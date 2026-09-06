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
  // Spotify is multi-process (Chromium): renderer/utility children keep
  // spawning for several seconds after the browser process starts, so one
  // deferred pass misses late arrivals. Sweep repeatedly; each pass only
  // touches new pids (Windows NtSuspendProcess is counted, so re-suspending
  // an already-suspended child would break its later resume).
  let root = child.id();
  let suspend = cfg.suspend_children;
  let trim = cfg.trim_working_set;
  let mem_limit = cfg.memory_limit_mb;
  if suspend || trim || mem_limit != 0 {
    std::thread::spawn(move || {
      let mut handled: std::collections::HashSet<u32> = std::collections::HashSet::new();
      for delay in [1_000u64, 3_000, 6_000, 10_000, 15_000] {
        std::thread::sleep(std::time::Duration::from_millis(delay));
        #[cfg(target_os = "linux")]
        {
          let kids = crate::platform::linux::descendants_of(root);
          let fresh: Vec<(u32, String)> = kids
            .into_iter()
            .filter(|(pid, _)| handled.insert(*pid))
            .collect();
          if mem_limit != 0 {
            if !fresh.is_empty() {
              crate::platform::linux::apply_memory_limits(&fresh);
            } else if handled.is_empty() {
              crate::platform::linux::apply_memory_limits_pids(&[root]);
              handled.insert(root);
            }
          }
          if suspend || trim {
            for (pid, cmd) in fresh {
              crate::platform::linux::suspend_and_trim_child(pid, &cmd, suspend, trim);
            }
          }
        }
        #[cfg(windows)]
        {
          let kids = crate::platform::windows::descendants_of(root);
          let fresh: Vec<(u32, String)> = kids
            .into_iter()
            .filter(|(pid, _)| handled.insert(*pid))
            .collect();
          if mem_limit != 0 && handled.is_empty() {
            crate::platform::windows::apply_memory_limits(&[root]);
            handled.insert(root);
          }
          if suspend || trim {
            for (pid, cmd) in fresh {
              crate::platform::windows::suspend_and_trim_child(pid, &cmd, suspend, trim);
            }
          }
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

// --- Manual suspend/resume/exit handling (playbar button + settings toggle) ---

// These are best-effort no-ops on platforms without the helpers.

/// Find the running Spotify browser (root) process, whether or not sptune
/// launched it.
pub fn find_root_pid() -> Option<u32> {
  #[cfg(target_os = "linux")]
  {
    return crate::platform::linux::find_root_pid();
  }
  #[cfg(windows)]
  {
    return crate::platform::windows::find_root_pid();
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    None
  }
}

/// STOP/NtSuspendProcess every Spotify helper process (NetworkService kept
/// alive so connectivity survives).
pub fn suspend_processes(root: u32) {
  #[cfg(target_os = "linux")]
  {
    for (pid, cmd) in crate::platform::linux::descendants_of(root) {
      crate::platform::linux::suspend_only(pid, &cmd);
    }
    return;
  }
  #[cfg(windows)]
  {
    for (pid, cmd) in crate::platform::windows::descendants_of(root) {
      crate::platform::windows::suspend_only(pid, &cmd);
    }
    return;
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = root;
  }
}

/// CONT/NtResumeProcess every Spotify helper process.
pub fn resume_processes(root: u32) {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::resume_children_of(root);
    return;
  }
  #[cfg(windows)]
  {
    crate::platform::windows::resume_children_of(root);
    return;
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = root;
  }
}

/// Terminate the whole Spotify tree (used on exit when sptune launched it).
pub fn kill_tree(root: u32) {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::kill_tree(root);
    return;
  }
  #[cfg(windows)]
  {
    crate::platform::windows::kill_tree(root);
    return;
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = root;
  }
}
