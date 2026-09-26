use crate::user_config::SoloistConfig;
use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

// Soloist downloads are per-user and may not be redistributed; builds expire
// after 90 days. Both hints point here.
pub const DOWNLOADS_URL: &str =
  "https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates";

// Pid of the soloist child sptune spawned (0 = none). Only ever our own child
// is killed — never pids matched by name.
static CHILD_PID: AtomicU32 = AtomicU32::new(0);

// Remembered successful launch for the first-render WS watch (see
// mark_started/take_runtime); avoids threading the data dir through start_ui.
static RUNTIME: std::sync::Mutex<Option<(PathBuf, String)>> = std::sync::Mutex::new(None);

/// Directory holding this launch's soloist session + runtime files (ws.addr/ws.port).
pub fn data_dir_under(app_dir: &Path) -> PathBuf {
  app_dir.join("soloist")
}

/// Remember a successful launch for the first-render WS watch. Called by main.
pub fn mark_started(data_dir: PathBuf, device_name: String) {
  *RUNTIME.lock().unwrap() = Some((data_dir, device_name));
}

/// Take the remembered launch exactly once; None if soloist was not started.
pub fn take_runtime() -> Option<(PathBuf, String)> {
  RUNTIME.lock().unwrap().take()
}

fn resolve_binary(cfg: &SoloistConfig) -> Option<PathBuf> {
  if !cfg.binary_path.trim().is_empty() {
    let p = PathBuf::from(cfg.binary_path.trim());
    return p.is_file().then_some(p);
  }
  std::env::var_os("PATH").and_then(|paths| {
    std::env::split_paths(&paths)
      .map(|dir| dir.join("soloist"))
      .find(|p| p.is_file())
  })
}

/// True when a soloist binary is usable (explicit path or PATH lookup).
pub fn binary_found(cfg: &SoloistConfig) -> bool {
  resolve_binary(cfg).is_some()
}
/// Spawn soloist as a headless Spotify Connect device. Blocking ~1s: an
/// instant exit almost always means an expired build, and this runs before
/// the TUI opens so the hint reaches the terminal.
pub fn launch(cfg: &SoloistConfig, api_key: &str, data_dir: &Path) -> anyhow::Result<()> {
  let bin = resolve_binary(cfg).ok_or_else(|| {
    anyhow::anyhow!(
      "soloist binary not found — download it (per-user, do not redistribute) from {} and set soloist.binary_path or put it on PATH",
      DOWNLOADS_URL
    )
  })?;
  std::fs::create_dir_all(data_dir)
    .with_context(|| format!("failed to create soloist data dir {}", data_dir.display()))?;
  let data_dir_str = data_dir.to_string_lossy().into_owned();
  let mut cmd = Command::new(bin);
  cmd.args([
    "--device-name",
    cfg.device_name.as_str(),
    "--api-key",
    api_key,
    "-D",
    data_dir_str.as_str(),
    "-w",
    "127.0.0.1:0",
  ]);
  cmd.stdin(Stdio::null());
  cmd.stdout(Stdio::null());
  cmd.stderr(Stdio::null());
  let mut child = cmd.spawn()?;
  std::thread::sleep(std::time::Duration::from_secs(1));
  if let Ok(Some(status)) = child.try_wait() {
    // Exit code 10 is the expired-build signal; other codes get a generic message.
    if status.code() == Some(10) {
      anyhow::bail!(
        "soloist exited immediately ({}) — the build may have expired (90-day lifetime), download a newer one from {}",
        status,
        DOWNLOADS_URL
      );
    }
    anyhow::bail!("soloist exited immediately ({})", status);
  }
  CHILD_PID.store(child.id(), Ordering::SeqCst);
  // Reap on natural exit so no zombie is left; kill_spawned still owns shutdown.
  std::thread::spawn(move || {
    let _ = child.wait();
  });
  Ok(())
}

/// Kill the soloist child sptune spawned (panic hook + clean exit). No-op when
/// sptune never launched soloist.
pub fn kill_spawned() {
  let pid = CHILD_PID.swap(0, Ordering::SeqCst);
  if pid == 0 {
    return;
  }
  #[cfg(target_os = "linux")]
  crate::platform::linux::kill_process(pid);
  #[cfg(windows)]
  crate::platform::windows::kill_process(pid);
  #[cfg(not(any(target_os = "linux", windows)))]
  let _ = pid;
}
