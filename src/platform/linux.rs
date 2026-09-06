use super::chromium_flags::is_network_service;
use super::DEFAULT_SPOTIFY_MEMORY_LIMIT_MB;

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
  // NetworkService must stay responsive — never STOP or trim it (user reported
  // `--utility-sub-type=network.mojom.NetworkService` was being suspended).
  if is_network_service(cmd) {
    return;
  }
  let _ = std::process::Command::new("kill")
    .args(["-STOP", &pid.to_string()])
    .output();
  let _ = std::fs::write(format!("/proc/{pid}/clear_refs"), "1");
}

#[cfg(target_os = "linux")]
fn cmdline_of(pid: u32) -> String {
  std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
    .unwrap_or_default()
    .replace('\0', " ")
}

#[cfg(target_os = "linux")]
fn ppid_of(pid: u32) -> Option<u32> {
  let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
  for line in s.lines() {
    if line.starts_with("PPid:") {
      return line.split_whitespace().nth(1)?.parse().ok();
    }
  }
  None
}

#[cfg(target_os = "linux")]
pub fn descendants_of(root: u32) -> Vec<(u32, String)> {
  use std::collections::{HashMap, HashSet};
  let mut ppid_map: HashMap<u32, Vec<u32>> = HashMap::new();
  let mut cmd_map: HashMap<u32, String> = HashMap::new();
  let Ok(entries) = std::fs::read_dir("/proc") else {
    return Vec::new();
  };
  for entry in entries.flatten() {
    let name = entry.file_name();
    let s = name.to_string_lossy();
    if let Ok(pid) = s.parse::<u32>() {
      if let Some(ppid) = ppid_of(pid) {
        ppid_map.entry(ppid).or_default().push(pid);
        cmd_map.insert(pid, cmdline_of(pid));
      }
    }
  }
  let mut result = Vec::new();
  let mut stack = vec![root];
  let mut visited: HashSet<u32> = HashSet::new();
  visited.insert(root);
  while let Some(cur) = stack.pop() {
    if let Some(children) = ppid_map.get(&cur) {
      for &child in children {
        if visited.insert(child) {
          stack.push(child);
          let cmd = cmd_map.get(&child).cloned().unwrap_or_default();
          result.push((child, cmd));
        }
      }
    }
  }
  result
}

#[cfg(target_os = "linux")]
pub fn suspend_children_of(root: u32, do_suspend: bool, do_trim: bool) {
  for (pid, cmd) in descendants_of(root) {
    if do_suspend {
      suspend_and_trim(pid, &cmd);
    } else if do_trim && !is_network_service(&cmd) {
      let _ = std::fs::write(format!("/proc/{pid}/clear_refs"), "1");
    }
  }
}

#[cfg(not(target_os = "linux"))]
pub fn apply_memory_limits(_pids: &[(u32, String)]) {}

#[cfg(not(target_os = "linux"))]
pub fn suspend_and_trim(_pid: u32, _cmd: &str) {}
