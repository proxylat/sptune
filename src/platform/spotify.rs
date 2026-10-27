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
  // The suspend/trim/mem-limit policy is enforced by the periodic sweeper
  // thread started in main (it also covers externally launched Spotify and
  // children that spawn seconds after the browser process).
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = (
      cfg.memory_limit_mb,
      cfg.suspend_children,
      cfg.trim_working_set,
    );
  }
  Ok(child)
}

// --- Suspend/resume/exit handling (auto sweeper + playbar/settings toggle) ---

// Pids this process has already suspended. Windows NtSuspendProcess is
// reference-counted, so re-suspending an already-suspended pid would make a
// later single resume leave it half-frozen; every suspend path goes through
// the registry so each pid is only ever suspended once per sptune session.
fn handled_registry() -> &'static std::sync::Mutex<std::collections::HashSet<u32>> {
  use std::sync::{Mutex, OnceLock};
  static REGISTRY: OnceLock<Mutex<std::collections::HashSet<u32>>> = OnceLock::new();
  REGISTRY.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// One sweep pass over the Spotify tree under `root`:
/// 1. `--type=renderer` helpers are killed outright (they are the memory
///    hogs; Spotify respawns them on demand).
/// 2. every other child is suspended (and trimmed when enabled) — EXCEPT the
///    NetworkService utility process, which is never suspended and never
///    trimmed.
///    Safe to call repeatedly — late-spawning children get caught on a later
///    pass, already-handled ones are skipped. Returns how many pids were touched.
pub fn sweep_processes(root: u32, do_suspend: bool, do_trim: bool) -> usize {
  let is_network = crate::platform::chromium_flags::is_network_service;
  let mut registry = handled_registry().lock().unwrap();
  let mut touched = 0;
  for (pid, cmd) in descendants(root) {
    if is_network(&cmd) {
      // The one exclusion: never suspended, never trimmed.
      continue;
    }
    if do_suspend && cmd.contains("--type=renderer") {
      kill_process(pid);
      registry.remove(&pid); // a respawned renderer gets a new pid anyway
      touched += 1;
      continue;
    }
    if registry.insert(pid) {
      touched += 1;
      if do_suspend {
        #[cfg(target_os = "linux")]
        crate::platform::linux::suspend_process(pid);
        #[cfg(windows)]
        crate::platform::windows::suspend_only(pid);
      }
      if do_trim {
        #[cfg(target_os = "linux")]
        crate::platform::linux::trim_process(pid, &cmd);
        #[cfg(windows)]
        crate::platform::windows::trim_process(pid, &cmd);
      }
    }
  }
  touched
}

/// Terminate one process outright (renderer cleanup).
fn kill_process(pid: u32) {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::kill_process(pid);
  }
  #[cfg(windows)]
  {
    crate::platform::windows::kill_process(pid);
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = pid;
  }
}

fn descendants(root: u32) -> Vec<(u32, String)> {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::descendants_of(root)
  }
  #[cfg(windows)]
  {
    return crate::platform::windows::descendants_of(root);
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {
    let _ = root;
    Vec::new()
  }
}

/// Find the running Spotify browser (root) process, whether or not sptune
/// launched it.
pub fn find_root_pid() -> Option<u32> {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::find_root_pid()
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

/// Manual suspend: freeze every Spotify helper process currently running,
/// NetworkService included (only its working set is spared from trimming).
/// Registry-deduped, so pids the auto sweeper already suspended are not
/// touched twice.
pub fn suspend_processes(root: u32) {
  sweep_processes(root, true, false);
}

/// Resume everything sptune suspended this session, then clear the registry
/// so a later manual suspend starts fresh.
pub fn resume_processes(root: u32) {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::resume_children_of(root);
  }
  #[cfg(windows)]
  {
    crate::platform::windows::resume_children_of(root);
  }
  handled_registry().lock().unwrap().clear();
}

/// Terminate every Spotify process on the machine — used on sptune exit.
/// Matches by exe/cmdline, not by tree walking, so nothing survives a
/// missed parent link; works on suspended processes.
pub fn kill_all_spotify() {
  #[cfg(target_os = "linux")]
  {
    crate::platform::linux::kill_all();
  }
  #[cfg(windows)]
  {
    crate::platform::windows::kill_all();
    return;
  }
  #[cfg(not(any(target_os = "linux", windows)))]
  {}
}
