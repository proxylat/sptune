use super::chromium_flags::is_network_service;

// Single 150 MB limit for all except NetworkService (1). NetworkService exempt from limit + WorkingSet clean but still suspended.
pub const DEFAULT_SPOTIFY_MEMORY_LIMIT_MB: usize = 150;

#[cfg(windows)]
pub fn apply_memory_limits(_pids: &[u32]) {
  // real impl uses CreateJobObject + SetInformationJobObject + AssignProcessToJobObject
  // stub: enumerate children, one job for all except network, one job (no limit) for network
}

#[cfg(windows)]
pub fn suspend_and_trim(pid: u32, cmd: &str) {
  let is_network = is_network_service(cmd);
  // NtSuspendProcess(pid) always
  if !is_network {
    // EmptyWorkingSet / SetProcessWorkingSetSizeEx
    let _ = pid;
  }
}
