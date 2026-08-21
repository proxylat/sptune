use super::chromium_flags::is_network_service;

pub const DEFAULT_SPOTIFY_MEMORY_LIMIT_MB: usize = 150;

#[cfg(target_os = "linux")]
fn cgroup_base() -> std::path::PathBuf {
  // systemd --user delegation preferred: $XDG_RUNTIME_DIR not writable for cgroup, so try user.slice if it exists
  if let Ok(uid) = std::fs::read_to_string("/proc/self/loginuid") {
    let uid = uid.trim();
    let p = std::path::PathBuf::from(format!("/sys/fs/cgroup/user.slice/user-{uid}.slice/sptune-spotify"));
    if std::path::Path::new("/sys/fs/cgroup/user.slice").exists() {
      return p;
    }
  }
  std::path::PathBuf::from("/sys/fs/cgroup/sptune-spotify")
}

#[cfg(target_os = "linux")]
fn ensure_cgroup() -> Option<std::path::PathBuf> {
  let base = cgroup_base();
  let _ = std::fs::create_dir_all(&base);
  // 150M
  let max = format!("{}M", DEFAULT_SPOTIFY_MEMORY_LIMIT_MB);
  let _ = std::fs::write(base.join("memory.max"), &max);
  // enable controllers if needed
  Some(base)
}

#[cfg(target_os = "linux")]
pub fn apply_memory_limits(pids: &[(u32, String)]) {
  // single 150MB cgroup for all except NetworkService
  if let Some(base) = ensure_cgroup() {
    for (pid, cmd) in pids {
      if is_network_service(cmd) {
        continue;
      }
      let _ = std::fs::write(base.join("cgroup.procs"), pid.to_string());
      // fallback per-pid if cgroup write failed: prctl not implemented here
    }
  }
}

// compat overload for callers passing &[u32]
#[cfg(target_os = "linux")]
pub fn apply_memory_limits_pids(pids: &[u32]) {
  let v: Vec<(u32, String)> = pids.iter().map(|p| (*p, String::new())).collect();
  apply_memory_limits(&v);
}

#[cfg(target_os = "linux")]
pub fn suspend_and_trim(pid: u32, cmd: &str) {
  // SIGSTOP always, even for NetworkService — skip WorkingSet trim for NetworkService
  let _ = std::process::Command::new("kill")
    .args(["-STOP", &pid.to_string()])
    .output();
  if !is_network_service(cmd) {
    let _ = std::fs::write(format!("/proc/{pid}/clear_refs"), "1");
    let _ = pid;
  }
}

#[cfg(not(target_os = "linux"))]
pub fn apply_memory_limits(_pids: &[(u32, String)]) {}

#[cfg(not(target_os = "linux"))]
pub fn suspend_and_trim(_pid: u32, _cmd: &str) {}
