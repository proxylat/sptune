use super::chromium_flags::is_network_service;

// --- Minimal WinAPI FFI (no extra crates) -------------------------------
// Suspend/resume via ntdll's NtSuspendProcess/NtResumeProcess resolved at
// runtime: the old per-PID PowerShell approach spawned two shells per child
// per sweep, which was slow enough that children kept spawning unsuspended
// and the sweep effectively never caught up ("suspend not working").
#[cfg(windows)]
mod ffi {
  use std::ffi::c_void;
  pub type Handle = *mut c_void;

  pub const PROCESS_SUSPEND_RESUME: u32 = 0x0800;

  #[link(name = "kernel32")]
  extern "system" {
    pub fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> Handle;
    pub fn CloseHandle(hObject: Handle) -> i32;
    fn GetModuleHandleA(lpModuleName: *const i8) -> Handle;
    fn GetProcAddress(hModule: Handle, lpProcName: *const i8) -> *mut c_void;
    pub fn GetConsoleMode(hConsoleHandle: Handle, lpMode: *mut u32) -> i32;
    pub fn SetConsoleMode(hConsoleHandle: Handle, dwMode: u32) -> i32;
    pub fn GetStdHandle(nStdHandle: u32) -> Handle;
  }

  /// Resolve `NtSuspendProcess` / `NtResumeProcess` / `K32EmptyWorkingSet`
  /// (function name passed by the caller) from ntdll/kernel32.
  pub unsafe fn system_proc(
    module: &str,
    name: &str,
  ) -> Option<unsafe extern "system" fn(Handle) -> i32> {
    let module = std::ffi::CString::new(module).ok()?;
    let name = std::ffi::CString::new(name).ok()?;
    let handle = GetModuleHandleA(module.as_ptr());
    if handle.is_null() {
      return None;
    }
    let proc = GetProcAddress(handle, name.as_ptr());
    if proc.is_null() {
      return None;
    }
    Some(std::mem::transmute(proc))
  }

  pub const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;

  pub unsafe fn with_process<R>(pid: u32, f: impl FnOnce(Handle) -> R) -> Option<R> {
    let handle = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
    if handle.is_null() {
      return None;
    }
    let out = f(handle);
    CloseHandle(handle);
    Some(out)
  }

  // ENABLE_QUICK_EDIT_MODE / ENABLE_EXTENDED_FLAGS: QuickEdit turns console
  // mouse clicks into text selection, stealing them from the app and making
  // buttons feel offset/unresponsive on conhost.
  pub const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
  pub const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
}

#[cfg(windows)]
pub fn disable_quick_edit() {
  unsafe {
    let conin = ffi::GetStdHandle(ffi::STD_INPUT_HANDLE);
    if conin.is_null() {
      return;
    }
    let mut mode: u32 = 0;
    if ffi::GetConsoleMode(conin, &mut mode) != 0 {
      let new_mode = (mode | ffi::ENABLE_EXTENDED_FLAGS) & !ffi::ENABLE_QUICK_EDIT_MODE;
      ffi::SetConsoleMode(conin, new_mode);
    }
  }
}

#[cfg(windows)]
pub fn apply_memory_limits(_pids: &[u32]) {
  // Real impl: CreateJobObject + SetInformationJobObject(JOBOBJECT_EXTENDED_LIMIT_INFORMATION { ProcessMemoryLimit = 150<<20 })
  // + AssignProcessToJobObject for each pid where !is_network_service(cmd). NetworkService skipped.
  // Keeping as no-op for now — requires Job API and per-pid command line; cgroup-style
  // limit is Linux-only. Suspend/trim is the user-visible toggle on Windows.
}

#[cfg(windows)]
pub fn suspend_and_trim(pid: u32, cmd: &str) {
  // NetworkService must stay responsive — skip entirely (user reported it was being suspended)
  if is_network_service(cmd) {
    return;
  }
  suspend_only(pid, cmd);
  trim_process(pid);
}

