use super::chromium_flags::is_network_service;

// Default single 150 MB limit. One Job Object for all Spotify children
// except NetworkService; NetworkService gets no limit/job and no
// WorkingSet trim (it must stay responsive) but is still suspended.
pub const DEFAULT_SPOTIFY_MEMORY_LIMIT_MB: usize = 150;

#[cfg(windows)]
pub fn apply_memory_limits(_pids: &[u32]) {
  // Real impl: CreateJobObject + SetInformationJobObject(JOBOBJECT_EXTENDED_LIMIT_INFORMATION { ProcessMemoryLimit = 150<<20 })
  // + AssignProcessToJobObject for each pid where !is_network_service(cmd). NetworkService skipped.
}

#[cfg(windows)]
pub fn suspend_and_trim(pid: u32, cmd: &str) {
  let is_network = is_network_service(cmd);
  // NtSuspendProcess(pid) — always, even for NetworkService
  let _ = is_network;
  if !is_network_service(cmd) {
    // EmptyWorkingSet / SetProcessWorkingSetSizeEx(pid, -1, -1) — skip for NetworkService
    let _ = pid;
  }
}
