//! OS media-session watcher: Windows GSMTC via win-gsmtc, Linux MPRIS via
//! zbus. Forwards track/playback changes as `IoEvent::GetCurrentPlayback`
//! so the playbar learns the truth without spending API tokens on a poll.
//! Any init failure ends only this thread; the regular API poll is untouched.
//!
//! Both backends push OS-native events (COM callbacks / D-Bus signals):
//! latency is OS event (~ms) + one Spotify API roundtrip, which is the
//! floor without the Dealer socket (it rejects third-party tokens).

use crate::backend::IoEvent;

/// True when the session belongs to Spotify (app name or id contains it).
pub fn is_spotify_session(source_name: Option<&str>, source_id: &str) -> bool {
  // ponytail: two small allocs on a rare path; keeps matching case-proof.
  source_name
    .map(|n| n.to_lowercase().contains("spotify"))
    .unwrap_or(false)
    || source_id.to_lowercase().contains("spotify")
}

/// One OS media signal, already translated from the native backend event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsMediaSignal {
  /// Current media item changed (track switch).
  TrackChanged,
  /// Play/pause/stop transition.
  PlaybackChanged,
  /// This session became the system's current one.
  SessionBecameCurrent { name: Option<String>, id: String },
  /// No session is current anymore (player vanished: re-check, clear UI).
  NoCurrentSession,
}

/// True when the signal is worth one Spotify API poll.
pub fn should_refresh_signal(signal: &OsMediaSignal) -> bool {
  match signal {
    // No source attached: the API poll itself tells us whether Spotify moved.
    OsMediaSignal::TrackChanged
    | OsMediaSignal::PlaybackChanged
    | OsMediaSignal::NoCurrentSession => true,
    OsMediaSignal::SessionBecameCurrent { name, id } => is_spotify_session(name.as_deref(), id),
  }
}

#[cfg(windows)]
pub fn spawn_media_watcher(io_tx: std::sync::mpsc::Sender<IoEvent>) {
  let _ = std::thread::Builder::new()
    .name("sptune-media-watch".into())
    .spawn(move || {
      let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
      else {
        return;
      };
      rt.block_on(watch_gsmtc(io_tx));
    });
}

/// True GSMTC push: COM callbacks, no polling anywhere in this path.
#[cfg(windows)]
async fn watch_gsmtc(io_tx: std::sync::mpsc::Sender<IoEvent>) {
  let Ok(mut rx) = gsmtc::SessionManager::create().await else {
    return;
  };
  // session id -> source AUMID, so CurrentSessionChanged can gate on Spotify.
  let mut sources: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
  while let Some(evt) = rx.recv().await {
    match evt {
      gsmtc::ManagerEvent::SessionCreated {
        session_id,
        rx: session_rx,
        source,
      } => {
        sources.insert(session_id, source.clone());
        // Only Spotify sessions get a task; other apps' noise never wakes us.
        if is_spotify_session(Some(&source), &source) {
          let tx = io_tx.clone();
          tokio::spawn(async move {
            let mut rx = session_rx;
            let mut last: Option<gsmtc::PlaybackStatus> = None;
            while let Some(evt) = rx.recv().await {
              let signal = match evt {
                // New media item = new track, always refresh.
                gsmtc::SessionUpdateEvent::Media(..) => Some(OsMediaSignal::TrackChanged),
                // Model covers playback + timeline: forward status flips
                // only, so position/volume noise stays silent.
                gsmtc::SessionUpdateEvent::Model(m) => {
                  let status = m.playback.as_ref().map(|p| p.status.clone());
                  if status != last {
                    last = status;
                    Some(OsMediaSignal::PlaybackChanged)
                  } else {
                    None
                  }
                }
              };
              if let Some(s) = signal {
                if should_refresh_signal(&s) {
                  let _ = tx.send(IoEvent::GetCurrentPlayback);
                }
              }
            }
          });
        }
      }
      gsmtc::ManagerEvent::SessionRemoved { session_id } => {
        sources.remove(&session_id);
      }
      gsmtc::ManagerEvent::CurrentSessionChanged { session_id } => {
        let signal = match session_id {
          None => OsMediaSignal::NoCurrentSession,
          Some(id) => match sources.get(&id) {
            Some(src) => OsMediaSignal::SessionBecameCurrent {
              name: Some(src.clone()),
              id: src.clone(),
            },
            // Unknown (already-removed or never-Spotify) session: ignore.
            None => continue,
          },
        };
        if should_refresh_signal(&signal) {
          let _ = io_tx.send(IoEvent::GetCurrentPlayback);
        }
      }
    }
  }
}

#[cfg(target_os = "linux")]
pub fn spawn_media_watcher(io_tx: std::sync::mpsc::Sender<IoEvent>) {
  let _ = std::thread::Builder::new()
    .name("sptune-media-watch".into())
    .spawn(move || {
      let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
      else {
        return;
      };
      rt.block_on(async {
        let Ok(conn) = zbus::Connection::session().await else {
          return;
        };
        watch_mpris(io_tx, conn).await;
      });
    });
}

#[cfg(target_os = "linux")]
const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

/// Minimal typed proxy: only the two properties whose change means
/// "the playbar is stale". Position/volume seekers never fire these.
#[cfg(target_os = "linux")]
#[zbus::proxy(
  interface = "org.mpris.MediaPlayer2.Player",
  default_path = "/org/mpris/MediaPlayer2"
)]
trait MprisPlayer {
  #[zbus(property)]
  fn metadata(&self)
    -> zbus::Result<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>;

