//! Local Unix-socket IPC bridge for the native taskbar tray popover.
//! Serves live `App` playback state as JSON and dispatches control commands
//! without spending extra Spotify API tokens.

#[cfg(target_os = "linux")]
mod imp {
  use rspotify::prelude::Id;
  use std::path::PathBuf;
  use std::sync::Arc;
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
  use tokio::net::UnixListener;
  use tokio::sync::Mutex;

  use crate::app::App;
  use crate::backend::IoEvent;

  pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
      if !dir.is_empty() {
        return PathBuf::from(dir).join("sptune-tray.sock");
      }
    }
    PathBuf::from(format!("/tmp/sptune-tray-{}.sock", std::process::id()))
  }

  pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
  }

  fn color_hex(c: ratatui::style::Color, fallback: &str) -> String {
    match c {
      ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
      ratatui::style::Color::Black => "#000000".to_string(),
      ratatui::style::Color::White => "#ffffff".to_string(),
      ratatui::style::Color::Red => "#e06c75".to_string(),
      ratatui::style::Color::Green => "#98c379".to_string(),
      ratatui::style::Color::Yellow => "#e5c07b".to_string(),
      ratatui::style::Color::Blue => "#61afef".to_string(),
      ratatui::style::Color::Magenta => "#c678dd".to_string(),
      ratatui::style::Color::Cyan => "#56b6c2".to_string(),
      ratatui::style::Color::Gray => "#abb2bf".to_string(),
      ratatui::style::Color::DarkGray => "#5c6370".to_string(),
      ratatui::style::Color::LightRed => "#ff7b72".to_string(),
      ratatui::style::Color::LightGreen => "#7ee787".to_string(),
      ratatui::style::Color::LightYellow => "#f2cc60".to_string(),
      ratatui::style::Color::LightBlue => "#79c0ff".to_string(),
      ratatui::style::Color::LightMagenta => "#d2a8ff".to_string(),
      ratatui::style::Color::LightCyan => "#a5d6ff".to_string(),
      _ => fallback.to_string(),
    }
  }

  async fn build_state_json(app: &Arc<Mutex<App>>) -> String {
    let guard = app.lock().await;
    let th = &guard.user_config.theme;
    let theme_json = serde_json::json!({
      "bg": color_hex(th.background, "#0e101c"),
      "border": color_hex(th.active, "#5965a6"),
      "accent": color_hex(th.playbar_progress, "#f5c79e"),
      "selected": color_hex(th.selected, "#f6e6d4"),
      "text": color_hex(th.text, "#e6dfd5"),
      "muted": color_hex(th.inactive, "#8c91ab"),
    });
    let elapsed_ms = if guard
      .current_playback_context
      .as_ref()
      .is_some_and(|c| c.is_playing)
    {
      guard
        .instant_since_last_current_playback_poll
        .elapsed()
        .as_millis()
    } else {
      0
    };
    let raw_progress = guard.seek_ms.unwrap_or(guard.song_progress_ms);
    if let Some(ctx) = &guard.current_playback_context {
      let (title, artist, album, duration_ms, cover_path, spotify_url) = match &ctx.item {
        Some(rspotify::model::PlayableItem::Track(t)) => {
          let artists = t
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
          let cover = crate::cover_art::pick_url(&t.album.images)
            .map(|u| crate::cover_art::cache_file(&u))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
          let url = t
            .album
            .external_urls
            .get("spotify")
            .cloned()
            .or_else(|| {
              t.album
                .id
                .as_ref()
                .map(|id| format!("https://open.spotify.com/album/{}", id.id()))
            })
            .unwrap_or_default();
          (
            t.name.clone(),
            artists,
            t.album.name.clone(),
            t.duration.num_milliseconds() as u64,
            cover,
            url,
          )
        }
        Some(rspotify::model::PlayableItem::Episode(e)) => {
          let cover = crate::cover_art::pick_url(&e.images)
            .map(|u| crate::cover_art::cache_file(&u))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
          let url = e
            .external_urls
            .get("spotify")
            .cloned()
            .unwrap_or_else(|| format!("https://open.spotify.com/episode/{}", e.id.id()));
          (
            e.name.clone(),
            e.show.name.clone(),
            String::new(),
            e.duration.num_milliseconds() as u64,
            cover,
            url,
          )
        }
        _ => (
          "Nothing playing".to_string(),
          String::new(),
          String::new(),
          0,
          String::new(),
          String::new(),
        ),
      };
      // Anchor on ctx.progress so we never double-count elapsed_ms on top of
      // App::song_progress_ms (which update_on_tick already advances).
      let base_progress_ms = ctx
        .progress
        .map(|p| p.num_milliseconds().max(0) as u128)
        .unwrap_or(raw_progress);
      let progress_ms = if let Some(seek) = guard.seek_ms {
        seek
      } else {
        (base_progress_ms + elapsed_ms).min(duration_ms as u128)
      };
      let repeat = match ctx.repeat_state {
        rspotify::model::RepeatState::Off => "off",
        rspotify::model::RepeatState::Context => "context",
        rspotify::model::RepeatState::Track => "track",
      };
      let volume = guard
        .volume_preview
        .or_else(|| ctx.device.volume_percent.map(|v| v as u8))
        .unwrap_or(50);
      let mut seen_q = std::collections::HashSet::new();
      let queue: Vec<serde_json::Value> = guard
        .queue_items
        .iter()
        .filter_map(|item| match item {
          rspotify::model::PlayableItem::Track(t) => {
            let uri = t.id.as_ref().map(|id| id.uri()).unwrap_or_default();
            if uri.is_empty() || !seen_q.insert(uri.clone()) {
              return None;
            }
            let artists = t
              .artists
              .iter()
              .map(|a| a.name.as_str())
              .collect::<Vec<_>>()
              .join(", ");
            Some(serde_json::json!({ "title": t.name, "artist": artists, "uri": uri }))
          }
          rspotify::model::PlayableItem::Episode(e) => {
            let uri = e.id.uri();
            if !seen_q.insert(uri.clone()) {
              return None;
            }
            Some(serde_json::json!({ "title": e.name, "artist": e.show.name, "uri": uri }))
          }
          _ => None,
        })
        .take(15)
        .collect();
      serde_json::json!({
        "title": title,
        "artist": artist,
        "album": album,
        "cover_path": cover_path,
        "spotify_url": spotify_url,
        "progress_ms": progress_ms,
        "duration_ms": duration_ms,
        "is_playing": ctx.is_playing,
        "shuffle": ctx.shuffle_state,
        "repeat": repeat,
        "volume": volume,
        "device": ctx.device.name,
        "queue": queue,
        "theme": theme_json,
        "window_xid": crate::platform::window::current_xid().unwrap_or(0),
      })
      .to_string()
    } else {
      serde_json::json!({
        "title": "Nothing playing",
        "artist": "Start playback on Spotify",
        "album": "",
        "cover_path": "",
        "spotify_url": "",
        "progress_ms": 0,
        "duration_ms": 0,
        "is_playing": false,
        "shuffle": false,
        "repeat": "off",
        "volume": 50,
        "device": "No device",
        "theme": theme_json,
        "window_xid": crate::platform::window::current_xid().unwrap_or(0),
      })
      .to_string()
    }
  }

  async fn handle_command(cmd: &str, app: &Arc<Mutex<App>>) {
    let mut guard = app.lock().await;
    match cmd.trim() {
      "toggle" => guard.toggle_playback(),
      "next" => guard.dispatch(IoEvent::NextTrack),
      "prev" => guard.previous_track(),
      "shuffle" => guard.shuffle(),
      "repeat" => guard.repeat(),
      "queue" => guard.dispatch(IoEvent::GetQueue),
      "open" => {
        crate::platform::window::restore_from_tray();
        guard.ui_dirty = true;
      }
      s if s.starts_with("play_uri:") => {
        let uri = s["play_uri:".len()..].trim().to_string();
        if !uri.is_empty() {
          let ctx_uri = guard
            .current_playback_context
            .as_ref()
            .and_then(|c| c.context.as_ref())
            .map(|ctx| ctx.uri.clone());
          if let Some(ctx_uri) = ctx_uri {
            guard.dispatch(IoEvent::StartPlaybackAt(Some(ctx_uri), Some(uri)));
          } else {
            let mut uris = vec![uri.clone()];
            let mut after = false;
            for item in &guard.queue_items {
              let item_uri = match item {
                rspotify::model::PlayableItem::Track(t) => t.id.as_ref().map(|id| id.uri()),
                rspotify::model::PlayableItem::Episode(e) => Some(e.id.uri()),
                _ => None,
              };
              if let Some(u) = item_uri {
                if u == uri {
                  after = true;
                } else if after && !uris.contains(&u) {
                  uris.push(u);
                }
              }
            }
            guard.dispatch(IoEvent::StartPlayback(None, Some(uris), Some(0)));
          }
        }
      }
      s if s.starts_with("seek:") => {
        if let Ok(ms) = s["seek:".len()..].parse::<u32>() {
          guard.seek_to(ms);
        }
      }
      s if s.starts_with("volume:") => {
        if let Ok(pct) = s["volume:".len()..].parse::<u8>() {
          let pct = pct.min(100);
          guard.volume_preview = Some(pct);
          guard.dispatch(IoEvent::ChangeVolume(pct));
        }
      }
      _ => {}
    }
    guard.ui_dirty = true;
  }

  pub fn spawn(app: Arc<Mutex<App>>) {
    let _ = std::thread::Builder::new()
      .name("sptune-tray-ipc".into())
      .spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
          .enable_all()
          .build()
        else {
          return;
        };
        rt.block_on(async move {
          let path = socket_path();
          let _ = std::fs::remove_file(&path);
          let Ok(listener) = UnixListener::bind(&path) else {
            return;
          };
          loop {
            let Ok((stream, _)) = listener.accept().await else {
              break;
            };
            let app = Arc::clone(&app);
            tokio::spawn(async move {
              let (reader, mut writer) = stream.into_split();
              let mut lines = BufReader::new(reader).lines();
              while let Ok(Some(line)) = lines.next_line().await {
                if line == "STATE" {
                  let json = build_state_json(&app).await;
                  let _ = writer.write_all(json.as_bytes()).await;
                  let _ = writer.write_all(b"\n").await;
                } else if let Some(cmd) = line.strip_prefix("CMD ") {
                  handle_command(cmd, &app).await;
                  let _ = writer.write_all(b"OK\n").await;
                }
              }
            });
          }
        });
      });
  }
}

#[cfg(not(target_os = "linux"))]
mod imp {
  use std::sync::Arc;
  use tokio::sync::Mutex;

  use crate::app::App;

  pub fn socket_path() -> std::path::PathBuf {
    std::path::PathBuf::new()
  }
  pub fn cleanup() {}
  pub fn spawn(_app: Arc<Mutex<App>>) {}
}

pub fn socket_path() -> std::path::PathBuf {
  imp::socket_path()
}

pub fn cleanup() {
  imp::cleanup();
}

pub fn spawn(app: std::sync::Arc<tokio::sync::Mutex<crate::app::App>>) {
  imp::spawn(app);
}
