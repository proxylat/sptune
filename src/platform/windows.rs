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
  pub const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
  pub const PROCESS_VM_READ: u32 = 0x0010;
  pub const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

  #[link(name = "kernel32")]
  extern "system" {
    pub fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> Handle;
    pub fn CloseHandle(hObject: Handle) -> i32;
    fn GetModuleHandleA(lpModuleName: *const i8) -> Handle;
    fn GetProcAddress(hModule: Handle, lpProcName: *const i8) -> *mut c_void;
    pub fn GetConsoleMode(hConsoleHandle: Handle, lpMode: *mut u32) -> i32;
    pub fn SetConsoleMode(hConsoleHandle: Handle, dwMode: u32) -> i32;
    pub fn GetStdHandle(nStdHandle: u32) -> Handle;
    pub fn ReadProcessMemory(
      hProcess: Handle,
      lpBaseAddress: *const c_void,
      lpBuffer: *mut c_void,
      nSize: usize,
      lpNumberOfBytesRead: *mut usize,
    ) -> i32;
    pub fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> Handle;
    pub fn Process32FirstW(hSnapshot: Handle, lppe: *mut ProcessEntry32W) -> i32;
    pub fn Process32NextW(hSnapshot: Handle, lppe: *mut ProcessEntry32W) -> i32;
  }

  /// 5-arg resolver for NtQueryInformationProcess (signature differs from the
  /// 1-arg suspend/resume/trim helpers).
  pub unsafe fn proc5(
    module: &str,
    name: &str,
  ) -> Option<unsafe extern "system" fn(Handle, u32, *mut c_void, u32, *mut u32) -> i32> {
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

  /// Minimal Toolhelp process entry (x64 layout: ULONG_PTR member forces
  /// 8-byte alignment, so th32DefaultHeapID starts at offset 16).
  #[repr(C)]
  pub struct ProcessEntry32W {
    pub dw_size: u32,
    pub cnt_usage: u32,
    pub th32_process_id: u32,
    pub th32_default_heap_id: usize,
    pub th32_module_id: u32,
    pub cnt_threads: u32,
    pub th32_parent_process_id: u32,
    pub pc_pri_class_base: i32,
    pub dw_flags: u32,
    pub sz_exe_file: [u16; 260],
  }

  impl ProcessEntry32W {
    pub fn new() -> Self {
      Self {
        dw_size: std::mem::size_of::<Self>() as u32,
        cnt_usage: 0,
        th32_process_id: 0,
        th32_default_heap_id: 0,
        th32_module_id: 0,
        cnt_threads: 0,
        th32_parent_process_id: 0,
        pc_pri_class_base: 0,
        dw_flags: 0,
        sz_exe_file: [0; 260],
      }
    }
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

/// All processes as (pid, ppid, exe name) from a single Toolhelp snapshot —
/// in-process, no shells, no WMI, no encoding pitfalls.
#[cfg(windows)]
fn process_snapshot() -> Vec<(u32, u32, String)> {
  unsafe {
    let snapshot = ffi::CreateToolhelp32Snapshot(ffi::TH32CS_SNAPPROCESS, 0);
    if snapshot.is_null() || snapshot as isize == -1 {
      return Vec::new();
    }
    let mut out = Vec::new();
    let mut entry = ffi::ProcessEntry32W::new();
    if ffi::Process32FirstW(snapshot, &mut entry) != 0 {
      loop {
        let len = entry.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(260);
        let exe = String::from_utf16_lossy(&entry.sz_exe_file[..len]);
        out.push((entry.th32_process_id, entry.th32_parent_process_id, exe));
        if ffi::Process32NextW(snapshot, &mut entry) == 0 {
          break;
        }
      }
    }
    ffi::CloseHandle(snapshot);
    out
  }
}

/// Read a process's command line via PEB -> ProcessParameters (x64 offsets),
/// so the NetworkService exclusion works without WMI/PowerShell.
#[cfg(windows)]
fn cmdline_of_pid(pid: u32) -> Option<String> {
  use std::ffi::c_void;
  unsafe {
    let handle = ffi::OpenProcess(ffi::PROCESS_QUERY_INFORMATION | ffi::PROCESS_VM_READ, 0, pid);
    if handle.is_null() {
      return None;
    }
    let out = (|| {
      let nt_query = ffi::proc5("ntdll.dll", "NtQueryInformationProcess")?;
      let read = |addr: usize, buf: &mut [u8]| -> bool {
        let mut n: usize = 0;
        ffi::ReadProcessMemory(
          handle,
          addr as *const c_void,
          buf.as_mut_ptr() as *mut c_void,
          buf.len(),
          &mut n,
        ) != 0 && n == buf.len()
      };
      // PROCESS_BASIC_INFORMATION: PebBaseAddress at offset 8 (x64)
      let mut pbi = [0u8; 48];
      let mut ret_len: u32 = 0;
      if nt_query(handle, 0, pbi.as_mut_ptr() as *mut c_void, 48, &mut ret_len) != 0 {
        return None;
      }
      let peb = usize::from_le_bytes(pbi[8..16].try_into().ok()?);
      if peb == 0 {
        return None;
      }
      // PEB: ProcessParameters pointer at 0x20 (x64)
      let mut params_bytes = [0u8; 8];
      if !read(peb + 0x20, &mut params_bytes) {
        return None;
      }
      let params = usize::from_le_bytes(params_bytes);
      if params == 0 {
        return None;
      }
      // RTL_USER_PROCESS_PARAMETERS: CommandLine UNICODE_STRING at 0x70 (x64)
      let mut us = [0u8; 16];
      if !read(params + 0x70, &mut us) {
        return None;
      }
      let len = u16::from_le_bytes(us[0..2].try_into().ok()?) as usize;
      let buffer = usize::from_le_bytes(us[8..16].try_into().ok()?);
      if len == 0 || buffer == 0 {
        return Some(String::new());
      }
      let mut raw = vec![0u8; len];
      if !read(buffer, &mut raw) {
        return None;
      }
      let wide: Vec<u16> = raw
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
      Some(String::from_utf16_lossy(&wide))
    })();
    ffi::CloseHandle(handle);
    out
  }
}

// Children of `root` as (pid, command line), from the Toolhelp snapshot.
// Command lines are read per child only so the NetworkService exclusion
// keeps working without WMI.
#[cfg(windows)]
pub fn descendants_of(root: u32) -> Vec<(u32, String)> {
  let snapshot = process_snapshot();
  let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
  for (pid, ppid, _) in &snapshot {
    children.entry(*ppid).or_default().push(*pid);
  }
  let mut result = Vec::new();
  let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
  visited.insert(root);
  let mut stack = vec![root];
  while let Some(cur) = stack.pop() {
    if let Some(kids) = children.get(&cur) {
      for &child in kids {
        if visited.insert(child) {
          stack.push(child);
          let cmd = cmdline_of_pid(child).unwrap_or_default();
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
      // NTSTATUS 0 = success; OpenProcess failure surfaces as None here and
      // must fall through to the PowerShell fallback, not count as success.
      match ffi::with_process(pid, |h| nt_suspend(h)) {
        Some(0) => return,
        _ => {}
      }
    }
  }
  // Fallback: the old PowerShell NtSuspendProcess (slow, ~1 shell spawn, but
  // works even if the ntdll resolution path ever fails).
  let _ = std::process::Command::new("powershell")
    .args([
      "-NoProfile",
      "-Command",
      &format!(
        "$p=Get-Process -Id {pid} -ErrorAction SilentlyContinue; if($p){{ Add-Type -MemberDefinition '[DllImport(\"ntdll.dll\")] public static extern int NtSuspendProcess(IntPtr h);' -Name NSS -Namespace NSS -ErrorAction SilentlyContinue | Out-Null; [NSS.NSS]::NtSuspendProcess($p.Handle) | Out-Null }}"
      ),
    ])
    .output();
}

#[cfg(windows)]
pub fn resume_children_of(root: u32) {
  let mut ffi_ok = true;
  unsafe {
    if let Some(nt_resume) = ffi::system_proc("ntdll.dll", "NtResumeProcess") {
      for (pid, _) in descendants_of(root) {
        if ffi::with_process(pid, |h| nt_resume(h)).unwrap_or(-1) != 0 {
          ffi_ok = false;
        }
      }
    } else {
      ffi_ok = false;
    }
  }
  if !ffi_ok {
    let _ = std::process::Command::new("powershell")
      .args([
        "-NoProfile",
        "-Command",
        &format!(
          "Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -match 'Spotify' }} | ForEach-Object {{ $p=Get-Process -Id $($_.ProcessId) -ErrorAction SilentlyContinue; if($p){{ Add-Type -MemberDefinition '[DllImport(\"ntdll.dll\")] public static extern int NtResumeProcess(IntPtr h);' -Name NRP -Namespace NRP -ErrorAction SilentlyContinue | Out-Null; [NRP.NRP]::NtResumeProcess($p.Handle) | Out-Null }}"
        ),
      ])
      .output();
  }
}

#[cfg(windows)]
pub fn kill_tree(root: u32) {
  // /T walks the child tree, /F force-terminates; Chromium helpers die with it.
  let _ = std::process::Command::new("taskkill")
    .args(["/PID", &root.to_string(), "/T", "/F"])
    .output();
}

// Topmost process whose exe is Spotify.exe and whose parent's is not — the
// Chromium browser process, whether sptune launched it or the user did.
#[cfg(windows)]
pub fn find_root_pid() -> Option<u32> {
  let snapshot = process_snapshot();
  let is_spotify_pid = |pid: u32| {
    snapshot
      .iter()
      .find(|(p, _, _)| *p == pid)
      .map(|(_, _, exe)| exe.to_lowercase().contains("spotify.exe"))
      .unwrap_or(false)
  };
  let mut candidates: Vec<u32> = snapshot
    .iter()
    .filter(|(_, ppid, exe)| {
      exe.to_lowercase().contains("spotify.exe") && !is_spotify_pid(*ppid)
    })
    .map(|(pid, _, _)| *pid)
    .collect();
  candidates.sort_unstable();
  candidates.into_iter().next()
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