#[cfg(windows)]
fn trim_process(pid: u32) {
  // EmptyWorkingSet lives in psapi.dll (as K32EmptyWorkingSet forward in
  // kernel32 on Win7+); resolve from psapi first, kernel32 as fallback.
  unsafe {
    let func = ffi::system_proc("psapi.dll", "EmptyWorkingSet")
      .or_else(|| ffi::system_proc("kernel32.dll", "EmptyWorkingSet"));
    if let Some(empty_working_set) = func {
      ffi::with_process(pid, |h| {
        empty_working_set(h);
      });
    }
  }
}

#[cfg(windows)]
fn ppid_map_via_powershell() -> Option<(std::collections::HashMap<u32, Vec<u32>>, std::collections::HashMap<u32, String>)> {
  let out = std::process::Command::new("powershell")
    .args([
      "-NoProfile",
      "-Command",
      "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)|$($_.ParentProcessId)|$($_.CommandLine)\" }",
    ])
    .output()
    .ok()?;
  if !out.status.success() {
    return None;
  }
  let text = String::from_utf8_lossy(&out.stdout);
  let mut ppid_map: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
  let mut cmd_map: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
  for line in text.lines() {
    let line = line.trim();
    if line.is_empty() {
      continue;
    }
    let mut parts = line.splitn(3, '|');
    let pid_s = parts.next()?;
    let ppid_s = parts.next()?;
    let cmd = parts.next().unwrap_or("").to_string();
    if let (Ok(pid), Ok(ppid)) = (pid_s.trim().parse::<u32>(), ppid_s.trim().parse::<u32>()) {
      ppid_map.entry(ppid).or_default().push(pid);
      cmd_map.insert(pid, if cmd == "null" || cmd == "" { String::new() } else { cmd });
    }
  }
  Some((ppid_map, cmd_map))
}

#[cfg(windows)]
pub fn descendants_of(root: u32) -> Vec<(u32, String)> {
  let Some((ppid_map, cmd_map)) = ppid_map_via_powershell() else {
    return Vec::new();
  };
  let mut result = Vec::new();
  let mut stack = vec![root];
  let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
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

// Manual suspend (no trim): NtSuspendProcess only, so Resume restores the prior state.
#[cfg(windows)]
pub fn suspend_only(pid: u32, cmd: &str) {
  if is_network_service(cmd) {
    return;
  }
  unsafe {
    if let Some(nt_suspend) = ffi::system_proc("ntdll.dll", "NtSuspendProcess") {
      ffi::with_process(pid, |h| {
        nt_suspend(h);
      });
    }
  }
}

#[cfg(windows)]
pub fn resume_children_of(root: u32) {
  unsafe {
    if let Some(nt_resume) = ffi::system_proc("ntdll.dll", "NtResumeProcess") {
      for (pid, _) in descendants_of(root) {
        ffi::with_process(pid, |h| {
          nt_resume(h);
        });
      }
    }
  }
}

#[cfg(windows)]
pub fn kill_tree(root: u32) {
  // /T walks the child tree, /F force-terminates; Chromium helpers die with it.
  let _ = std::process::Command::new("taskkill")
    .args(["/PID", &root.to_string(), "/T", "/F"])
    .output();
}

// Topmost process whose command line mentions Spotify.exe (its parent's does
// not) — the Chromium browser process sptune would have launched.
#[cfg(windows)]
pub fn find_root_pid() -> Option<u32> {
  let (ppid_map, cmd_map) = ppid_map_via_powershell()?;
  let is_spotify = |pid: u32| {
    cmd_map
      .get(&pid)
      .map(|c| c.to_lowercase().contains("spotify.exe"))
      .unwrap_or(false)
  };
  let mut candidates: Vec<u32> = ppid_map
    .values()
    .flatten()
    .copied()
    .filter(|&pid| is_spotify(pid))
    .collect();
  candidates.sort_unstable();
  candidates.into_iter().find(|&pid| {
    ppid_map
      .iter()
      .find(|(_, kids)| kids.contains(&pid))
      .map(|(ppid, _)| !is_spotify(*ppid))
      .unwrap_or(true)
  })
}

// Sweep helper: suspend+trim, or trim only, for one freshly spawned child.
#[cfg(windows)]
pub fn suspend_and_trim_child(pid: u32, cmd: &str, do_suspend: bool, do_trim: bool) {
  if do_suspend {
    suspend_and_trim(pid, cmd);
  } else if do_trim && !is_network_service(cmd) {
    trim_process(pid);
  }
}
