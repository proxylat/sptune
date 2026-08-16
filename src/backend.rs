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
use ratatui::style::Color;
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use rspotify::{
  clients::{BaseClient, OAuthClient},
  model::{
    AdditionalType, AlbumId, AlbumType, ArtistId, Country, CurrentPlaybackContext, CursorBasedPage,
    Device, EpisodeId, FullArtist, FullTrack, Id, LibraryId, Market, Offset, Page,
    PlayContextId,
    PlayHistory, PlayableId, PlayableItem, PlaylistId, PlaylistItem, PrivateUser,
    Recommendations,
    RecommendationsAttribute, RepeatState, SavedAlbum, SavedTrack, SearchResult, SearchType,
    Show, ShowId, SimplifiedAlbum, SimplifiedPlaylist, SimplifiedShow, Token, TrackId,
    audio::AudioAnalysis,
  },
  AuthCodeSpotify, Config, Credentials, OAuth,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
  collections::HashMap,
  fs,
  path::PathBuf,
  sync::Arc,
  time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;

mod mock;
use mock::MockState;

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

/// (items_len, total, offset, limit) of a page, for paging decisions.
fn page_meta<T: serde::de::DeserializeOwned>(page: Option<&Page<T>>) -> Option<(usize, u32, u32, u32)> {
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
  let mut seen = items.iter().map(|item| key(item)).collect::<std::collections::HashSet<_>>();
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
  GetSearchResults(String, Option<Country>),
  GetMoreSearchResults(SearchResultBlock),
  SetTracksToTable(Vec<FullTrack>),
  GetMadeForYouPlaylistItems(String, u32),
  GetPlaylistItems(String, u32),
  LoadAllPlaylistItems(String),
  ReconcilePlaylistTracks(String),
  GetCurrentSavedTracks(Option<u32>),
  StartPlayback(Option<String>, Option<Vec<String>>, Option<usize>),
  UpdateSearchLimits(u32, u32),
  Seek(u32),
  NextTrack,
  PreviousTrack,
  Shuffle(bool),
  Repeat(RepeatState),
  PausePlayback,
  ChangeVolume(u8),
  GetArtist(String, String, Option<Country>),
  GetAlbumTracks(Box<SimplifiedAlbum>),
  GetAlbumTracksMore(String, u32),
  GetRecommendationsForSeed(
    Option<Vec<String>>,
    Option<Vec<String>>,
    Box<Option<FullTrack>>,
    Option<Country>,
  ),
  GetCurrentUserSavedAlbums(Option<u32>),
  CurrentUserSavedAlbumsContains(Vec<String>),
  CurrentUserSavedAlbumDelete(String),
  CurrentUserSavedAlbumAdd(String),
  UserUnfollowArtists(Vec<String>),
  UserFollowArtists(Vec<String>),
  UserFollowPlaylist(String, String, Option<bool>),
  UserUnfollowPlaylist(String, String),
  MadeForYouExpand(String, usize),
  GetAudioAnalysis(String),
  GetAudioFeatures(String),
  GetLyrics,
  GetMonthlyListeners(String),
  GetTrackCredits(String),
  GetQueue,
  GetArtistAlbumsMore(String, u32),
  GetArtistTopTracksMore(String, String, u32),
  GetUser,
  RefreshUser,
  ToggleSaveTrack(String),
  GetRecommendationsForTrackId(String, Option<Country>),
  GetRecentlyPlayed,
  GetMoreRecentlyPlayed(Option<String>),
  GetFollowedArtists(Option<String>),
  SetArtistsToTable(Vec<FullArtist>),
  UserArtistFollowCheck(Vec<String>),
  GetAlbum(String),
  TransferPlaybackToDevice(String),
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
  SaveState,
  CleanCache,
  RefreshPlaylists,
  RefreshSavedTracks,
  RefreshSavedAlbums,
  RefreshSavedShows,
  RefreshPlaylistTracks(String),
  ResumeState(SavedState),
}

pub fn get_spotify(token: Token, client_config: &ClientConfig) -> (AuthCodeSpotify, SystemTime) {
  let token_expiry = token
    .expires_at
    .map(|t| SystemTime::from(t - chrono::Duration::try_seconds(10).unwrap()))
    .unwrap_or_else(SystemTime::now);

  let creds = Credentials::new(&client_config.client_id, &client_config.client_secret);
  let spotify =
    AuthCodeSpotify::from_token_with_config(token, creds, OAuth::default(), Config::default());
  (spotify, token_expiry)
}

#[derive(Clone)]
pub struct Network<'a> {
  pub spotify: AuthCodeSpotify,
  large_search_limit: u32,
  small_search_limit: u32,
  pub client_config: ClientConfig,
  pub app: &'a Arc<Mutex<App>>,
  mock: bool,
  mock_state: MockState,
  playlist_cache: PlaylistCache,
  library_cache: LibraryCache,
  saved_checked: HashMap<String, Instant>,
  last_api_call: Instant,
}


/// Last session's volume + track, persisted so a restart picks up where the
/// previous session left off. Real and mock mode use separate files so mock
/// testing never clobbers a real session.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedState {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub volume: Option<u8>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub track_uri: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub is_playing: Option<bool>,
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
  pub resume_track: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub restore_settings: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_library: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_playlists: Option<bool>,
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
  pub visualizer_style: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub enable_add_to_playlist: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub show_liked_icon: Option<bool>,
}

