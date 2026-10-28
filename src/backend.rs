use crate::app::{
  ActiveBlock, AlbumTableContext, App, Artist, ArtistBlock, EpisodeTableContext, RouteId,
  ScrollableResultPages, SearchResultBlock, SelectedAlbum, SelectedFullAlbum, SelectedFullShow,
  SelectedShow, TrackSortColumn, TrackTableContext,
};
use crate::client_creds::ClientConfig;
use crate::library_cache::LibraryCache;
use crate::playlist_cache::{playlist_item_uri, PlaylistCache};
use crate::user_config::theme_presets;
use anyhow::anyhow;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ratatui::style::Color;
use rspotify::{
  clients::{BaseClient, OAuthClient},
  model::{
    AdditionalType, AlbumId, AlbumType, ArtistId, EpisodeId, FullArtist, FullTrack, Id, LibraryId,
    Market, Offset, Page, PlayContextId, PlayableId, PlayableItem, PlaylistId, PlaylistItem,
    PrivateUser, RepeatState, SavedAlbum, SavedTrack, SearchResult, SearchType, Show, ShowId,
    SimplifiedAlbum, SimplifiedPlaylist, SimplifiedShow, Token, TrackId, UserId,
  },
  AuthCodePkceSpotify, Config, Credentials, OAuth,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use std::{
  collections::{HashMap, HashSet},
  fs,
  path::PathBuf,
  sync::Arc,
  sync::OnceLock,
  time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;

/// Parse a context (spotify:album/artist/playlist/show:...) URI into a [`PlayContextId`].
fn play_context_from_uri<'a>(uri: &'a str) -> Option<PlayContextId<'a>> {
  let id = match () {
    _ if uri.starts_with("spotify:album:") => PlayContextId::Album(AlbumId::from_uri(uri).ok()?),
    _ if uri.starts_with("spotify:artist:") => PlayContextId::Artist(ArtistId::from_uri(uri).ok()?),
    _ if uri.starts_with("spotify:playlist:") => {
      PlayContextId::Playlist(PlaylistId::from_uri(uri).ok()?)
    }
    _ if uri.starts_with("spotify:show:") => PlayContextId::Show(ShowId::from_uri(uri).ok()?),
    _ => return None,
  };
  Some(id)
}

// TOTP/anon partner API removed for terms compliance — algorithmic playlists now via Web API only

// ponytail: partner API removed — below kept as stub for callers, returns error
#[cfg(test)]
/// Map a partner-API `itemV2.data` track object into an internal
/// [`FullTrack`] by reshaping it into the Web API shape rspotify expects. (removed)
fn partner_track_from_value(data: &serde_json::Value) -> anyhow::Result<Option<FullTrack>> {
  let uri = match data["uri"].as_str() {
    Some(u) if u.starts_with("spotify:track:") => u.to_string(),
    _ => return Ok(None),
  };
  let id = uri.trim_start_matches("spotify:track:").to_string();
  let name = data["name"].as_str().unwrap_or("").to_string();
  let explicit = data["contentRating"]["label"].as_str() == Some("EXPLICIT");
  let is_playable = data["playability"]["playable"].as_bool().unwrap_or(false);
  let duration_ms = data["trackDuration"]["totalMilliseconds"]
    .as_u64()
    .unwrap_or(0);
  let disc_number = data["discNumber"].as_i64().unwrap_or(1) as i32;
  let track_number = data["trackNumber"].as_u64().unwrap_or(1) as u32;

  let artists: Vec<serde_json::Value> = data["artists"]["items"]
    .as_array()
    .map(|items| {
      items
        .iter()
        .map(|a| {
          let auri = a["uri"].as_str().unwrap_or("");
          json!({
            "external_urls": {},
            "href": null,
            "id": auri.trim_start_matches("spotify:artist:"),
            "name": a["profile"]["name"].as_str().unwrap_or(""),
            "type": "artist",
            "uri": auri
          })
        })
        .collect()
    })
    .unwrap_or_default();

  let album = &data["albumOfTrack"];
  let album_uri = album["uri"].as_str().unwrap_or("");
  let album_id = album_uri.trim_start_matches("spotify:album:").to_string();
  let album_artists: Vec<serde_json::Value> = album["artists"]["items"]
    .as_array()
    .map(|items| {
      items
        .iter()
        .map(|a| {
          let auri = a["uri"].as_str().unwrap_or("");
          json!({
            "external_urls": {},
            "href": null,
            "id": auri.trim_start_matches("spotify:artist:"),
            "name": a["profile"]["name"].as_str().unwrap_or(""),
            "type": "artist",
            "uri": auri
          })
        })
        .collect()
    })
    .unwrap_or_default();
  let images: Vec<serde_json::Value> = album["coverArt"]["sources"]
    .as_array()
    .map(|sources| {
      sources
        .iter()
        .filter_map(|s| {
          s["url"].as_str().map(|url| {
            json!({
              "height": null,
              "url": url,
              "width": null
            })
          })
        })
        .collect()
    })
    .unwrap_or_default();

  let track_json = json!({
    "album": {
      "album_type": null,
      "artists": album_artists,
      "available_markets": [],
      "external_urls": {},
      "href": null,
      "id": album_id,
      "images": images,
      "name": album["name"].as_str().unwrap_or(""),
      "release_date": album["date"]["isoString"].as_str(),
      "release_date_precision": "day",
      "type": "album",
      "uri": album_uri
    },
    "artists": artists,
    "available_markets": [],
    "disc_number": disc_number,
    "duration_ms": duration_ms,
    "explicit": explicit,
    "external_ids": {},
    "external_urls": {},
    "href": null,
    "id": id,
    "is_local": false,
    "is_playable": is_playable,
    "name": name,
    "popularity": 0,
    "preview_url": null,
    "track_number": track_number,
    "type": "track",
    "uri": uri
  });
  serde_json::from_value(track_json)
    .map(Some)
    .map_err(|e| anyhow!("partner track mapping failed: {}", e))
}

/// Parse a track/episode URI into a [`PlayableId`].
fn playable_from_uri<'a>(uri: &'a str) -> Option<PlayableId<'a>> {
  if uri.starts_with("spotify:track:") {
    Some(PlayableId::Track(TrackId::from_uri(uri).ok()?))
  } else if uri.starts_with("spotify:episode:") {
    Some(PlayableId::Episode(EpisodeId::from_uri(uri).ok()?))
  } else {
    None
  }
}
/// Whether a playlist page holds `uri` (tracks and episodes alike).
fn playlist_page_contains(page: &Page<PlaylistItem>, uri: &str) -> bool {
  page
    .items
    .iter()
    .any(|item| playlist_item_uri(item).as_deref() == Some(uri))
}

/// (items_len, total, offset, limit) of a page, for paging decisions.
fn page_meta<T: serde::de::DeserializeOwned>(
  page: Option<&Page<T>>,
) -> Option<(usize, u32, u32, u32)> {
  page.map(|p| (p.items.len(), p.total, p.offset, p.limit))
}

/// Concatenate a freshly fetched page onto the previously loaded items,
/// skipping items whose key (id) is already present. Offset pagination can
/// overlap pages, and duplicate URIs in a play context are rejected by
/// Spotify, so the dedup keeps loaded rows playable too.
fn merge_page<T: serde::de::DeserializeOwned + Clone>(
  old: Option<&Page<T>>,
  new: Page<T>,
  offset: u32,
  total: u32,
  key: impl Fn(&T) -> String,
) -> Page<T> {
  let mut items = old.map(|p| p.items.clone()).unwrap_or_default();
  let mut seen = items
    .iter()
    .map(&key)
    .collect::<std::collections::HashSet<_>>();
  for item in new.items {
    if seen.insert(key(&item)) {
      items.push(item);
    }
  }
  Page {
    href: new.href,
    limit: new.limit,
    next: new.next,
    offset,
    previous: new.previous,
    total,
    items,
  }
}

#[derive(Debug, PartialEq)]
pub enum IoEvent {
  GetCurrentPlayback,
  RefreshAuthentication,
  GetPlaylists,
  GetDevices,
  GetSearchResults(String),
  GetMoreSearchResults(SearchResultBlock),
  SetTracksToTable(Vec<FullTrack>),
  GetMadeForYouPlaylistItems(String, u32),
  GetPlaylistItems(String, u32),
  LoadAllPlaylistItems(String),
  ReconcilePlaylistTracks(String),
  GetCurrentSavedTracks(Option<u32>),
  StartPlayback(Option<String>, Option<Vec<String>>, Option<usize>),
  StartPlaybackAt(Option<String>, Option<String>),
  UpdateSearchLimits(u32, u32),
  Seek(u32),
  NextTrack,
  PreviousTrack,
  Shuffle(bool),
  Repeat(RepeatState),
  PausePlayback,
  ChangeVolume(u8),
  GetArtist(String, String),
  GetAlbumTracks(Box<SimplifiedAlbum>),
  GetAlbumTracksMore(String, u32),
  GetCurrentUserSavedAlbums(Option<u32>),
  CurrentUserSavedAlbumsContains(Vec<String>),
  CurrentUserSavedAlbumDelete(String),
  CurrentUserSavedAlbumAdd(String),
  UserUnfollowArtists(Vec<String>),
  UserFollowArtists(Vec<String>),
  UserFollowPlaylist(String, String, Option<bool>),
  UserUnfollowPlaylist(String, String),
  GetLyrics,
  GetTrackCredits(String),
  GetQueue,
  ToggleProcessSuspend,
  ResumeLastTrack(String),
  GetArtistAlbumsMore(String, u32),
  GetArtistSinglesMore(String, u32),
  GetArtistAppearsOnMore(String, u32),
  GetArtistTopTracksMore(String, String, u32),
  GetUser,
  RefreshUser,
  ToggleSaveTrack(String),
  GetRecentlyPlayed,
  GetMoreRecentlyPlayed(Option<String>),
  GetFollowedArtists(Option<String>),
  SetArtistsToTable(Vec<FullArtist>),
  UserArtistFollowCheck(Vec<String>),
  GetAlbum(String),
  TransferPlaybackToDevice(String),
  TransferPlaybackToDeviceName(String),
  SoloistWsWatch {
    data_dir: PathBuf,
    device_name: String,
  },
  GetAlbumForTrack(String),
  CurrentUserSavedTracksContains(Vec<String>),
  GetCurrentUserSavedShows(Option<u32>),
  CurrentUserSavedShowsContains(Vec<String>),
  CurrentUserSavedShowDelete(String),
  CurrentUserSavedShowAdd(String),
  GetShowEpisodes(Box<SimplifiedShow>),
  GetShow(String),
  GetCurrentShowEpisodes(String, Option<u32>),
  AddItemToQueue(String),
  AddTrackToPlaylist(String, String),
  /// Create a private playlist, then add the track to it (name, track uri).
  CreatePlaylistThenAdd(String, String),
  RemoveTrackFromPlaylist(String, String),
  CleanCache,
  RefreshPlaylists,
  RefreshSavedTracks,
  RefreshSavedAlbums,
  RefreshSavedShows,
  RefreshPlaylistTracks(String),
  /// Startup-only restore: saved volume/shuffle/repeat apply only when
  /// nothing is already playing; last page/sort always restores.
  AutoRestore(SavedState),
}

pub fn get_spotify(
  token: Token,
  client_config: &ClientConfig,
) -> (AuthCodePkceSpotify, SystemTime) {
  let token_expiry = token
    .expires_at
    .map(|t| SystemTime::from(t - chrono::Duration::try_seconds(10).unwrap()))
    .unwrap_or_else(SystemTime::now);

  let creds = Credentials::new_pkce(&client_config.client_id);
  // ponytail: Config::default would re-enable token_refreshing (library
  // auto_reauth drops the PKCE refresh_token) and point at a relative
  // ".spotify_token_cache.json". sptune owns refresh, so disable the
  // library's and keep the real cache path so write-back persists.
  let cache_path = client_config
    .get_or_build_paths()
    .map(|p| p.token_cache_path)
    .unwrap_or_else(|_| PathBuf::from(rspotify::DEFAULT_CACHE_PATH));
  let config = Config {
    cache_path,
    token_cached: true,
    token_refreshing: false,
    ..Default::default()
  };
  let spotify = AuthCodePkceSpotify::from_token_with_config(token, creds, OAuth::default(), config);
  (spotify, token_expiry)
}

#[derive(Clone)]
pub struct Network<'a> {
  pub spotify: AuthCodePkceSpotify,
  large_search_limit: u32,
  pub client_config: ClientConfig,
  pub app: &'a Arc<Mutex<App>>,
  playlist_cache: PlaylistCache,
  library_cache: LibraryCache,
  saved_checked: HashMap<String, Instant>,
  tokens: f64,
  last_refill: Instant,
  api_backoff_until: Option<Instant>,
  // Bare id of the now-playing track the last heart check ran for; the 5s
  // playback poll only re-checks when the song actually changed.
  // When the last control reconcile ran. Rapid next/prev spam skips the
  // confirm GET if one ran <500ms ago; the periodic poll is the backstop.
  last_reconcile: Instant,
  last_heart_check: Option<String>,
  // Playlists probed by the once-per-launch background version check
  // (reconcile). In-memory only, so every sptune start re-checks.
  probed_playlists: HashSet<String>,
}

/// Last session's volume + track, persisted so a restart picks up where the
/// previous session left off.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedState {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub volume: Option<u8>,
  /// Track URI for Resume Last Listening (manual, no position — the seconds
  /// into the song are not tracked).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_played_track: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub mouse_enabled: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub theme_preset: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub shuffle: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub repeat: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub track_sort: Option<(String, bool)>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_page: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub seek_by_typing: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub restore_settings: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_library: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_playlists: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub sidebar_minimized: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub sidebar_width: Option<u16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub library_height: Option<u16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub volume_ramp_bar: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub black_background: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_album_column: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_artist_column: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_length_column: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_date_added_column: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub enable_add_to_playlist: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub enable_remove_from_playlist: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub max_display_length: Option<u16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub dev_view: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub enable_animations: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub playback_poll_secs: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub made_for_you_custom: Option<Vec<(String, String)>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub spotify_auto_launch: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub spotify_use_flags: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub spotify_suspend: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub spotify_trim_ws: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub spotify_mem_limit: Option<u32>,
}

