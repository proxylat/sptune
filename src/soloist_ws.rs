//! Level 2 push: Soloist WebSocket playback snapshots.
//!
//! Long-lived `watch` task. Connects to soloist's local WS server
//! (`ws://127.0.0.1:{port}`, port from `<data_dir>/ws.port`), translates
//! `playback_state` snapshots into [`CurrentPlaybackContext`] and stores them
//! on [`App`] exactly like an API poll would. No commands are ever sent
//! except `get_state` queries; control stays on the Web API.

use crate::app::App;
use crate::backend::IoEvent;
use futures::{Sink, SinkExt, StreamExt};
use rspotify::model::{
  Actions, AlbumId, ArtistId, Context, CurrentPlaybackContext, CurrentlyPlayingType, Device,
  DeviceType, FullTrack, Image, PlayableItem, RepeatState, SimplifiedAlbum, SimplifiedArtist,
  TrackId, Type,
};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

// --- Soloist protocol structs (liberal: every struct defaulted) ---

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entity {
  uri: String,
  entity_type: String,
  decorations: Decorations,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Decorations {
  identity: Identity,
  visual_identity: VisualIdentity,
  parent: Option<Box<Entity>>,
  creators: Vec<Creator>,
  playback: PlaybackDecorations,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Identity {
  name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct VisualIdentity {
  cover: Vec<Cover>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Cover {
  url: String,
  size: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Creator {
  entity: Entity,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PlaybackDecorations {
  duration_ms: i64,
  content_ratings: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Position {
  position_ms: i64,
  timestamp_ms: i64,
  speed: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PlaybackOptions {
  shuffle: bool,
  repeat: String,
  playback_speed: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PlaybackState {
  status: String,
  item: Entity,
  context: Entity,
  position: Position,
  volume: u32,
  is_active: bool,
  options: PlaybackOptions,
  available_actions: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PositionSync {
  position: Position,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AuthState {
  logged_in: bool,
  is_active: bool,
  device_name: String,
}

// --- Entry point ---

pub async fn watch(
  app: std::sync::Arc<Mutex<App>>,
  data_dir: std::path::PathBuf,
  device_name: String,
) {
  let Some(port) = wait_for_port(&data_dir).await else {
    return;
  };
  let url = format!("ws://127.0.0.1:{port}");
  let mut backoff = Duration::from_secs(1);
  loop {
    if let Ok((ws, _)) = connect_async(url.clone()).await {
      backoff = Duration::from_secs(1);
      run_connection(&app, ws, &device_name).await;
    }
    // Stream ended or connect failed: mark dead, quit when soloist is gone,
    // otherwise back off (1s x2, capped at 30s) and retry.
    app.lock().await.soloist_live = false;
    if !data_dir.join("ws.port").exists() {
      return;
    }
    tokio::time::sleep(backoff).await;
    backoff = (backoff * 2).min(Duration::from_secs(30));
  }
}

/// Poll `<data_dir>/ws.port` every 250ms up to 15s; parse trimmed content as u16.
async fn wait_for_port(data_dir: &Path) -> Option<u16> {
  for _ in 0..60 {
    if let Ok(content) = tokio::fs::read_to_string(data_dir.join("ws.port")).await {
      if let Ok(port) = content.trim().parse::<u16>() {
        return Some(port);
      }
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
  }
  None
}

async fn run_connection(
  app: &Arc<Mutex<App>>,
  ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
  device_name: &str,
) {
  let (mut sink, mut stream) = ws.split();
  // Open with a full snapshot; every later delta just re-asks.
  send_get_state(&mut sink).await;
  let mut live = false;
  let mut last_heart: Option<String> = None;
  while let Some(msg) = stream.next().await {
    let msg = match msg {
      Ok(msg) => msg,
      Err(_) => break,
    };
    let Message::Text(text) = msg else { continue };
    let text: &str = &text;
    let value: serde_json::Value = match serde_json::from_str(text) {
      Ok(value) => value,
      Err(_) => continue,
    };
    match value.get("type").and_then(|t| t.as_str()) {
      Some("playback_state") => {
        let Ok(state) = serde_json::from_value::<PlaybackState>(value) else {
          continue;
        };
        handle_playback_state(app, state, device_name, &mut live, &mut last_heart).await;
      }
      Some("position_sync") => {
        if !live {
          continue;
        }
        let Ok(sync) = serde_json::from_value::<PositionSync>(value) else {
          continue;
        };
        let mut guard = app.lock().await;
        if let Some(ctx) = guard.current_playback_context.as_mut() {
          ctx.progress = Some(chrono::Duration::milliseconds(sync.position.position_ms));
          guard.instant_since_last_current_playback_poll = Instant::now();
          guard.ui_dirty = true;
        }
      }
      Some("auth_state") => {
        let Ok(auth) = serde_json::from_value::<AuthState>(value) else {
          continue;
        };
        if !auth.logged_in {
          set_live(app, &mut live, false).await;
        }
      }
      // Nothing to merge: queue/command acks/errors carry no playback state.
      Some("queue_changed") | Some("command_result") | Some("error") => {}
      // Deltas we deliberately don't merge (track/volume/device/context/
      // options changes, unknown): re-query the snapshot instead.
      _ => {
        send_get_state(&mut sink).await;
      }
    }
  }
}

async fn handle_playback_state(
  app: &Arc<Mutex<App>>,
  state: PlaybackState,
  device_name: &str,
  live: &mut bool,
  last_heart: &mut Option<String>,
) {
  if !state.is_active {
    // Another device took over; leave the context for the API poll.
    set_live(app, live, false).await;
    return;
  }
  if state.item.entity_type != "track" {
    // Episodes/ads/unknown stay API-driven.
    set_live(app, live, false).await;
    return;
  }
  let ctx = build_context(&state, device_name);
  // Bare track id for the heart prime (spotify:track:XXX -> XXX).
  let heart = state
    .item
    .uri
    .strip_prefix("spotify:track:")
    .map(str::to_owned);
  let mut guard = app.lock().await;
  guard.current_playback_context = Some(ctx);
  guard.playback_known = true;
  guard.instant_since_last_current_playback_poll = Instant::now();
  guard.ui_dirty = true;
  if !*live {
    *live = true;
    guard.soloist_live = true;
  }
  if heart.as_ref() != last_heart.as_ref() {
    *last_heart = heart.clone();
    if let Some(id) = heart {
      guard.dispatch(IoEvent::CurrentUserSavedTracksContains(vec![id]));
    }
  }
}

/// Mirror a loop-local liveness change onto the app (guard dropped at once).
async fn set_live(app: &Arc<Mutex<App>>, live: &mut bool, value: bool) {
  if *live != value {
    *live = value;
    app.lock().await.soloist_live = value;
  }
}

async fn send_get_state<S>(sink: &mut S)
where
  S: Sink<Message> + Unpin,
{
  let query = serde_json::json!({"type": "command", "command": "get_state"}).to_string();
  let _ = sink.send(Message::Text(query.into())).await;
}

/// Map a `playback_state` snapshot onto [`CurrentPlaybackContext`].
#[allow(deprecated)]
fn build_context(state: &PlaybackState, device_name: &str) -> CurrentPlaybackContext {
  let item = &state.item;
  let decorations = &item.decorations;
  let parent = decorations.parent.as_deref();

  let album_id = parent.and_then(|e| {
    AlbumId::from_id_or_uri(&e.uri)
      .ok()
      .map(|id| id.into_static())
  });
  let album_name = parent
    .map(|e| e.decorations.identity.name.clone())
    .unwrap_or_default();
  let images = parent
    .map(|e| {
      e.decorations
        .visual_identity
        .cover
        .iter()
        .map(|c| Image {
          height: None,
          width: None,
          url: c.url.clone(),
        })
        .collect()
    })
    .unwrap_or_default();

  let artists = decorations
    .creators
    .iter()
    .map(|c| &c.entity)
    .filter(|e| e.entity_type == "artist")
    .map(|e| SimplifiedArtist {
      external_urls: Default::default(),
      href: None,
      id: ArtistId::from_id_or_uri(&e.uri)
        .ok()
        .map(|id| id.into_static()),
      name: e.decorations.identity.name.clone(),
    })
    .collect();

  let track = FullTrack {
    album: SimplifiedAlbum {
      album_group: None,
      album_type: None,
      artists: vec![],
      available_markets: vec![],
      external_urls: Default::default(),
      href: None,
      id: album_id,
      images,
      name: album_name,
      release_date: None,
      release_date_precision: None,
      restrictions: None,
    },
    artists,
    available_markets: vec![],
    disc_number: 0,
    duration: chrono::Duration::milliseconds(decorations.playback.duration_ms),
    explicit: decorations
      .playback
      .content_ratings
      .iter()
      .any(|r| r == "explicit"),
    external_ids: Default::default(),
    external_urls: Default::default(),
    href: None,
    id: TrackId::from_id_or_uri(&item.uri)
      .ok()
      .map(|id| id.into_static()),
    is_local: false,
    is_playable: None,
    linked_from: None,
    restrictions: None,
    name: decorations.identity.name.clone(),
    popularity: 0,
    preview_url: None,
    track_number: 0,
    r#type: Type::Track,
  };

  let context = if state.context.uri.is_empty() {
    None
  } else {
    let kind = match state.context.entity_type.as_str() {
      "playlist" => Some(Type::Playlist),
      "album" => Some(Type::Album),
      "artist" => Some(Type::Artist),
      "show" => Some(Type::Show),
      "episode" => Some(Type::Episode),
      _ => None,
    };
    kind.map(|_type| Context {
      uri: state.context.uri.clone(),
      href: String::new(),
      external_urls: Default::default(),
      _type,
    })
  };

  CurrentPlaybackContext {
    device: Device {
      id: None,
      is_active: state.is_active,
      is_private_session: false,
      is_restricted: false,
      name: device_name.to_owned(),
      _type: DeviceType::Speaker,
      volume_percent: Some(state.volume),
    },
    repeat_state: match state.options.repeat.as_str() {
      "track" => RepeatState::Track,
      "context" => RepeatState::Context,
      _ => RepeatState::Off,
    },
    shuffle_state: state.options.shuffle,
    smart_shuffle: false,
    context,
    timestamp: chrono::Utc::now(),
    progress: Some(chrono::Duration::milliseconds(state.position.position_ms)),
    is_playing: state.status == "playing" || state.status == "buffering",
    item: Some(PlayableItem::Track(track)),
    currently_playing_type: CurrentlyPlayingType::Track,
    actions: Actions { disallows: vec![] },
  }
}