  #[zbus(property)]
  fn playback_status(&self) -> zbus::Result<String>;
}

#[cfg(target_os = "linux")]
fn is_mpris_spotify(bus_name: &str) -> bool {
  bus_name.starts_with(MPRIS_PREFIX) && is_spotify_session(None, bus_name)
}

/// True D-Bus push: PropertiesChanged + NameOwnerChanged signals only.
/// Rebuilds the watch set whenever Spotify players come or go.
#[cfg(target_os = "linux")]
async fn watch_mpris(io_tx: std::sync::mpsc::Sender<IoEvent>, conn: zbus::Connection) {
  use futures::{select, StreamExt};

  let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
    return;
  };
  // ponytail: full rebuild per membership change (ListNames + proxies, all
  // local, ~ms). Player churn is rare; no incremental bookkeeping.
  loop {
    let mut players: Vec<String> = Vec::new();
    if let Ok(names) = dbus.list_names().await {
      for name in names {
        let bus = name.to_string();
        if is_mpris_spotify(&bus) {
          players.push(bus);
        }
      }
    }
    let mut events: futures::stream::SelectAll<futures::stream::BoxStream<'_, OsMediaSignal>> =
      futures::stream::SelectAll::new();
    for bus in &players {
      let Ok(proxy) = MprisPlayerProxy::builder(&conn)
        .destination(bus.as_str())
        .build()
        .await
      else {
        continue;
      };
      events.push(
        proxy
          .receive_metadata_changed()
          .await
          .map(|_| OsMediaSignal::TrackChanged)
          .boxed(),
      );
      events.push(
        proxy
          .receive_playback_status_changed()
          .await
          .map(|_| OsMediaSignal::PlaybackChanged)
          .boxed(),
      );
    }
    let membership = dbus.receive_name_owner_changed().await;
    let Ok(membership) = membership else {
      return;
    };
    // Boxed: guarantees Unpin for select!, whatever the native stream types are.
    let mut events = events.boxed().fuse();
    let mut membership = membership.map(spotify_membership_signal).boxed().fuse();
    // Player event -> refresh, stay armed. Spotify membership change (or a
    // dead watch set) -> refresh and rebuild.
    loop {
      select! {
        e = events.next() => match e {
          Some(signal) => {
            if should_refresh_signal(&signal) {
              let _ = io_tx.send(IoEvent::GetCurrentPlayback);
            }
          }
          // Every watched player is gone: rebuild.
          None => break,
        },
        m = membership.next() => match m {
          Some(Some(signal)) => {
            if should_refresh_signal(&signal) {
              let _ = io_tx.send(IoEvent::GetCurrentPlayback);
            }
            break;
          }
          // Unrelated player churn: stay armed.
          Some(None) => {}
          None => break,
        },
      }
    }
  }
}

/// Maps a NameOwnerChanged signal to a refresh signal for Spotify MPRIS
/// players (appear -> focus check, vanish -> re-check and clear).
/// Unrelated players map to None.
#[cfg(target_os = "linux")]
fn spotify_membership_signal(signal: zbus::fdo::NameOwnerChanged) -> Option<OsMediaSignal> {
  let Ok(args) = signal.args() else {
    return None;
  };
  let id = args.name().to_string();
  if !is_mpris_spotify(&id) {
    return None;
  }
  if args.new_owner.as_ref().is_some() {
    Some(OsMediaSignal::SessionBecameCurrent { name: None, id })
  } else if args.old_owner.as_ref().is_some() {
    Some(OsMediaSignal::NoCurrentSession)
  } else {
    None
  }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn spawn_media_watcher(io_tx: std::sync::mpsc::Sender<IoEvent>) {
  // No OS media API wired here yet; the regular API poll covers it.
  let _ = io_tx;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn spotify_matches_by_name_or_id() {
    assert!(is_spotify_session(Some("Spotify"), "spotify"));
    assert!(is_spotify_session(Some("SPOTIFY"), "whatever"));
    assert!(is_spotify_session(None, "org.mpris.MediaPlayer2.spotify"));
    assert!(is_spotify_session(None, "Spotify.exe"));
    // Windows AUMID form.
    assert!(is_spotify_session(
      Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
      "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"
    ));
    assert!(!is_spotify_session(Some("Chrome"), "chrome"));
    assert!(!is_spotify_session(None, "vlc"));
  }

  #[test]
  fn track_playback_and_empty_session_always_refresh() {
    assert!(should_refresh_signal(&OsMediaSignal::TrackChanged));
    assert!(should_refresh_signal(&OsMediaSignal::PlaybackChanged));
    assert!(should_refresh_signal(&OsMediaSignal::NoCurrentSession));
  }

  #[test]
  fn session_focus_refreshes_only_for_spotify() {
    assert!(should_refresh_signal(
      &OsMediaSignal::SessionBecameCurrent {
        name: Some("Spotify".into()),
        id: "spotify".into(),
      }
    ));
    assert!(!should_refresh_signal(
      &OsMediaSignal::SessionBecameCurrent {
        name: Some("Chrome".into()),
        id: "chrome".into(),
      }
    ));
  }
}