impl SavedState {
  fn file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".config").join("sptune").join("state.json"))
  }

  /// Full snapshot for the single exit-time save: playback state (volume,
  /// shuffle, repeat, last track) plus every settings/UI field. Nothing is
  /// written to disk during the session — sptune saves once, on exit.
  pub fn snapshot_from_app(app: &App) -> SavedState {
    let mut volume = None;
    let mut shuffle = None;
    let mut repeat = None;
    let mut last_played_track = None;
    if let Some(context) = &app.current_playback_context {
      volume = context.device.volume_percent.map(|v| v as u8);
      shuffle = Some(context.shuffle_state);
      repeat = Some(
        match context.repeat_state {
          RepeatState::Off => "off",
          RepeatState::Context => "context",
          RepeatState::Track => "track",
        }
        .to_string(),
      );
      last_played_track = match &context.item {
        Some(PlayableItem::Track(track)) => track.id.as_ref().map(|id| id.uri()),
        _ => None,
      };
    }
    let track_sort = app
      .track_table_sort
      .map(|(column, desc)| (sort_column_name(column).to_string(), desc));
    let last_page = {
      let mut page = None;
      if app.get_current_route().id == RouteId::TrackTable
        && app.track_table.context == Some(TrackTableContext::MyPlaylists)
      {
        if let (Some(playlists), Some(index)) = (&app.playlists, app.selected_playlist_index) {
          if let Some(playlist) = playlists.items.get(index) {
            page = Some(format!("playlist:{}", playlist.id));
          }
        }
      }
      page
    };
    SavedState {
      volume,
      last_played_track,
      shuffle,
      repeat,
      track_sort,
      last_page,
      seek_by_typing: Some(app.user_config.behavior.seek_by_typing),
      restore_settings: Some(app.user_config.behavior.restore_settings),
      mouse_enabled: Some(app.user_config.behavior.enable_mouse),
      theme_preset: app
        .theme_preset_index
        .and_then(|i| theme_presets().get(i).map(|(name, _)| name.to_string())),
      show_library: Some(app.show_library),
      show_playlists: Some(app.show_playlists),
      sidebar_minimized: Some(app.sidebar_minimized),
      sidebar_width: app.sidebar_width_override,
      library_height: app.library_height_override,
      volume_ramp_bar: Some(app.user_config.behavior.volume_ramp_bar),
      black_background: Some(app.user_config.theme.background == Color::Rgb(0, 0, 0)),
      show_album_column: Some(app.user_config.behavior.show_album_column),
      show_artist_column: Some(app.user_config.behavior.show_artist_column),
      show_length_column: Some(app.user_config.behavior.show_length_column),
      show_date_added_column: Some(app.user_config.behavior.show_date_added_column),
      enable_add_to_playlist: Some(app.user_config.behavior.enable_add_to_playlist),
      enable_remove_from_playlist: Some(app.user_config.behavior.enable_remove_from_playlist),
      max_display_length: Some(app.user_config.behavior.max_display_length),
      dev_view: Some(app.dev_view),
      enable_animations: Some(app.user_config.behavior.enable_animations),
      playback_poll_secs: Some(app.user_config.behavior.playback_poll_secs),
      made_for_you_custom: Some(app.made_for_you_custom.clone()),
      spotify_auto_launch: Some(app.user_config.spotify.auto_launch),
      spotify_use_flags: Some(app.user_config.spotify.use_chromium_flags),
      spotify_suspend: Some(app.user_config.spotify.suspend_children),
      spotify_trim_ws: Some(app.user_config.spotify.trim_working_set),
      spotify_mem_limit: Some(app.user_config.spotify.memory_limit_mb),
    }
  }

  pub fn load() -> Option<SavedState> {
    let path = SavedState::file_path()?;
    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
  }

  pub fn save(&self) {
    let Some(path) = SavedState::file_path() else {
      return;
    };
    if let Some(dir) = path.parent() {
      let _ = fs::create_dir_all(dir);
    }
    // Merge instead of overwrite: concurrent instances (a stale process, two
    // terminals) each update only their own keys instead of erasing the other
    // session's fields and resetting the gear-menu settings on next launch.
    let mut root = fs::read_to_string(&path)
      .ok()
      .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
      .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = root.as_object_mut() {
      if let Some(value) = &self.volume {
        obj.insert("volume".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.last_played_track {
        obj.insert("last_played_track".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.mouse_enabled {
        obj.insert("mouse_enabled".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.theme_preset {
        obj.insert("theme_preset".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.shuffle {
        obj.insert("shuffle".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.repeat {
        obj.insert("repeat".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.track_sort {
        obj.insert("track_sort".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.last_page {
        obj.insert("last_page".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.seek_by_typing {
        obj.insert("seek_by_typing".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.restore_settings {
        obj.insert("restore_settings".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.show_library {
        obj.insert("show_library".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.show_playlists {
        obj.insert("show_playlists".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.sidebar_minimized {
        obj.insert("sidebar_minimized".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.sidebar_width {
        obj.insert("sidebar_width".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.library_height {
        obj.insert("library_height".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.volume_ramp_bar {
        obj.insert("volume_ramp_bar".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.black_background {
        obj.insert("black_background".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.show_album_column {
        obj.insert("show_album_column".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.show_artist_column {
        obj.insert("show_artist_column".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.show_length_column {
        obj.insert("show_length_column".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.show_date_added_column {
        obj.insert(
          "show_date_added_column".to_string(),
          serde_json::json!(value),
        );
      }
      if let Some(value) = &self.enable_remove_from_playlist {
        obj.insert(
          "enable_remove_from_playlist".to_string(),
          serde_json::json!(value),
        );
      }
      if let Some(value) = &self.max_display_length {
        obj.insert("max_display_length".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.dev_view {
        obj.insert("dev_view".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.enable_animations {
        obj.insert("enable_animations".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.made_for_you_custom {
        obj.insert("made_for_you_custom".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.spotify_auto_launch {
        obj.insert("spotify_auto_launch".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.spotify_use_flags {
        obj.insert("spotify_use_flags".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.spotify_suspend {
        obj.insert("spotify_suspend".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.spotify_trim_ws {
        obj.insert("spotify_trim_ws".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.spotify_mem_limit {
        obj.insert("spotify_mem_limit".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.enable_add_to_playlist {
        obj.insert(
          "enable_add_to_playlist".to_string(),
          serde_json::json!(value),
        );
      }
      // dedup: earlier inserts already covered these; keep one copy above spotify
    }
    let _ = fs::write(path, root.to_string());
  }
}

fn sort_column_name(column: TrackSortColumn) -> &'static str {
  match column {
    TrackSortColumn::Title => "title",
    TrackSortColumn::Artist => "artist",
    TrackSortColumn::Album => "album",
    TrackSortColumn::Length => "length",
    TrackSortColumn::DateAdded => "date_added",
  }
}

fn sort_column_from_name(name: &str) -> Option<TrackSortColumn> {
  match name {
    "title" => Some(TrackSortColumn::Title),
    "artist" => Some(TrackSortColumn::Artist),
    "album" => Some(TrackSortColumn::Album),
    "length" => Some(TrackSortColumn::Length),
    "date_added" => Some(TrackSortColumn::DateAdded),
    _ => None,
  }
}

// Global token bucket: at most API_BURST requests can fire back-to-back,
// then the bucket refills at API_RATE_PER_SEC tokens/second.
// 6/s ≈ the pre-bucket fixed 150ms pace (bcfd9cb) — fast enough for
// instant cached playlist opens, slow enough to avoid 429s.
pub const API_RATE_PER_SEC: f64 = 6.0;
pub const API_BURST: f64 = 10.0;
const SAVED_CHECK_TTL: Duration = Duration::from_secs(300);
// Max page size Spotify allows on paged endpoints (playlist items, saved
// tracks/albums/shows, etc.). One request should always ask for it.
//
// Shared HTTP client: one connection pool for all ad-hoc Web API calls
// (oEmbed, lyrics, credits) so repeat requests skip TCP+TLS handshakes.
static SHARED_HTTP: OnceLock<reqwest::Client> = OnceLock::new();

pub(crate) fn shared_http_client() -> reqwest::Client {
  SHARED_HTTP
    .get_or_init(|| {
      reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client with timeout builds")
    })
    .clone()
}

pub const API_MAX_LIMIT: u32 = 50;
// Search is the exception: Spotify cut its limit cap to 10 (Feb 2026).
const SEARCH_MAX_LIMIT: u32 = 10;

/// Merge live Smart Shuffle state with the local flag. API-true sticks
/// (Smart enabled on another client lights up here); a locally-enabled
/// Smart survives while shuffle stays on; shuffle-off clears both, so the
/// label can never claim Smart for shuffle that is actually off.
fn merge_smart_shuffle(api_smart: bool, local_smart: bool, shuffle_on: bool) -> bool {
  api_smart || (local_smart && shuffle_on)
}

/// Merge a `library_contains` answer into the heart set. Returns true when
/// the set changed (insert or removal) so the caller can repaint.
fn apply_saved_flags(set: &mut HashSet<String>, ids: &[String], is_saved: &[bool]) -> bool {
  let mut changed = false;
  for (i, id) in ids.iter().enumerate() {
    match is_saved.get(i) {
      Some(true) => {
        changed |= set.insert(id.clone());
      }
      // The song is not liked, so check if it should be removed.
      Some(false) => {
        changed |= set.remove(id);
      }
      None => {}
    }
  }
  changed
}

impl<'a> Network<'a> {
  pub fn new(
    spotify: AuthCodePkceSpotify,
    client_config: ClientConfig,
    app: &'a Arc<Mutex<App>>,
  ) -> Self {
    Network {
      spotify,
      large_search_limit: API_MAX_LIMIT,
      client_config,
      app,
      playlist_cache: PlaylistCache::new(),
      library_cache: LibraryCache::new(),
      saved_checked: HashMap::new(),
      tokens: API_BURST,
      last_refill: Instant::now(),
      api_backoff_until: None,
      last_reconcile: Instant::now(),
      last_heart_check: None,
      probed_playlists: HashSet::new(),
    }
  }

  /// Throttle outbound API calls so bursts (opening playlists, search tab
  /// expands) stay under Spotify's rate limit instead of tripping 429s.
  async fn pace(&mut self) {
    if let Some(until) = self.api_backoff_until {
      if let Some(remaining) = until.checked_duration_since(Instant::now()) {
        tokio::time::sleep(remaining).await;
      }
      self.api_backoff_until = None;
    }
    let now = Instant::now();
    self.tokens = (self.tokens
      + now.duration_since(self.last_refill).as_secs_f64() * API_RATE_PER_SEC)
      .clamp(0.0, API_BURST);
    self.last_refill = now;
    if self.tokens < 1.0 {
      let wait = Duration::from_secs_f64((1.0 - self.tokens) / API_RATE_PER_SEC);
      tokio::time::sleep(wait).await;
      self.tokens = 0.0;
      self.last_refill = Instant::now();
    } else {
      self.tokens -= 1.0;
    }
  }

  pub async fn handle_network_event(&mut self, io_event: IoEvent) {
    let debug = format!("{:?}", io_event);
    let text = match debug.find('(') {
      Some(pos) => debug[..pos].to_string(),
      None => debug,
    };
    // Cached opens must not wait for the token bucket — serve immediately
    // and only pace the background reconcile. Otherwise a fully-cached
    // playlist open pays 200-500ms for no network work.
    let skip_pace = match &io_event {
      IoEvent::GetPlaylistItems(pid, 0) => {
        self.playlist_cache.ensure_loaded();
        self
          .playlist_cache
          .lookup(pid)
          .map(|e| !e.items.is_empty() && !self.playlist_cache.is_polluted(pid))
          .unwrap_or(false)
      }
      IoEvent::GetMadeForYouPlaylistItems(pid, 0) => {
        self.playlist_cache.ensure_loaded();
        self
          .playlist_cache
          .lookup(pid)
          .map(|e| !e.items.is_empty() && !self.playlist_cache.is_polluted(pid))
          .unwrap_or(false)
      }
      IoEvent::GetCurrentSavedTracks(None)
      | IoEvent::GetCurrentUserSavedAlbums(None)
      | IoEvent::GetCurrentUserSavedShows(None) => {
        self.library_cache.ensure_loaded();
        match &io_event {
          IoEvent::GetCurrentSavedTracks(None) => {
            self.library_cache.get("saved_tracks").is_some()
          }
          IoEvent::GetCurrentUserSavedAlbums(None) => {
            self.library_cache.get("saved_albums").is_some()
          }
          IoEvent::GetCurrentUserSavedShows(None) => {
            self.library_cache.get("saved_shows").is_some()
          }
          _ => false,
        }
      }
      IoEvent::GetPlaylists => {
        self.library_cache.ensure_loaded();
        self.library_cache.get("playlists").is_some()
      }
      IoEvent::GetUser => {
        self.library_cache.ensure_loaded();
        self
          .library_cache
          .get_typed::<PrivateUser>("profile")
          .is_some()
      }
      IoEvent::SetTracksToTable(_)
      | IoEvent::SetArtistsToTable(_)
      | IoEvent::UpdateSearchLimits(_, _)
      | IoEvent::CleanCache
      // Latency-sensitive single user actions: same token cost, no bucket wait.
      | IoEvent::Repeat(_)
      | IoEvent::Shuffle(_) => true,
      _ => false,
    };
    if !skip_pace {
      self.app.lock().await.log_request(text);
    }
    if skip_pace {
      // keep token state fresh for the panel without sleeping
      let now = Instant::now();
      self.tokens = (self.tokens
        + now.duration_since(self.last_refill).as_secs_f64() * API_RATE_PER_SEC)
        .clamp(0.0, API_BURST);
      self.last_refill = now;
    } else {
      self.pace().await;
    }
    // Expose rate limiter state for the dev panel.
    {
      let mut app = self.app.lock().await;
      app.api_tokens = self.tokens;
      app.api_tokens_updated = Some(Instant::now());
      app.api_backoff_until = self.api_backoff_until;
    }
    match io_event {
      IoEvent::RefreshAuthentication => {
        self.refresh_authentication().await;
      }
      IoEvent::GetPlaylists => {
        self.get_current_user_playlists().await;
      }
      IoEvent::GetUser => {
        self.get_user().await;
      }
      IoEvent::RefreshUser => {
        self.refresh_user().await;
      }
      IoEvent::GetDevices => {
        self.get_devices().await;
      }
      IoEvent::GetCurrentPlayback => {
        self.get_current_playback().await;
      }
      IoEvent::SetTracksToTable(full_tracks) => {
        let count = full_tracks.len();
        self
          .set_tracks_to_table(full_tracks, vec![None; count], (0..count).collect(), false)
          .await;
      }
      IoEvent::GetSearchResults(search_term) => {
        self.get_search_results(search_term).await;
      }
      IoEvent::GetMoreSearchResults(block) => {
        self.get_more_search_results(block).await;
      }
      IoEvent::GetMadeForYouPlaylistItems(playlist_id, made_for_you_offset) => {
        self
          .get_made_for_you_playlist_tracks(playlist_id, made_for_you_offset)
          .await;
      }
      IoEvent::GetPlaylistItems(playlist_id, playlist_offset) => {
        self.get_playlist_tracks(playlist_id, playlist_offset).await;
      }
      IoEvent::LoadAllPlaylistItems(playlist_id) => {
        self.load_all_playlist_items(playlist_id).await;
      }
      IoEvent::ReconcilePlaylistTracks(playlist_id) => {
        if self.reconcile_playlist_tracks(&playlist_id).await {
          self.serve_playlist_cache(&playlist_id).await;
          let mut app = self.app.lock().await;
          // Keep the For-you mirror in sync with the re-served cache.
          if app.track_table.context == Some(TrackTableContext::MadeForYou) {
            if let Some(page) = app.playlist_tracks.clone() {
              app.made_for_you_tracks = Some(page);
            }
          }
          // Re-serving replaces the table and resets the sort (set_tracks_to_table
          // replace sets track_table_sort = None); re-apply the user's column sort
          // so the corrected list doesn't flip back to raw order under them.
          if let Some((sort_column, desc)) = app.track_table_sort {
            app.track_table_sort = Some((sort_column, desc));
            if sort_column == TrackSortColumn::DateAdded {
              app.materialize_date_added();
            } else {
              app.sort_tracks();
            }
          }
        }
      }
      IoEvent::AutoRestore(saved) => {
        // Startup sends GetCurrentPlayback first, so the live state is
        // usually already known — skip the redundant fetch and read the
        // context directly. Only fetch when nothing has landed yet.
        let known = {
          let app = self.app.lock().await;
          app.playback_known
        };
        if !known {
          self.get_current_playback().await;
        }
        let idle = {
          let app = self.app.lock().await;
          app
            .current_playback_context
            .as_ref()
            .and_then(|c| c.item.as_ref())
            .is_none()
        };
        if idle {
          if let Some(volume) = saved.volume {
            self.change_volume(volume).await;
          }
          self.restore_saved_playback_settings(&saved).await;
        }
        self.restore_saved_local_view(&saved).await;
      }
      IoEvent::GetCurrentSavedTracks(offset) => {
        self.get_current_user_saved_tracks(offset).await;
      }
      IoEvent::StartPlayback(context_uri, uris, offset) => {
        self.start_playback(context_uri, uris, offset).await;
      }
      IoEvent::StartPlaybackAt(context_uri, track_uri) => {
        self.start_playback_at(context_uri, track_uri).await;
      }
      IoEvent::UpdateSearchLimits(large_search_limit, _) => {
        self.large_search_limit = large_search_limit.min(API_MAX_LIMIT);
        let mut app = self.app.lock().await;
        app.large_search_limit = self.large_search_limit;
      }
      IoEvent::Seek(position_ms) => {
        self.seek(position_ms).await;
      }
      IoEvent::NextTrack => {
        self.next_track().await;
      }
      IoEvent::PreviousTrack => {
        self.previous_track().await;
      }
      IoEvent::Repeat(repeat_state) => {
        self.repeat(repeat_state).await;
      }
      IoEvent::PausePlayback => {
        self.pause_playback().await;
      }
      IoEvent::ChangeVolume(volume) => {
        self.change_volume(volume).await;
      }
      IoEvent::GetArtist(artist_id, input_artist_name) => {
        self.get_artist(artist_id, input_artist_name).await;
      }
      IoEvent::GetAlbumTracks(album) => {
        self.get_album_tracks(album).await;
      }
      IoEvent::GetAlbumTracksMore(album_id, offset) => {
        self.get_album_tracks_more(album_id, offset).await;
      }
      IoEvent::GetCurrentUserSavedAlbums(offset) => {
        self.get_current_user_saved_albums(offset).await;
      }
      IoEvent::CurrentUserSavedAlbumsContains(album_ids) => {
        self.current_user_saved_albums_contains(album_ids).await;
      }
      IoEvent::CurrentUserSavedAlbumDelete(album_id) => {
        self.current_user_saved_album_delete(album_id).await;
      }
      IoEvent::CurrentUserSavedAlbumAdd(album_id) => {
        self.current_user_saved_album_add(album_id).await;
      }
      IoEvent::UserUnfollowArtists(artist_ids) => {
        self.user_unfollow_artists(artist_ids).await;
      }
      IoEvent::UserFollowArtists(artist_ids) => {
        self.user_follow_artists(artist_ids).await;
      }
      IoEvent::UserFollowPlaylist(playlist_owner_id, playlist_id, is_public) => {
        self
          .user_follow_playlist(playlist_owner_id, playlist_id, is_public)
          .await;
      }
      IoEvent::UserUnfollowPlaylist(user_id, playlist_id) => {
        self.user_unfollow_playlist(user_id, playlist_id).await;
      }
      IoEvent::GetLyrics => {
        self.get_lyrics().await;
      }
      IoEvent::GetTrackCredits(track_id) => {
        self.get_track_credits(track_id).await;
      }
      IoEvent::GetQueue => {
        self.get_queue().await;
      }
      IoEvent::ResumeLastTrack(uri) => {
        self.resume_last_track(uri).await;
      }
      IoEvent::ToggleProcessSuspend => {
        let suspended = { self.app.lock().await.processes_suspended };
        match crate::platform::spotify::find_root_pid() {
          Some(root) => {
            if suspended {
              crate::platform::spotify::suspend_processes(root);
            } else {
              crate::platform::spotify::resume_processes(root);
            }
          }
          None => {
            let mut app = self.app.lock().await;
            app.processes_suspended = false;
            app.api_error = "Spotify process not found".to_string();
          }
        }
      }
      IoEvent::GetArtistAlbumsMore(artist_id, offset) => {
        self.get_artist_albums_more(artist_id, offset).await;
      }
      IoEvent::GetArtistSinglesMore(artist_id, offset) => {
        self.get_artist_singles_more(artist_id, offset).await;
      }
      IoEvent::GetArtistAppearsOnMore(artist_id, offset) => {
        self.get_artist_appears_on_more(artist_id, offset).await;
      }
      IoEvent::GetArtistTopTracksMore(artist_id, artist_name, offset) => {
        self
          .get_artist_top_tracks_more(artist_id, artist_name, offset)
          .await;
      }
      IoEvent::ToggleSaveTrack(track_id) => {
        self.toggle_save_track(track_id).await;
      }
      IoEvent::GetRecentlyPlayed => {
        self.get_recently_played().await;
      }
      IoEvent::GetMoreRecentlyPlayed(before) => {
        self.get_more_recently_played(before).await;
      }
      IoEvent::GetFollowedArtists(after) => {
        self.get_followed_artists(after).await;
      }
      IoEvent::SetArtistsToTable(full_artists) => {
        self.set_artists_to_table(full_artists).await;
      }
      IoEvent::UserArtistFollowCheck(artist_ids) => {
        self.user_artist_check_follow(artist_ids).await;
      }
      IoEvent::GetAlbum(album_id) => {
        self.get_album(album_id).await;
      }
      IoEvent::TransferPlaybackToDevice(device_id) => {
        self.transfert_playback_to_device(device_id).await;
      }
      IoEvent::TransferPlaybackToDeviceName(name) => {
        // Background, never inline: the network loop is sequential and this
        // polls up to 10x3s — inline it stalls every queued event, so a
        // playlist opened at startup sits behind it as a black panel.
        // Found id re-enters the loop as TransferPlaybackToDevice (quick).
        let spotify = self.spotify.clone();
        let app = std::sync::Arc::clone(self.app);
        tokio::spawn(async move {
          for _ in 0..10 {
            if let Ok(devices) = spotify.device().await {
              if let Some(d) = devices.iter().find(|d| d.name.eq_ignore_ascii_case(&name)) {
                match d.id.clone() {
                  Some(id) => {
                    app
                      .lock()
                      .await
                      .dispatch(IoEvent::TransferPlaybackToDevice(id));
                  }
                  None => { /* nameless device: nothing to transfer to */ }
                }
                return;
              }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
          }
          app.lock().await.handle_error(anyhow!(
            "device '{}' not found — on first soloist run, pair it once from the Spotify desktop or mobile app (after that it persists)",
            name
          ));
        });
      }
      IoEvent::SoloistWsWatch {
        data_dir,
        device_name,
      } => {
        let app = std::sync::Arc::clone(self.app);
        tokio::spawn(async move {
          crate::soloist_ws::watch(app, data_dir, device_name).await;
        });
      }
      IoEvent::GetAlbumForTrack(track_id) => {
        self.get_album_for_track(track_id).await;
      }
      IoEvent::Shuffle(shuffle_state) => {
        self.shuffle(shuffle_state).await;
      }
      IoEvent::CurrentUserSavedTracksContains(track_ids) => {
        self.current_user_saved_tracks_contains(track_ids).await;
      }
      IoEvent::GetCurrentUserSavedShows(offset) => {
        self.get_current_user_saved_shows(offset).await;
      }
      IoEvent::CurrentUserSavedShowsContains(show_ids) => {
        self.current_user_saved_shows_contains(show_ids).await;
      }
      IoEvent::CurrentUserSavedShowDelete(show_id) => {
        self.current_user_saved_shows_delete(show_id).await;
      }
      IoEvent::CurrentUserSavedShowAdd(show_id) => {
        self.current_user_saved_shows_add(show_id).await;
      }
      IoEvent::GetShowEpisodes(show) => {
        self.get_show_episodes(show).await;
      }
      IoEvent::GetShow(show_id) => {
        self.get_show(show_id).await;
      }
      IoEvent::GetCurrentShowEpisodes(show_id, offset) => {
        self.get_current_show_episodes(show_id, offset).await;
      }
      IoEvent::AddItemToQueue(item) => {
        self.add_item_to_queue(item).await;
      }
      IoEvent::AddTrackToPlaylist(uri, playlist_id) => {
        self.add_track_to_playlist(uri, playlist_id).await;
      }
      IoEvent::CreatePlaylistThenAdd(name, uri) => {
        self.create_playlist_then_add(name, uri).await;
      }
      IoEvent::RemoveTrackFromPlaylist(uri, playlist_id) => {
        self.remove_track_from_playlist(uri, playlist_id).await;
      }
      IoEvent::CleanCache => {
        self.library_cache.clear();
        self.playlist_cache.clear();
        let mut app = self.app.lock().await;
        app.playlist_uri_map.clear();
      }
      IoEvent::RefreshPlaylists => {
        self.refresh_playlists().await;
      }
      IoEvent::RefreshSavedTracks => {
        self.refresh_saved_tracks().await;
      }
      IoEvent::RefreshSavedAlbums => {
        self.refresh_saved_albums().await;
      }
      IoEvent::RefreshSavedShows => {
        self.refresh_saved_shows().await;
      }
      IoEvent::RefreshPlaylistTracks(playlist_id) => {
        self.refresh_playlist_tracks(&playlist_id).await;
      }
    };

    let mut app = self.app.lock().await;
    app.is_loading = false;
  }

  fn parse_retry_after(msg: &str) -> Option<u64> {
    // ponytail: minimal Retry-After parsing from rspotify error string; full header parsing would need ClientError access
    msg.find("Retry-After").and_then(|i| {
      msg[i..]
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
    })
  }

  async fn handle_error(&mut self, e: anyhow::Error) {
    let message = e.to_string();
    // Full OpenAPI codes: 401→refresh, 429→backoff with Retry-After, others surface via toast
    if message.contains("401") || message.contains("Unauthorized") {
      // ponytail: immediate refresh on 401, don't wait for SystemTime tick
      let mut app = self.app.lock().await;
      if !app.auth_reauth_required {
        app.dispatch(crate::backend::IoEvent::RefreshAuthentication);
      }
      app.handle_error(e);
      return;
    }
    if message.contains("429") || message.contains("Too Many Requests") {
      let secs = Self::parse_retry_after(&message).unwrap_or(30);
      // ponytail: 200ms jitter avoids thundering herd after shared Retry-After window
      self.api_backoff_until =
        Some(Instant::now() + Duration::from_secs(secs) + Duration::from_millis(200));
    }
    let mut app = self.app.lock().await;
    app.handle_error(e);
  }

  async fn get_user(&mut self) {
    let cached = self
      .library_cache
      .get_typed::<PrivateUser>("profile")
      .and_then(|(users, _)| users.into_iter().next());
    // ponytail: cached profile is served as-is on startup; refresh is explicit now
    if cached.is_some() {
      self.app.lock().await.user = cached;
      return;
    }
    match self.spotify.current_user().await {
      Ok(user) => {
        self
          .library_cache
          .put("profile", std::slice::from_ref(&user), 1);
        let mut app = self.app.lock().await;
        app.user = Some(user);
      }
      Err(e) => self.handle_error(anyhow!(e)).await,
    }
  }

  async fn refresh_user(&mut self) {
    // ponytail: profile is a single object — clear the key and reuse the cold path
    self.library_cache.remove("profile");
    self.get_user().await;
  }

  async fn get_devices(&mut self) {
    if let Ok(result) = self.spotify.device().await {
      let mut app = self.app.lock().await;
      app.push_navigation_stack(RouteId::SelectedDevice, ActiveBlock::SelectDevice);
      if !result.is_empty() {
        app.devices = Some(result);
        // Select the first device in the list
        app.selected_device_index = Some(0);
      }
    }
  }

  async fn get_current_playback(&mut self) {
    let context = self
      .spotify
      .current_playback(
        None,
        Some([AdditionalType::Episode, AdditionalType::Track].iter()),
      )
      .await;

    match context {
      Ok(Some(c)) => {
        let mut app = self.app.lock().await;
        // OSC 7501: emit only on play/pause flips (polls run every few
        // seconds; the toggle path already emitted the user's own presses).
        let was_playing = app
          .current_playback_context
          .as_ref()
          .is_some_and(|ctx| ctx.is_playing);
        app.current_playback_context = Some(c.clone());
        if was_playing != c.is_playing {
          crate::app::emit_program_status(c.is_playing, &crate::app::now_playing_title(Some(&c)));
        }
        app.playback_known = true;
        // Sync the real Smart Shuffle flag (undocumented API field); the
        // merge keeps a locally-enabled Smart while shuffle stays on.
        app.smart_shuffle =
          merge_smart_shuffle(c.smart_shuffle, app.smart_shuffle, c.shuffle_state);
        app.instant_since_last_current_playback_poll = Instant::now();
        // Prime the playbar heart when the song changed (TTL-cached, silent).
        let now_playing = c.item.as_ref().and_then(|item| match item {
          rspotify::model::PlayableItem::Track(t) => t.id.as_ref().map(|id| id.to_string()),
          _ => None,
        });
        if now_playing.as_deref() != self.last_heart_check.as_deref() {
          self.last_heart_check = now_playing.clone();
          // Song changed: repaint instantly instead of waiting for the tick.
          app.ui_dirty = true;
          if let Some(id) = now_playing {
            app.dispatch(IoEvent::CurrentUserSavedTracksContains(vec![id]));
          }
        }
      }
      Ok(None) => {
        let mut app = self.app.lock().await;
        app.playback_known = true;
        app.instant_since_last_current_playback_poll = Instant::now();
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }

    let mut app = self.app.lock().await;
    app.seek_ms.take();
    app.is_fetching_current_playback = false;
  }

  async fn current_user_saved_tracks_contains(&mut self, ids: Vec<String>) {
    let now = Instant::now();
    self
      .saved_checked
      .retain(|_, checked_at| now.duration_since(*checked_at) < SAVED_CHECK_TTL);
    let fresh: Vec<String> = ids
      .into_iter()
      .filter(|id| !self.saved_checked.contains_key(id))
      .collect();
    if fresh.is_empty() {
      return;
    }
    let library_ids = fresh
      .iter()
      .map(|id| LibraryId::Track(TrackId::from_id_or_uri(id).unwrap()));
    // Best-effort: the heart is cosmetic, and Spotify throttles this
    // endpoint hard (503s under load). Leave the TTL untouched so the
    // next playlist open retries instead of showing a dead error screen.
    if let Ok(is_saved_vec) = self.spotify.library_contains(library_ids).await {
      let mut app = self.app.lock().await;
      // Repaint when the heart set actually changes: the main loop skips
      // redraws while idle, so without this the late heart data sits in
      // memory invisible until the next interaction.
      if apply_saved_flags(&mut app.liked_song_ids_set, &fresh, &is_saved_vec) {
        app.ui_dirty = true;
      }
      for id in &fresh {
        self.saved_checked.insert(id.clone(), now);
      }
    }
  }

  /// Manual refresh (from a refresh control), never on screen open: probe for
  /// Manual refresh: always do a full refetch bypassing the snapshot/tail
  /// probe. The reconcile path is for background opens, not user-requested
  /// refresh.
  async fn refresh_playlist_tracks(&mut self, playlist_id: &str) {
    self.playlist_cache.remove(playlist_id);
    self.get_playlist_tracks(playlist_id.to_string(), 0).await;
  }

  /// Refresh one of the cached library lists. Opening a screen serves the
  /// cache as-is (zero requests); the delta probe runs only here.
  async fn refresh_saved_tracks(&mut self) {
    self.library_cache.ensure_loaded();
    if self.library_cache.get("saved_tracks").is_some() {
      self.reconcile_saved_tracks().await;
      self.serve_saved_tracks_cache().await;
    } else {
      self.get_current_user_saved_tracks(None).await;
    }
  }

  async fn refresh_saved_albums(&mut self) {
    self.library_cache.ensure_loaded();
    if self.library_cache.get("saved_albums").is_some() {
      self.reconcile_saved_albums().await;
      self.serve_saved_albums_cache().await;
    } else {
      self.get_current_user_saved_albums(None).await;
    }
  }

  async fn refresh_saved_shows(&mut self) {
    self.library_cache.ensure_loaded();
    if self.library_cache.get("saved_shows").is_some() {
      self.reconcile_saved_shows().await;
      self.serve_saved_shows_cache().await;
    } else {
      self.get_current_user_saved_shows(None).await;
    }
  }

  async fn refresh_playlists(&mut self) {
    self.refetch_playlists().await;
    // refetch writes file; serve must read it and force in-memory + UI cache refresh
    self.library_cache.ensure_loaded();
    if let Some((items, total)) = self
      .library_cache
      .get_typed::<SimplifiedPlaylist>("playlists")
    {
      let page = Page::<SimplifiedPlaylist> {
        href: String::new(),
        limit: 0,
        next: None,
        offset: 0,
        previous: None,
        total,
        items,
      };
      let mut app = self.app.lock().await;
      app.playlists = Some(page);
      // bust tui.rs PLAYLIST_ITEMS thread-local (ptr, len) so next frame rebuilds strings
      crate::tui::bust_playlist_items_cache();
    }
  }

  // A playlist pasted into the search box is added to For you. Must run after
  // the track table is populated: set_tracks_to_table clears playlist_view.
  async fn consume_pending_for_you_add(&mut self, playlist_id: &str, name: Option<String>) {
    if self.app.lock().await.pending_for_you_add.as_deref() != Some(playlist_id) {
      return;
    }
    let name = match name {
      Some(name) => name,
      None => {
        let public = self
          .spotify
          .playlist(
            PlaylistId::from_id_or_uri(playlist_id).unwrap(),
            None,
            Some(Market::FromToken),
          )
          .await
          .map(|playlist| playlist.name)
          .unwrap_or_default();
        if public.is_empty() {
          // ponytail: partner API removed — fallback to oEmbed (no auth) for name, else id
          Self::oembed_playlist_name(playlist_id)
            .await
            .unwrap_or_default()
        } else {
          public
        }
      }
    };
    let mut app = self.app.lock().await;
    app.pending_for_you_add = None;
    // A pasted playlist is not a list view, so drop any stale context (a
    // leftover MyPlaylists/SavedTracks/MadeForYou would turn on the "Date
    // added" column).
    app.track_table.context = None;
    app.playlist_view = Some((name.clone(), playlist_id.to_string()));
    app.add_pasted_playlist_to_for_you(name, playlist_id.to_string());
  }

  async fn get_playlist_tracks(&mut self, playlist_id: String, playlist_offset: u32) {
    if playlist_offset == 0 {
      self.playlist_cache.ensure_loaded();
      if self.playlist_cache.lookup(&playlist_id).is_some()
        && self.serve_playlist_cache(&playlist_id).await
      {
        // The cache is served as-is. A once-per-launch background version
        // check appends newly added songs; anything else waits for the
        // manual refresh button.
        self
          .app
          .lock()
          .await
          .dispatch(IoEvent::ReconcilePlaylistTracks(playlist_id.clone()));
        self.consume_pending_for_you_add(&playlist_id, None).await;
        return;
      }
    }
    if playlist_offset > 0 {
      // The offset is snapshotted at dispatch time (load_more_tracks), so two
      // queued events can carry the same offset. Skip a page whose range is
      // already covered instead of appending it twice.
      let covered = {
        let app = self.app.lock().await;
        app
          .playlist_tracks
          .as_ref()
          .map(|existing| playlist_offset < existing.items.len() as u32)
          .unwrap_or(false)
      };
      if covered {
        self.app.lock().await.is_fetching_next_page = false;
        return;
      }
    }
    match self
      .spotify
      .playlist_items_manual(
        PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
        None,
        Some(Market::FromToken),
        Some(API_MAX_LIMIT),
        Some(playlist_offset),
      )
      .await
    {
      Ok(playlist_tracks) => {
        self
          .apply_playlist_tracks(&playlist_id, playlist_tracks, playlist_offset)
          .await;
        if playlist_offset == 0 {
          self.consume_pending_for_you_add(&playlist_id, None).await;
          // First fetch caches only page 1: without a background fill the
          // entry stays partial forever and every later open re-fires the
          // full LoadAll burst. Fill it now so the next open serves whole.
          let partial = {
            let app = self.app.lock().await;
            app
              .playlist_tracks
              .as_ref()
              .is_some_and(|p| (p.items.len() as u32) < p.total)
          };
          if partial {
            self
              .app
              .lock()
              .await
              .dispatch(IoEvent::LoadAllPlaylistItems(playlist_id.clone()));
          }
        }
      }
      Err(e) => {
        let hint = if e.to_string().contains("404") {
          " — Spotify's public Web API returns 404 for algorithmic playlists (Daily Mixes, Discover Weekly, etc.) even when the link opens in the Spotify web player; sptune uses the public API, which does not expose those track lists"
        } else {
          ""
        };
        self
          .handle_error(anyhow!(
            "failed to load tracks of playlist '{}': {}{}",
            playlist_id,
            e,
            hint
          ))
          .await;
        self.app.lock().await.is_fetching_next_page = false;
      }
    }
  }

  /// Share the successful playlist-page handling between the public API path
  /// and the partner-API fallback.
  async fn apply_playlist_tracks(
    &mut self,
    playlist_id: &str,
    playlist_tracks: Page<PlaylistItem>,
    playlist_offset: u32,
  ) {
    // View guard: a page belongs on screen only while its playlist owns the
    // table (or is the pending pasted playlist, which has no list context).
    // A stale in-flight page still updates the keyed cache, never the UI.
    // The background fill may also paint: it runs only for fill_target.
    let paint = {
      let app = self.app.lock().await;
      match app.track_table_playlist_uri() {
        Some(viewed) => viewed == playlist_id,
        None => app.pending_for_you_add.as_deref() == Some(playlist_id),
      }
    };
    let filling = {
      let app = self.app.lock().await;
      app.fill_target.as_deref() == Some(playlist_id)
    };
    if paint || (playlist_offset > 0 && filling) {
      self
        .set_playlist_tracks_to_table(&playlist_tracks, playlist_offset > 0)
        .await;
    }
    let mut app = self.app.lock().await;
    self.playlist_cache.update(
      playlist_id,
      playlist_tracks.items.clone(),
      playlist_tracks.total,
      playlist_offset,
      playlist_offset > 0,
    );
    self.sync_playlist_uris(&mut app);
    let touch = paint || filling;
    if playlist_offset > 0 {
      if touch {
        if let Some(existing) = app.playlist_tracks.as_mut() {
          // Positional convergence: a retried or double-dispatched page lands
          // on its own offset instead of appending a second copy.
          existing.items.truncate(playlist_offset as usize);
          existing.items.extend(playlist_tracks.items.clone());
          existing.total = playlist_tracks.total;
        }
      }
    } else if paint {
      app.playlist_tracks = Some(playlist_tracks);
      // A fresh first page (re)arms the background fill while partial; a
      // whole one, or a stale page for a playlist left behind, disarms it.
      let partial = app
        .playlist_tracks
        .as_ref()
        .is_some_and(|p| (p.items.len() as u32) < p.total);
      app.fill_target = partial.then(|| playlist_id.to_string());
    }
    app.is_fetching_next_page = false;
    if paint {
      app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
    }
  }

  async fn oembed_playlist_name(playlist_id: &str) -> anyhow::Result<String> {
    let url = format!(
      "https://open.spotify.com/oembed?url=https://open.spotify.com/playlist/{}",
      playlist_id
    );
    let resp: serde_json::Value = shared_http_client().get(&url).send().await?.json().await?;
    resp["title"]
      .as_str()
      .map(|s| s.to_string())
      .ok_or_else(|| anyhow!("no title in oembed"))
  }

  /// Complete the open playlist in the background (first-ever open only;
  /// cached opens serve as-is, zero requests). Stops early when another
  /// playlist takes over the table, so switching lists never queues behind
  /// a 500-track fill.
  async fn load_all_playlist_items(&mut self, playlist_id: String) {
    const MAX_PAGES: u32 = 200; // safety cap: 200 pages × 50 = 10,000 tracks
    let mut pages = 0u32;
    loop {
      if pages >= MAX_PAGES {
        break;
      }
      // Early exit: another playlist armed its own fill, or a different
      // playlist owns the table now. The cache keeps what arrived; scroll
      // or the next open completes the rest.
      let keep = {
        let app = self.app.lock().await;
        app.fill_target.as_deref() == Some(playlist_id.as_str())
          && match app.track_table_playlist_uri() {
            Some(viewed) => viewed == playlist_id,
            None => true,
          }
      };
      if !keep {
        return;
      }
      let (offset, total) = {
        let app = self.app.lock().await;
        match &app.playlist_tracks {
          Some(p) => (p.items.len() as u32, p.total),
          None => (0, 0),
        }
      };
      if offset >= total {
        break;
      }
      self.pace().await;
      match self
        .spotify
        .playlist_items_manual(
          PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
          None,
          Some(Market::FromToken),
          Some(API_MAX_LIMIT),
          Some(offset),
        )
        .await
      {
        Ok(page) => {
          // Same ownership rule as apply: a no-owner table (pasted playlist)
          // belongs to this fill; another playlist's table is never touched.
          let (paint_table, mirror_mfy) = {
            let app = self.app.lock().await;
            let paint = match app.track_table_playlist_uri() {
              Some(viewed) => viewed == playlist_id,
              // No owner: the pasted-playlist fill paints (scroll can't page
              // it in). But a MadeForYou table showing another list is owned.
              None => {
                app.track_table.context != Some(TrackTableContext::MadeForYou)
                  || app
                    .made_for_you_playlist_id(app.made_for_you_index)
                    .as_deref()
                    == Some(playlist_id.as_str())
              }
            };
            let mirror = app.track_table.context == Some(TrackTableContext::MadeForYou)
              && app
                .made_for_you_playlist_id(app.made_for_you_index)
                .as_deref()
                == Some(playlist_id.as_str());
            (paint, mirror)
          };
          if paint_table {
            self.set_playlist_tracks_to_table(&page, true).await;
          }
          let mut app = self.app.lock().await;
          self
            .playlist_cache
            .update(&playlist_id, page.items.clone(), page.total, offset, true);
          self.sync_playlist_uris(&mut app);
          if paint_table {
            if let Some(existing) = app.playlist_tracks.as_mut() {
              existing.items.truncate(offset as usize);
              existing.items.extend(page.items.clone());
              existing.total = page.total;
            }
          }
          if mirror_mfy {
            if let Some(existing) = app.made_for_you_tracks.as_mut() {
              existing.items.truncate(offset as usize);
              existing.items.extend(page.items);
              existing.total = page.total;
            }
          }
        }
        Err(_) => break,
      }
      pages += 1;
    }
    let mut app = self.app.lock().await;
    if app.date_added_pending {
      app.materialize_date_added();
    }
  }

  /// Serve the cached track list of a playlist, whole or partial, with zero
  /// requests. Legacy doubled entries are served as-is; one manual refresh
  /// rewrites them clean. Nothing here ever refetches — full refetch is the
  /// refresh button's job alone.
  async fn serve_playlist_cache(&mut self, playlist_id: &str) -> bool {
    self.playlist_cache.ensure_loaded();
    let Some(entry) = self.playlist_cache.lookup(playlist_id) else {
      return false;
    };
    if entry.items.is_empty() {
      return false;
    }
    let page = Page::<PlaylistItem> {
      href: String::new(),
      items: entry.items.clone(),
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total: entry.total,
    };
    self.set_playlist_tracks_to_table(&page, false).await;
    let mut app = self.app.lock().await;
    app.playlist_tracks = Some(page);
    app.is_fetching_next_page = false;
    // Serving a cached list disarms any in-flight fill for a previous one.
    app.fill_target = None;
    app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
    true
  }

  /// Once-per-launch background version check for a cached playlist. Runs at
  /// most once per id per sptune start (the set is in-memory only). Returns
  /// whether the cache changed (caller re-serves it). Spotify changes a
  /// playlist's snapshot id on ANY edit, so one cheap request detects
  /// staleness; purely appended songs are merged as tail pages, anything
  /// else (removals, reorders) is left for the manual refresh button.
  async fn reconcile_playlist_tracks(&mut self, playlist_id: &str) -> bool {
    if !self.probed_playlists.insert(playlist_id.to_string()) {
      return false;
    }
    let Some(entry) = self.playlist_cache.lookup(playlist_id) else {
      return false;
    };
    let cached_len = entry.items.len() as u32;
    if cached_len == 0 || cached_len != entry.total {
      return false;
    }
    let live_snapshot = match self
      .spotify
      .playlist(
        PlaylistId::from_id_or_uri(playlist_id).unwrap(),
        None,
        Some(Market::FromToken),
      )
      .await
    {
      Ok(p) => p.snapshot_id,
      Err(_) => {
        // Probe failed (transient network/API issue): keep the cache.
        return false;
      }
    };
    if !entry.snapshot.is_empty() && entry.snapshot == live_snapshot {
      return false;
    }
    if entry.snapshot.is_empty() {
      // Written by the normal fetch path, which never records snapshots; the
      // entry is fresh by definition, so just record it instead of merging.
      self.playlist_cache.set_snapshot(playlist_id, live_snapshot);
      return false;
    }
    // Snapshot changed. Fetch the first page and compare its item URIs to
    // the cached head: an identical head + larger total means songs were
    // only appended at the end -> append just the missing tail.
    let cached_head: Vec<Option<String>> = entry.items.iter().map(playlist_item_uri).collect();
    let first_page = match self
      .spotify
      .playlist_items_manual(
        PlaylistId::from_id_or_uri(playlist_id).unwrap(),
        None,
        Some(Market::FromToken),
        Some(API_MAX_LIMIT),
        Some(0),
      )
      .await
    {
      Ok(p) => p,
      Err(_) => return false,
    };
    let head_matches = first_page
      .items
      .iter()
      .zip(cached_head.iter())
      .all(|(live, cached)| playlist_item_uri(live) == *cached);
    if head_matches && first_page.total > cached_len {
      // Tail append: fetch from the cached end to the live total, 50/page.
      let mut offset = cached_len;
      loop {
        let page = match self
          .spotify
          .playlist_items_manual(
            PlaylistId::from_id_or_uri(playlist_id).unwrap(),
            None,
            None,
            Some(API_MAX_LIMIT),
            Some(offset),
          )
          .await
        {
          Ok(p) => p,
          Err(_) => break,
        };
        self
          .playlist_cache
          .update(playlist_id, page.items.clone(), page.total, offset, true);
        offset += page.items.len() as u32;
        if offset >= page.total || page.items.is_empty() {
          break;
        }
        self.pace().await;
      }
    }
    // Non-additive changes keep the served cache; the refresh button does
    // the full refetch. Record the snapshot either way so the same tail
    // isn't re-merged on a later probe.
    self.playlist_cache.set_snapshot(playlist_id, live_snapshot);
    head_matches && first_page.total > cached_len
  }

  /// Whether a playlist item occupies a slot in the player's context. Spotify's
  /// context offset counts every item in the playlist except explicitly
  /// unavailable tracks (is_playable: false); episodes, unknown items, and
  /// empty entries all keep their position.
  fn playlist_item_counts_for_offset(item: &PlaylistItem) -> bool {
    !matches!(
      item.item.as_ref(),
      Some(PlayableItem::Track(t)) if t.is_playable == Some(false)
    )
  }

  async fn set_playlist_tracks_to_table(
    &mut self,
    playlist_track_page: &Page<PlaylistItem>,
    append: bool,
  ) {
    // Spotify's context offset skips explicitly unavailable tracks
    // (is_playable: false); they are hidden from the table and dropped from
    // the offset count. Episodes, unknown items, and empty entries keep their
    // position.
    let base = if append {
      let app = self.app.lock().await;
      app
        .playlist_tracks
        .as_ref()
        .map(|existing| {
          existing
            .items
            .iter()
            .filter(|item| Self::playlist_item_counts_for_offset(item))
            .count()
        })
        .unwrap_or(0)
    } else {
      0
    };
    let mut added_at: Vec<Option<DateTime<Utc>>> = Vec::new();
    let mut tracks: Vec<FullTrack> = Vec::new();
    let mut raw_index: Vec<usize> = Vec::new();
    let mut pos = base;
    for item in playlist_track_page.items.iter() {
      // B fix: show unavailable as disabled grey rows so 30→30 (was 30→27 via _=>{} drop)
      if !Self::playlist_item_counts_for_offset(item) {
        // is_playable==false still occupies? spec says skip offset, but we show disabled
        // keep pos not incremented for these per spec, and render as disabled
        let placeholder: FullTrack = serde_json::from_value(serde_json::json!({
          "album": { "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "" }], "external_urls": {}, "href": null, "id": null, "images": [], "name": "" },
          "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "" }],
          "disc_number": 1, "duration_ms": 0, "explicit": false, "external_ids": {}, "external_urls": {}, "href": null, "id": null, "is_local": false, "name": "— Unavailable —", "preview_url": null, "track_number": 1, "type": "track", "is_playable": false
        }))
        .unwrap();
        added_at.push(item.added_at);
        tracks.push(placeholder);
        raw_index.push(pos);
        // do not pos+=1 for is_playable false per spec? keep spec: pos unchanged
        continue;
      }
      match item.item.as_ref() {
        Some(PlayableItem::Track(t)) => {
          added_at.push(item.added_at);
          tracks.push(t.clone());
          raw_index.push(pos);
        }
        Some(PlayableItem::Episode(e)) => {
          let placeholder: FullTrack = serde_json::from_value(serde_json::json!({
            "album": { "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "" }], "external_urls": {}, "href": null, "id": null, "images": [], "name": "" },
            "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "" }],
            "disc_number": 1, "duration_ms": e.duration.num_milliseconds() as u64, "explicit": false, "external_ids": {}, "external_urls": {}, "href": null, "id": null, "is_local": false, "name": format!("Episode: {}", e.name), "preview_url": null, "track_number": 1, "type": "track", "is_playable": false
          }))
          .unwrap_or_else(|_| serde_json::from_value(serde_json::json!({
            "album": { "artists": [], "external_urls": {}, "href": null, "id": null, "images": [], "name": "" },
            "artists": [], "disc_number": 1, "duration_ms": 0, "explicit": false, "external_ids": {}, "external_urls": {}, "href": null, "id": null, "is_local": false, "name": "— Unavailable —", "preview_url": null, "track_number": 1, "type": "track", "is_playable": false
          })).unwrap());
          added_at.push(item.added_at);
          tracks.push(placeholder);
          raw_index.push(pos);
        }
        _ => {
          // None / unknown → placeholder so count matches total (30)
          let placeholder: FullTrack = serde_json::from_value(serde_json::json!({
            "album": { "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "" }], "external_urls": {}, "href": null, "id": null, "images": [], "name": "" },
            "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "" }],
            "disc_number": 1, "duration_ms": 0, "explicit": false, "external_ids": {}, "external_urls": {}, "href": null, "id": null, "is_local": false, "name": "— Unavailable —", "preview_url": null, "track_number": 1, "type": "track", "is_playable": false
          }))
          .unwrap();
          added_at.push(item.added_at);
          tracks.push(placeholder);
          raw_index.push(pos);
        }
      }
      pos += 1;
    }
    self
      .set_tracks_to_table(tracks, added_at, raw_index, append)
      .await;

    // Prime the liked-heart column for the rows just loaded; the check is
    // TTL-cached in `saved_checked`, and the icon can be turned off in the
    // gear menu so nothing is fetched when it is hidden.
    let track_ids = playlist_track_page
      .items
      .iter()
      .filter_map(|item| match item.item.as_ref() {
        Some(PlayableItem::Track(t)) => t.id.as_ref().map(|id| id.to_string()),
        _ => None,
      })
      .collect::<Vec<String>>();
    if !track_ids.is_empty() {
      let mut app = self.app.lock().await;
      {
        app.dispatch(IoEvent::CurrentUserSavedTracksContains(track_ids));
      }
    }
  }

  async fn set_tracks_to_table(
    &mut self,
    tracks: Vec<FullTrack>,
    added_at: Vec<Option<DateTime<Utc>>>,
    raw_index: Vec<usize>,
    append: bool,
  ) {
    let mut app = self.app.lock().await;
    if append {
      app.track_table.tracks.extend(tracks.clone());
      app.track_table_added_at.extend(added_at.clone());
      app.track_table_raw_index.extend(raw_index.clone());
      // Defer re-sorting while a Date Added load is in flight; the reversal
      // materializes once the full playlist has arrived.
      if app.track_table_sort.is_some() && !app.date_added_pending {
        app.sort_tracks();
      }
    } else {
      app.track_table.tracks = tracks.clone();
      app.track_table_added_at = added_at.clone();
      app.track_table_raw_index = raw_index.clone();
      app.track_table_sort = None;
      // A fresh table must not highlight its first row until the user
      // moves the selection or clicks a row.
      app.selection_engaged = false;
      // Not a playlist being browsed anymore; the next playlist open sets
      // this again from its own name lookup.
      app.playlist_view = None;
    }
  }

  async fn set_artists_to_table(&mut self, artists: Vec<FullArtist>) {
    let mut app = self.app.lock().await;
    app.artists = artists;
    app.selection_engaged = false;
  }

  async fn get_made_for_you_playlist_tracks(
    &mut self,
    playlist_id: String,
    made_for_you_offset: u32,
  ) {
    if made_for_you_offset == 0 {
      self.playlist_cache.ensure_loaded();
      if self.playlist_cache.lookup(&playlist_id).is_some()
        && self.serve_playlist_cache(&playlist_id).await
      {
        // serve_playlist_cache fills app.playlist_tracks, but the For-you
        // table tracks its own accumulator; mirror it so Load-more math
        // (items.len() < total) keeps working.
        let mut app = self.app.lock().await;
        if let Some(page) = app.playlist_tracks.clone() {
          app.made_for_you_tracks = Some(page);
        }
        let name = app
          .made_for_you_custom
          .iter()
          .find(|(_, id)| *id == playlist_id)
          .map(|(name, _)| name.clone())
          .unwrap_or_default();
        app.playlist_view = Some((name, playlist_id.clone()));
        drop(app);
        // Stale-guard: probe the live total in the background and refetch
        // when the entry changed.
        self
          .app
          .lock()
          .await
          .dispatch(IoEvent::ReconcilePlaylistTracks(playlist_id));
        return;
      }
    }
    match self
      .spotify
      .playlist_items_manual(
        PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
        None,
        Some(Market::FromToken),
        Some(API_MAX_LIMIT),
        Some(made_for_you_offset),
      )
      .await
    {
      Ok(made_for_you_tracks) => {
        // View guard + covered check (the shared-playlist path has both;
        // this one never did): a stale or already-covered page updates the
        // keyed cache only, never another list's table.
        let paint = {
          let app = self.app.lock().await;
          app.track_table.context == Some(TrackTableContext::MadeForYou)
            && app
              .made_for_you_playlist_id(app.made_for_you_index)
              .as_deref()
              == Some(playlist_id.as_str())
        };
        if made_for_you_offset > 0 {
          let covered = {
            let app = self.app.lock().await;
            app
              .made_for_you_tracks
              .as_ref()
              .map(|existing| made_for_you_offset < existing.items.len() as u32)
              .unwrap_or(false)
          };
          if covered || !paint {
            self.app.lock().await.is_fetching_next_page = false;
            return;
          }
        }
        if paint {
          self
            .set_playlist_tracks_to_table(&made_for_you_tracks, made_for_you_offset > 0)
            .await;
        }

        let cache_items = made_for_you_tracks.items.clone();
        let cache_total = made_for_you_tracks.total;
        let mut app = self.app.lock().await;
        if made_for_you_offset == 0 {
          let name = self
            .spotify
            .playlist(
              PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
              None,
              Some(Market::FromToken),
            )
            .await
            .map(|playlist| playlist.name)
            .unwrap_or_default();
          if paint {
            app.playlist_view = Some((name, playlist_id.clone()));
            // Opening any list disarms another list's background fill; Made
            // For You pages in via scroll, so nothing is armed here.
            app.fill_target = None;
            app.made_for_you_tracks = Some(made_for_you_tracks);
          }
        } else if paint {
          if let Some(existing) = app.made_for_you_tracks.as_mut() {
            existing.items.truncate(made_for_you_offset as usize);
            existing.items.extend(made_for_you_tracks.items);
            existing.total = made_for_you_tracks.total;
          }
        }
        drop(app);
        self.playlist_cache.update(
          &playlist_id,
          cache_items,
          cache_total,
          made_for_you_offset,
          made_for_you_offset > 0,
        );
        let mut app = self.app.lock().await;
        app.is_fetching_next_page = false;
        if app.get_current_route().id != RouteId::TrackTable {
          app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
        }
        self.sync_playlist_uris(&mut app);
      }
      Err(e) => {
        self
          .handle_error(anyhow!(
            "failed to load tracks of playlist '{}': {}",
            playlist_id,
            e
          ))
          .await;
        self.app.lock().await.is_fetching_next_page = false;
      }
    }
  }

  async fn get_current_user_saved_shows(&mut self, offset: Option<u32>) {
    if offset.is_none()
      && self.library_cache.get("saved_shows").is_some()
      && self.serve_saved_shows_cache().await
    {
      return;
    }
    match self
      .spotify
      .get_saved_show_manual(Some(API_MAX_LIMIT), offset)
      .await
    {
      Ok(saved_shows) => {
        // not to show a blank page
        if !saved_shows.items.is_empty() {
          match offset {
            Some(off) if off > 0 => {
              let shrink = self
                .library_cache
                .get_typed::<Show>("saved_shows")
                .map(|(cached, _)| saved_shows.total < cached.len() as u32)
                .unwrap_or(false);
              if shrink {
                self.refetch_saved_shows().await;
              } else {
                self
                  .library_cache
                  .append("saved_shows", &saved_shows.items, saved_shows.total);
              }
            }
            _ => {
              self
                .library_cache
                .put("saved_shows", &saved_shows.items, saved_shows.total);
            }
          }
          let mut app = self.app.lock().await;
          app.library.saved_shows.add_pages(saved_shows);
        }
      }
      Err(e) => {
        if offset.is_none() && self.serve_saved_shows_cache().await {
          return;
        }
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  /// Serve the cached saved-shows list; see `serve_saved_tracks_cache`.
  async fn serve_saved_shows_cache(&mut self) -> bool {
    self.library_cache.ensure_loaded();
    let Some((items, total)) = self.library_cache.get_typed::<Show>("saved_shows") else {
      return false;
    };
    if items.is_empty() {
      return false;
    }
    let page = Page::<Show> {
      href: String::new(),
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total,
      items,
    };
    let mut app = self.app.lock().await;
    app.library.saved_shows.add_pages(page);
    true
  }

  /// Delta probe on Pods open; see `reconcile_saved_tracks`.
  async fn reconcile_saved_shows(&mut self) {
    let Some((cached, total)) = self.library_cache.get_typed::<Show>("saved_shows") else {
      return;
    };
    if cached.len() as u32 != total {
      return;
    }
    let page = match self
      .spotify
      .get_saved_show_manual(Some(API_MAX_LIMIT), None)
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    if page.total < total {
      self.refetch_saved_shows().await;
      return;
    }
    let cached_ids: std::collections::HashSet<String> =
      cached.iter().map(|s| s.show.id.to_string()).collect();
    let new_head: Vec<Show> = page
      .items
      .into_iter()
      .take_while(|s| !cached_ids.contains(&s.show.id.to_string()))
      .collect();
    if new_head.is_empty() {
      return;
    }
    if page.total == total + new_head.len() as u32 {
      self
        .library_cache
        .prepend("saved_shows", &new_head, page.total);
    } else {
      self.refetch_saved_shows().await;
    }
  }

  /// Full sequential refetch of saved shows; see `refetch_saved_tracks`.
  async fn refetch_saved_shows(&mut self) {
    let mut all: Vec<Show> = Vec::new();
    loop {
      let page = match self
        .spotify
        .get_saved_show_manual(Some(API_MAX_LIMIT), Some(all.len() as u32))
        .await
      {
        Ok(p) => p,
        Err(_) => return,
      };
      all.extend(page.items);
      if all.len() as u32 >= page.total {
        self.library_cache.put("saved_shows", &all, page.total);
        return;
      }
    }
  }

  async fn current_user_saved_shows_contains(&mut self, show_ids: Vec<String>) {
    if let Ok(are_followed) = self
      .spotify
      .library_contains(
        show_ids
          .iter()
          .map(|id| LibraryId::Show(ShowId::from_id_or_uri(id).unwrap())),
      )
      .await
    {
      let mut app = self.app.lock().await;
      show_ids.iter().enumerate().for_each(|(i, id)| {
        if are_followed[i] {
          app.saved_show_ids_set.insert(id.to_owned());
        } else {
          app.saved_show_ids_set.remove(id);
        }
      })
    }
  }

  async fn get_show_episodes(&mut self, show: Box<SimplifiedShow>) {
    let cloned_show = (*show).clone();
    let show_id = show.id;
    match self
      .spotify
      .get_shows_episodes_manual(show_id, None, Some(API_MAX_LIMIT), Some(0))
      .await
    {
      Ok(episodes) => {
        if !episodes.items.is_empty() {
          let mut app = self.app.lock().await;
          app.library.show_episodes = ScrollableResultPages::new();
          app.library.show_episodes.add_pages(episodes);

          app.selected_show_simplified = Some(SelectedShow { show: cloned_show });

          app.episode_table_context = EpisodeTableContext::Simplified;

          app.push_navigation_stack(RouteId::PodcastEpisodes, ActiveBlock::EpisodeTable);
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_show(&mut self, show_id: String) {
    match self
      .spotify
      .get_a_show(ShowId::from_id_or_uri(&show_id).unwrap(), None)
      .await
    {
      Ok(show) => {
        let selected_show = SelectedFullShow { show };

        let mut app = self.app.lock().await;

        app.selected_show_full = Some(selected_show);

        app.episode_table_context = EpisodeTableContext::Full;
        app.push_navigation_stack(RouteId::PodcastEpisodes, ActiveBlock::EpisodeTable);
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_current_show_episodes(&mut self, show_id: String, offset: Option<u32>) {
    match self
      .spotify
      .get_shows_episodes_manual(
        ShowId::from_id_or_uri(&show_id).unwrap(),
        None,
        Some(API_MAX_LIMIT),
        offset,
      )
      .await
    {
      Ok(episodes) => {
        if !episodes.items.is_empty() {
          let mut app = self.app.lock().await;
          app.library.show_episodes.add_pages(episodes);
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_search_results(&mut self, search_term: String) {
    // Lazy: only record the term. Each tab fetches its own first page on
    // demand (GetMoreSearchResults), so searching is instant.
    let mut app = self.app.lock().await;
    app.reset_search_results();
    app.search_results.query = search_term;
    // ponytail: new search auto-expands the Songs tab (fetch on demand)
    app.load_search_block(&SearchResultBlock::SongSearch);
  }

  async fn get_more_search_results(&mut self, block: SearchResultBlock) {
    let search_type = match block {
      SearchResultBlock::SongSearch => SearchType::Track,
      SearchResultBlock::ArtistSearch => SearchType::Artist,
      SearchResultBlock::AlbumSearch => SearchType::Album,
      SearchResultBlock::PlaylistSearch => SearchType::Playlist,
      SearchResultBlock::ShowSearch => SearchType::Show,
      SearchResultBlock::Empty => {
        self.app.lock().await.is_fetching_next_page = false;
        return;
      }
    };

    let (offset, total_override, query) = {
      let app = self.app.lock().await;
      let meta = match block {
        SearchResultBlock::SongSearch => page_meta(app.search_results.tracks.as_ref()),
        SearchResultBlock::ArtistSearch => page_meta(app.search_results.artists.as_ref()),
        SearchResultBlock::AlbumSearch => page_meta(app.search_results.albums.as_ref()),
        SearchResultBlock::PlaylistSearch => page_meta(app.search_results.playlists.as_ref()),
        SearchResultBlock::ShowSearch => page_meta(app.search_results.shows.as_ref()),
        SearchResultBlock::Empty => None,
      };
      match meta {
        // A full last page means more can be fetched: the search `total`
        // under-reports for many queries, so gating on it kills the
        // load-more row early (same disease as the artist top tracks).
        Some((items_len, total, page_offset, limit))
          if limit > 0 && items_len >= limit as usize =>
        {
          (
            page_offset + SEARCH_MAX_LIMIT,
            Some(total),
            app.search_results.query.clone(),
          )
        }
        // First load for this tab: no page yet, start from the top.
        None => (0, None, app.search_results.query.clone()),
        _ => {
          drop(app);
          self.app.lock().await.is_fetching_next_page = false;
          return;
        }
      }
    };

    match self
      .spotify
      .search(
        &query,
        search_type,
        Some(Market::FromToken),
        None,
        Some(SEARCH_MAX_LIMIT),
        Some(offset),
      )
      .await
    {
      Ok(search_result) => {
        let mut app = self.app.lock().await;
        app.is_fetching_next_page = false;
        if app.search_results.query != query {
          return; // a newer search replaced these results mid-fetch
        }
        match block {
          SearchResultBlock::SongSearch => {
            if let SearchResult::Tracks(page) = search_result {
              let was_on_row = app.search_results.selected_tracks_index
                == app.search_results.tracks.as_ref().map(|t| t.items.len());
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.tracks.as_ref(),
                page,
                offset,
                total,
                |track: &FullTrack| {
                  track
                    .id
                    .as_ref()
                    .map(|id| id.to_string())
                    .unwrap_or_default()
                },
              );
              let new_len = merged.items.len();
              app.search_results.tracks = Some(merged);
              // Prime the liked-heart column for song search results.
              let track_ids = app
                .search_results
                .tracks
                .as_ref()
                .map(|p| {
                  p.items
                    .iter()
                    .filter_map(|t| t.id.as_ref().map(|id| id.to_string()))
                    .collect::<Vec<String>>()
                })
                .unwrap_or_default();
              if !track_ids.is_empty() {
                app.dispatch(IoEvent::CurrentUserSavedTracksContains(track_ids));
              }
              if was_on_row {
                app.search_results.selected_tracks_index = Some(new_len);
              }
            }
          }
          SearchResultBlock::ArtistSearch => {
            if let SearchResult::Artists(page) = search_result {
              let was_on_row = app.search_results.selected_artists_index
                == app.search_results.artists.as_ref().map(|a| a.items.len());
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.artists.as_ref(),
                page,
                offset,
                total,
                |artist: &FullArtist| artist.id.to_string(),
              );
              let new_len = merged.items.len();
              let artist_ids = merged
                .items
                .iter()
                .map(|item| item.id.to_string())
                .collect();
              app.search_results.artists = Some(merged);
              if was_on_row {
                app.search_results.selected_artists_index = Some(new_len);
              }
              app.dispatch(IoEvent::UserArtistFollowCheck(artist_ids));
            }
          }
          SearchResultBlock::AlbumSearch => {
            if let SearchResult::Albums(page) = search_result {
              let was_on_row = app.search_results.selected_album_index
                == app.search_results.albums.as_ref().map(|a| a.items.len());
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.albums.as_ref(),
                page,
                offset,
                total,
                |album: &SimplifiedAlbum| {
                  album
                    .id
                    .as_ref()
                    .map(|id| id.to_string())
                    .unwrap_or_default()
                },
              );
              let new_len = merged.items.len();
              let album_ids = merged
                .items
                .iter()
                .filter_map(|album| album.id.as_ref().map(|id| id.to_string()))
                .collect();
              app.search_results.albums = Some(merged);
              if was_on_row {
                app.search_results.selected_album_index = Some(new_len);
              }
              app.dispatch(IoEvent::CurrentUserSavedAlbumsContains(album_ids));
            }
          }
          SearchResultBlock::PlaylistSearch => {
            if let SearchResult::Playlists(page) = search_result {
              let was_on_row = app.search_results.selected_playlists_index
                == app.search_results.playlists.as_ref().map(|p| p.items.len());
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.playlists.as_ref(),
                page,
                offset,
                total,
                |playlist: &SimplifiedPlaylist| playlist.id.to_string(),
              );
              let new_len = merged.items.len();
              app.search_results.playlists = Some(merged);
              if was_on_row {
                app.search_results.selected_playlists_index = Some(new_len);
              }
            }
          }
          SearchResultBlock::ShowSearch => {
            if let SearchResult::Shows(page) = search_result {
              let was_on_row = app.search_results.selected_shows_index
                == app.search_results.shows.as_ref().map(|s| s.items.len());
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.shows.as_ref(),
                page,
                offset,
                total,
                |show: &SimplifiedShow| show.id.to_string(),
              );
              let new_len = merged.items.len();
              let show_ids = merged
                .items
                .iter()
                .map(|show| show.id.to_string())
                .collect();
              app.search_results.shows = Some(merged);
              if was_on_row {
                app.search_results.selected_shows_index = Some(new_len);
              }
              app.dispatch(IoEvent::CurrentUserSavedShowsContains(show_ids));
            }
          }
          SearchResultBlock::Empty => {}
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
        self.app.lock().await.is_fetching_next_page = false;
      }
    }
  }

  async fn get_current_user_saved_tracks(&mut self, offset: Option<u32>) {
    if offset.is_none()
      && self.library_cache.get("saved_tracks").is_some()
      && self.serve_saved_tracks_cache().await
    {
      return;
    }
    match self
      .spotify
      .current_user_saved_tracks_manual(Some(Market::FromToken), Some(API_MAX_LIMIT), offset)
      .await
    {
      Ok(saved_tracks) => {
        match offset {
          Some(off) if off > 0 => {
            let shrink = self
              .library_cache
              .get_typed::<SavedTrack>("saved_tracks")
              .map(|(cached, _)| saved_tracks.total < cached.len() as u32)
              .unwrap_or(false);
            if shrink {
              self.refetch_saved_tracks().await;
            } else {
              self
                .library_cache
                .append("saved_tracks", &saved_tracks.items, saved_tracks.total);
            }
          }
          _ => {
            self
              .library_cache
              .put("saved_tracks", &saved_tracks.items, saved_tracks.total);
          }
        }
        let mut app = self.app.lock().await;
        let append = offset.unwrap_or(0) > 0;
        if append {
          app
            .track_table
            .tracks
            .extend(saved_tracks.items.iter().map(|item| item.track.clone()));
          app
            .track_table_added_at
            .extend(saved_tracks.items.iter().map(|item| Some(item.added_at)));
          if app.track_table_sort.is_some() {
            app.sort_tracks();
          }
        } else {
          app.track_table.tracks = saved_tracks
            .items
            .clone()
            .into_iter()
            .map(|item| item.track)
            .collect::<Vec<FullTrack>>();
          app.track_table_added_at = saved_tracks
            .items
            .iter()
            .map(|item| Some(item.added_at))
            .collect();
          app.track_table_sort = None;
        }

        saved_tracks.items.iter().for_each(|item| {
          if let Some(track_id) = &item.track.id {
            app.liked_song_ids_set.insert(track_id.to_string());
          }
        });

        app.library.saved_tracks.add_pages(saved_tracks);
        app.track_table.context = Some(TrackTableContext::SavedTracks);
        app.is_fetching_next_page = false;
      }
      Err(e) => {
        if offset.is_none() && self.serve_saved_tracks_cache().await {
          return;
        }
        self.handle_error(anyhow!(e)).await;
        self.app.lock().await.is_fetching_next_page = false;
      }
    }
  }

  /// Serve the cached liked-songs page (cache never expires).
  async fn serve_saved_tracks_cache(&mut self) -> bool {
    self.library_cache.ensure_loaded();
    let Some((items, total)) = self.library_cache.get_typed::<SavedTrack>("saved_tracks") else {
      return false;
    };
    let page = Page::<SavedTrack> {
      href: String::new(),
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total,
      items,
    };
    let mut app = self.app.lock().await;
    app.track_table.tracks = page.items.iter().map(|item| item.track.clone()).collect();
    app.track_table_added_at = page.items.iter().map(|item| Some(item.added_at)).collect();
    app.track_table_sort = None;
    page.items.iter().for_each(|item| {
      if let Some(track_id) = &item.track.id {
        app.liked_song_ids_set.insert(track_id.to_string());
      }
    });
    app.library.saved_tracks.add_pages(page);
    app.track_table.context = Some(TrackTableContext::SavedTracks);
    app.is_fetching_next_page = false;
    true
  }

  /// Delta probe on Liked Songs open: fetch the newest page and prepend the
  /// items that are not cached yet. Skipped until the list is fully loaded;
  /// skipped silently when the probe fails (cache already shown).
  async fn reconcile_saved_tracks(&mut self) {
    let Some((cached, total)) = self.library_cache.get_typed::<SavedTrack>("saved_tracks") else {
      return;
    };
    if cached.len() as u32 != total {
      return;
    }
    let page = match self
      .spotify
      .current_user_saved_tracks_manual(Some(Market::FromToken), Some(API_MAX_LIMIT), None)
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    if page.total < total {
      self.refetch_saved_tracks().await;
      return;
    }
    let cached_ids: std::collections::HashSet<String> = cached
      .iter()
      .filter_map(|i| i.track.id.as_ref().map(|id| id.to_string()))
      .collect();
    let new_head: Vec<SavedTrack> = page
      .items
      .into_iter()
      .take_while(|i| {
        !cached_ids.contains(
          &i.track
            .id
            .as_ref()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        )
      })
      .collect();
    if new_head.is_empty() {
      return;
    }
    if page.total == total + new_head.len() as u32 {
      self
        .library_cache
        .prepend("saved_tracks", &new_head, page.total);
    } else {
      self.refetch_saved_tracks().await;
    }
  }

  /// Full sequential refetch of liked songs, replacing the cached list.
  async fn refetch_saved_tracks(&mut self) {
    let mut all: Vec<SavedTrack> = Vec::new();
    loop {
      let page = match self
        .spotify
        .current_user_saved_tracks_manual(
          Some(Market::FromToken),
          Some(API_MAX_LIMIT),
          Some(all.len() as u32),
        )
        .await
      {
        Ok(p) => p,
        Err(_) => return,
      };
      all.extend(page.items);
      if all.len() as u32 >= page.total {
        self.library_cache.put("saved_tracks", &all, page.total);
        return;
      }
    }
  }

  /// Resolve an active device id without user interaction.
  /// Order: cached config → current playback context → cached device list → live device list (active or first).
  /// Persists the chosen id so subsequent calls succeed synchronously.
  async fn effective_device_id(&mut self) -> Option<String> {
    if let Some(id) = self.client_config.device_id.clone() {
      return Some(id);
    }
    let from_app = {
      if let Ok(app) = self.app.try_lock() {
        if let Some(id) = app
          .current_playback_context
          .as_ref()
          .and_then(|c| c.device.id.clone())
        {
          Some(id)
        } else if let Some(devs) = app.devices.as_ref() {
          devs.iter().find_map(|d| d.id.clone())
        } else {
          None
        }
      } else {
        None
      }
    };
    if let Some(id) = from_app {
      let _ = self.client_config.set_device_id(id.clone());
      return Some(id);
    }
    if let Ok(devs) = self.spotify.device().await {
      let picked = devs
        .iter()
        .find(|d| d.is_active)
        .or(devs.first())
        .and_then(|d| d.id.clone());
      if let Some(ref id) = picked {
        let _ = self.client_config.set_device_id(id.clone());
        if let Ok(mut app) = self.app.try_lock() {
          app.devices = Some(devs);
        }
      } else if let Ok(mut app) = self.app.try_lock() {
        app.devices = Some(devs);
      }
      return picked;
    }
    None
  }

  async fn start_playback(
    &mut self,
    context_uri: Option<String>,
    uris: Option<Vec<String>>,
    offset: Option<usize>,
  ) {
    let (uris, context_uri) = if context_uri.is_some() {
      (None, context_uri)
    } else if uris.is_some() {
      (uris, None)
    } else {
      (None, None)
    };

    let offset = offset.map(|o| Offset::Position(ChronoDuration::milliseconds(o as i64)));

    let Some(device_id) = self.effective_device_id().await else {
      self
        .handle_error(anyhow!(
          "No device available — open Spotify on a device and retry (press `d` to select)"
        ))
        .await;
      self.get_devices().await;
      return;
    };

    let result = if let Some(context_uri) = context_uri {
      match play_context_from_uri(&context_uri) {
        Some(cid) => self
          .spotify
          .start_context_playback(cid, Some(&device_id), offset, None)
          .await
          .map_err(|e| anyhow!(e)),
        None => Err(anyhow!("Invalid context uri")),
      }
    } else if let Some(uris) = uris {
      self
        .spotify
        .start_uris_playback(
          uris.iter().filter_map(|u| playable_from_uri(u)),
          Some(&device_id),
          offset,
          None,
        )
        .await
        .map_err(|e| anyhow!(e))
    } else {
      self
        .spotify
        .resume_playback(Some(&device_id), None)
        .await
        .map_err(|e| anyhow!(e))
    };

    match result {
      Ok(()) => {
        let mut app = self.app.lock().await;
        app.song_progress_ms = 0;
        app.dispatch(IoEvent::GetCurrentPlayback);
      }
      Err(e) => {
        self.handle_error(e).await;
      }
    }
  }

  // Start a playlist/album context at a specific track, resolved by URI
  // instead of a position offset. Spotify counts the context position itself,
  // so this is immune to stale is_playable flags, unavailable-track drops,
  // and sort remapping.
  async fn start_playback_at(&mut self, context_uri: Option<String>, track_uri: Option<String>) {
    let (Some(context_uri), Some(track_uri)) = (context_uri, track_uri) else {
      self
        .handle_error(anyhow!("Missing context or track uri"))
        .await;
      return;
    };
    let Some(device_id) = self.effective_device_id().await else {
      self
        .handle_error(anyhow!(
          "No device available — open Spotify on a device and retry (press `d` to select)"
        ))
        .await;
      self.get_devices().await;
      return;
    };
    let result = match play_context_from_uri(&context_uri) {
      Some(cid) => self
        .spotify
        .start_context_playback(cid, Some(&device_id), Some(Offset::Uri(track_uri)), None)
        .await
        .map_err(|e| anyhow!(e)),
      None => Err(anyhow!("Invalid context uri")),
    };

    match result {
      Ok(()) => {
        let mut app = self.app.lock().await;
        app.song_progress_ms = 0;
        app.dispatch(IoEvent::GetCurrentPlayback);
      }
      Err(e) => {
        self.handle_error(e).await;
      }
    }
  }

  /// Poll-until-matches: light progress checks so a fast server seek apply
  /// reconciles in ~250ms instead of a fixed 1000ms floor. Worst case
  /// (3 misses) the caller still lands its full update at ~750ms.
  async fn wait_until_position_ms(&self, position_ms: u32) {
    let target_ms = i64::from(position_ms);
    for _ in 0..3 {
      tokio::time::sleep(Duration::from_millis(250)).await;
      if let Ok(Some(ctx)) = self
        .spotify
        .current_playback(
          None,
          Some([AdditionalType::Episode, AdditionalType::Track].iter()),
        )
        .await
      {
        if ctx
          .progress
          .is_some_and(|p| (p.num_milliseconds() - target_ms).abs() < 3000)
        {
          break;
        }
      }
    }
  }

  async fn seek(&mut self, position_ms: u32) {
    let Some(device_id) = self.effective_device_id().await else {
      self
        .handle_error(anyhow!(
          "No device available — open Spotify on a device and retry (press `d` to select)"
        ))
        .await;
      self.get_devices().await;
      return;
    };
    match self
      .spotify
      .seek_track(
        ChronoDuration::milliseconds(position_ms as i64),
        Some(device_id.as_str()),
      )
      .await
    {
      Ok(()) => {
        self.wait_until_position_ms(position_ms).await;
        self.get_current_playback().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
        // Reconcile: the optimistic seek failed, so refetch server truth.
        // seek_ms is left intact — a newer queued seek keeps it and runs next.
        self.get_current_playback().await;
      }
    };
  }

  /// Trailing-edge reconcile: skips the confirm GET when one ran <500ms
  /// ago. Rapid next/prev spam converges on the last state anyway; the
  /// periodic poll is the backstop.
  async fn reconcile(&mut self) {
    let now = Instant::now();
    if now.duration_since(self.last_reconcile) < Duration::from_millis(500) {
      return;
    }
    self.last_reconcile = now;
    self.get_current_playback().await;
  }

  async fn next_track(&mut self) {
    let device_id = self.client_config.device_id.clone().or_else(|| {
      self.app.try_lock().ok().and_then(|a| {
        a.current_playback_context
          .as_ref()
          .and_then(|c| c.device.id.clone())
      })
    });
    match self.spotify.next_track(device_id.as_deref()).await {
      Ok(()) => {
        self.reconcile().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn previous_track(&mut self) {
    let device_id = self.client_config.device_id.clone().or_else(|| {
      self.app.try_lock().ok().and_then(|a| {
        a.current_playback_context
          .as_ref()
          .and_then(|c| c.device.id.clone())
      })
    });
    match self.spotify.previous_track(device_id.as_deref()).await {
      Ok(()) => {
        self.reconcile().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn shuffle(&mut self, shuffle_state: bool) {
    let device_id = self.client_config.device_id.clone().or_else(|| {
      self.app.try_lock().ok().and_then(|a| {
        a.current_playback_context
          .as_ref()
          .and_then(|c| c.device.id.clone())
      })
    });
    match self
      .spotify
      .shuffle(!shuffle_state, device_id.as_deref())
      .await
    {
      Ok(()) => {
        // Update the UI eagerly (otherwise the UI will wait until the next 5 second interval
        // due to polling playback context)
        let mut app = self.app.lock().await;
        if let Some(current_playback_context) = &mut app.current_playback_context {
          current_playback_context.shuffle_state = !shuffle_state;
        };
        // Repaint instantly instead of waiting for the next 1 Hz tick.
        app.ui_dirty = true;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn repeat(&mut self, repeat_state: RepeatState) {
    let next_repeat_state = match repeat_state {
      RepeatState::Off => RepeatState::Context,
      RepeatState::Context => RepeatState::Track,
      RepeatState::Track => RepeatState::Off,
    };
    match self
      .spotify
      .repeat(next_repeat_state, self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => {
        let mut app = self.app.lock().await;
        if let Some(current_playback_context) = &mut app.current_playback_context {
          current_playback_context.repeat_state = next_repeat_state;
        };
        // Repaint instantly instead of waiting for the next 1 Hz tick.
        app.ui_dirty = true;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn pause_playback(&mut self) {
    match self
      .spotify
      .pause_playback(self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => {
        // Trust-Ok: the optimistic flip stands; the periodic poll reconciles.
        // Saves one GET per pause.
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
        // Reconcile: the optimistic flip failed, so refetch server truth.
        self.get_current_playback().await;
      }
    };
  }

  /// Saved shuffle/repeat → live device. Shared by the gated startup
  /// AutoRestore and the manual resume path.
  async fn restore_saved_playback_settings(&mut self, saved: &SavedState) {
    if let Some(shuffle) = saved.shuffle {
      match self
        .spotify
        .shuffle(shuffle, self.client_config.device_id.as_deref())
        .await
      {
        Ok(_) => {
          let mut app = self.app.lock().await;
          if let Some(context) = &mut app.current_playback_context {
            context.shuffle_state = shuffle;
          }
        }
        Err(e) => self.handle_error(anyhow!(e)).await,
      }
    }
    if let Some(repeat) = &saved.repeat {
      let state = match repeat.as_str() {
        "track" => RepeatState::Track,
        "context" => RepeatState::Context,
        _ => RepeatState::Off,
      };
      match self
        .spotify
        .repeat(state, self.client_config.device_id.as_deref())
        .await
      {
        Ok(_) => {
          let mut app = self.app.lock().await;
          if let Some(context) = &mut app.current_playback_context {
            context.repeat_state = state;
          }
        }
        Err(e) => self.handle_error(anyhow!(e)).await,
      }
    }
  }

  /// Saved last page + column sort → local UI only. Never touches the device.
  async fn restore_saved_local_view(&mut self, saved: &SavedState) {
    if let Some(last_page) = &saved.last_page {
      if let Some(playlist_id) = last_page.strip_prefix("playlist:") {
        self.get_playlist_tracks(playlist_id.to_string(), 0).await;
        let mut app = self.app.lock().await;
        app.playlist_offset = 0;
        app.track_table.context = Some(TrackTableContext::MyPlaylists);
        if let Some((name, desc)) = &saved.track_sort {
          if let Some(column) = sort_column_from_name(name) {
            app.track_table_sort = Some((column, *desc));
            app.sort_tracks();
          }
        }
      }
    }
  }

  /// Resume Last Listening: apply the saved volume/repeat/shuffle, then
  /// start the last track from the beginning. Manual only (playbar button /
  /// key) and only when nothing is playing; there is no position tracking.
  async fn resume_last_track(&mut self, track_uri: String) {
    if let Some(saved) = SavedState::load() {
      if let Some(volume) = saved.volume {
        self.change_volume(volume).await;
      }
      if let Some(shuffle) = saved.shuffle {
        let _ = self
          .spotify
          .shuffle(shuffle, self.client_config.device_id.as_deref())
          .await;
      }
      if let Some(repeat) = &saved.repeat {
        let state = match repeat.as_str() {
          "track" => RepeatState::Track,
          "context" => RepeatState::Context,
          _ => RepeatState::Off,
        };
        let _ = self
          .spotify
          .repeat(state, self.client_config.device_id.as_deref())
          .await;
      }
    }
    self
      .start_playback(None, Some(vec![track_uri]), Some(0))
      .await;
  }

  async fn change_volume(&mut self, volume_percent: u8) {
    match self
      .spotify
      .volume(volume_percent, self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => {
        let mut app = self.app.lock().await;
        if let Some(current_playback_context) = &mut app.current_playback_context {
          current_playback_context.device.volume_percent = Some(volume_percent as u32);
        };
        // Clear a matching optimistic preview; a newer one (rapid presses) is kept.
        if app.volume_preview == Some(volume_percent) {
          app.volume_preview.take();
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
        // Reconcile back to server truth: drop a matching optimistic preview so the
        // bar falls back to the context volume, then refetch. A newer preview is kept.
        let mut app = self.app.lock().await;
        if app.volume_preview == Some(volume_percent) {
          app.volume_preview.take();
        }
        drop(app);
        self.get_current_playback().await;
      }
    };
  }

  // A failed sub-request (one 403s) must not kill the whole artist profile;
  // each of the three sections falls back to an empty page instead.
  fn empty_page<T: serde::de::DeserializeOwned>() -> Page<T> {
    Page {
      href: String::new(),
      items: vec![],
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total: 0,
    }
  }

  async fn get_artist(&mut self, artist_id: String, input_artist_name: String) {
    let cloneable_artist_id = ArtistId::from_id_or_uri(&artist_id).unwrap();
    let artist_id = cloneable_artist_id.to_string();
    // Spotify removed artist_top_tracks and artist_related_artists in Feb
    // 2026 (related artists 403s); top tracks come from a track search on
    // the artist name, and albums are fetched eagerly right after the
    // profile opens (GetArtistAlbumsMore, dispatched by app.get_artist).
    // Resolve the display name from the FullArtist fetch when the caller
    // didn't supply one; fall back to "" on failure (rate limit / token expired).
    let artist_name = if input_artist_name.is_empty() {
      match self.spotify.artist(cloneable_artist_id.clone()).await {
        Ok(fa) => fa.name.clone(),
        Err(_) => String::new(),
      }
    } else {
      input_artist_name
    };
    let query = format!("artist:\"{}\"", artist_name);
    // Spotify capped the search limit at 10 (Feb 2026); larger values 400.
    let top_tracks = self
      .spotify
      .search(
        &query,
        SearchType::Track,
        Some(Market::FromToken),
        None,
        Some(10),
        Some(0),
      )
      .await;
    let (top_tracks, top_tracks_total) = match top_tracks {
      Ok(SearchResult::Tracks(page)) => (page.items, page.total as usize),
      _ => (vec![], 0),
    };
    // Search returns relevance order, not most-played-first; per-track
    // playcounts are blocked (api-partner token), and the per-track
    // popularity field was removed by Spotify. Keep search order.
    // Always offer "Load more": the search can under-match the real track
    // count. A short page on the next fetch clears the flag.
    let top_tracks_has_more = true;
    {
      let mut app = self.app.lock().await;
      app.artist = Some(Artist {
        artist_id,
        artist_name,
        albums: Self::empty_page(),
        singles: Self::empty_page(),
        eps: Self::empty_page(),
        appears_on: Self::empty_page(),
        related_artists: Vec::new(),
        top_tracks,
        top_tracks_total,
        top_tracks_has_more,
        selected_album_index: 0,
        selected_singles_index: 0,
        selected_eps_index: 0,
        selected_appears_on_index: 0,
        selected_related_artist_index: 0,
        selected_top_track_index: 0,
        artist_hovered_block: ArtistBlock::TopTracks,
        artist_selected_block: ArtistBlock::TopTracks,
      });
      // Prime the liked-heart column; TTL-cached, silent on throttle.
      let track_ids = app
        .artist
        .as_ref()
        .map(|a| {
          a.top_tracks
            .iter()
            .filter_map(|t| t.id.as_ref().map(|id| id.to_string()))
            .collect::<Vec<String>>()
        })
        .unwrap_or_default();
      if !track_ids.is_empty() {
        app.dispatch(IoEvent::CurrentUserSavedTracksContains(track_ids));
      }
    }
  }

  async fn get_artist_albums_more(&mut self, artist_id: String, offset: u32) {
    self
      .get_artist_albums_by_type(artist_id, vec![AlbumType::Album], offset, |a, p| {
        a.albums.items.append(&mut p.items);
        a.albums.offset = p.offset;
        a.albums.total = p.total;
      })
      .await;
  }

  async fn get_artist_singles_more(&mut self, artist_id: String, offset: u32) {
    self
      .get_artist_albums_by_type(artist_id, vec![AlbumType::Single], offset, |a, p| {
        a.singles.items.append(&mut p.items);
        a.singles.offset = p.offset;
        a.singles.total = p.total;
        // derive EPs: SimplifiedAlbum no longer has total_tracks (removed 0.16); use name/EP hint
        let eps_new = p
          .items
          .iter()
          .filter(|al| al.name.to_lowercase().contains("ep"))
          .cloned()
          .collect::<Vec<_>>();
        a.eps.items.extend(eps_new);
        a.eps.total = a.eps.items.len() as u32;
      })
      .await;
  }

  async fn get_artist_appears_on_more(&mut self, artist_id: String, offset: u32) {
    self
      .get_artist_albums_by_type(artist_id, vec![AlbumType::AppearsOn], offset, |a, p| {
        a.appears_on.items.append(&mut p.items);
        a.appears_on.offset = p.offset;
        a.appears_on.total = p.total;
      })
      .await;
  }

  async fn get_artist_albums_by_type<F>(
    &mut self,
    artist_id: String,
    types: Vec<AlbumType>,
    offset: u32,
    apply: F,
  ) where
    F: FnOnce(&mut Artist, &mut Page<SimplifiedAlbum>),
  {
    let cloneable_artist_id = ArtistId::from_id_or_uri(&artist_id).unwrap();
    match self
      .spotify
      .artist_albums_manual(
        cloneable_artist_id,
        types,
        Some(Market::FromToken),
        Some(10),
        Some(offset),
      )
      .await
    {
      Ok(mut page) => {
        let mut app = self.app.lock().await;
        if let Some(artist) = &mut app.artist {
          apply(artist, &mut page);
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_artist_top_tracks_more(
    &mut self,
    _artist_id: String,
    artist_name: String,
    offset: u32,
  ) {
    let query = format!("artist:\"{}\"", artist_name);
    // Limit 10: Spotify capped the search limit at 10 in Feb 2026, larger
    // values 400. The artist search only surfaces a handful of relevant
    // tracks, so load-more pages exhaust quickly — that is the API ceiling.
    match self
      .spotify
      .search(
        &query,
        SearchType::Track,
        Some(Market::FromToken),
        None,
        Some(10),
        Some(offset),
      )
      .await
    {
      Ok(SearchResult::Tracks(page)) => {
        let mut app = self.app.lock().await;
        if let Some(artist) = &mut app.artist {
          artist.top_tracks_has_more = page.items.len() == 10;
          // Offset search pagination can overlap pages, so drop duplicate
          // track ids — against the already-loaded tracks, not just the new
          // page. Duplicate URIs in a play context get rejected by Spotify,
          // so deduping keeps the loaded rows playable too.
          let mut seen: std::collections::HashSet<String> = artist
            .top_tracks
            .iter()
            .filter_map(|track| track.id.as_ref().map(|id| id.to_string()))
            .collect();
          for track in page.items {
            if track
              .id
              .as_ref()
              .is_none_or(|id| seen.insert(id.to_string()))
            {
              artist.top_tracks.push(track);
            }
          }
          artist.top_tracks_total = page.total as usize;
          // The list was extended: the old selection index no longer names
          // the load-more row (it silently points at a song). Land on top
          // instead of a phantom selection.
          artist.selected_top_track_index = 0;
        }
        // Prime hearts for the newly loaded rows (TTL dedups the old ones).
        let track_ids = app
          .artist
          .as_ref()
          .map(|a| {
            a.top_tracks
              .iter()
              .filter_map(|t| t.id.as_ref().map(|id| id.to_string()))
              .collect::<Vec<String>>()
          })
          .unwrap_or_default();
        if !track_ids.is_empty() {
          app.dispatch(IoEvent::CurrentUserSavedTracksContains(track_ids));
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
      _ => {}
    }
  }

  async fn get_album_tracks(&mut self, album: Box<SimplifiedAlbum>) {
    if let Some(album_id) = &album.id {
      let album_id_str = album_id.to_string();
      match self
        .spotify
        .album_track_manual(
          AlbumId::from_id_or_uri(&album_id_str).unwrap(),
          Some(Market::FromToken),
          Some(API_MAX_LIMIT),
          Some(0),
        )
        .await
      {
        Ok(tracks) => {
          let track_ids = tracks
            .items
            .iter()
            .filter_map(|item| item.id.as_ref().map(|t| t.to_string()))
            .collect::<Vec<String>>();

          let mut app = self.app.lock().await;
          app.selected_album_simplified = Some(SelectedAlbum {
            album: *album,
            tracks,
            selected_index: 0,
          });

          app.album_table_context = AlbumTableContext::Simplified;
          app.selection_engaged = false;
          app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
          app.dispatch(IoEvent::CurrentUserSavedTracksContains(track_ids));
        }
        Err(e) => {
          self.handle_error(anyhow!(e)).await;
        }
      }
    }
  }

  async fn get_album_tracks_more(&mut self, album_id: String, offset: u32) {
    match self
      .spotify
      .album_track_manual(
        AlbumId::from_id_or_uri(&album_id).unwrap(),
        Some(Market::FromToken),
        Some(API_MAX_LIMIT),
        Some(offset),
      )
      .await
    {
      Ok(mut page) => {
        let track_ids = page
          .items
          .iter()
          .filter_map(|item| item.id.as_ref().map(|t| t.to_string()))
          .collect::<Vec<String>>();
        let mut app = self.app.lock().await;
        if let Some(album) = &mut app.selected_album_simplified {
          album.tracks.items.append(&mut page.items);
          album.tracks.offset = page.offset;
          album.tracks.total = page.total;
        }
        if !track_ids.is_empty() {
          app.dispatch(IoEvent::CurrentUserSavedTracksContains(track_ids));
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn toggle_save_track(&mut self, track_id: String) {
    let library_id = LibraryId::Track(TrackId::from_id_or_uri(&track_id).unwrap());
    self.saved_checked.remove(&track_id);
    match self
      .spotify
      .library_contains(vec![library_id.clone()])
      .await
    {
      Ok(saved) => {
        if saved.first() == Some(&true) {
          match self.spotify.library_remove(vec![library_id]).await {
            Ok(()) => {
              let track_uri = TrackId::from_id_or_uri(&track_id)
                .map(|t| t.uri())
                .unwrap_or_else(|_| track_id.clone());
              self.library_cache.remove("saved_tracks");
              let mut app = self.app.lock().await;
              app.liked_song_ids_set.remove(&track_id);
              // Repaint the ♥ flip instantly; in Liked Songs cut the row too.
              app.ui_dirty = true;
              if app.track_table.context == Some(crate::app::TrackTableContext::SavedTracks) {
                app.remove_track_from_table(&track_uri);
              }
            }
            Err(e) => {
              self.handle_error(anyhow!(e)).await;
            }
          }
        } else {
          match self.spotify.library_add(vec![library_id]).await {
            Ok(()) => {
              self.library_cache.remove("saved_tracks");
              // TODO: This should ideally use the same logic as `self.current_user_saved_tracks_contains`
              let mut app = self.app.lock().await;
              app.liked_song_ids_set.insert(track_id);
              app.ui_dirty = true;
            }
            Err(e) => {
              self.handle_error(anyhow!(e)).await;
            }
          }
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn get_followed_artists(&mut self, after: Option<String>) {
    match self
      .spotify
      .current_user_followed_artists(after.as_deref(), Some(API_MAX_LIMIT))
      .await
    {
      Ok(saved_artists) => {
        let mut app = self.app.lock().await;
        app.artists = saved_artists.items.to_owned();
        app.library.saved_artists.add_pages(saved_artists);
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }
  async fn user_artist_check_follow(&mut self, artist_ids: Vec<String>) {
    if let Ok(are_followed) = self
      .spotify
      .library_contains(
        artist_ids
          .iter()
          .map(|id| LibraryId::Artist(ArtistId::from_id_or_uri(id).unwrap())),
      )
      .await
    {
      let mut app = self.app.lock().await;
      artist_ids.iter().enumerate().for_each(|(i, id)| {
        if are_followed[i] {
          app.followed_artist_ids_set.insert(id.to_owned());
        } else {
          app.followed_artist_ids_set.remove(id);
        }
      });
    }
  }

  async fn get_current_user_saved_albums(&mut self, offset: Option<u32>) {
    if offset.is_none()
      && self.library_cache.get("saved_albums").is_some()
      && self.serve_saved_albums_cache().await
    {
      return;
    }
    match self
      .spotify
      .current_user_saved_albums_manual(Some(Market::FromToken), Some(API_MAX_LIMIT), offset)
      .await
    {
      Ok(saved_albums) => {
        // not to show a blank page
        if !saved_albums.items.is_empty() {
          match offset {
            Some(off) if off > 0 => {
              let shrink = self
                .library_cache
                .get_typed::<SavedAlbum>("saved_albums")
                .map(|(cached, _)| saved_albums.total < cached.len() as u32)
                .unwrap_or(false);
              if shrink {
                self.refetch_saved_albums().await;
              } else {
                self
                  .library_cache
                  .append("saved_albums", &saved_albums.items, saved_albums.total);
              }
            }
            _ => {
              self
                .library_cache
                .put("saved_albums", &saved_albums.items, saved_albums.total);
            }
          }
          let mut app = self.app.lock().await;
          app.library.saved_albums.add_pages(saved_albums);
        }
      }
      Err(e) => {
        if offset.is_none() && self.serve_saved_albums_cache().await {
          return;
        }
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  /// Serve the cached saved-albums list; see `serve_saved_tracks_cache`.
  async fn serve_saved_albums_cache(&mut self) -> bool {
    self.library_cache.ensure_loaded();
    let Some((items, total)) = self.library_cache.get_typed::<SavedAlbum>("saved_albums") else {
      return false;
    };
    if items.is_empty() {
      return false;
    }
    let page = Page::<SavedAlbum> {
      href: String::new(),
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total,
      items,
    };
    let mut app = self.app.lock().await;
    app.library.saved_albums.add_pages(page);
    true
  }

  /// Delta probe on Albums open; see `reconcile_saved_tracks`.
  async fn reconcile_saved_albums(&mut self) {
    let Some((cached, total)) = self.library_cache.get_typed::<SavedAlbum>("saved_albums") else {
      return;
    };
    if cached.len() as u32 != total {
      return;
    }
    let page = match self
      .spotify
      .current_user_saved_albums_manual(Some(Market::FromToken), Some(API_MAX_LIMIT), None)
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    if page.total < total {
      self.refetch_saved_albums().await;
      return;
    }
    let cached_ids: std::collections::HashSet<String> =
      cached.iter().map(|a| a.album.id.to_string()).collect();
    let new_head: Vec<SavedAlbum> = page
      .items
      .into_iter()
      .take_while(|a| !cached_ids.contains(&a.album.id.to_string()))
      .collect();
    if new_head.is_empty() {
      return;
    }
    if page.total == total + new_head.len() as u32 {
      self
        .library_cache
        .prepend("saved_albums", &new_head, page.total);
    } else {
      self.refetch_saved_albums().await;
    }
  }

  /// Full sequential refetch of saved albums; see `refetch_saved_tracks`.
  async fn refetch_saved_albums(&mut self) {
    let mut all: Vec<SavedAlbum> = Vec::new();
    loop {
      let page = match self
        .spotify
        .current_user_saved_albums_manual(
          Some(Market::FromToken),
          Some(API_MAX_LIMIT),
          Some(all.len() as u32),
        )
        .await
      {
        Ok(p) => p,
        Err(_) => return,
      };
      all.extend(page.items);
      if all.len() as u32 >= page.total {
        self.library_cache.put("saved_albums", &all, page.total);
        return;
      }
    }
  }

  async fn current_user_saved_albums_contains(&mut self, album_ids: Vec<String>) {
    if let Ok(are_followed) = self
      .spotify
      .library_contains(
        album_ids
          .iter()
          .map(|id| LibraryId::Album(AlbumId::from_id_or_uri(id).unwrap())),
      )
      .await
    {
      let mut app = self.app.lock().await;
      album_ids.iter().enumerate().for_each(|(i, id)| {
        if are_followed[i] {
          app.saved_album_ids_set.insert(id.to_owned());
        } else {
          app.saved_album_ids_set.remove(id);
        }
      });
    }
  }

  pub async fn current_user_saved_album_delete(&mut self, album_id: String) {
    match self
      .spotify
      .library_remove(vec![LibraryId::Album(
        AlbumId::from_id_or_uri(&album_id).unwrap(),
      )])
      .await
    {
      Ok(_) => {
        self.library_cache.remove("saved_albums");
        self.get_current_user_saved_albums(None).await;
        let mut app = self.app.lock().await;
        app.saved_album_ids_set.remove(&album_id.to_owned());
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn current_user_saved_album_add(&mut self, album_id: String) {
    match self
      .spotify
      .library_add(vec![LibraryId::Album(
        AlbumId::from_id_or_uri(&album_id).unwrap(),
      )])
      .await
    {
      Ok(_) => {
        self.library_cache.remove("saved_albums");
        let mut app = self.app.lock().await;
        app.saved_album_ids_set.insert(album_id.to_owned());
      }
      Err(e) => self.handle_error(anyhow!(e)).await,
    }
  }

  async fn current_user_saved_shows_delete(&mut self, show_id: String) {
    match self
      .spotify
      .library_remove(vec![LibraryId::Show(
        ShowId::from_id_or_uri(&show_id).unwrap(),
      )])
      .await
    {
      Ok(_) => {
        self.library_cache.remove("saved_shows");
        self.get_current_user_saved_shows(None).await;
        let mut app = self.app.lock().await;
        app.saved_show_ids_set.remove(&show_id.to_owned());
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn current_user_saved_shows_add(&mut self, show_id: String) {
    match self
      .spotify
      .library_add(vec![LibraryId::Show(
        ShowId::from_id_or_uri(&show_id).unwrap(),
      )])
      .await
    {
      Ok(_) => {
        self.library_cache.remove("saved_shows");
        self.get_current_user_saved_shows(None).await;
        let mut app = self.app.lock().await;
        app.saved_show_ids_set.insert(show_id.to_owned());
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }
  async fn user_unfollow_artists(&mut self, artist_ids: Vec<String>) {
    match self
      .spotify
      .library_remove(
        artist_ids
          .iter()
          .map(|id| LibraryId::Artist(ArtistId::from_id_or_uri(id).unwrap())),
      )
      .await
    {
      Ok(_) => {
        self.get_followed_artists(None).await;
        let mut app = self.app.lock().await;
        artist_ids.iter().for_each(|id| {
          app.followed_artist_ids_set.remove(&id.to_owned());
        });
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }
  async fn user_follow_artists(&mut self, artist_ids: Vec<String>) {
    match self
      .spotify
      .library_add(
        artist_ids
          .iter()
          .map(|id| LibraryId::Artist(ArtistId::from_id_or_uri(id).unwrap())),
      )
      .await
    {
      Ok(_) => {
        self.get_followed_artists(None).await;
        let mut app = self.app.lock().await;
        artist_ids.iter().for_each(|id| {
          app.followed_artist_ids_set.insert(id.to_owned());
        });
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn user_follow_playlist(
    &mut self,
    _playlist_owner_id: String,
    playlist_id: String,
    _is_public: Option<bool>,
  ) {
    match self
      .spotify
      .library_add([LibraryId::Playlist(
        PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
      )])
      .await
    {
      Ok(_) => {
        self.get_current_user_playlists().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn user_unfollow_playlist(&mut self, user_id: String, playlist_id: String) {
    let _ = user_id;
    match self
      .spotify
      .library_remove([LibraryId::Playlist(
        PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
      )])
      .await
    {
      Ok(_) => {
        self.playlist_cache.remove(&playlist_id);
        let mut app = self.app.lock().await;
        self.sync_playlist_uris(&mut app);
        drop(app);
        self.get_current_user_playlists().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn bearer_token(&self) -> Option<String> {
    match self.spotify.token.lock().await {
      Ok(guard) => (*guard).as_ref().map(|t| t.access_token.clone()),
      Err(_) => None,
    }
  }

  /// Fetch synced lyrics from LRCLIB (no auth, LRC format). Exact match on
  /// track/artist/duration first; fuzzy search as fallback for near-misses.
  async fn get_lyrics(&mut self) {
    let (track_name, artist_name, duration_ms) = {
      let app = self.app.lock().await;
      match &app.current_playback_context {
        Some(ctx) => match &ctx.item {
          Some(PlayableItem::Track(track)) => (
            track.name.clone(),
            track
              .artists
              .first()
              .map(|a| a.name.clone())
              .unwrap_or_default(),
            track.duration.num_milliseconds() as u64,
          ),
          _ => (String::new(), String::new(), 0),
        },
        None => (String::new(), String::new(), 0),
      }
    };
    if track_name.is_empty() || artist_name.is_empty() {
      return;
    }

    let base = "https://lrclib.net/api/".to_string();
    let exact = Self::lrclib_get(&base, "get", &track_name, &artist_name, Some(duration_ms));
    let result = match exact.await {
      Ok(Some(lyrics)) => Ok(Some(lyrics)),
      _ => Self::lrclib_get(&base, "search", &track_name, &artist_name, None).await,
    };

    let mut app = self.app.lock().await;
    match result {
      Ok(Some(lyrics)) => app.lyrics = Some(lyrics),
      Ok(None) => app.lyrics = None,
      Err(e) => {
        // Surface the failure in the dev request log instead of silently
        // showing "No lyrics available" in the Music View.
        app.lyrics = None;
        app.log_request(format!("GetLyrics failed: {e}"));
      }
    }
  }

  /// One LRCLIB query: `get` returns a single object (404 = no match), `search`
  /// returns a list. The first non-instrumental hit with synced lyrics wins.
  async fn lrclib_get(
    base: &str,
    path: &str,
    track_name: &str,
    artist_name: &str,
    duration_ms: Option<u64>,
  ) -> anyhow::Result<Option<Vec<(u128, String)>>> {
    let mut url = reqwest::Url::parse_with_params(
      &format!("{base}{path}"),
      &[("track_name", track_name), ("artist_name", artist_name)],
    )?;
    if let Some(ms) = duration_ms {
      url
        .query_pairs_mut()
        .append_pair("duration", &format!("{}", ms / 1000));
    }
    let res = shared_http_client().get(url).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
      return Ok(None);
    }
    if !res.status().is_success() {
      return Err(anyhow::anyhow!("HTTP {}", res.status()));
    }
    let json: serde_json::Value = res.json().await?;
    let candidates: Vec<&serde_json::Value> = match path {
      "search" => json
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default(),
      _ => vec![&json],
    };
    let hit = candidates
      .into_iter()
      .find(|c| c.pointer("/instrumental").and_then(|v| v.as_bool()) != Some(true));
    let Some(hit) = hit else { return Ok(None) };
    if let Some(lrc) = hit.pointer("/syncedLyrics").and_then(|l| l.as_str()) {
      let lines = parse_lrc(lrc);
      if !lines.is_empty() {
        return Ok(Some(lines));
      }
    }
    if let Some(plain) = hit.pointer("/plainLyrics").and_then(|l| l.as_str()) {
      // Un-synced lyrics: one line per verse, all at 0ms (current-line
      // highlight then sits on the first entry).
      return Ok(Some(
        plain
          .lines()
          .map(|line| (0u128, line.to_string()))
          .collect(),
      ));
    }
    Ok(None)
  }

  async fn get_track_credits(&mut self, track_id: String) {
    let Some(token) = self.bearer_token().await else {
      return;
    };
    let url = format!("https://spclient.wg.spotify.com/track-credits/v1/track/{track_id}");
    let response = shared_http_client()
      .get(&url)
      .header("Authorization", format!("Bearer {token}"))
      .send()
      .await;
    let mut app = self.app.lock().await;
    app.track_credits = match response {
      Ok(res) => match res.json::<serde_json::Value>().await {
        Ok(json) => json
          .pointer("/roleCredits")
          .and_then(|r| r.as_array())
          .map(|roles| {
            roles
              .iter()
              .filter_map(|role| {
                let title = role.pointer("/roleTitle")?.as_str()?;
                let names = role
                  .pointer("/artists")?
                  .as_array()?
                  .iter()
                  .filter_map(|a| a.pointer("/name").and_then(|n| n.as_str()))
                  .collect::<Vec<_>>()
                  .join(", ");
                Some(format!("{title}: {names}"))
              })
              .collect()
          }),
        Err(_) => None,
      },
      Err(_) => None,
    };
  }

  async fn get_queue(&mut self) {
    let result = self.spotify.current_user_queue().await;
    let mut app = self.app.lock().await;
    app.queue_next = match result {
      Ok(queue) => queue.queue.first().map(|item| match item {
        PlayableItem::Track(track) => {
          let artists = track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
          if artists.is_empty() {
            track.name.clone()
          } else {
            format!("{} - {}", track.name, artists)
          }
        }
        PlayableItem::Episode(episode) => episode.name.clone(),
        _ => String::new(),
      }),
      Err(_) => None,
    };
  }

  async fn get_current_user_playlists(&mut self) {
    if self.library_cache.get("playlists").is_some() && self.serve_playlists_cache().await {
      // Cache paints instantly; refetch right after so playlists created
      // elsewhere appear without a manual refresh.
      self.refresh_playlists().await;
    } else {
      let playlists = self
        .spotify
        .current_user_playlists_manual(Some(API_MAX_LIMIT), None)
        .await;

      match playlists {
        Ok(p) => {
          self.library_cache.put("playlists", &p.items, p.total);
          let mut app = self.app.lock().await;
          app.playlists = Some(p);
        }
        Err(e) => {
          if !self.serve_playlists_cache().await {
            self.handle_error(anyhow!(e)).await;
            return;
          }
        }
      };
    }
    // Populate the "already in playlist" map from the persisted cache at
    // startup; otherwise the ✓ icon stays invisible until a playlist is
    // opened (the only path that used to sync it).
    self.playlist_cache.ensure_loaded();
    let mut app = self.app.lock().await;
    self.sync_playlist_uris(&mut app);
  }

  /// Serve the cached playlists list; see `serve_saved_tracks_cache`.
  async fn serve_playlists_cache(&mut self) -> bool {
    self.library_cache.ensure_loaded();
    let Some((items, total)) = self
      .library_cache
      .get_typed::<SimplifiedPlaylist>("playlists")
    else {
      return false;
    };
    if items.is_empty() {
      return false;
    }
    let page = Page::<SimplifiedPlaylist> {
      href: String::new(),
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total,
      items,
    };
    let mut app = self.app.lock().await;
    app.playlists = Some(page);
    true
  }

  /// Full sequential refetch of the playlists list; see `refetch_saved_tracks`.
  async fn refetch_playlists(&mut self) {
    // First page gives the total; remaining offsets are known upfront,
    // so fetch them concurrently instead of one RTT per page.
    let first = match self
      .spotify
      .current_user_playlists_manual(Some(API_MAX_LIMIT), Some(0))
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    let total = first.total;
    let mut all = first.items;
    if (all.len() as u32) < total {
      let rest = futures::future::join_all(
        (all.len() as u32..total)
          .step_by(API_MAX_LIMIT as usize)
          .map(|offset| {
            self
              .spotify
              .current_user_playlists_manual(Some(API_MAX_LIMIT), Some(offset))
          }),
      )
      .await;
      for page in rest {
        match page {
          Ok(p) => all.extend(p.items),
          Err(_) => return,
        }
      }
    }
    self.library_cache.put("playlists", &all, total);
  }

  async fn get_recently_played(&mut self) {
    match self
      .spotify
      .current_user_recently_played(Some(API_MAX_LIMIT), None)
      .await
    {
      Ok(result) => {
        let track_ids = result
          .items
          .iter()
          .filter_map(|item| item.track.id.clone().map(|id| id.to_string()))
          .collect::<Vec<String>>();

        self.current_user_saved_tracks_contains(track_ids).await;

        let mut app = self.app.lock().await;

        app.recently_played.result = Some(result.clone());
        app.selection_engaged = false;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_more_recently_played(&mut self, before: Option<String>) {
    // The page cursor is a unix-millis timestamp string; rspotify wants it
    // as a TimeLimits::Before. No cursor (or a bad one) falls back to the
    // most recent page, which the dedup below makes harmless.
    let time_limits = before
      .as_deref()
      .and_then(|s| s.parse::<i64>().ok())
      .and_then(chrono::DateTime::from_timestamp_millis)
      .map(rspotify::model::TimeLimits::Before);
    match self
      .spotify
      .current_user_recently_played(Some(API_MAX_LIMIT), time_limits)
      .await
    {
      Ok(result) => {
        let track_ids = result
          .items
          .iter()
          .filter_map(|item| item.track.id.clone().map(|id| id.to_string()))
          .collect::<Vec<String>>();
        self.current_user_saved_tracks_contains(track_ids).await;
        let mut app = self.app.lock().await;
        if let Some(existing) = &mut app.recently_played.result {
          let mut seen = std::collections::HashSet::new();
          for item in &existing.items {
            if let Some(id) = &item.track.id {
              seen.insert(id.to_string());
            }
          }
          for item in result.items {
            let dup = item
              .track
              .id
              .as_ref()
              .is_some_and(|id| !seen.insert(id.to_string()));
            if !dup {
              existing.items.push(item);
            }
          }
          existing.limit = result.limit;
          existing.total = result.total;
          existing.cursors = result.cursors;
        }
        app.selection_engaged = false;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_album(&mut self, album_id: String) {
    match self
      .spotify
      .album(
        AlbumId::from_id_or_uri(&album_id).unwrap(),
        Some(Market::FromToken),
      )
      .await
    {
      Ok(album) => {
        let selected_album = SelectedFullAlbum {
          album,
          selected_index: 0,
        };

        let mut app = self.app.lock().await;

        app.selected_album_full = Some(selected_album);
        app.album_table_context = AlbumTableContext::Full;
        app.selection_engaged = false;
        app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_album_for_track(&mut self, track_id: String) {
    match self
      .spotify
      .track(
        TrackId::from_id_or_uri(&track_id).unwrap(),
        Some(Market::FromToken),
      )
      .await
    {
      Ok(track) => {
        // It is unclear when the id can ever be None, but perhaps a track can be album-less. If
        // so, there isn't much to do here anyways, since we're looking for the parent album.
        let album_id = match track.album.id {
          Some(id) => id,
          None => return,
        };

        if let Ok(album) = self.spotify.album(album_id, Some(Market::FromToken)).await {
          // The way we map to the UI is zero-indexed, but Spotify is 1-indexed.
          let zero_indexed_track_number = track.track_number - 1;
          let selected_album = SelectedFullAlbum {
            album,
            // Overflow should be essentially impossible here, so we prefer the cleaner 'as'.
            selected_index: zero_indexed_track_number as usize,
          };

          let mut app = self.app.lock().await;

          app.selected_album_full = Some(selected_album.clone());
          app.saved_album_tracks_index = selected_album.selected_index;
          app.album_table_context = AlbumTableContext::Full;
          app.selection_engaged = false;
          app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn transfert_playback_to_device(&mut self, device_id: String) {
    match self.spotify.transfer_playback(&device_id, Some(true)).await {
      Ok(()) => {
        self.get_current_playback().await;
      }
      Err(e) => {
        self
          .handle_error(anyhow!("failed to page your playlists: {}", e))
          .await;
        return;
      }
    };

    match self.client_config.set_device_id(device_id) {
      Ok(()) => {
        let mut app = self.app.lock().await;
        app.pop_navigation_stack();
      }
      Err(e) => {
        self.handle_error(e).await;
      }
    };
  }

  async fn refresh_authentication(&mut self) {
    match self.spotify.refetch_token().await {
      Ok(Some(mut new_token_info)) => {
        // Spotify's refresh response often omits `refresh_token`: without
        // this carry-over the rebuilt client could never refresh again and
        // every later attempt would land in the Ok(None) arm below.
        if new_token_info.refresh_token.is_none() {
          if let Some(old) = self.spotify.get_token().lock().await.unwrap().as_ref() {
            new_token_info.refresh_token = old.refresh_token.clone();
          }
        }
        let (new_spotify, new_token_expiry) = get_spotify(new_token_info, &self.client_config);
        self.spotify = new_spotify;
        // ponytail: the rebuilt client never auto-persists (library refresh
        // is disabled), so write the carried-over token explicitly — without
        // this the next launch re-reads the stale pre-refresh file.
        let _ = self.spotify.write_token_cache().await;
        #[cfg(unix)]
        {
          use std::os::unix::fs::PermissionsExt;
          let _ = std::fs::set_permissions(
            self.spotify.get_config().cache_path.clone(),
            std::fs::Permissions::from_mode(0o600),
          );
        }
        let mut app = self.app.lock().await;
        app.spotify_token_expiry = new_token_expiry;
      }
      Ok(None) => {
        // No refresh token in memory. With the carry-over above this is
        // unreachable in a healthy session, so the session is dead: stop
        // the refresh loop (same flag the grant-death arm sets) and tell
        // the user to restart. No cache wipe — the file may still hold a
        // good token that a restart re-reads.
        let mut app = self.app.lock().await;
        app.auth_reauth_required = true;
        app.handle_error(anyhow!(
          "Spotify session can't be refreshed — restart sptune to log in again in the browser."
        ));
      }
      Err(e) => {
        let msg = e.to_string();
        if msg.contains("invalid_grant") || msg.contains("invalid_client") {
          // Stale pre-PKCE grant: the refresh token was minted under the
          // secret flow and can never refresh without it. Wipe the cache so
          // the next launch re-authenticates through the browser via PKCE.
          let _ = std::fs::remove_file(self.spotify.get_config().cache_path.clone());
          // println! is invisible behind the TUI: raise the visible error
          // route instead, and set the flag that stops the 401 path and the
          // expiry tick from re-dispatching a refresh that cannot succeed.
          let mut app = self.app.lock().await;
          app.auth_reauth_required = true;
          app.handle_error(anyhow!(
            "Spotify login expired (old pre-PKCE grant). Token cache deleted — restart sptune to log in again in the browser."
          ));
        } else {
          // Transient (network blip, 5xx): stay silent, the 60s expiry tick
          // and the 401 path retry automatically. Printing here only
          // produced scary scrollback for self-healing blips.
        }
      }
    }
  }

  async fn add_item_to_queue(&mut self, item: String) {
    let Some(item) = playable_from_uri(&item) else {
      return;
    };

    match self
      .spotify
      .add_item_to_queue(item, self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => (),
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  /// Create a private playlist, show it in the sidebar, then add the track
  /// through the verified add path (same read-after-write guarantees).
  async fn create_playlist_then_add(&mut self, name: String, uri: String) {
    let user_id = {
      let app = self.app.lock().await;
      app.user.as_ref().map(|u| u.id.to_string())
    };
    let Some(user_id) = user_id else {
      self
        .handle_error(anyhow::anyhow!("Cannot create playlist: user unknown"))
        .await;
      return;
    };
    let Ok(user_id) = UserId::from_id(user_id) else {
      self
        .handle_error(anyhow::anyhow!("Cannot create playlist: bad user id"))
        .await;
      return;
    };
    match self
      .spotify
      .user_playlist_create(user_id, &name, Some(false), Some(false), None)
      .await
    {
      Ok(playlist) => {
        // Sidebar shows the new list immediately; the verified add below
        // keeps counts and caches in sync.
        self.refresh_playlists().await;
        let new_id = playlist.id.to_string();
        {
          let mut app = self.app.lock().await;
          // Land on the created playlist's panel: point the sidebar
          // selection and the track view at it (find by id, not position
          // — sidebar ordering isn't guaranteed) and push the route now,
          // since the cache-hit path below returns without pushing.
          if let Some(page) = app.playlists.as_ref() {
            if let Some(idx) = page
              .items
              .iter()
              .position(|p| p.id.id() == playlist.id.id())
            {
              app.selected_playlist_index = Some(idx);
              app.active_playlist_index = Some(idx);
            }
          }
          app.track_table.context = Some(TrackTableContext::MyPlaylists);
          app.playlist_offset = 0;
          app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
        }
        self.add_track_to_playlist(uri, new_id.clone()).await;
        // Paint the panel (cache when the verify confirmed, live refetch
        // when it didn't — self-heals instead of showing a lagged list).
        self.get_playlist_tracks(new_id, 0).await;
      }
      Err(e) => self.handle_error(anyhow!(e)).await,
    }
  }

  async fn add_track_to_playlist(&mut self, uri: String, playlist_id: String) {
    let Some(item) = playable_from_uri(&uri) else {
      return;
    };
    let Ok(playlist_id) = PlaylistId::from_id_or_uri(&playlist_id) else {
      return;
    };
    let playlist_id_str = playlist_id.id().to_string();
    match self
      .spotify
      .playlist_add_items(playlist_id.clone(), vec![item], None)
      .await
    {
      Ok(result) => {
        let snapshot = result.snapshot_id;
        let mut app = self.app.lock().await;
        // Keep the sidebar count in sync without refetching the whole library.
        if let Some(page) = app.playlists.as_mut() {
          if let Some(pl) = page.items.iter_mut().find(|p| p.id.id() == playlist_id_str) {
            pl.items.total = pl.items.total.saturating_add(1);
          }
        }
        // Optimistic ✓ marker so the track flips to ✓ without waiting for a
        // background fetch; the cache is invalidated so the next open sees the
        // real track metadata (placeholder name would be wrong).
        app
          .playlist_uri_map
          .entry(playlist_id_str.clone())
          .or_default()
          .insert(uri.clone());
        let current_view_is_target =
          app.viewed_playlist_id().as_deref() == Some(playlist_id_str.as_str());
        drop(app);
        // Invalidate first: the verify below never caches, so a lagged read
        // can't poison the entry (see fetch_verified_add_page).
        self.playlist_cache.remove(&playlist_id_str);
        let (page, confirmed) = self.fetch_verified_add_page(&playlist_id_str, &uri).await;
        if current_view_is_target {
          // Repaint from the best-known page. Only a CONFIRMED list may stay
          // cached: on exhaust evict again so the next open refetches live
          // (self-heals) instead of serving the lagged list forever.
          if let Some(page) = page {
            self.apply_playlist_tracks(&playlist_id_str, page, 0).await;
          }
          if confirmed {
            self.playlist_cache.set_snapshot(&playlist_id_str, snapshot);
          } else {
            self.playlist_cache.remove(&playlist_id_str);
          }
          self.app.lock().await.ui_dirty = true;
        } else if confirmed {
          // Pre-warm the cache (no table touch, no nav push): opening the
          // playlist later serves the confirmed list instantly.
          if let Some(page) = page {
            self
              .playlist_cache
              .update(&playlist_id_str, page.items.clone(), page.total, 0, false);
            self.playlist_cache.set_snapshot(&playlist_id_str, snapshot);
            let mut app = self.app.lock().await;
            self.sync_playlist_uris(&mut app);
          }
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  /// Confirm Spotify's reads show the just-added song. Spotify's write and
  /// read paths converge slowly: a refetch issued right after
  /// playlist_add_items can return the PRE-add list, and caching that list
  /// poisons the entry (empty-snapshot entries are trusted once by the
  /// reconcile probe), so the song stays missing across reopens. Polls the
  /// head page plus the tail slot (appends land at the end) a few times and
  /// returns the last head page seen plus whether the song was confirmed.
  /// Never caches: the caller decides what may stay cached.
  async fn fetch_verified_add_page(
    &mut self,
    playlist_id_str: &str,
    uri: &str,
  ) -> (Option<Page<PlaylistItem>>, bool) {
    const ATTEMPTS: u32 = 5;
    let mut last: Option<Page<PlaylistItem>> = None;
    for attempt in 0..ATTEMPTS {
      self.pace().await;
      if let Ok(page) = self
        .spotify
        .playlist_items_manual(
          PlaylistId::from_id_or_uri(playlist_id_str).unwrap(),
          None,
          Some(Market::FromToken),
          Some(API_MAX_LIMIT),
          Some(0),
        )
        .await
      {
        if playlist_page_contains(&page, uri) {
          return (Some(page), true);
        }
        // Appends land at the end: when the list spans pages the head can't
        // prove anything, so probe the tail slot directly.
        if page.total > page.items.len() as u32 {
          self.pace().await;
          if let Ok(tail) = self
            .spotify
            .playlist_items_manual(
              PlaylistId::from_id_or_uri(playlist_id_str).unwrap(),
              None,
              Some(Market::FromToken),
              Some(1),
              Some(page.total.saturating_sub(1)),
            )
            .await
          {
            if playlist_page_contains(&tail, uri) {
              return (Some(page), true);
            }
          }
        }
        last = Some(page);
      }
      if attempt + 1 < ATTEMPTS {
        tokio::time::sleep(Duration::from_secs(1)).await;
      }
    }
    (last, false)
  }

  async fn remove_track_from_playlist(&mut self, uri: String, playlist_id: String) {
    let Some(item) = playable_from_uri(&uri) else {
      return;
    };
    let Ok(playlist_id) = PlaylistId::from_id_or_uri(&playlist_id) else {
      return;
    };
    let playlist_id_str = playlist_id.id().to_string();
    match self
      .spotify
      .playlist_remove_all_occurrences_of_items(playlist_id, vec![item], None)
      .await
    {
      Ok(result) => {
        let mut app = self.app.lock().await;
        self
          .playlist_cache
          .remove_item(&playlist_id_str, &uri, result.snapshot_id);
        if let Some(page) = app.playlists.as_mut() {
          if let Some(pl) = page.items.iter_mut().find(|p| p.id.id() == playlist_id_str) {
            pl.items.total = pl.items.total.saturating_sub(1);
          }
        }
        self.sync_playlist_uris(&mut app);
        // Cut the row from the visible table instantly; the re-serve below
        // confirms against the updated cache. Page mirror drops it too so
        // paging can't resurrect the row before the next refresh.
        if app.track_table.context == Some(crate::app::TrackTableContext::MyPlaylists) {
          if app.remove_track_from_table(&uri) {
            app.ui_dirty = true;
          }
          if let Some(page) = app.playlist_tracks.as_mut() {
            let before = page.items.len();
            page
              .items
              .retain(|item| playlist_item_uri(item).as_deref() != Some(uri.as_str()));
            let cut = before - page.items.len();
            page.total = page.total.saturating_sub(cut as u32);
          }
        }
        // If the removed playlist is on screen, refresh the table so the row
        // disappears without a manual refresh.
        let still_viewing = app.viewed_playlist_id().as_deref() == Some(playlist_id_str.as_str());
        drop(app);
        if still_viewing {
          // Serve already updated the cache; just re-serve the truncated list.
          if self.serve_playlist_cache(&playlist_id_str).await {
            self.app.lock().await.ui_dirty = true;
          }
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  /// Mirror the playlist cache into the app so the drawing layer can mark
  /// "already in playlist" rows without touching the filesystem or the API.
  /// For-you playlists (Spotify-made mixes) are excluded: the ✓ means "in one
  /// of MY playlists", and a mix's own tracks would otherwise all match.
  fn sync_playlist_uris(&self, app: &mut App) {
    let for_you_ids: Vec<String> = app
      .made_for_you_custom
      .iter()
      .flat_map(|(_, id)| {
        let bare = id
          .strip_prefix("spotify:playlist:")
          .unwrap_or(id)
          .to_string();
        [id.clone(), bare]
      })
      .collect();
    app.playlist_uri_map = self
      .playlist_cache
      .map
      .iter()
      .filter(|(id, _)| !for_you_ids.contains(id))
      .map(|(id, entry)| {
        (
          id.clone(),
          entry.items.iter().filter_map(playlist_item_uri).collect(),
        )
      })
      .collect();
  }
}

/// Parse LRC lyrics (`[mm:ss.xx] words`, metadata lines like `[ti:...]` skipped)
/// into (start_ms, words) pairs in time order.
fn parse_lrc(lrc: &str) -> Vec<(u128, String)> {
  let mut out = Vec::new();
  for line in lrc.lines() {
    let mut rest = line;
    let mut first_time: Option<u128> = None;
    while let Some(open) = rest.find('[') {
      let Some(close) = rest[open + 1..].find(']') else {
        break;
      };
      let tag = &rest[open + 1..open + 1 + close];
      rest = &rest[open + 1 + close + 1..];
      let Some(colon) = tag.find(':') else { continue };
      let (Ok(min), Ok(sec)) = (
        tag[..colon].parse::<u128>(),
        tag[colon + 1..].parse::<f64>(),
      ) else {
        continue;
      };
      let ms = min * 60_000 + (sec * 1000.0) as u128;
      if first_time.is_none() {
        first_time = Some(ms);
      }
    }
    if let Some(ms) = first_time {
      if !rest.is_empty() {
        out.push((ms, rest.trim().to_string()));
      }
    }
  }
  out.sort_by_key(|(ms, _)| *ms);
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;

  /// Minimal Network for unit tests (no mock dispatch, no API calls).
  fn test_network(app: &Arc<Mutex<App>>) -> Network<'_> {
    let spotify = AuthCodePkceSpotify::with_config(
      Credentials::default(),
      OAuth {
        redirect_uri: "http://localhost:8888".to_string(),
        scopes: std::collections::HashSet::new(),
        ..Default::default()
      },
      Config {
        token_cached: false,
        ..Default::default()
      },
    );
    Network::new(spotify, ClientConfig::default(), app)
  }

  #[test]
  fn apply_saved_flags_reports_change_only_on_mutation() {
    let mut set = HashSet::new();
    let ids = vec!["a".to_string(), "b".to_string()];
    // Fresh likes insert -> changed.
    assert!(apply_saved_flags(&mut set, &ids, &[true, false]));
    assert!(set.contains("a"));
    // Identical re-check -> no change, no wasted repaint.
    assert!(!apply_saved_flags(&mut set, &ids, &[true, false]));
    // Unlike removes -> changed.
    assert!(apply_saved_flags(&mut set, &ids, &[false, false]));
    assert!(!set.contains("a"));
    // Short answer vector ignores the tail without touching the set.
    assert!(!apply_saved_flags(&mut set, &ids, &[false]));
  }

  /// Construct a mock track JSON value with the given index.
  fn mock_track_json(i: usize) -> serde_json::Value {
    json!({
      "id": format!("mocktrack{}", i),
      "uri": format!("spotify:track:mocktrack{}", i),
      "name": format!("Mock Song {}", i),
      "track_number": i + 1,
      "disc_number": 1,
      "duration_ms": 200_000,
      "explicit": false,
      "is_playable": true,
      "is_local": false,
      "popularity": 0,
      "preview_url": null,
      "type": "track",
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "artists": [],
      "album": {
        "id": "mockalbum0",
        "uri": "spotify:album:mockalbum0",
        "name": "Mock Album",
        "artists": [],
        "available_markets": [],
        "external_urls": {},
        "href": null,
        "images": [],
        "release_date": "2024-01-01",
        "release_date_precision": "day",
        "album_type": "album",
        "type": "album",
      },
    })
  }

  /// Build a Page<SavedTrack> of 20 items for the given page offset.
  fn mock_saved_tracks_page(offset: u32) -> Page<SavedTrack> {
    let total = 20u32;
    let end = std::cmp::min(offset + 20, total);
    let items: Vec<SavedTrack> = (offset..end)
      .map(|i| {
        let track: FullTrack = serde_json::from_value(mock_track_json(i as usize + 130)).unwrap();
        SavedTrack {
          added_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).single().unwrap(),
          track,
        }
      })
      .collect();
    Page {
      href: String::new(),
      items,
      limit: 20,
      next: if end < total {
        Some(format!("?offset={}", end))
      } else {
        None
      },
      offset,
      previous: if offset > 0 {
        Some("?offset=0".to_string())
      } else {
        None
      },
      total,
    }
  }

  #[test]
  fn unavailable_tracks_do_not_count_toward_playback_offsets() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = test_network(&app);

    let mut blocked = mock_track_json(1);
    blocked["is_playable"] = serde_json::Value::Bool(false);
    let item = |track: serde_json::Value| -> PlaylistItem {
      serde_json::from_value(json!({
        "added_at": null,
        "added_by": null,
        "is_local": false,
        "track": track,
      }))
      .unwrap()
    };
    let page = Page {
      href: String::new(),
      items: vec![
        item(mock_track_json(0)),
        item(blocked),
        item(mock_track_json(2)),
      ],
      limit: 3,
      next: None,
      offset: 0,
      previous: None,
      total: 3,
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
      network.set_playlist_tracks_to_table(&page, false).await;
    });
    let app = app.try_lock().unwrap();
    assert_eq!(app.track_table.tracks.len(), 3);
    assert_eq!(app.track_table_raw_index, vec![0, 1, 1]);
    assert_eq!(app.track_table.tracks[0].name, "Mock Song 0");
    assert_eq!(app.track_table.tracks[1].name, "— Unavailable —");
    assert_eq!(app.track_table.tracks[2].name, "Mock Song 2");
  }

  #[test]
  fn non_track_items_keep_their_playback_slot() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = test_network(&app);

    let item = |track: serde_json::Value| -> PlaylistItem {
      serde_json::from_value(json!({
        "added_at": null,
        "added_by": null,
        "is_local": false,
        "track": track,
      }))
      .unwrap()
    };
    let not_a_track: PlaylistItem = serde_json::from_value(json!({
      "added_at": null,
      "added_by": null,
      "is_local": false,
      "track": null,
    }))
    .unwrap();
    let page = Page {
      href: String::new(),
      items: vec![
        item(mock_track_json(0)),
        not_a_track,
        item(mock_track_json(2)),
      ],
      limit: 3,
      next: None,
      offset: 0,
      previous: None,
      total: 3,
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
      network.set_playlist_tracks_to_table(&page, false).await;
    });
    let app = app.try_lock().unwrap();
    assert_eq!(app.track_table.tracks.len(), 3);
    assert_eq!(app.track_table_raw_index, vec![0, 1, 2]);
    assert_eq!(app.track_table.tracks[0].name, "Mock Song 0");
    assert_eq!(app.track_table.tracks[1].name, "— Unavailable —");
    assert_eq!(app.track_table.tracks[2].name, "Mock Song 2");
  }

  #[test]
  fn opening_cached_playlist_dispatches_background_reconcile() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = test_network(&app);
    let dir = std::env::temp_dir().join(format!("sptune_bk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    network
      .playlist_cache
      .set_path(dir.join("playlist_cache.json"));

    let item: PlaylistItem = serde_json::from_value(json!({
      "added_at": "2026-01-01T00:00:00Z",
      "added_by": null,
      "is_local": false,
      "track": mock_track_json(0),
    }))
    .unwrap();
    network
      .playlist_cache
      .update("spotify:playlist:test", vec![item], 1, 0, false);

    let (tx, rx) = std::sync::mpsc::channel();
    app.try_lock().unwrap().io_tx = Some(tx);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
      network
        .get_playlist_tracks("spotify:playlist:test".to_string(), 0)
        .await;
    });

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert!(
      dispatched.iter().any(
        |e| matches!(e, IoEvent::ReconcilePlaylistTracks(id) if id == "spotify:playlist:test")
      ),
      "opening a cached playlist must dispatch a background reconcile"
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// One sidebar playlist for view-guard tests (bare id; uri() -> full URI).
  fn mock_sidebar_playlist(id: &str) -> rspotify::model::SimplifiedPlaylist {
    serde_json::from_value(json!({
      "collaborative": false,
      "external_urls": {},
      "href": format!("spotify:playlist:{id}"),
      "id": id,
      "images": [],
      "name": format!("Mock {id}"),
      "owner": {
        "display_name": "Mock User",
        "external_urls": {},
        "followers": null,
        "href": "spotify:user:mockuser",
        "id": "mockuser",
        "images": [],
      },
      "public": null,
      "snapshot_id": "snap",
      "tracks": { "href": "", "total": 1 },
      "type": "playlist",
    }))
    .unwrap()
  }

  fn mock_playlist_items(range: std::ops::Range<usize>) -> Vec<PlaylistItem> {
    range
      .map(|i| {
        serde_json::from_value(json!({
          "added_at": null,
          "added_by": null,
          "is_local": false,
          "track": mock_track_json(i),
        }))
        .unwrap()
      })
      .collect()
  }

  fn mock_items_page(items: Vec<PlaylistItem>, total: u32) -> Page<PlaylistItem> {
    Page {
      href: String::new(),
      items,
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total,
    }
  }

  #[test]
  fn stale_first_page_updates_cache_only() {
    // Open A, then B: A's late first page must not clobber B's table.
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = test_network(&app);
    let dir = std::env::temp_dir().join(format!("sptune_bk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    network
      .playlist_cache
      .set_path(dir.join("playlist_cache.json"));
    {
      let mut guard = app.try_lock().unwrap();
      guard.track_table.context = Some(TrackTableContext::MyPlaylists);
      guard.playlists = Some(Page {
        href: String::new(),
        items: vec![mock_sidebar_playlist("B")],
        limit: 50,
        next: None,
        offset: 0,
        previous: None,
        total: 1,
      });
      guard.active_playlist_index = Some(0);
      guard.playlist_tracks = Some(mock_items_page(mock_playlist_items(0..3), 3));
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
      network
        .apply_playlist_tracks(
          "spotify:playlist:A",
          mock_items_page(mock_playlist_items(100..103), 3),
          0,
        )
        .await;
    });
    let guard = app.try_lock().unwrap();
    // UI still shows B (first item is track 0, not track 100)...
    let shown = guard.playlist_tracks.as_ref().unwrap();
    assert_eq!(shown.items.len(), 3);
    assert_eq!(
      playlist_item_uri(&shown.items[0]).as_deref(),
      Some("spotify:track:mocktrack0")
    );
    // ...A's page landed in the keyed cache, and no fill was armed for it.
    assert_eq!(
      network
        .playlist_cache
        .lookup("spotify:playlist:A")
        .unwrap()
        .items
        .len(),
      3
    );
    assert!(guard.fill_target.is_none());
    drop(guard);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn overlapping_pages_converge_without_dupes() {
    // The same offset-50 page applied twice (double dispatch) lands on its
    // own offset instead of appending a second copy.
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = test_network(&app);
    let dir = std::env::temp_dir().join(format!("sptune_bk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    network
      .playlist_cache
      .set_path(dir.join("playlist_cache.json"));
    {
      let mut guard = app.try_lock().unwrap();
      guard.track_table.context = Some(TrackTableContext::MyPlaylists);
      guard.playlists = Some(Page {
        href: String::new(),
        items: vec![mock_sidebar_playlist("A")],
        limit: 50,
        next: None,
        offset: 0,
        previous: None,
        total: 1,
      });
      guard.active_playlist_index = Some(0);
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
      network
        .apply_playlist_tracks(
          "spotify:playlist:A",
          mock_items_page(mock_playlist_items(0..50), 120),
          0,
        )
        .await;
      let page50 = mock_items_page(mock_playlist_items(50..100), 120);
      network
        .apply_playlist_tracks("spotify:playlist:A", page50.clone(), 50)
        .await;
      network
        .apply_playlist_tracks("spotify:playlist:A", page50, 50)
        .await;
    });
    let guard = app.try_lock().unwrap();
    assert_eq!(guard.playlist_tracks.as_ref().unwrap().items.len(), 100);
    assert_eq!(
      network
        .playlist_cache
        .lookup("spotify:playlist:A")
        .unwrap()
        .items
        .len(),
      100
    );
    drop(guard);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn reconcile_probes_each_playlist_once_per_launch() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = test_network(&app);
    let dir = std::env::temp_dir().join(format!("sptune_bk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    network
      .playlist_cache
      .set_path(dir.join("playlist_cache.json"));
    network.playlist_cache.update(
      "spotify:playlist:test",
      mock_playlist_items(0..1),
      1,
      0,
      false,
    );
    network
      .playlist_cache
      .set_snapshot("spotify:playlist:test", "snap-old".to_string());
    // Already probed this launch: no second probe, even with a stale
    // snapshot and no network assertion possible here.
    network
      .probed_playlists
      .insert("spotify:playlist:test".to_string());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let changed = rt.block_on(async {
      network
        .reconcile_playlist_tracks("spotify:playlist:test")
        .await
    });
    assert!(!changed);
    // A fresh network records the probe even for a missing entry, so the
    // next open in the same launch skips it.
    let mut fresh = test_network(&app);
    let missing = rt.block_on(async {
      fresh
        .reconcile_playlist_tracks("spotify:playlist:nope")
        .await
    });
    assert!(!missing);
    assert!(fresh.probed_playlists.contains("spotify:playlist:nope"));
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn verified_add_page_contains_finds_song() {
    // Same PlaylistItem shape as playlist_cache tests (proven to extract
    // uris via playlist_item_uri).
    let item = |uri: &str| {
      let mut track = mock_track_json(0);
      track["id"] = serde_json::json!(uri.rsplit(':').next().unwrap());
      track["uri"] = serde_json::json!(uri);
      serde_json::from_value::<PlaylistItem>(serde_json::json!({
        "added_at": null,
        "added_by": null,
        "is_local": false,
        "item": {
          "album": track["album"],
          "artists": [],
          "available_markets": [],
          "disc_number": 1,
          "duration_ms": 1000,
          "explicit": false,
          "external_ids": {},
          "external_urls": {},
          "href": null,
          "id": track["id"],
          "is_local": false,
          "is_playable": true,
          "name": "Track",
          "popularity": 0,
          "preview_url": null,
          "track_number": 1,
          "type": "track",
          "uri": track["uri"]
        }
      }))
      .unwrap()
    };
    let page = Page::<PlaylistItem> {
      href: String::new(),
      items: vec![
        item("spotify:track:mocktrack1"),
        item("spotify:track:mocktrack2"),
      ],
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total: 2,
    };
    assert!(playlist_page_contains(&page, "spotify:track:mocktrack2"));
    assert!(!playlist_page_contains(&page, "spotify:track:missing"));
    assert!(!playlist_page_contains(
      &Page::<PlaylistItem> {
        items: vec![],
        total: 0,
        ..page
      },
      "spotify:track:mocktrack1"
    ));
  }

  #[test]
  fn merge_smart_shuffle_cases() {
    // API-true sticks even with no local flag (Smart enabled elsewhere).
    assert!(merge_smart_shuffle(true, false, true));
    // Local Smart survives while shuffle stays on.
    assert!(merge_smart_shuffle(false, true, true));
    // Shuffle-off clears both: no stale Smart label.
    assert!(!merge_smart_shuffle(false, true, false));
    assert!(!merge_smart_shuffle(false, false, false));
    assert!(!merge_smart_shuffle(false, false, true));
  }

  #[test]
  fn saved_tracks_page_is_a_partial_subset() {
    let page = mock_saved_tracks_page(0);
    assert_eq!(page.total, 20);
    assert!(page.next.is_none());
    let ids: Vec<&str> = page
      .items
      .iter()
      .map(|t| t.track.id.as_ref().unwrap().id())
      .collect();
    assert_eq!(ids.len(), 20);
    assert_eq!(ids[0], "mocktrack130");
    assert_eq!(ids[1], "mocktrack131");
    assert_eq!(ids[2], "mocktrack132");
    assert_eq!(ids[19], "mocktrack149");
    let page2 = mock_saved_tracks_page(15);
    assert_eq!(page2.items.len(), 5);
    assert_eq!(page2.total, 20);
    assert!(page2.next.is_none());
  }

  #[test]
  fn track_numbers_are_sequential() {
    let v11 = mock_track_json(11);
    let v12 = mock_track_json(12);
    assert_eq!(v11["track_number"], 12);
    assert_eq!(v12["track_number"], 13);
  }

  #[test]
  fn gear_settings_round_trip() {
    let tmp = std::env::temp_dir().join(format!("sptune-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    std::env::set_var("HOME", &tmp);
    let app = Arc::new(Mutex::new(App::default()));
    let rt = tokio::runtime::Builder::new_current_thread()
      .build()
      .unwrap();
    rt.block_on(async {
      let mut app_guard = app.lock().await;
      app_guard.show_library = false;
      app_guard.sidebar_minimized = true;
      app_guard.user_config.theme.background = Color::Rgb(0, 0, 0);
      SavedState::snapshot_from_app(&app_guard).save();
    });
    let saved = SavedState::load().expect("state file should exist");
    assert_eq!(saved.show_library, Some(false));
    assert_eq!(saved.show_playlists, Some(true));
    assert_eq!(saved.sidebar_minimized, Some(true));
    assert_eq!(saved.volume_ramp_bar, Some(false));
    assert_eq!(saved.black_background, Some(true));
    assert_eq!(saved.restore_settings, Some(false));
  }

  #[test]
  fn parse_lrc_handles_common_timestamps_and_metadata() {
    let lrc = "[ti:Bohemian Rhapsody]\n\
      [ar:Queen]\n\
      [00:00.15]Is this the real life?\n\
      [00:07.13]Caught in a landslide\n\
      [01:02]Minute marker\n\
      [01:02.500]Same minute, more precision\n\
      no tag line";
    let parsed = parse_lrc(lrc);
    assert_eq!(
      parsed,
      vec![
        (150, "Is this the real life?".to_string()),
        (7_130, "Caught in a landslide".to_string()),
        (62_000, "Minute marker".to_string()),
        (62_500, "Same minute, more precision".to_string()),
      ]
    );
    // Untagged trailing line and metadata tags are dropped.
    assert!(!parsed.iter().any(|(_, w)| w == "no tag line"));
  }

  #[test]
  fn partner_track_json_maps_to_full_track() {
    let data = json!({
      "__typename": "Track",
      "uri": "spotify:track:abc123",
      "name": "Bohemian Rhapsody",
      "contentRating": { "label": "EXPLICIT" },
      "playability": { "playable": true },
      "trackDuration": { "totalMilliseconds": 354000 },
      "discNumber": 1,
      "trackNumber": 5,
      "albumOfTrack": {
        "uri": "spotify:album:alb999",
        "name": "A Night at the Opera",
        "artists": { "items": [ { "uri": "spotify:artist:art1", "profile": { "name": "Queen" } } ] },
        "coverArt": { "sources": [ { "url": "https://example.com/cover.jpg" } ] }
      },
      "artists": { "items": [ { "uri": "spotify:artist:art1", "profile": { "name": "Queen" } } ] }
    });
    let track = partner_track_from_value(&data).unwrap().expect("track");
    assert_eq!(track.name, "Bohemian Rhapsody");
    assert!(track.explicit);
    assert_eq!(track.duration.num_milliseconds(), 354000);
    assert_eq!(track.track_number, 5);
    assert_eq!(track.album.name, "A Night at the Opera");
    assert_eq!(track.artists.len(), 1);
    assert_eq!(track.artists[0].name, "Queen");
    assert_eq!(
      track.id.as_ref().map(|id| id.to_string()).as_deref(),
      Some("spotify:track:abc123")
    );
  }

  #[test]
  fn partner_track_json_skips_non_track_and_unplayable() {
    let episode = json!({ "__typename": "Episode", "uri": "spotify:episode:e1", "name": "x" });
    assert!(partner_track_from_value(&episode).unwrap().is_none());
    let unplayable = json!({
      "__typename": "Track",
      "uri": "spotify:track:t1",
      "name": "x",
      "playability": { "playable": false },
      "trackDuration": { "totalMilliseconds": 1000 },
      "albumOfTrack": { "uri": "spotify:album:a1", "name": "a" },
      "artists": { "items": [] }
    });
    let track = partner_track_from_value(&unplayable).unwrap().unwrap();
    assert_eq!(track.is_playable, Some(false));
  }
}
