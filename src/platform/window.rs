//! Native X11 terminal window capture, minimize-to-tray, and restore helpers.
//! Uses `xprop` (once at startup) and `libX11.so.6` via Python `ctypes` so it
//! works on X11 desktops without requiring `xdotool` or `wmctrl`.

#[cfg(target_os = "linux")]
mod imp {
  use std::process::{Command, Stdio};
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::OnceLock;

  static WINDOW_XID: OnceLock<Option<u64>> = OnceLock::new();
  static WAS_HIDDEN: AtomicBool = AtomicBool::new(false);

  pub fn parse_xprop_xid(stdout: &str) -> Option<u64> {
    let hex = stdout
      .split_whitespace()
      .rev()
      .find(|tok| tok.starts_with("0x") || tok.starts_with("0X"))?;
    let trimmed = hex.trim_start_matches("0x").trim_start_matches("0X");
    let id = u64::from_str_radix(trimmed, 16).ok()?;
    (id != 0).then_some(id)
  }

  fn parse_env_xid(val: &str) -> Option<u64> {
    let s = val.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
      u64::from_str_radix(hex, 16).ok().filter(|&id| id != 0)
    } else {
      s.parse::<u64>().ok().filter(|&id| id != 0)
    }
  }

  fn capture_xid() -> Option<u64> {
    if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
      return None;
    }
    if let Ok(val) = std::env::var("WINDOWID") {
      if let Some(id) = parse_env_xid(&val) {
        return Some(id);
      }
    }
    let out = Command::new("xprop")
      .args(["-root", "_NET_ACTIVE_WINDOW"])
      .stdin(Stdio::null())
      .stderr(Stdio::null())
      .output()
      .ok()?;
    if !out.status.success() {
      return None;
    }
    parse_xprop_xid(&String::from_utf8_lossy(&out.stdout))
  }

  pub fn init() {
    let _ = WINDOW_XID.get_or_init(capture_xid);
  }

  pub fn current_xid() -> Option<u64> {
    *WINDOW_XID.get_or_init(capture_xid)
  }

  pub fn was_hidden() -> bool {
    WAS_HIDDEN.load(Ordering::Relaxed)
  }

  pub fn owns_window() -> bool {
    std::env::var_os("SPTUNE_OWN_WINDOW").is_some_and(|v| v == "1")
  }

  pub fn minimize_to_tray() -> bool {
    if !owns_window() {
      return false;
    }
    let Some(xid) = current_xid() else {
      return false;
    };
    const PY_HIDE: &str = r#"
import ctypes, sys
x11 = ctypes.CDLL("libX11.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
dpy = x11.XOpenDisplay(None)
if not dpy:
    sys.exit(1)
scr = x11.XDefaultScreen(ctypes.c_void_p(dpy))
x11.XIconifyWindow(ctypes.c_void_p(dpy), ctypes.c_ulong(int(sys.argv[1])), scr)
x11.XFlush(ctypes.c_void_p(dpy))
x11.XCloseDisplay(ctypes.c_void_p(dpy))
"#;
    let ok = Command::new("python3")
      .args(["-c", PY_HIDE, &xid.to_string()])
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .is_ok();
    if ok {
      WAS_HIDDEN.store(true, Ordering::Relaxed);
    }
    ok
  }

  pub fn restore_from_tray() {
    let Some(xid) = current_xid() else {
      return;
    };
    WAS_HIDDEN.store(false, Ordering::Relaxed);
    const PY_SHOW: &str = r#"
import ctypes, sys
class XClientMessageEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("window", ctypes.c_ulong),
        ("message_type", ctypes.c_ulong),
        ("format", ctypes.c_int),
        ("data", ctypes.c_long * 5),
    ]
x11 = ctypes.CDLL("libX11.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XInternAtom.restype = ctypes.c_ulong
dpy = x11.XOpenDisplay(None)
if not dpy:
    sys.exit(1)
d = ctypes.c_void_p(dpy)
wid = int(sys.argv[1])
root = x11.XDefaultRootWindow(d)
x11.XMapRaised(d, ctypes.c_ulong(wid))
atom = x11.XInternAtom(d, b"_NET_ACTIVE_WINDOW", 0)
ev = XClientMessageEvent()
ev.type = 33
ev.send_event = 1
ev.display = dpy
ev.window = wid
ev.message_type = atom
ev.format = 32
ev.data[0] = 1
ev.data[1] = 0
ev.data[2] = 0
x11.XSendEvent(d, ctypes.c_ulong(root), 0, ctypes.c_long((1 << 20) | (1 << 19)), ctypes.byref(ev))
x11.XFlush(d)
x11.XCloseDisplay(d)
"#;
    let _ = Command::new("python3")
      .args(["-c", PY_SHOW, &xid.to_string()])
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn();
  }

  #[cfg(test)]
  mod tests {
    use super::*;

    #[test]
    fn parses_xprop_active_window_output() {
      assert_eq!(
        parse_xprop_xid("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4a00003\n"),
        Some(0x4a00003)
      );
      assert_eq!(
        parse_xprop_xid("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n"),
        None
      );
      assert_eq!(parse_xprop_xid(""), None);
    }

    #[test]
    fn parses_env_windowid() {
      assert_eq!(parse_env_xid("77594627"), Some(77594627));
      assert_eq!(parse_env_xid("0x4a00003"), Some(0x4a00003));
      assert_eq!(parse_env_xid("0"), None);
    }
  }
}

#[cfg(not(target_os = "linux"))]
mod imp {
  pub fn init() {}
  pub fn current_xid() -> Option<u64> {
    None
  }
  pub fn was_hidden() -> bool {
    false
  }
  pub fn minimize_to_tray() -> bool {
    false
  }
  pub fn restore_from_tray() {}
}

pub fn init() {
  imp::init();
}

pub fn current_xid() -> Option<u64> {
  imp::current_xid()
}

pub fn was_hidden() -> bool {
  imp::was_hidden()
}

pub fn minimize_to_tray() -> bool {
  imp::minimize_to_tray()
}

pub fn restore_from_tray() {
  imp::restore_from_tray();
}
