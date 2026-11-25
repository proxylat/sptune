//! Native Linux system tray icon (`org.kde.StatusNotifierItem` via `zbus`).
//! Runs on its own lightweight background thread; silent no-op if no tray
//! watcher is present or on non-Linux targets.

#[cfg(target_os = "linux")]
const GTK_POPOVER_PY: &str = r##"
import json, os, socket, subprocess, sys
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib

SOCK = sys.argv[1]
POS_X = int(sys.argv[2]) if len(sys.argv) > 2 else -1
POS_Y = int(sys.argv[3]) if len(sys.argv) > 3 else -1

def ipc(msg):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(0.4)
    s.connect(SOCK)
    s.sendall((msg + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        chunk = s.recv(4096)
        if not chunk:
            break
        buf += chunk
    s.close()
    return buf.decode().strip()

def fmt_ms(ms):
    sec = max(0, int(ms // 1000))
    return f"{sec // 60}:{sec % 60:02d}"

def make_css(th):
    bg = th.get("bg", "#0e101c")
    border = th.get("border", "#5965a6")
    accent = th.get("accent", "#f5c79e")
    sel = th.get("selected", "#f6e6d4")
    txt = th.get("text", "#e6dfd5")
    muted = th.get("muted", "#8c91ab")
    return f"""
window {{
  background-color: {bg};
  color: {txt};
  border: 1px solid {border};
}}
.title {{ font-weight: 700; font-size: 13px; color: {sel}; }}
.artist {{ font-size: 11.5px; color: {txt}; }}
.album {{ font-size: 11px; color: {muted}; }}
.time, .footer-dev, .vol-num {{ font-size: 11px; color: {muted}; }}
button.ctrl {{
  background: transparent;
  border: none;
  box-shadow: none;
  color: {muted};
  font-size: 15px;
  padding: 4px 8px;
}}
button.ctrl.active, button.ctrl.main {{ color: {accent}; }}
button.ctrl:hover {{ color: {sel}; }}
button.open-btn {{
  background: transparent;
  border: none;
  box-shadow: none;
  color: {txt};
  font-size: 11.5px;
  padding: 2px 4px;
}}
button.open-btn:hover {{ color: {accent}; }}
scale trough {{
  min-height: 7px;
  border-radius: 4px;
  background-color: rgba(128, 128, 160, 0.25);
}}
scale highlight {{
  border-radius: 4px;
  background-color: {accent};
}}
scale slider {{
  min-width: 14px;
  min-height: 14px;
  border-radius: 99px;
  background-color: {accent};
  border: none;
  margin: -4px 0;
}}
scale.prog highlight {{
  background-color: #ffffff;
}}
scale.prog slider {{
  background-color: #ffffff;
}}
separator {{ background-color: rgba(128, 128, 160, 0.25); min-height: 1px; }}
""".encode()

provider = Gtk.CssProvider()
Gtk.StyleContext.add_provider_for_screen(
    Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
)

win = Gtk.Window(type=Gtk.WindowType.TOPLEVEL)
win.set_decorated(False)
win.set_resizable(False)
win.set_skip_taskbar_hint(True)
win.set_skip_pager_hint(True)
win.set_keep_above(True)
win.set_type_hint(Gdk.WindowType.TOPLEVEL and Gdk.WindowTypeHint.POPUP_MENU)
win.set_default_size(420, 1)

root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
root.set_margin_top(14); root.set_margin_bottom(12)
root.set_margin_start(16); root.set_margin_end(16)
win.add(root)

top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
cover = Gtk.Image()
cover.set_size_request(96, 96)
top.pack_start(cover, False, False, 0)

meta = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
meta.set_valign(Gtk.Align.CENTER)
lbl_title = Gtk.Label(xalign=0); lbl_title.get_style_context().add_class("title"); lbl_title.set_ellipsize(3); lbl_title.set_max_width_chars(30)
lbl_artist = Gtk.Label(xalign=0); lbl_artist.get_style_context().add_class("artist"); lbl_artist.set_ellipsize(3); lbl_artist.set_max_width_chars(32)
lbl_album = Gtk.Label(xalign=0); lbl_album.get_style_context().add_class("album"); lbl_album.set_ellipsize(3); lbl_album.set_max_width_chars(32)
meta.pack_start(lbl_title, False, False, 0)
meta.pack_start(lbl_artist, False, False, 0)
meta.pack_start(lbl_album, False, False, 0)
top.pack_start(meta, True, True, 0)
root.pack_start(top, False, False, 0)

ctrls = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
ctrls.set_halign(Gtk.Align.CENTER)
def mk_btn(txt, cls=()):
    b = Gtk.Button(label=txt)
    b.get_style_context().add_class("ctrl")
    for c in cls: b.get_style_context().add_class(c)
    return b
btn_shuf = mk_btn("⇄")
btn_prev = mk_btn("⏮", ("main",))
btn_play = mk_btn("⏸", ("main",))
btn_next = mk_btn("⏭", ("main",))
btn_rep  = mk_btn("↻")
for b in (btn_shuf, btn_prev, btn_play, btn_next, btn_rep):
    ctrls.pack_start(b, False, False, 0)
root.pack_start(ctrls, False, False, 0)

prog_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
lbl_el = Gtk.Label(label="0:00"); lbl_el.get_style_context().add_class("time")
prog_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 1000, 1)
prog_scale.get_style_context().add_class("prog")
prog_scale.set_draw_value(False)
lbl_dur = Gtk.Label(label="0:00"); lbl_dur.get_style_context().add_class("time")
prog_row.pack_start(lbl_el, False, False, 0)
prog_row.pack_start(prog_scale, True, True, 0)
prog_row.pack_end(lbl_dur, False, False, 0)
root.pack_start(prog_row, False, False, 0)

vol_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
lbl_vol = Gtk.Label(label="50%", xalign=0)
lbl_vol.set_width_chars(4)
lbl_vol.get_style_context().add_class("vol-num")
vol_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
vol_scale.set_draw_value(False)
vol_box.pack_start(lbl_vol, False, False, 0)
vol_box.pack_start(vol_scale, True, True, 0)
root.pack_start(vol_box, False, False, 0)

root.pack_start(Gtk.Separator(), False, False, 2)

foot = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
sp_mark = Gtk.DrawingArea()
sp_mark.set_size_request(26, 26)
sp_mark.set_valign(Gtk.Align.CENTER)
def draw_sp_mark(w, cr):
    import math
    s = min(w.get_allocated_width(), w.get_allocated_height())
    r = s / 2.0
    cr.arc(r, r, r - 0.5, 0, 2 * math.pi)
    cr.set_source_rgb(0.114, 0.725, 0.329)
    cr.fill()
    cr.set_source_rgb(0.07, 0.08, 0.10)
    cr.set_line_cap(1)
    for cy, rad, lw in ((r * 1.45, r * 0.92, s * 0.10), (r * 1.62, r * 0.80, s * 0.085), (r * 1.78, r * 0.68, s * 0.07)):
        cr.set_line_width(lw)
        cr.arc(r * 0.92, cy, rad, -2.15, -0.95)
        cr.stroke()
sp_mark.connect("draw", draw_sp_mark)
lbl_dev = Gtk.Label(xalign=0); lbl_dev.get_style_context().add_class("footer-dev")
btn_q = Gtk.Button(label="☰ Queue"); btn_q.get_style_context().add_class("open-btn")
btn_sp = Gtk.Button(label="↗ Spotify"); btn_sp.get_style_context().add_class("open-btn")
btn_open = Gtk.Button(label="↗ Open"); btn_open.get_style_context().add_class("open-btn")
foot.pack_start(sp_mark, False, False, 0)
foot.pack_start(lbl_dev, True, True, 0)
foot.pack_end(btn_open, False, False, 0)
foot.pack_end(btn_sp, False, False, 0)
foot.pack_end(btn_q, False, False, 0)
root.pack_start(foot, False, False, 0)

q_scroll = Gtk.ScrolledWindow()
q_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
q_scroll.set_max_content_height(140)
q_scroll.set_propagate_natural_height(True)
q_scroll.set_no_show_all(True)
q_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
q_list.set_margin_top(4)
q_scroll.add(q_list)
root.pack_start(q_scroll, False, False, 0)

state = {"dur": 0, "last_cover": "", "last_theme": None, "last_queue": None, "seeking": False, "vol_drag": False, "playing": False, "spotify_url": "", "window_xid": 0}

def send_cmd(c):
    try: ipc("CMD " + c)
    except Exception: pass
    if c == "toggle":
        state["playing"] = not state["playing"]
        btn_play.set_label("⏸" if state["playing"] else "▶")
    GLib.timeout_add(120, lambda: refresh() and False)

btn_shuf.connect("clicked", lambda *_: send_cmd("shuffle"))
btn_prev.connect("clicked", lambda *_: send_cmd("prev"))
btn_play.connect("clicked", lambda *_: send_cmd("toggle"))
btn_next.connect("clicked", lambda *_: send_cmd("next"))
btn_rep.connect("clicked",  lambda *_: send_cmd("repeat"))

def raise_x11_window(wid):
    if not wid:
        return
    try:
        import ctypes
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
            return
        d = ctypes.c_void_p(dpy)
        root = x11.XDefaultRootWindow(d)
        x11.XMapRaised(d, ctypes.c_ulong(int(wid)))
        atom = x11.XInternAtom(d, b"_NET_ACTIVE_WINDOW", 0)
        ev = XClientMessageEvent()
        ev.type = 33
        ev.send_event = 1
        ev.display = dpy
        ev.window = int(wid)
        ev.message_type = atom
        ev.format = 32
        ev.data[0] = 1
        x11.XSendEvent(d, ctypes.c_ulong(root), 0, ctypes.c_long((1 << 20) | (1 << 19)), ctypes.byref(ev))
        x11.XFlush(d)
        x11.XCloseDisplay(d)
    except Exception:
        pass

def on_open(*_):
    raise_x11_window(state.get("window_xid", 0))
    send_cmd("open")
    hide_win()

def on_spotify(*_):
    u = state.get("spotify_url")
    if u:
        try:
            subprocess.Popen(["xdg-open", u], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except Exception:
            pass
    hide_win()

def on_toggle_queue(*_):
    show = not q_scroll.get_visible()
    q_scroll.set_visible(show)
    if show:
        q_list.show_all()
        send_cmd("queue")
    else:
        win.resize(420, 1)

btn_open.connect("clicked", on_open)
btn_sp.connect("clicked", on_spotify)
btn_q.connect("clicked", on_toggle_queue)

def on_prog_changed(w, *_):
    if state["seeking"] and state["dur"] > 0:
        ms = int((w.get_value() / 1000.0) * state["dur"])
        lbl_el.set_text(fmt_ms(ms))

def on_prog_release(w, *_):
    state["seeking"] = False
    if state["dur"] > 0:
        ms = int((w.get_value() / 1000.0) * state["dur"])
        lbl_el.set_text(fmt_ms(ms))
        send_cmd(f"seek:{ms}")

prog_scale.connect("button-press-event", lambda *_: state.__setitem__("seeking", True) or False)
prog_scale.connect("value-changed", on_prog_changed)
prog_scale.connect("button-release-event", on_prog_release)

def on_vol_changed(w, *_):
    if state["vol_drag"]:
        lbl_vol.set_text(f"{int(w.get_value())}%")

def on_vol_release(w, *_):
    state["vol_drag"] = False
    v = int(w.get_value())
    lbl_vol.set_text(f"{v}%")
    send_cmd(f"volume:{v}")

vol_scale.connect("button-press-event", lambda *_: state.__setitem__("vol_drag", True) or False)
vol_scale.connect("value-changed", on_vol_changed)
vol_scale.connect("button-release-event", on_vol_release)

def refresh():
    try:
        d = json.loads(ipc("STATE"))
    except Exception:
        return True
    th = d.get("theme")
    if th and th != state["last_theme"]:
        provider.load_from_data(make_css(th))
        state["last_theme"] = th
    title = d.get("title", "")
    artist = d.get("artist", "")
    album = d.get("album", "")
    lbl_title.set_text(title); lbl_title.set_tooltip_text(title or None)
    lbl_artist.set_text(artist); lbl_artist.set_tooltip_text(artist or None)
    lbl_album.set_text(album); lbl_album.set_tooltip_text(album or None)
    dur = int(d.get("duration_ms", 0))
    pos = int(d.get("progress_ms", 0))
    state["dur"] = dur
    state["playing"] = bool(d.get("is_playing"))
    if not state["seeking"]:
        lbl_el.set_text(fmt_ms(pos))
        prog_scale.set_value((pos / dur) * 1000.0 if dur > 0 else 0)
    lbl_dur.set_text(fmt_ms(dur))
    if not state["vol_drag"]:
        v = int(d.get("volume", 50))
        vol_scale.set_value(v)
        lbl_vol.set_text(f"{v}%")
    btn_play.set_label("⏸" if state["playing"] else "▶")
    for b, on in ((btn_shuf, d.get("shuffle")), (btn_rep, d.get("repeat", "off") != "off")):
        ctx = b.get_style_context()
        ctx.add_class("active") if on else ctx.remove_class("active")
    dev = d.get("device", "")
    lbl_dev.set_text(f"Spotify · {dev}" if dev else "Spotify")
    sp_url = d.get("spotify_url", "")
    state["spotify_url"] = sp_url
    state["window_xid"] = d.get("window_xid", 0)
    btn_sp.set_visible(bool(sp_url))
    if sp_url:
        btn_sp.set_tooltip_text(sp_url)
    q_items = d.get("queue", [])
    if q_items != state["last_queue"]:
        state["last_queue"] = q_items
        for child in q_list.get_children():
            q_list.remove(child)
        if not q_items:
            empty_lbl = Gtk.Label(label="Queue is empty", xalign=0)
            empty_lbl.get_style_context().add_class("album")
            q_list.pack_start(empty_lbl, False, False, 0)
        else:
            for idx, qi in enumerate(q_items, 1):
                t = qi.get("title", "")
                a = qi.get("artist", "")
                u = qi.get("uri", "")
                row_lbl = Gtk.Label(label=f"{idx}. {t} — {a}" if a else f"{idx}. {t}", xalign=0)
                row_lbl.set_ellipsize(3)
                row_lbl.set_max_width_chars(46)
                row_lbl.get_style_context().add_class("album")
                row_btn = Gtk.Button()
                row_btn.get_style_context().add_class("open-btn")
                row_btn.add(row_lbl)
                if u:
                    row_btn.connect("clicked", lambda _, uri=u: (send_cmd(f"play_uri:{uri}"), send_cmd("queue")))
                q_list.pack_start(row_btn, False, False, 0)
        q_list.show_all()
    cp = d.get("cover_path", "")
    if cp and cp != state["last_cover"] and os.path.exists(cp):
        try:
            pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(cp, 96, 96, True)
            try:
                import cairo, math
                w, h = pb.get_width(), pb.get_height()
                surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
                cr = cairo.Context(surf)
                r = 4.0
                cr.new_sub_path()
                cr.arc(w - r, r, r, -math.pi / 2, 0)
                cr.arc(w - r, h - r, r, 0, math.pi / 2)
                cr.arc(r, h - r, r, math.pi / 2, math.pi)
                cr.arc(r, r, r, math.pi, 3 * math.pi / 2)
                cr.close_path()
                cr.clip()
                Gdk.cairo_set_source_pixbuf(cr, pb, 0, 0)
                cr.paint()
                pb = Gdk.pixbuf_get_from_surface(surf, 0, 0, w, h)
            except Exception:
                pass
            cover.set_from_pixbuf(pb)
            state["last_cover"] = cp
        except Exception:
            pass
    return True

refresh()
timer_id = [None]
def start_timer():
    if timer_id[0] is None:
        timer_id[0] = GLib.timeout_add(150, refresh)
def stop_timer():
    if timer_id[0] is not None:
        GLib.source_remove(timer_id[0])
        timer_id[0] = None

def hide_win(*_):
    stop_timer()
    win.hide()
    return True

def show_at(px, py):
    refresh()
    disp = Gdk.Display.get_default()
    mon = disp.get_monitor_at_point(max(0, px), max(0, py)) if px >= 0 else disp.get_primary_monitor()
    win.show_all()
    if not q_scroll.get_visible():
        q_scroll.hide()
    if mon:
        geo = mon.get_workarea()
        w, h = win.get_size()
        x = (px - w // 2) if px >= 0 else (geo.x + geo.width - w - 12)
        y = (py + 12) if py >= 0 else (geo.y + 8)
        x = max(geo.x + 8, min(x, geo.x + geo.width - w - 8))
        y = max(geo.y + 8, min(y, geo.y + geo.height - h - 8))
        win.move(x, y)
    win.present()
    start_timer()

def on_stdin(src, cond):
    line = src.readline()
    if not line:
        Gtk.main_quit()
        return False
    parts = line.strip().split()
    if not parts:
        return True
    cmd = parts[0]
    if cmd == "TOGGLE":
        if win.get_visible():
            hide_win()
        else:
            px = int(parts[1]) if len(parts) > 1 else -1
            py = int(parts[2]) if len(parts) > 2 else -1
            show_at(px, py)
    elif cmd == "HIDE":
        hide_win()
    elif cmd == "QUIT":
        Gtk.main_quit()
        return False
    return True

win.connect("key-press-event", lambda _, e: hide_win() if e.keyval == Gdk.KEY_Escape else False)
win.connect("focus-out-event", hide_win)
GLib.io_add_watch(sys.stdin, GLib.IO_IN | GLib.IO_HUP, on_stdin)
if POS_X >= 0 or POS_Y >= 0:
    show_at(POS_X, POS_Y)
Gtk.main()
"##;

#[cfg(target_os = "linux")]
struct PersistentPopover {
  child: std::process::Child,
  stdin: std::process::ChildStdin,
}

#[cfg(target_os = "linux")]
fn popover_slot() -> &'static std::sync::Mutex<Option<PersistentPopover>> {
  static POPUP: std::sync::OnceLock<std::sync::Mutex<Option<PersistentPopover>>> =
    std::sync::OnceLock::new();
  POPUP.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(target_os = "linux")]
fn ensure_popover_started(guard: &mut Option<PersistentPopover>) {
  use std::process::{Command, Stdio};
  if let Some(p) = guard.as_mut() {
    if p.child.try_wait().ok().flatten().is_none() {
      return;
    }
  }
  *guard = None;
  let sock = crate::platform::tray_ipc::socket_path();
  let sock_str = sock.to_string_lossy().into_owned();
  let mut cmd = Command::new("python3");
  cmd
    .args(["-c", GTK_POPOVER_PY, &sock_str, "-1", "-1"])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
  if let Ok(mut child) = cmd.spawn() {
    if let Some(stdin) = child.stdin.take() {
      *guard = Some(PersistentPopover { child, stdin });
    }
  }
}

#[cfg(target_os = "linux")]
pub fn prewarm_popover() {
  if let Ok(mut guard) = popover_slot().lock() {
    ensure_popover_started(&mut guard);
  }
}

#[cfg(target_os = "linux")]
pub fn shutdown_popover() {
  use std::io::Write as _;
  if let Ok(mut guard) = popover_slot().lock() {
    if let Some(mut p) = guard.take() {
      let _ = writeln!(p.stdin, "QUIT");
      let _ = p.child.kill();
      let _ = p.child.wait();
    }
  }
}

#[cfg(target_os = "linux")]
pub fn toggle_integrated_popover(pos: Option<(i32, i32)>) {
  use std::io::Write as _;
  let Ok(mut guard) = popover_slot().lock() else {
    return;
  };
  ensure_popover_started(&mut guard);
  let (x, y) = pos.unwrap_or((-1, -1));
  if let Some(p) = guard.as_mut() {
    if writeln!(p.stdin, "TOGGLE {x} {y}").is_err() {
      *guard = None;
      ensure_popover_started(&mut guard);
      if let Some(p2) = guard.as_mut() {
        let _ = writeln!(p2.stdin, "TOGGLE {x} {y}");
      }
    }
  }
}

#[cfg(not(target_os = "linux"))]
pub fn prewarm_popover() {}

#[cfg(not(target_os = "linux"))]
pub fn shutdown_popover() {}

#[cfg(not(target_os = "linux"))]
pub fn toggle_integrated_popover(_pos: Option<(i32, i32)>) {}

#[cfg(target_os = "linux")]
mod imp {
  use std::sync::{mpsc::Sender, Arc};
  use tokio::sync::Mutex;
  use zbus::interface;

  use crate::app::App;
  use crate::backend::IoEvent;

  struct SptuneTray {
    app: Arc<Mutex<App>>,
  }

  #[interface(name = "org.kde.StatusNotifierItem")]
  impl SptuneTray {
    #[zbus(property)]
    fn category(&self) -> &str {
      "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
      "sptune"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
      "sptune"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
      "Active"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
      "multimedia-player"
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
      false
    }

    async fn activate(&self, x: i32, y: i32) {
      super::toggle_integrated_popover(Some((x, y)));
    }

    async fn secondary_activate(&self, _x: i32, _y: i32) {
      let mut app = self.app.lock().await;
      app.toggle_playback();
    }

    async fn scroll(&self, delta: i32, _orientation: &str) {
      let mut app = self.app.lock().await;
      if delta > 0 {
        app.increase_volume();
      } else if delta < 0 {
        app.decrease_volume();
      }
    }
  }

  pub fn spawn(_io_tx: Sender<IoEvent>, app: Arc<Mutex<App>>) {
    let _ = std::thread::Builder::new()
      .name("sptune-tray".into())
      .spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
          .enable_all()
          .build()
        else {
          return;
        };
        rt.block_on(async move {
          let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
          let Ok(conn) = zbus::connection::Builder::session()
            .name(service.as_str())
            .serve_at("/StatusNotifierItem", SptuneTray { app })
            .build()
            .await
          else {
            return;
          };
          let _ = conn
            .call_method(
              Some("org.kde.StatusNotifierWatcher"),
              "/StatusNotifierWatcher",
              Some("org.kde.StatusNotifierWatcher"),
              "RegisterStatusNotifierItem",
              &(service.as_str(),),
            )
            .await;
          super::prewarm_popover();
          std::future::pending::<()>().await;
        });
      });
  }
}

#[cfg(not(target_os = "linux"))]
mod imp {
  use std::sync::{mpsc::Sender, Arc};
  use tokio::sync::Mutex;

  use crate::app::App;
  use crate::backend::IoEvent;

  pub fn spawn(_io_tx: Sender<IoEvent>, _app: Arc<Mutex<App>>) {}
}

pub fn spawn_tray(
  io_tx: std::sync::mpsc::Sender<crate::backend::IoEvent>,
  app: std::sync::Arc<tokio::sync::Mutex<crate::app::App>>,
) {
  imp::spawn(io_tx, app);
}