impl SavedState {
  fn file_path(mock: bool) -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
      home
        .join(".config")
        .join("sptune")
        .join(if mock { "state.mock.json" } else { "state.json" })
    })
  }

  pub fn load(mock: bool) -> Option<SavedState> {
    let path = SavedState::file_path(mock)?;
    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
  }

  pub fn save(&self, mock: bool) {
    let Some(path) = SavedState::file_path(mock) else { return };
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
      if let Some(value) = &self.track_uri {
        obj.insert("track_uri".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.is_playing {
        obj.insert("is_playing".to_string(), serde_json::json!(value));
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
      if let Some(value) = &self.resume_track {
        obj.insert("resume_track".to_string(), serde_json::json!(value));
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
        obj.insert("show_date_added_column".to_string(), serde_json::json!(value));
      }
      if let Some(value) = &self.visualizer_style {
        obj.insert("visualizer_style".to_string(), serde_json::json!(value));
      }
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

const API_PACE: Duration = Duration::from_millis(150);
const SAVED_CHECK_TTL: Duration = Duration::from_secs(300);

impl<'a> Network<'a> {
  pub fn new(
    spotify: AuthCodeSpotify,
    client_config: ClientConfig,
    app: &'a Arc<Mutex<App>>,
  ) -> Self {
    Network {
      spotify,
      large_search_limit: 20,
      small_search_limit: 4,
      client_config,
      app,
      mock: false,
      mock_state: MockState::default(),
      playlist_cache: PlaylistCache::new(),
      library_cache: LibraryCache::new(),
      saved_checked: HashMap::new(),
      last_api_call: Instant::now(),
    }
  }

  pub fn new_mock(
    spotify: AuthCodeSpotify,
    client_config: ClientConfig,
    app: &'a Arc<Mutex<App>>,
  ) -> Self {
    Network {
      spotify,
      large_search_limit: 30,
      small_search_limit: 4,
      client_config,
      app,
      mock: true,
      mock_state: MockState::default(),
      playlist_cache: PlaylistCache::new(),
      library_cache: LibraryCache::new(),
      saved_checked: HashMap::new(),
      last_api_call: Instant::now(),
    }
  }

  /// Throttle outbound API calls so bursts (opening playlists, search tab
  /// expands) stay under Spotify's rate limit instead of tripping 429s.
  async fn pace(&mut self) {
    let elapsed = self.last_api_call.elapsed();
    if elapsed < API_PACE {
      tokio::time::sleep(API_PACE - elapsed).await;
    }
    self.last_api_call = Instant::now();
  }

  #[allow(clippy::cognitive_complexity)]
  pub async fn handle_network_event(&mut self, io_event: IoEvent) {
    let text = format!("{:?}", io_event);
    self.app.lock().await.log_request(text);
    if self.mock {
      self.handle_mock_event(io_event).await;
      let mut app = self.app.lock().await;
      app.is_loading = false;
      return;
    }
    self.pace().await;
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
      IoEvent::GetSearchResults(search_term, country) => {
        self.get_search_results(search_term, country).await;
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
          // Re-serving replaces the table and resets the sort (set_tracks_to_table
          // replace sets track_table_sort = None); re-apply the user's column sort
          // so the corrected list doesn't flip back to raw order under them.
          let mut app = self.app.lock().await;
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
      IoEvent::ResumeState(saved) => {
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
      IoEvent::GetCurrentSavedTracks(offset) => {
        self.get_current_user_saved_tracks(offset).await;
      }
      IoEvent::StartPlayback(context_uri, uris, offset) => {
        self.start_playback(context_uri, uris, offset).await;
      }
      IoEvent::UpdateSearchLimits(large_search_limit, small_search_limit) => {

        // (Feb 2026), playlist pagination still allows 50.
        self.large_search_limit = large_search_limit.min(50);
        self.small_search_limit = small_search_limit.min(10);
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
      IoEvent::GetArtist(artist_id, input_artist_name, country) => {
        self.get_artist(artist_id, input_artist_name, country).await;
      }
      IoEvent::GetAlbumTracks(album) => {
        self.get_album_tracks(album).await;
      }
      IoEvent::GetAlbumTracksMore(album_id, offset) => {
        self.get_album_tracks_more(album_id, offset).await;
      }
      IoEvent::GetRecommendationsForSeed(seed_artists, seed_tracks, first_track, country) => {
        self
          .get_recommendations_for_seed(seed_artists, seed_tracks, first_track, country)
          .await;
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
      IoEvent::MadeForYouExpand(name, slot) => {
        self.made_for_you_expand(name, slot).await;
      }
      IoEvent::GetAudioAnalysis(uri) => {
        self.get_audio_analysis(uri).await;
      }
      IoEvent::GetAudioFeatures(uri) => {
        self.get_audio_features(uri).await;
      }
      IoEvent::GetLyrics => {
        self.get_lyrics().await;
      }
      IoEvent::GetMonthlyListeners(artist_id) => {
        self.get_monthly_listeners(artist_id).await;
      }
      IoEvent::GetTrackCredits(track_id) => {
        self.get_track_credits(track_id).await;
      }
      IoEvent::GetQueue => {
        self.get_queue().await;
      }
      IoEvent::GetArtistAlbumsMore(artist_id, offset) => {
        self.get_artist_albums_more(artist_id, offset).await;
      }
      IoEvent::GetArtistTopTracksMore(artist_id, artist_name, offset) => {
        self.get_artist_top_tracks_more(artist_id, artist_name, offset).await;
      }
      IoEvent::ToggleSaveTrack(track_id) => {
        self.toggle_save_track(track_id).await;
      }
      IoEvent::GetRecommendationsForTrackId(track_id, country) => {
        self
          .get_recommendations_for_track_id(track_id, country)
          .await;
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
      IoEvent::SaveState => {
        let app = self.app.lock().await;
        self.save_settings_from_app(&app);
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



  /// Persist the current volume/track snapshot so the next launch can resume it.
  fn save_state_from_app(&self, app: &App) {
    let mut volume = None;
    let mut track_uri = None;
    let mut is_playing = None;
    let mut shuffle = None;
    let mut repeat = None;
    if let Some(context) = &app.current_playback_context {
      volume = context.device.volume_percent.map(|v| v as u8);
      is_playing = Some(context.is_playing);
      shuffle = Some(context.shuffle_state);
      repeat = Some(
        match context.repeat_state {
          RepeatState::Off => "off",
          RepeatState::Context => "context",
          RepeatState::Track => "track",
        }
        .to_string(),
      );
      track_uri = match &context.item {
        Some(PlayableItem::Track(track)) => track.id.as_ref().map(|id| id.uri()),
        _ => None,
      };
    }
    let track_sort = app.track_table_sort.map(|(column, desc)| (sort_column_name(column).to_string(), desc));
    // keep the current page if the sort reset cleared it (the resume event
    // re-applies the saved sort after the fetch, so this only shapes what
    // `save_state_from_app` sees next run)
    let last_page = {
      let mut page = None;
      if app.get_current_route().id == RouteId::TrackTable
        && app.track_table.context == Some(TrackTableContext::MyPlaylists)
      {
        if let (Some(playlists), Some(index)) = (&app.playlists, app.selected_playlist_index) {
          if let Some(playlist) = playlists.items.get(index) {
            page = Some(format!("playlist:{}", playlist.id.to_string()));
          }
        }
      }
      page
    };
    SavedState {
      volume,
      track_uri,
      is_playing,
      shuffle,
      repeat,
      track_sort,
      last_page,
      // Playback-path saves must NOT touch the settings fields: a stale
      // process (or an interleaved save) would otherwise clobber the gear
      // menu choices. Restore treats null as "keep current".
      mouse_enabled: None,
      theme_preset: None,
      seek_by_typing: None,
      show_library: None,
      show_playlists: None,
      volume_ramp_bar: None,
      black_background: None,
      show_album_column: None,
      show_artist_column: None,
      show_length_column: None,
      show_date_added_column: None,
      resume_track: None,
      restore_settings: None,
      visualizer_style: None,
      enable_add_to_playlist: None,
      show_liked_icon: None,
    }
    .save(self.mock);
  }

  /// Persist the gear-menu settings in isolation, leaving the volume/track
  /// fields untouched.
  fn save_settings_from_app(&self, app: &App) {
    SavedState {
      volume: None,
      track_uri: None,
      is_playing: None,
      shuffle: None,
      repeat: None,
      track_sort: None,
      last_page: None,
      seek_by_typing: Some(app.user_config.behavior.seek_by_typing),
      resume_track: Some(app.user_config.behavior.resume_track),
      restore_settings: Some(app.user_config.behavior.restore_settings),
      mouse_enabled: Some(app.user_config.behavior.enable_mouse),
      theme_preset: app.theme_preset_index.and_then(|i| {
        theme_presets()
          .get(i)
          .map(|(name, _)| name.to_string())
      }),
      show_library: Some(app.show_library),
      show_playlists: Some(app.show_playlists),
      volume_ramp_bar: Some(app.user_config.behavior.volume_ramp_bar),
      black_background: Some(app.user_config.theme.background == Color::Rgb(0, 0, 0)),
      show_album_column: Some(app.user_config.behavior.show_album_column),
      show_artist_column: Some(app.user_config.behavior.show_artist_column),
      show_length_column: Some(app.user_config.behavior.show_length_column),
      show_date_added_column: Some(app.user_config.behavior.show_date_added_column),
      visualizer_style: Some(app.user_config.behavior.visualizer_style.as_str().to_string()),
      enable_add_to_playlist: Some(app.user_config.behavior.enable_add_to_playlist),
      show_liked_icon: Some(app.user_config.behavior.show_liked_icon),
    }
    .save(self.mock);
  }


  async fn handle_error(&mut self, e: anyhow::Error) {
    let message = e.to_string();
    // On rate-limit errors, push the next pace window out so the app
    // naturally backs off instead of hammering the API.
    if message.contains("429") || message.contains("Too Many Requests") {
      self.last_api_call = Instant::now() - API_PACE + Duration::from_secs(30);
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
        self.library_cache.put("profile", &[user.clone()], 1);
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
        app.current_playback_context = Some(c.clone());
        app.instant_since_last_current_playback_poll = Instant::now();
        // Keep the visualizer's audio data in sync with the playing track;
        // audio features (stable endpoint) are fetched once per track change.
        if let Some(item) = &c.item {
          if let PlayableItem::Track(track) = item {
            let uri = track.id.as_ref().map(|t| t.to_string());
            if let Some(uri) = uri {
              let stale = app
                .audio_features
                .as_ref()
                .map(|(u, _)| u != &uri)
                .unwrap_or(true);
              if stale {
                // The envelope belongs to a previous track: drop it and
                // refetch the audio analysis for the new one. The drawing
                // side also gates on uri so a stale envelope never wins.
                app.audio_envelope = None;
                app.dispatch(IoEvent::GetAudioFeatures(uri.clone()));
                app.dispatch(IoEvent::GetAudioAnalysis(uri));
              }
            }
          }
        }
        self.save_state_from_app(&app);
      }
      Ok(None) => {
        let mut app = self.app.lock().await;
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
    match self.spotify.library_contains(library_ids).await {
      Ok(is_saved_vec) => {
        let mut app = self.app.lock().await;
        for (i, id) in fresh.iter().enumerate() {
          if let Some(is_liked) = is_saved_vec.get(i) {
            if *is_liked {
              app.liked_song_ids_set.insert(id.to_string());
            } else {
              // The song is not liked, so check if it should be removed
              if app.liked_song_ids_set.contains(id) {
                app.liked_song_ids_set.remove(id);
              }
            }
          };
        }
        for id in &fresh {
          self.saved_checked.insert(id.clone(), now);
        }
      }
      // Best-effort: the heart is cosmetic, and Spotify throttles this
      // endpoint hard (503s under load). Leave the TTL untouched so the
      // next playlist open retries instead of showing a dead error screen.
      Err(_) => {}
    }
  }

  /// Manual refresh (from a refresh control), never on screen open: probe for
  /// changes when cached, else cold-fetch. See `refresh_saved_tracks`.
  async fn refresh_playlist_tracks(&mut self, playlist_id: &str) {
    self.playlist_cache.ensure_loaded();
    if self.playlist_cache.lookup(playlist_id).is_some() {
      self.reconcile_playlist_tracks(playlist_id).await;
      self.serve_playlist_cache(playlist_id).await;
    } else {
      self.get_playlist_tracks(playlist_id.to_string(), 0).await;
    }
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
    self.library_cache.ensure_loaded();
    if self.library_cache.get("playlists").is_some() {
      self.reconcile_playlists().await;
      self.serve_playlists_cache().await;
    } else {
      self.get_current_user_playlists().await;
    }
  }

  async fn get_playlist_tracks(&mut self, playlist_id: String, playlist_offset: u32) {
    if playlist_offset == 0 {
      self.playlist_cache.ensure_loaded();
      if self.playlist_cache.lookup(&playlist_id).is_some() && self.serve_playlist_cache(&playlist_id).await {
        // The cache is served as-is, so a stale entry (songs removed elsewhere
        // since it was written) makes every playback offset lie. Probe the live
        // total in the background and refetch the entry when it changed.
        self
          .app
          .lock()
          .await
          .dispatch(IoEvent::ReconcilePlaylistTracks(playlist_id));
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
        None,
        Some(self.large_search_limit),
        Some(playlist_offset),
      )
      .await
    {
      Ok(playlist_tracks) => {
        self
          .set_playlist_tracks_to_table(&playlist_tracks, playlist_offset > 0)
          .await;
        let mut app = self.app.lock().await;
        self.playlist_cache.update(
          &playlist_id,
          playlist_tracks.items.clone(),
          playlist_tracks.total,
          playlist_offset > 0,
        );
        self.sync_playlist_uris(&mut app);
        if playlist_offset > 0 {
          // Keep the cumulative raw item list (incl. episodes) so the
          // load-more flag can compare loaded items against the total.
          if let Some(existing) = app.playlist_tracks.as_mut() {
            existing.items.extend(playlist_tracks.items.clone());
            existing.total = playlist_tracks.total;
          }
        } else {
          app.playlist_tracks = Some(playlist_tracks);
        }
        app.is_fetching_next_page = false;
        app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
        self.app.lock().await.is_fetching_next_page = false;
      }
    }
  }

  /// Load every remaining page of a playlist so a column sort spans the
  /// whole real list, not just the loaded (possibly cached) slice. Pages are
  /// fetched sequentially from the current cumulative length until the total
  /// is reached; each page appends and re-sorts (pacing kept ~150ms/page).
  async fn load_all_playlist_items(&mut self, playlist_id: String) {
    loop {
      let (offset, total) = {
        let app = self.app.lock().await;
        match &app.playlist_tracks {
          Some(p) => (p.items.len() as u32, p.total),
          None => (0, 0),
        }
      };
      if offset >= total {
        let mut app = self.app.lock().await;
        if app.date_added_pending {
          app.materialize_date_added();
        }
        return;
      }
      match self
        .spotify
        .playlist_items_manual(
          PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
          None,
          None,
          Some(self.large_search_limit),
          Some(offset),
        )
        .await
      {
        Ok(page) => {
          self.set_playlist_tracks_to_table(&page, true).await;
          let mut app = self.app.lock().await;
          self.playlist_cache.update(
            &playlist_id,
            page.items.clone(),
            page.total,
            true,
          );
          self.sync_playlist_uris(&mut app);
          if let Some(existing) = app.playlist_tracks.as_mut() {
            existing.items.extend(page.items);
            existing.total = page.total;
          }
        }
        Err(_) => return,
      }
      tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
  }

  /// Serve the cached track list of a playlist (cache never expires).
  async fn serve_playlist_cache(&mut self, playlist_id: &str) -> bool {
    self.playlist_cache.ensure_loaded();
    let Some(entry) = self.playlist_cache.lookup(playlist_id) else {
      return false;
    };
    if entry.items.is_empty() {
      return false;
    }
    // A polluted entry (duplicate items from older sessions' offset-snapshot
    // bug) breaks the raw_index mapping (displayed row -> real playlist
    // position), so serving it makes clicks play the wrong song. Drop it and
    // refetch from scratch.
    if self.playlist_cache.is_polluted(playlist_id) {
      self.playlist_cache.remove(playlist_id);
      return false;
    }
    let partial = entry.items.len() < entry.total as usize;
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
    app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
    // A partial cache entry (older sessions captured only a slice) leaves a
    // perpetual "Load more" row; kick off a background completion so the
    // playlist fills to its real total and the row disappears.
    if partial {
      app.dispatch(IoEvent::LoadAllPlaylistItems(playlist_id.to_string()));
    }
    true
  }

  /// Delta probe on playlist open: fetch the tail past the cached items and
  /// merge only the new ones. Skipped until the list is fully loaded;
  /// skipped silently when the probe fails (cache already shown). A
  /// shrunken total means items were removed elsewhere -> full refetch.
  /// Probe the live playlist against the cached entry. Returns whether the
  /// cache changed (songs removed elsewhere → full refetch; songs added →
  /// append the tail pages). The probe compares the first page's item URIs
  /// against the cached ones — totals alone miss playlists whose first items
  /// changed (e.g. songs removed from the top and the same count added at
  /// the bottom, which keeps the total identical).
  /// Probe the live playlist against the cached entry. Returns whether the
  /// cache changed (then the caller re-serves it). Spotify changes a
  /// playlist's snapshot id on ANY edit, so one cheap request tells whether
  /// the cached items/order are stale — mid-list removals/additions that
  /// keep the total identical were invisible to the old first-page compare.
  async fn reconcile_playlist_tracks(&mut self, playlist_id: &str) -> bool {
    let Some(entry) = self.playlist_cache.lookup(playlist_id) else {
      return false;
    };
    let cached_len = entry.items.len() as u32;
    if cached_len == 0 || cached_len != entry.total {
      return false;
    }
    let live_snapshot = match self
      .spotify
      .playlist(PlaylistId::from_id_or_uri(playlist_id).unwrap(), None, None)
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
    self.refetch_playlist_tracks(playlist_id).await;
    self.playlist_cache.set_snapshot(playlist_id, live_snapshot);
    true
  }

  /// Full sequential refetch of a playlist's tracks (items removed
  /// elsewhere), replacing the cached list.
  async fn refetch_playlist_tracks(&mut self, playlist_id: &str) {
    let mut all: Vec<PlaylistItem> = Vec::new();
    loop {
      let page = match self
        .spotify
        .playlist_items_manual(
          PlaylistId::from_id_or_uri(playlist_id).unwrap(),
          None,
          None,
          Some(self.large_search_limit),
          Some(all.len() as u32),
        )
        .await
      {
        Ok(p) => p,
        Err(_) => return,
      };
      all.extend(page.items);
      if all.len() as u32 >= page.total {
        self.playlist_cache.update(playlist_id, all, page.total, false);
        let mut app = self.app.lock().await;
        self.sync_playlist_uris(&mut app);
        return;
      }
    }
  }

  /// Whether a playlist item occupies a position in the player's context.
  /// Spotify skips unavailable tracks (is_playable: false) when indexing a
  /// context, so they must not count toward playback offsets.
  fn playlist_item_is_playable(item: &PlaylistItem) -> bool {
    match item.item.as_ref() {
      Some(PlayableItem::Track(t)) => t.is_playable != Some(false),
      Some(PlayableItem::Episode(_)) => true,
      _ => false,
    }
  }

  async fn set_playlist_tracks_to_table(
    &mut self,
    playlist_track_page: &Page<PlaylistItem>,
    append: bool,
  ) {
    // The player's context index counts only playable items (unavailable
    // tracks are skipped by Spotify), so raw_index must count only playable
    // items too, or every playback offset after an unplayable track is shifted.
    let base = if append {
      let app = self.app.lock().await;
      app
        .playlist_tracks
        .as_ref()
        .map(|existing| {
          existing
            .items
            .iter()
            .filter(|item| Self::playlist_item_is_playable(item))
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
      match item.item.as_ref() {
        Some(PlayableItem::Track(t)) if Self::playlist_item_is_playable(item) => {
          added_at.push(item.added_at);
          tracks.push(t.clone());
          raw_index.push(pos);
          pos += 1;
        }
        Some(PlayableItem::Episode(_)) => pos += 1,
        _ => {}
      }
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
      if app.user_config.behavior.show_liked_icon {
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
    match self
      .spotify
      .playlist_items_manual(
        PlaylistId::from_id_or_uri(&playlist_id).unwrap(),
        None,
        None,
        Some(self.large_search_limit),
        Some(made_for_you_offset),
      )
      .await
    {
      Ok(made_for_you_tracks) => {
        self
          .set_playlist_tracks_to_table(&made_for_you_tracks, made_for_you_offset > 0)
          .await;

        let mut app = self.app.lock().await;
        app.made_for_you_tracks = Some(made_for_you_tracks);
        app.is_fetching_next_page = false;
        if app.get_current_route().id != RouteId::TrackTable {
          app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
        self.app.lock().await.is_fetching_next_page = false;
      }
    }
  }

  async fn get_current_user_saved_shows(&mut self, offset: Option<u32>) {
    if offset.is_none() && self.library_cache.get("saved_shows").is_some() && self.serve_saved_shows_cache().await {
      return;
    }
    match self
      .spotify
      .get_saved_show_manual(Some(self.large_search_limit), offset)
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
      .get_saved_show_manual(Some(self.large_search_limit), None)
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    if page.total < total {
      self.refetch_saved_shows().await;
      return;
    }
    let cached_ids: std::collections::HashSet<String> = cached
      .iter()
      .map(|s| s.show.id.to_string())
      .collect();
    let new_head: Vec<Show> = page
      .items
      .into_iter()
      .take_while(|s| !cached_ids.contains(&s.show.id.to_string()))
      .collect();
    if new_head.is_empty() {
      return;
    }
    if page.total == total + new_head.len() as u32 {
      self.library_cache.prepend("saved_shows", &new_head, page.total);
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
        .get_saved_show_manual(Some(self.large_search_limit), Some(all.len() as u32))
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
      .get_shows_episodes_manual(show_id, None, Some(self.large_search_limit), Some(0))
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
        Some(self.large_search_limit),
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

  async fn get_search_results(&mut self, search_term: String, _country: Option<Country>) {
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

    let (offset, total_override, query, country) = {
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
        Some((items_len, total, page_offset, limit)) if limit > 0 && items_len >= limit as usize => (
          page_offset + self.small_search_limit,
          Some(total),
          app.search_results.query.clone(),
          app.get_user_country(),
        ),
        // First load for this tab: no page yet, start from the top.
        None => (0, None, app.search_results.query.clone(), app.get_user_country()),
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
        country.map(Market::Country),
        None,
        Some(self.small_search_limit),
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
              app.search_results.tracks = Some(merged);
            }
          }
          SearchResultBlock::ArtistSearch => {
            if let SearchResult::Artists(page) = search_result {
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.artists.as_ref(),
                page,
                offset,
                total,
                |artist: &FullArtist| artist.id.to_string(),
              );
              let artist_ids = merged
                .items
                .iter()
                .map(|item| item.id.to_string())
                .collect();
              app.search_results.artists = Some(merged);
              app.dispatch(IoEvent::UserArtistFollowCheck(artist_ids));
            }
          }
          SearchResultBlock::AlbumSearch => {
            if let SearchResult::Albums(page) = search_result {
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
              let album_ids = merged
                .items
                .iter()
                .filter_map(|album| album.id.as_ref().map(|id| id.to_string()))
                .collect();
              app.search_results.albums = Some(merged);
              app.dispatch(IoEvent::CurrentUserSavedAlbumsContains(album_ids));
            }
          }
          SearchResultBlock::PlaylistSearch => {
            if let SearchResult::Playlists(page) = search_result {
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.playlists.as_ref(),
                page,
                offset,
                total,
                |playlist: &SimplifiedPlaylist| playlist.id.to_string(),
              );
              app.search_results.playlists = Some(merged);
            }
          }
          SearchResultBlock::ShowSearch => {
            if let SearchResult::Shows(page) = search_result {
              let total = total_override.unwrap_or(page.total);
              let merged = merge_page(
                app.search_results.shows.as_ref(),
                page,
                offset,
                total,
                |show: &SimplifiedShow| show.id.to_string(),
              );
              let show_ids = merged
                .items
                .iter()
                .map(|show| show.id.to_string())
                .collect();
              app.search_results.shows = Some(merged);
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
    if offset.is_none() && self.library_cache.get("saved_tracks").is_some() && self.serve_saved_tracks_cache().await {
      return;
    }
    match self
      .spotify
      .current_user_saved_tracks_manual(None, Some(self.large_search_limit), offset)
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
      .current_user_saved_tracks_manual(None, Some(self.large_search_limit), None)
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
        !cached_ids
          .contains(&i.track.id.as_ref().map(|id| id.to_string()).unwrap_or_default())
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
          None,
          Some(self.large_search_limit),
          Some(all.len() as u32),
        )
        .await
      {
        Ok(p) => p,
        Err(_) => return,
      };
      all.extend(page.items);
      if all.len() as u32 >= page.total {
        self
          .library_cache
          .put("saved_tracks", &all, page.total);
        return;
      }
    }
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

    let result = match &self.client_config.device_id {
      Some(device_id) => {
        if let Some(context_uri) = context_uri {
          match play_context_from_uri(&context_uri) {
            Some(cid) => self
              .spotify
              .start_context_playback(cid, Some(device_id), offset, None)
              .await
              .map_err(|e| anyhow!(e)),
            None => Err(anyhow!("Invalid context uri")),
          }
        } else if let Some(uris) = uris {
          self
            .spotify
            .start_uris_playback(
              uris.iter().filter_map(|u| playable_from_uri(u)),
              Some(device_id),
              offset,
              None,
            )
            .await
            .map_err(|e| anyhow!(e))
        } else {
          self
            .spotify
            .resume_playback(Some(device_id), None)
            .await
            .map_err(|e| anyhow!(e))
        }
      }
      None => Err(anyhow!("No device_id selected")),
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

  async fn seek(&mut self, position_ms: u32) {
    if let Some(device_id) = &self.client_config.device_id {
      match self
        .spotify
        .seek_track(
          ChronoDuration::milliseconds(position_ms as i64),
          Some(device_id.as_str()),
        )
        .await
      {
        Ok(()) => {
          // Wait between seek and status query.
          // Without it, the Spotify API may return the old progress.
          tokio::time::sleep(Duration::from_millis(1000)).await;
          self.get_current_playback().await;
        }
Err(e) => {
          self.handle_error(anyhow!(e)).await;
        }
      };
    }
  }

  async fn next_track(&mut self) {
    match self
      .spotify
      .next_track(self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => {
        self.get_current_playback().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn previous_track(&mut self) {
    match self
      .spotify
      .previous_track(self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => {
        self.get_current_playback().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  async fn shuffle(&mut self, shuffle_state: bool) {
    match self
      .spotify
      .shuffle(!shuffle_state, self.client_config.device_id.as_deref())
      .await
    {
      Ok(()) => {
        // Update the UI eagerly (otherwise the UI will wait until the next 5 second interval
        // due to polling playback context)
        let mut app = self.app.lock().await;
        if let Some(current_playback_context) = &mut app.current_playback_context {
          current_playback_context.shuffle_state = !shuffle_state;
        };
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
        self.get_current_playback().await;
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    };
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
        self.save_state_from_app(&app);
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
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

  async fn get_artist(
    &mut self,
    artist_id: String,
    input_artist_name: String,
    country: Option<Country>,
  ) {
    let cloneable_artist_id = ArtistId::from_id_or_uri(&artist_id).unwrap();
    let artist_id = cloneable_artist_id.to_string();
    // Spotify removed artist_top_tracks and artist_related_artists in Feb
    // 2026 (related artists 403s); top tracks come from a track search on
    // the artist name, and albums are only fetched when the user opens that
    // tab (GetArtistAlbumsMore), so opening an artist stays a single request.
    let artist_name = if input_artist_name.is_empty() {
      self
        .spotify
        .artist(cloneable_artist_id.clone())
        .await
        .map(|full_artist| full_artist.name)
        .unwrap_or_default()
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
        country.map(Market::Country),
        None,
        Some(10),
        Some(0),
      )
      .await;
    let (mut top_tracks, top_tracks_total) = match top_tracks {
      Ok(SearchResult::Tracks(page)) => (page.items, page.total as usize),
      _ => (vec![], 0),
    };
    // Search returns relevance order, not most-played-first. Per-track
    // playcounts are blocked (api-partner token), so popularity is the
    // closest available proxy for "top" — most popular first.
    top_tracks.sort_by(|a, b| b.popularity.cmp(&a.popularity));
    // Always offer "Load more": the search can under-match the real track
    // count. A short page on the next fetch clears the flag.
    let top_tracks_has_more = true;
    {
      let mut app = self.app.lock().await;
      app.artist = Some(Artist {
        artist_id,
        artist_name,
        albums: Self::empty_page(),
        related_artists: Vec::new(),
        top_tracks,
        top_tracks_total,
        top_tracks_has_more,
        selected_album_index: 0,
        selected_related_artist_index: 0,
        selected_top_track_index: 0,
        artist_hovered_block: ArtistBlock::TopTracks,
        artist_selected_block: ArtistBlock::TopTracks,
      });
    }
  }


  async fn get_artist_albums_more(&mut self, artist_id: String, offset: u32) {
    let cloneable_artist_id = ArtistId::from_id_or_uri(&artist_id).unwrap();
    match self
      .spotify
      .artist_albums_manual(
        cloneable_artist_id,
        std::iter::empty::<AlbumType>(),
        None,
        Some(10),
        Some(offset),
      )
      .await
    {
      Ok(mut page) => {
        let mut app = self.app.lock().await;
        if let Some(artist) = &mut app.artist {
          artist.albums.items.append(&mut page.items);
          artist.albums.offset = page.offset;
          artist.albums.total = page.total;
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_artist_top_tracks_more(&mut self, _artist_id: String, artist_name: String, offset: u32) {
    let query = format!("artist:\"{}\"", artist_name);
    let country = {
      let app = self.app.lock().await;
      app.get_user_country()
    };
    // Limit 10: Spotify capped the search limit at 10 in Feb 2026, larger
    // values 400. The artist search only surfaces a handful of relevant
    // tracks, so load-more pages exhaust quickly — that is the API ceiling.
    match self
      .spotify
      .search(
        &query,
        SearchType::Track,
        country.map(Market::Country),
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
            if track.id.as_ref().map_or(true, |id| seen.insert(id.to_string())) {
              artist.top_tracks.push(track);
            }
          }
          artist.top_tracks_total = page.total as usize;
          artist
            .top_tracks
            .sort_by(|a, b| b.popularity.cmp(&a.popularity));
          // The list was re-sorted and extended: the old selection index no
          // longer names the load-more row (it silently points at a song).
          // Land on top instead of a phantom selection.
          artist.selected_top_track_index = 0;
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
          None,
          Some(self.large_search_limit),
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
        None,
        Some(self.large_search_limit),
        Some(offset),
      )
      .await
    {
      Ok(mut page) => {
        let mut app = self.app.lock().await;
        if let Some(album) = &mut app.selected_album_simplified {
          album.tracks.items.append(&mut page.items);
          album.tracks.offset = page.offset;
          album.tracks.total = page.total;
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  async fn get_recommendations_for_seed(
    &mut self,
    seed_artists: Option<Vec<String>>,
    seed_tracks: Option<Vec<String>>,
    first_track: Box<Option<FullTrack>>,
    country: Option<Country>,
  ) {
    match self
      .spotify
      .recommendations(
        std::iter::empty::<RecommendationsAttribute>(),
        seed_artists.as_ref().map(|v| {
          v.iter()
            .filter_map(|s| ArtistId::from_id_or_uri(s.as_str()).ok())
        }),
        None::<Vec<&str>>,
        seed_tracks.as_ref().map(|v| {
          v.iter()
            .filter_map(|s| TrackId::from_id_or_uri(s.as_str()).ok())
        }),
        country.map(Market::Country),
        Some(self.large_search_limit),
      )
      .await
    {
      Ok(result) => {
        if let Some(mut recommended_tracks) = self.extract_recommended_tracks(&result).await {
          //custom first track
          if let Some(track) = *first_track {
            recommended_tracks.insert(0, track);
          }

          let track_ids = recommended_tracks
            .iter()
            .map(|x| x.id.as_ref().map(|id| id.to_string()).unwrap_or_default())
            .collect::<Vec<String>>();

          let count = recommended_tracks.len();
          self
            .set_tracks_to_table(
              recommended_tracks.clone(),
              vec![None; count],
              (0..count).collect(),
              false,
            )
            .await;

          let mut app = self.app.lock().await;
          app.recommended_tracks = recommended_tracks;
          app.track_table.context = Some(TrackTableContext::RecommendedTracks);

          if app.get_current_route().id != RouteId::Recommendations {
            app.push_navigation_stack(RouteId::Recommendations, ActiveBlock::TrackTable);
          };

          app.dispatch(IoEvent::StartPlayback(None, Some(track_ids), Some(0)));
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  #[allow(deprecated)]
  async fn extract_recommended_tracks(
    &mut self,
    recommendations: &Recommendations,
  ) -> Option<Vec<FullTrack>> {
    let tracks = recommendations
      .clone()
      .tracks
      .into_iter()
      .filter_map(|item| item.id)
      .collect::<Vec<TrackId>>();
    if let Ok(result) = self.spotify.tracks(tracks, None).await {
      return Some(result);
    }

    None
  }

  async fn get_recommendations_for_track_id(&mut self, id: String, country: Option<Country>) {
    if let Ok(track) = self
      .spotify
      .track(TrackId::from_id_or_uri(&id).unwrap(), None)
      .await
    {
      let track_id_list = track.id.as_ref().map(|id| vec![id.to_string()]);
      self
        .get_recommendations_for_seed(None, track_id_list, Box::new(Some(track)), country)
        .await;
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
              self.library_cache.remove("saved_tracks");
              let mut app = self.app.lock().await;
              app.liked_song_ids_set.remove(&track_id);
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
      .current_user_followed_artists(after.as_deref(), Some(self.large_search_limit))
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
    if offset.is_none() && self.library_cache.get("saved_albums").is_some() && self.serve_saved_albums_cache().await {
      return;
    }
    match self
      .spotify
      .current_user_saved_albums_manual(None, Some(self.large_search_limit), offset)
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
      .current_user_saved_albums_manual(None, Some(self.large_search_limit), None)
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    if page.total < total {
      self.refetch_saved_albums().await;
      return;
    }
    let cached_ids: std::collections::HashSet<String> = cached
      .iter()
      .map(|a| a.album.id.to_string())
      .collect();
    let new_head: Vec<SavedAlbum> = page
      .items
      .into_iter()
      .take_while(|a| !cached_ids.contains(&a.album.id.to_string()))
      .collect();
    if new_head.is_empty() {
      return;
    }
    if page.total == total + new_head.len() as u32 {
      self.library_cache.prepend("saved_albums", &new_head, page.total);
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
          None,
          Some(self.large_search_limit),
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

  async fn made_for_you_expand(&mut self, name: String, slot: usize) {
    const SPOTIFY_ID: &str = "spotify";

    match self
      .spotify
      .search(&name, SearchType::Playlist, None, None, Some(10), Some(0))
      .await
    {
      Ok(SearchResult::Playlists(search_playlists)) => {
        let playlist = search_playlists.items.into_iter().find(|playlist| {
          playlist.owner.id.id() == SPOTIFY_ID && playlist.name == name
        });
        if let Some(playlist) = playlist {
          let mut app = self.app.lock().await;
          app.made_for_you_ids[slot] = Some(playlist.id.to_string());
          drop(app);
          self
            .get_made_for_you_playlist_tracks(playlist.id.to_string(), 0)
            .await;
        }
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
      _ => {}
    }
  }

  #[allow(deprecated)]
  /// Fold a track's audio analysis into a fixed 512-slot loudness envelope
/// (0..1 per song-time slot) for the visualizer.
fn audio_analysis_envelope(analysis: &AudioAnalysis) -> Vec<f32> {
  const SLOTS: usize = 512;
  let mut env = vec![0.0f32; SLOTS];
  if analysis.segments.is_empty() {
    return env;
  }
  let total = if analysis.track.duration > 0.0 {
    analysis.track.duration
  } else {
    let last = analysis.segments.last().unwrap();
    last.time_interval.start + last.time_interval.duration
  };
  if total <= 0.0 {
    return env;
  }
  for seg in &analysis.segments {
    let slot = ((seg.time_interval.start / total) * (SLOTS - 1) as f32) as usize;
    let v = ((seg.loudness_max + 60.0) / 60.0).clamp(0.0, 1.0);
    if v > env[slot] {
      env[slot] = v;
    }
  }
  env
}

async fn get_audio_analysis(&mut self, uri: String) {
    match self
      .spotify
      .track_analysis(TrackId::from_uri(&uri).unwrap())
      .await
    {
      Ok(result) => {
        let mut app = self.app.lock().await;
        app.audio_analysis = Some(result.clone());
        app.audio_envelope = Some((
          uri,
          Self::audio_analysis_envelope(&result),
        ));
      }
      Err(_) => {
        // The analysis endpoint is deprecated and 403s on many tracks; stay
        // silent and keep the previous envelope instead of pushing an error
        // panel on every track change.
      }
    }
  }

  async fn get_audio_features(&mut self, uri: String) {
    let track_id = TrackId::from_uri(&uri).unwrap();
    match self.spotify.track_features(track_id).await
    {
      Ok(features) => {
        let mut app = self.app.lock().await;
        app.audio_features = Some((uri, features));
      }
      Err(_) => {
        // The audio-features endpoint is stable (unlike track_analysis);
        // fail silently and keep the previous track's features.
      }
    }
  }

  async fn bearer_token(&self) -> Option<String> {
    match self.spotify.token.lock().await {
      Ok(guard) => match &*guard {
        Some(t) => Some(t.access_token.clone()),
        None => None,
      },
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
      &[
        ("track_name", track_name),
        ("artist_name", artist_name),
      ],
    )?;
    if let Some(ms) = duration_ms {
      url.query_pairs_mut()
        .append_pair("duration", &format!("{}", ms / 1000));
    }
    let res = reqwest::Client::new().get(url).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
      return Ok(None);
    }
    if !res.status().is_success() {
      return Err(anyhow::anyhow!("HTTP {}", res.status()));
    }
    let json: serde_json::Value = res.json().await?;
    let candidates: Vec<&serde_json::Value> = match path {
      "search" => json.as_array().map(|a| a.iter().collect()).unwrap_or_default(),
      _ => vec![&json],
    };
    let hit = candidates.into_iter().find(|c| {
      c.pointer("/instrumental").and_then(|v| v.as_bool()) != Some(true)
    });
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

  async fn get_monthly_listeners(&mut self, artist_id: String) {
    let Some(token) = self.bearer_token().await else {
      return;
    };
    let url = format!(
      "https://spclient.wg.spotify.com/artist-view/v1/artist/{artist_id}"
    );
    let response = reqwest::Client::new()
      .get(&url)
      .header("Authorization", format!("Bearer {token}"))
      .send()
      .await;
    let mut app = self.app.lock().await;
    app.monthly_listeners = match response {
      Ok(res) => match res.json::<serde_json::Value>().await {
        Ok(json) => json
          .pointer("/monthlyListeners")
          .or_else(|| json.pointer("/monthly_listeners"))
          .and_then(|v| v.as_u64()),
        Err(_) => None,
      },
      Err(_) => None,
    };
  }

  async fn get_track_credits(&mut self, track_id: String) {
    let Some(token) = self.bearer_token().await else {
      return;
    };
    let url = format!(
      "https://spclient.wg.spotify.com/track-credits/v1/track/{track_id}"
    );
    let response = reqwest::Client::new()
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
      return;
    }
    let playlists = self
      .spotify
      .current_user_playlists_manual(Some(self.large_search_limit), None)
      .await;

    match playlists {
      Ok(p) => {
        self
          .library_cache
          .put("playlists", &p.items, p.total);
        let mut app = self.app.lock().await;
        app.playlists = Some(p);
      }
      Err(e) => {
        if self.serve_playlists_cache().await {
          return;
        }
        self.handle_error(anyhow!(e)).await;
      }
    };
  }

  /// Serve the cached playlists list; see `serve_saved_tracks_cache`.
  async fn serve_playlists_cache(&mut self) -> bool {
    self.library_cache.ensure_loaded();
    let Some((items, total)) = self.library_cache.get_typed::<SimplifiedPlaylist>("playlists") else {
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

  /// Delta probe on launch; see `reconcile_saved_tracks`.
  async fn reconcile_playlists(&mut self) {
    let Some((cached, total)) = self.library_cache.get_typed::<SimplifiedPlaylist>("playlists")
    else {
      return;
    };
    if cached.len() as u32 != total {
      return;
    }
    let page = match self
      .spotify
      .current_user_playlists_manual(Some(self.large_search_limit), None)
      .await
    {
      Ok(p) => p,
      Err(_) => return,
    };
    if page.total < total {
      self.refetch_playlists().await;
      return;
    }
    let cached_ids: std::collections::HashSet<String> = cached
      .iter()
      .map(|p| p.id.to_string())
      .collect();
    let new_head: Vec<SimplifiedPlaylist> = page
      .items
      .into_iter()
      .take_while(|p| !cached_ids.contains(&p.id.to_string()))
      .collect();
    if new_head.is_empty() {
      return;
    }
    if page.total == total + new_head.len() as u32 {
      self.library_cache.prepend("playlists", &new_head, page.total);
    } else {
      self.refetch_playlists().await;
    }
  }

  /// Full sequential refetch of the playlists list; see `refetch_saved_tracks`.
  async fn refetch_playlists(&mut self) {
    let mut all: Vec<SimplifiedPlaylist> = Vec::new();
    loop {
      let page = match self
        .spotify
        .current_user_playlists_manual(
          Some(self.large_search_limit),
          Some(all.len() as u32),
        )
        .await
      {
        Ok(p) => p,
        Err(_) => return,
      };
      all.extend(page.items);
      if all.len() as u32 >= page.total {
        self.library_cache.put("playlists", &all, page.total);
        return;
      }
    }
  }

  async fn get_recently_played(&mut self) {
    match self
      .spotify
      .current_user_recently_played(Some(self.large_search_limit), None)
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
      .current_user_recently_played(Some(self.large_search_limit), time_limits)
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
              .map_or(false, |id| !seen.insert(id.to_string()));
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
      .album(AlbumId::from_id_or_uri(&album_id).unwrap(), None)
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
      .track(TrackId::from_id_or_uri(&track_id).unwrap(), None)
      .await
    {
      Ok(track) => {
        // It is unclear when the id can ever be None, but perhaps a track can be album-less. If
        // so, there isn't much to do here anyways, since we're looking for the parent album.
        let album_id = match track.album.id {
          Some(id) => id,
          None => return,
        };

        if let Ok(album) = self.spotify.album(album_id, None).await {
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
        self.handle_error(anyhow!(e)).await;
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
      Ok(Some(new_token_info)) => {
        let (new_spotify, new_token_expiry) = get_spotify(new_token_info, &self.client_config);
        self.spotify = new_spotify;
        let mut app = self.app.lock().await;
        app.spotify_token_expiry = new_token_expiry;
      }
      _ => println!("\nFailed to refresh authentication token"),
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

  async fn add_track_to_playlist(&mut self, uri: String, playlist_id: String) {
    let Some(item) = playable_from_uri(&uri) else {
      return;
    };
    let Ok(playlist_id) = PlaylistId::from_id_or_uri(&playlist_id) else {
      return;
    };
    match self
      .spotify
      .playlist_add_items(playlist_id, vec![item], None)
      .await
    {
      Ok(_) => {
        let mut app = self.app.lock().await;
        self.sync_playlist_uris(&mut app);
      }
      Err(e) => {
        self.handle_error(anyhow!(e)).await;
      }
    }
  }

  /// Mirror the playlist cache into the app so the drawing layer can mark
  /// "already in playlist" rows without touching the filesystem or the API.
  fn sync_playlist_uris(&self, app: &mut App) {
    app.playlist_uri_map = self
      .playlist_cache
      .map
      .iter()
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
    loop {
      let Some(open) = rest.find('[') else { break };
      let Some(close) = rest[open + 1..].find(']') else { break };
      let tag = &rest[open + 1..open + 1 + close];
      rest = &rest[open + 1 + close + 1..];
      let Some(colon) = tag.find(':') else { continue };
      let (Ok(min), Ok(sec)) = (tag[..colon].parse::<u128>(), tag[colon + 1..].parse::<f64>())
      else {
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

fn mock_date(i: u32) -> String {
  Utc
    .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
    .single()
    .unwrap()
    .checked_add_signed(ChronoDuration::days(i as i64))
    .unwrap()
    .to_rfc3339()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn new_mock(app: &Arc<Mutex<App>>) -> Network<'_> {
    let spotify = AuthCodeSpotify::with_config(
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
    Network::new_mock(spotify, ClientConfig::default(), app)
  }

  #[test]
  fn audio_analysis_envelope_maps_loud_segments_high() {
    // 2s of analysis, 600 samples: slots 0..256 cover the quiet intro,
    // 256..512 the loud part.
    let analysis: AudioAnalysis = serde_json::from_value(json!({
      "bars": [], "beats": [], "tatums": [], "sections": [],
      "meta": {
        "analyzer_version": "4.0.0", "platform": "Linux", "detailed_status": "OK",
        "status_code": 0, "timestamp": 0, "analysis_time": 1.0, "input_process": ""
      },
      "segments": [
        { "start": 0.0, "duration": 1.2, "confidence": 1.0, "loudness_start": -40.0, "loudness_max_time": 0.8, "loudness_max": -30.0, "pitches": [], "timbre": [] },
        { "start": 1.2, "duration": 0.8, "confidence": 1.0, "loudness_start": -10.0, "loudness_max_time": 1.8, "loudness_max": -2.0, "pitches": [], "timbre": [] }
      ],
      "track": {
        "num_samples": 600, "duration": 2.0, "sample_md5": "", "offset_seconds": 0,
        "window_seconds": 2, "analysis_sample_rate": 300, "analysis_channels": 1,
        "end_of_fade_in": 0.1, "start_of_fade_out": 1.9, "loudness": -20.0,
        "tempo": 120.0, "tempo_confidence": 1.0, "time_signature": 4,
        "time_signature_confidence": 1.0, "key": 0, "key_confidence": 1.0,
        "mode": 1, "mode_confidence": 1.0, "codestring": "", "code_version": 1.0,
        "echoprintstring": "", "echoprint_version": 1.0, "synchstring": "",
        "synch_version": 1.0, "rhythmstring": "", "rhythm_version": 1.0
      }
    }))
    .unwrap();
    let env = Network::audio_analysis_envelope(&analysis);
    assert_eq!(env.len(), 512);
    // Quiet intro (segment at 0.0s) normalized to 30/60 = 0.5 at slot 0.
    assert!((env[0] - 0.5).abs() < 1e-3);
    // Loud part (start 1.2s) lands on slot 306 of 511 and saturates near 1.0.
    assert!((env[306] - 58.0 / 60.0).abs() < 1e-3);
    // Some slot got a value.
    assert!(env.iter().any(|&v| v > 0.0));
  }

  #[test]
  fn unplayable_tracks_are_skipped_in_playback_positions() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = new_mock(&app);

    let mut blocked = network.mock_track_json(1);
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
        item(network.mock_track_json(0)),
        item(blocked),
        item(network.mock_track_json(2)),
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
    assert_eq!(app.track_table.tracks.len(), 2);
    assert_eq!(app.track_table_raw_index, vec![0, 1]);
    assert_eq!(app.track_table.tracks[0].name, "Mock Song 0");
    assert_eq!(app.track_table.tracks[1].name, "Mock Song 2");
  }

  #[test]
  fn opening_cached_playlist_dispatches_background_reconcile() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = new_mock(&app);
    let dir = std::env::temp_dir().join(format!("sptune_bk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    network.playlist_cache.set_path(dir.join("playlist_cache.json"));

    let item: PlaylistItem = serde_json::from_value(json!({
      "added_at": "2026-01-01T00:00:00Z",
      "added_by": null,
      "is_local": false,
      "track": network.mock_track_json(0),
    }))
    .unwrap();
    network
      .playlist_cache
      .update("spotify:playlist:test", vec![item], 1, false);

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
      dispatched
        .iter()
        .any(|e| matches!(e, IoEvent::ReconcilePlaylistTracks(id) if id == "spotify:playlist:test")),
      "opening a cached playlist must dispatch a background reconcile"
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn saved_tracks_page_is_a_partial_subset() {
    let app = Arc::new(Mutex::new(App::default()));
    let network = new_mock(&app);
    let page = network.mock_saved_tracks_page(0);
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
    // Paging stays inside the 20 saved tracks.
    let page2 = network.mock_saved_tracks_page(15);
    assert_eq!(page2.items.len(), 5);
    assert_eq!(page2.total, 20);
    assert!(page2.next.is_none());
  }

  #[test]
  fn track_numbers_are_sequential() {
    let app = Arc::new(Mutex::new(App::default()));
    let network = new_mock(&app);
    let v11 = network.mock_track_json(11);
    let v12 = network.mock_track_json(12);
    assert_eq!(v11["track_number"], 12);
    assert_eq!(v12["track_number"], 13);
  }

  #[test]
  fn start_playback_honors_the_clicked_song() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = new_mock(&app);
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(network.handle_network_event(IoEvent::StartPlayback(
      None,
      None,
      Some(5),
    )));
    let app_guard = rt.block_on(app.lock());
    let context = app_guard.current_playback_context.as_ref().unwrap();
    let item = context.item.as_ref().unwrap();
    let id = match item {
      PlayableItem::Track(track) => track.id.as_ref().unwrap().id().to_string(),
      _ => panic!("expected a track item"),
    };
    assert_eq!(id, "mocktrack5");
  }

  #[test]
  fn start_playback_from_album_honors_offset() {
    let app = Arc::new(Mutex::new(App::default()));
    let mut network = new_mock(&app);
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(network.handle_network_event(IoEvent::StartPlayback(
      Some("spotify:album:mockalbum0".to_string()),
      None,
      Some(25),
    )));
    let app_guard = rt.block_on(app.lock());
    let context = app_guard.current_playback_context.as_ref().unwrap();
    let item = context.item.as_ref().unwrap();
    let id = match item {
      PlayableItem::Track(track) => track.id.as_ref().unwrap().id().to_string(),
      _ => panic!("expected a track item"),
    };
    assert_eq!(id, "mocktrack25");
  }

  #[test]
  fn gear_settings_round_trip() {
    // Isolate from the user's real state file by pointing HOME at a temp dir.
    let tmp = std::env::temp_dir().join(format!("sptune-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    std::env::set_var("HOME", &tmp);
    let app = Arc::new(Mutex::new(App::default()));
    let network = new_mock(&app);
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
      let mut app_guard = app.lock().await;
      app_guard.show_library = false;
      app_guard.user_config.theme.background = Color::Rgb(0, 0, 0);
      network.save_settings_from_app(&app_guard);
    });
    let saved = SavedState::load(true).expect("state file should exist");
    assert_eq!(saved.show_library, Some(false));
    assert_eq!(saved.show_playlists, Some(true));
    assert_eq!(saved.volume_ramp_bar, Some(false));
    assert_eq!(saved.black_background, Some(true));
    assert_eq!(saved.resume_track, Some(false));
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
}
