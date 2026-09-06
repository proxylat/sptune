use super::chromium_flags::is_network_service;

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
  let ps_suspend = format!(
    "$p=Get-Process -Id {} -ErrorAction SilentlyContinue; if($p){{ Add-Type -MemberDefinition '[DllImport(\"ntdll.dll\")] public static extern int NtSuspendProcess(IntPtr h);' -Name NSS -Namespace NSS -ErrorAction SilentlyContinue | Out-Null; [NSS.NSS]::NtSuspendProcess($p.Handle) | Out-Null }}",
    pid
  );
  let _ = std::process::Command::new("powershell")
    .args(["-NoProfile", "-Command", &ps_suspend])
    .output();
  let ps_trim = format!(
    "$p=Get-Process -Id {} -ErrorAction SilentlyContinue; if($p){{ Add-Type -MemberDefinition '[DllImport(\"psapi.dll\")] public static extern bool EmptyWorkingSet(IntPtr h);' -Name EWS -Namespace EWS -ErrorAction SilentlyContinue | Out-Null; [EWS.EWS]::EmptyWorkingSet($p.Handle) | Out-Null }}",
    pid
  );
  let _ = std::process::Command::new("powershell")
    .args(["-NoProfile", "-Command", &ps_trim])
    .output();
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

#[cfg(windows)]
pub fn suspend_children_of(root: u32, do_suspend: bool, do_trim: bool) {
  for (pid, cmd) in descendants_of(root) {
    if do_suspend {
      suspend_and_trim(pid, &cmd);
    } else if do_trim && !is_network_service(&cmd) {
      // trim only — reuse EmptyWorkingSet path without suspend
      let ps_trim = format!(
        "$p=Get-Process -Id {} -ErrorAction SilentlyContinue; if($p){{ Add-Type -MemberDefinition '[DllImport(\"psapi.dll\")] public static extern bool EmptyWorkingSet(IntPtr h);' -Name EWS -Namespace EWS -ErrorAction SilentlyContinue | Out-Null; [EWS.EWS]::EmptyWorkingSet($p.Handle) | Out-Null }}",
        pid
      );
      let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_trim])
        .output();
    }
  }
}
