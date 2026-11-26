use super::user_config::{theme_presets, Theme, UserConfig};
use crate::backend::IoEvent;
use anyhow::anyhow;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::Color;
use rspotify::model::{
  album::{FullAlbum, SavedAlbum, SimplifiedAlbum},
  artist::FullArtist,
  context::CurrentPlaybackContext,
  device::Device,
  page::{CursorBasedPage, Page},
  playing::PlayHistory,
  playlist::{PlaylistItem, SimplifiedPlaylist},
  show::{FullShow, Show, SimplifiedEpisode, SimplifiedShow},
  track::{FullTrack, SavedTrack, SimplifiedTrack},
  user::PrivateUser,
  Id, PlayableItem, RepeatState,
};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;
use std::{
  cmp::{max, min, Ordering},
  collections::HashSet,
  time::{Duration, Instant, SystemTime},
};

use arboard::Clipboard;

fn artist_key(track: &FullTrack) -> String {
  track
    .artists
    .iter()
    .map(|artist| artist.name.as_str())
    .collect::<Vec<_>>()
    .join(", ")
}

pub const LIBRARY_OPTIONS: [&str; 7] = [
  "For you",
  "Recently Played",
  "Liked Songs",
  "Albums",
  "Artists",
  "Podcasts",
  "Queue",
];

/// Library sections not hidden via the '?' toggles (1-6).
pub fn visible_library_options(hidden: &[String]) -> Vec<&'static str> {
  LIBRARY_OPTIONS
    .iter()
    .copied()
    .filter(|option| !hidden.iter().any(|h| h == option))
    .collect()
}

/// Display width (columns) of a line, so the sidebar sizes itself to what
/// actually renders (♻ is two columns, not one).
fn disp_width(s: &str) -> usize {
  unicode_width::UnicodeWidthStr::width(s)
}

/// Sidebar block titles, shared by the drawer (tui.rs) and the width
/// computation (layout.rs) so the two can never drift apart.
pub fn library_block_title(app: &App) -> String {
  app
    .user
    .as_ref()
    .and_then(|u| u.display_name.as_deref())
    .map(|name| format!("{}({}) Library", crate::tui::layout::REFRESH_GLYPH, name))
    .unwrap_or_else(|| format!("{}{}", crate::tui::layout::REFRESH_GLYPH, "Library"))
}

pub fn playlists_block_title() -> String {
  format!("{}{}", crate::tui::layout::REFRESH_GLYPH, "Playlists")
}

/// Longest line the sidebar renders right now: both block titles plus every
/// visible library option and playlist name. Used to size the panel to its
/// content so nothing clips and no columns are wasted.
pub fn sidebar_max_content_len(app: &App) -> usize {
  let titles = [
    disp_width(&library_block_title(app)),
    disp_width(&playlists_block_title()),
  ];
  let library = visible_library_options(&app.hidden_library_sections)
    .iter()
    .map(|s| disp_width(s))
    .max()
    .unwrap_or(0);
  let playlists = cached_playlist_max_width(app);
  titles
    .into_iter()
    .chain([library, playlists])
    .max()
    .unwrap_or(0)
}

// Scanning every playlist name on every frame is O(n) over the whole library and
// was the one per-frame cost introduced for the no-truncation sidebar sizing.
// Cache it: the playlist set only changes when a new page is assigned (new
// pointer) or grown (new length), so key on both and recompute only then.
thread_local! {
  static PLAYLIST_MAX_WIDTH: std::cell::RefCell<(usize, usize, usize)> =
    const { std::cell::RefCell::new((0, 0, 0)) }; // (page_ptr, len, cached_width)
}

fn cached_playlist_max_width(app: &App) -> usize {
  let key = app
    .playlists
    .as_ref()
    .map(|p| (p as *const _ as usize, p.items.len()))
    .unwrap_or((0, 0));
  PLAYLIST_MAX_WIDTH.with(|cell| {
    let mut slot = cell.borrow_mut();
    if slot.0 == key.0 && slot.1 == key.1 {
      return slot.2;
    }
    let w = app
      .playlists
      .as_ref()
      .map(|p| {
        p.items
          .iter()
          .map(|i| disp_width(&i.name))
          .max()
          .unwrap_or(0)
      })
      .unwrap_or(0);
    *slot = (key.0, key.1, w);
    w
  })
}

const DEFAULT_ROUTE: Route = Route {
  id: RouteId::MadeForYou,
  active_block: ActiveBlock::Empty,
  hovered_block: ActiveBlock::Empty,
};

#[derive(Clone, Debug)]
pub struct RequestLogEntry {
  pub text: String,
  pub count: u32,
  pub instant: std::time::Instant,
}

#[derive(Clone)]
pub struct ScrollableResultPages<T> {
  index: usize,
  pub pages: Vec<T>,
}

impl<T> ScrollableResultPages<T> {
  pub fn new() -> ScrollableResultPages<T> {
    ScrollableResultPages {
      index: 0,
      pages: vec![],
    }
  }

  pub fn get_results(&self, at_index: Option<usize>) -> Option<&T> {
    self.pages.get(at_index.unwrap_or(self.index))
  }

  pub fn get_mut_results(&mut self, at_index: Option<usize>) -> Option<&mut T> {
    self.pages.get_mut(at_index.unwrap_or(self.index))
  }

  pub fn add_pages(&mut self, new_pages: T) {
    self.pages.push(new_pages);
    // Whenever a new page is added, set the active index to the end of the vector
    self.index = self.pages.len() - 1;
  }
}

impl<T> Default for ScrollableResultPages<T> {
  fn default() -> Self {
    Self::new()
  }
}

#[derive(Default)]
pub struct SpotifyResultAndSelectedIndex<T> {
  pub index: usize,
  pub result: T,
}

#[derive(Clone)]
pub struct Library {
  pub selected_index: usize,
  pub saved_tracks: ScrollableResultPages<Page<SavedTrack>>,
  pub saved_albums: ScrollableResultPages<Page<SavedAlbum>>,
  pub saved_shows: ScrollableResultPages<Page<Show>>,
  pub saved_artists: ScrollableResultPages<CursorBasedPage<FullArtist>>,
  pub show_episodes: ScrollableResultPages<Page<SimplifiedEpisode>>,
}

#[derive(PartialEq, Debug, Clone)]
pub enum SearchResultBlock {
  AlbumSearch,
  SongSearch,
  ArtistSearch,
  PlaylistSearch,
  ShowSearch,
  Empty,
}

/// A full last page means more results exist: the search `total`
/// under-reports for many queries, so gating on it kills the load-more
/// row early (same disease as the artist top tracks).
fn page_has_more<T: serde::de::DeserializeOwned>(page: &Page<T>) -> bool {
  page.limit > 0 && page.items.len() >= page.limit as usize
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum ArtistBlock {
  TopTracks,
  Albums,
  Singles,
  EPs,
  AppearsOn,
  Empty,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DialogContext {
  PlaylistWindow,
  PlaylistSearch,
  SeekTime,
  AddToPlaylist,
  ConfirmRemoveFromPlaylist,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ActiveBlock {
  PlayBar,
  AlbumTracks,
  AlbumList,
  ArtistBlock,
  Empty,
  Error,
  HelpMenu,
  Input,
  Library,
  MyPlaylists,
  Podcasts,
  EpisodeTable,
  Queue,
  RecentlyPlayed,
  SearchResultBlock,
  SelectDevice,
  TrackTable,
  MadeForYou,
  Artists,
  MusicView,
  RequestLog,
  Dialog(DialogContext),
}

#[derive(Clone, PartialEq, Debug)]
pub enum RouteId {
  AlbumTracks,
  AlbumList,
  Artist,
  MusicView,
  Error,
  Queue,
  RecentlyPlayed,
  Search,
  SelectedDevice,
  TrackTable,
  MadeForYou,
  Artists,
  Podcasts,
  PodcastEpisodes,
  Dialog,
}

#[derive(Debug)]
pub struct Route {
  pub id: RouteId,
  pub active_block: ActiveBlock,
  pub hovered_block: ActiveBlock,
}

#[derive(Debug)]
pub struct ApiError {
  pub text: String,
  pub count: usize,
}

// Is it possible to compose enums?
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum TrackTableContext {
  MyPlaylists,
  AlbumSearch,
  PlaylistSearch,
  SavedTracks,
  MadeForYou,
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub enum TrackSortColumn {
  Title,
  Artist,
  Album,
  Length,
  DateAdded,
}

// Is it possible to compose enums?
#[derive(Clone, PartialEq, Debug, Copy)]
pub enum AlbumTableContext {
  Simplified,
  Full,
}

#[derive(Clone, PartialEq, Debug, Copy)]
pub enum EpisodeTableContext {
  Simplified,
  Full,
}

pub struct SearchResult {
  pub albums: Option<Page<SimplifiedAlbum>>,
  pub artists: Option<Page<FullArtist>>,
  pub playlists: Option<Page<SimplifiedPlaylist>>,
  pub tracks: Option<Page<FullTrack>>,
  pub shows: Option<Page<SimplifiedShow>>,
  pub selected_album_index: Option<usize>,
  pub selected_artists_index: Option<usize>,
  pub selected_playlists_index: Option<usize>,
  pub selected_tracks_index: Option<usize>,
  pub selected_shows_index: Option<usize>,
  /// Term the current results were fetched with; needed to fetch more pages.
  pub query: String,
  pub hovered_block: SearchResultBlock,
  pub selected_block: SearchResultBlock,
}

#[derive(Default)]
pub struct TrackTable {
  pub tracks: Vec<FullTrack>,
  pub selected_index: usize,
  // Wheel-scrolled view offset for the track list; selection stays put while
  // the view scrolls (website-style). Keyboard/click selection resyncs it.
  pub scroll_offset: usize,
  pub context: Option<TrackTableContext>,
}

#[derive(Clone)]
pub struct SelectedShow {
  pub show: SimplifiedShow,
}

#[derive(Clone)]
pub struct SelectedFullShow {
  pub show: FullShow,
}

#[derive(Clone)]
pub struct SelectedAlbum {
  pub album: SimplifiedAlbum,
  pub tracks: Page<SimplifiedTrack>,
  pub selected_index: usize,
}

#[derive(Clone)]
pub struct SelectedFullAlbum {
  pub album: FullAlbum,
  pub selected_index: usize,
}

#[derive(Clone)]
pub struct Artist {
  pub artist_id: String,
  pub artist_name: String,
  pub albums: Page<SimplifiedAlbum>,
  pub singles: Page<SimplifiedAlbum>,
  pub eps: Page<SimplifiedAlbum>,
  pub appears_on: Page<SimplifiedAlbum>,
  pub related_artists: Vec<FullArtist>,
  pub top_tracks: Vec<FullTrack>,
  pub top_tracks_total: usize,
  // True while a fetch could still return more: set when the last page
  // arrived full (10 items). The search `total` can under-report, so the
  // load-more row keys off this instead.
  pub top_tracks_has_more: bool,
  pub selected_album_index: usize,
  pub selected_singles_index: usize,
  pub selected_eps_index: usize,
  pub selected_appears_on_index: usize,
  pub selected_related_artist_index: usize,
  pub selected_top_track_index: usize,
  pub artist_hovered_block: ArtistBlock,
  pub artist_selected_block: ArtistBlock,
  pub image_url: Option<String>,
  pub is_following: Option<bool>,
  pub followers: Option<u32>,
  pub genres: Vec<String>,
  pub meta: Option<crate::metadata::ArtistMeta>,
}

pub struct App {
  pub instant_since_last_current_playback_poll: Instant,
  navigation_stack: Vec<Route>,
  pub lyrics: Option<Vec<(u128, String)>>,
  pub cover_art: Option<(String, image::DynamicImage)>,
  pub cover_art_fetching: Option<String>,
  pub artist_cover: Option<(String, image::DynamicImage)>,
  pub artist_cover_fetching: Option<String>,
  pub cover_caps: crate::cover_art::CoverCaps,
  pub track_credits: Option<Vec<String>>,
  pub track_meta: Option<crate::metadata::TrackMeta>,
  pub queue_next: Option<String>,
  pub queue_items: Vec<PlayableItem>,
  pub queue_selected_index: usize,
  pub user_config: UserConfig,
  pub artists: Vec<FullArtist>,
  pub artist: Option<Artist>,
  pub album_table_context: AlbumTableContext,
  pub saved_album_tracks_index: usize,
  pub api_errors: Vec<ApiError>,
  pub current_playback_context: Option<CurrentPlaybackContext>,
  /// True while the soloist WS loop is feeding playback state; suppresses the Web API poll.
  pub soloist_live: bool,
  pub devices: Option<Vec<Device>>,
  // Inputs:
  // input is the string for input;
  // input_idx is the index of the cursor in terms of character;
  // input_cursor_position is the sum of the width of characters preceding the cursor.
  // Reason for this complication is due to non-ASCII characters, they may
  // take more than 1 bytes to store and more than 1 character width to display.
  pub input: Vec<char>,
  pub input_idx: usize,
  pub input_cursor_position: u16,
  pub liked_song_ids_set: HashSet<String>,
  // Rows are only highlighted once the user moves the selection or clicks a
  // row, so a freshly opened panel never "marks" its first row.
  pub selection_engaged: bool,
  // The sidebar block (Library/MyPlaylists) the user last engaged; its row
  // highlight stays visible while browsing the page opened from it, until
  // the user engages something outside the sidebar (search box, gear).
  pub sidebar_latched_block: Option<ActiveBlock>,
  pub hovered_library_index: Option<usize>,
  pub hovered_playlist_index: Option<usize>,
  pub hovered_list_index: Option<usize>,
  pub followed_artist_ids_set: HashSet<String>,
  pub saved_album_ids_set: HashSet<String>,
  pub saved_show_ids_set: HashSet<String>,
  pub large_search_limit: u32,
  pub library: Library,
  pub playlist_offset: u32,
  pub made_for_you_offset: u32,
  pub playlist_tracks: Option<Page<PlaylistItem>>,
  pub made_for_you_tracks: Option<Page<PlaylistItem>>,
  // Playlists the user pinned in "For you", as (name, playlist id).
  pub made_for_you_custom: Vec<(String, String)>,
  // The playlist opened via a link or search (name, id), so the song table
  // title can show its name even when no sidebar context is set.
  pub playlist_view: Option<(String, String)>,
  pub playlists: Option<Page<SimplifiedPlaylist>>,
  pub recently_played: SpotifyResultAndSelectedIndex<Option<CursorBasedPage<PlayHistory>>>,
  pub search_results: SearchResult,
  pub selected_album_simplified: Option<SelectedAlbum>,
  pub selected_album_full: Option<SelectedFullAlbum>,
  pub selected_device_index: Option<usize>,
  pub selected_playlist_index: Option<usize>,
  pub active_playlist_index: Option<usize>,
  pub size: Rect,
  pub song_progress_ms: u128,
  pub seek_ms: Option<u128>,
  pub scrubbing_playbar: bool,
  pub seek_pending: usize,
  pub play_toggle_gen: u64,
  pub play_toggle_pending: usize,
  pub repeat_pending: usize,
  pub track_end_polls: u8,
  pub volume_preview: Option<u8>,
  pub track_table: TrackTable,
  pub track_table_sort: Option<(TrackSortColumn, bool)>,
  pub active_playlist_id: Option<String>,
  pub playlist_sorts: std::collections::HashMap<String, (TrackSortColumn, bool)>,
  // When Some, the track-table view shows an in-playlist search bar (first
  // row) and only rows whose title or artist matches the query are listed.
  // None hides the bar and clears the filter. Toggled by the configurable
  // `search_in_playlist` key (default: Ctrl+F).
  pub playlist_filter: Option<String>,
  pub playlist_filter_idx: usize,
  pub playlist_filter_cursor_position: u16,
  // When the last remove-from-playlist request was dispatched; enforces the
  // 5-second cooldown so a user cannot mass-delete tracks.
  pub last_remove_time: Option<Instant>,
  // When the last load-more request was dispatched; min 2s between
  // consecutive "next page" fetches so scrolling can't hammer the API.
  pub last_load_more: Option<Instant>,
  // Rate limiter state exposed for the dev panel.
  pub api_tokens: f64,
  pub api_tokens_updated: Option<Instant>,
  pub api_backoff_until: Option<Instant>,
  pub track_table_added_at: Vec<Option<DateTime<Utc>>>,
  // Date Added was clicked while the playlist was only partially loaded;
  // the sort applies to the loaded slice (display-only hint, never a fetch).
  pub date_added_pending: bool,
  // Playlist id whose background fill may still mutate the UI. Set by a
  // partial first-page apply, cleared by any newer open/serve; the fill loop
  // exits as soon as another list takes over.
  pub fill_target: Option<String>,
  // For each displayed row, its position in the RAW playlist (including
  // episodes the table skips). Survives re-sorts; drives StartPlayback's
  // context offset so a sorted view plays the clicked song, not the one at
  // the same index in the original order.
  pub track_table_raw_index: Vec<usize>,
  pub episode_table_context: EpisodeTableContext,
  pub selected_show_simplified: Option<SelectedShow>,
  pub selected_show_full: Option<SelectedFullShow>,
  pub user: Option<PrivateUser>,
  pub album_list_index: usize,
  pub made_for_you_index: usize,
  pub artists_list_index: usize,
  pub clipboard: Option<Clipboard>,
  pub shows_list_index: usize,
  pub episode_list_index: usize,
  pub help_docs_size: u32,
  pub help_menu_page: u32,
  pub help_menu_max_lines: u32,
  pub help_scroll_offset: u32,
  pub help_show_shortcuts: bool,
  pub is_loading: bool,
  pub ui_dirty: bool,
  pub ui_wake: Option<crate::event::UiWake>,
  pub io_tx: Option<Sender<IoEvent>>,
  pub is_fetching_current_playback: bool,
  /// True once the first live playback response has landed. Gates the
  /// Resume affordance: before that the playbar cannot tell "nothing
  /// playing" from "not asked yet", so it must not offer Resume.
  pub playback_known: bool,
  pub is_fetching_next_page: bool,
  pub spotify_token_expiry: SystemTime,
  /// Set when the stored grant is dead (pre-PKCE refresh token). Refresh
  /// would fail forever, so the 401 path and the expiry tick must stop
  /// re-dispatching RefreshAuthentication; the user must restart to re-login.
  pub auth_reauth_required: bool,
  pub dialog: Option<String>,
  pub confirm: bool,
  /// Cached playlist contents mirrored from the backend cache: playlist id ->
  /// item uris. Drives the "already in playlist" marker without extra requests.
  pub playlist_uri_map: HashMap<String, HashSet<String>>,
  /// Track captured when the add-to-playlist picker opens; the picker
  /// navigation must not change which track gets added.
  pub pending_track_uri: Option<String>,
  pub playlist_picker_index: usize,
  /// Name-entry mode inside the add-to-playlist picker: Enter on the
  /// "+ New playlist" row flips this on, and typed chars land here.
  pub naming_playlist: bool,
  pub new_playlist_name: String,
  pub pending_remove_track_uri: Option<String>,
  pub pending_remove_playlist_id: Option<String>,
  pub pending_remove_track_name: Option<String>,
  pub pending_remove_playlist_name: Option<String>,
  pub show_library: bool,
  pub show_playlists: bool,
  pub sidebar_minimized: bool,
  pub sidebar_width_override: Option<u16>,
  pub library_height_override: Option<u16>,
  pub music_left_pct: Option<u16>,
  pub music_lyrics_pct: Option<u16>,
  pub music_cover_height: Option<u16>,
  pub music_copied_until: Option<std::time::Instant>,
  pub for_you_paste_input: Option<String>,
  pub hidden_library_sections: Vec<String>,
  pub config_theme: Theme,
  pub theme_preset_index: Option<usize>,
  pub dev_view: bool,
  pub request_log: VecDeque<RequestLogEntry>,
  pub request_log_gen: u64,
  pub startup_at: Instant,
  pub first_draw_ms: Option<u64>,
  // Manual suspend state for the Spotify client's helper processes
  // (playbar button + "Suspend/Resume Process" settings row).
  pub processes_suspended: bool,
  pub show_process_suspend: bool,
  pub desktop_notifications: bool,
  pub mock_mode: bool,
  pub last_notified_uri: Option<String>,
  pub visualizer: crate::visualizer::VizState,
  // Last track URI from the previous session (Resume Last Listening).
  pub saved_last_track: Option<String>,
  pub request_log_index: Option<usize>,
}

impl Default for App {
  fn default() -> Self {
    App {
      lyrics: None,
      cover_art: None,
      cover_art_fetching: None,
      artist_cover: None,
      artist_cover_fetching: None,
      cover_caps: crate::cover_art::CoverCaps::halfblocks(),
      track_credits: None,
      track_meta: None,
      queue_next: None,
      queue_items: vec![],
      queue_selected_index: 0,
      album_table_context: AlbumTableContext::Full,
      album_list_index: 0,
      made_for_you_index: 0,
      artists_list_index: 0,
      shows_list_index: 0,
      episode_list_index: 0,
      artists: vec![],
      artist: None,
      user_config: UserConfig::new(),
      saved_album_tracks_index: 0,
      recently_played: Default::default(),
      size: Rect::default(),
      selected_album_simplified: None,
      selected_album_full: None,
      library: Library {
        saved_tracks: ScrollableResultPages::new(),
        saved_albums: ScrollableResultPages::new(),
        saved_shows: ScrollableResultPages::new(),
        saved_artists: ScrollableResultPages::new(),
        show_episodes: ScrollableResultPages::new(),
        selected_index: 0,
      },
      liked_song_ids_set: HashSet::new(),
      selection_engaged: false,
      sidebar_latched_block: None,
      followed_artist_ids_set: HashSet::new(),
      saved_album_ids_set: HashSet::new(),
      saved_show_ids_set: HashSet::new(),
      navigation_stack: vec![DEFAULT_ROUTE],
      large_search_limit: 50,
      api_errors: Vec::new(),
      current_playback_context: None,
      soloist_live: false,
      devices: None,
      input: vec![],
      input_idx: 0,
      input_cursor_position: 0,
      playlist_offset: 0,
      made_for_you_offset: 0,
      playlist_tracks: None,
      made_for_you_tracks: None,
      made_for_you_custom: vec![],
      playlist_view: None,
      playlists: None,
      search_results: SearchResult {
        hovered_block: SearchResultBlock::SongSearch,
        selected_block: SearchResultBlock::Empty,
        albums: None,
        artists: None,
        playlists: None,
        shows: None,
        selected_album_index: None,
        selected_artists_index: None,
        selected_playlists_index: None,
        selected_tracks_index: None,
        selected_shows_index: None,
        query: String::new(),
        tracks: None,
      },
      song_progress_ms: 0,
      seek_ms: None,
      scrubbing_playbar: false,
      seek_pending: 0,
      play_toggle_gen: 0,
      play_toggle_pending: 0,
      repeat_pending: 0,
      track_end_polls: 0,
      volume_preview: None,
      selected_device_index: None,
      selected_playlist_index: None,
      active_playlist_index: None,
      track_table: Default::default(),
      track_table_sort: None,
      active_playlist_id: None,
      playlist_sorts: std::collections::HashMap::new(),
      playlist_filter: None,
      playlist_filter_idx: 0,
      playlist_filter_cursor_position: 0,
      last_remove_time: None,
      last_load_more: None,
      api_tokens: 10.0,
      api_tokens_updated: None,
      api_backoff_until: None,
      track_table_added_at: vec![],
      date_added_pending: false,
      fill_target: None,
      track_table_raw_index: vec![],
      episode_table_context: EpisodeTableContext::Full,
      selected_show_simplified: None,
      selected_show_full: None,
      user: None,
      instant_since_last_current_playback_poll: Instant::now(),
      clipboard: None,
      help_docs_size: 0,
      help_menu_page: 0,
      help_menu_max_lines: 0,
      help_scroll_offset: 0,
      help_show_shortcuts: false,
      is_loading: false,
      // Set by the network thread after add/remove/unlike mutations; the UI
      // loop repaints on the next iteration without waiting for input.
      ui_dirty: false,
      ui_wake: None,
      io_tx: None,
      is_fetching_current_playback: false,
      playback_known: false,
      is_fetching_next_page: false,
      spotify_token_expiry: SystemTime::now(),
      auth_reauth_required: false,
      dialog: None,
      confirm: false,
      playlist_uri_map: HashMap::new(),
      pending_track_uri: None,
      playlist_picker_index: 0,
      naming_playlist: false,
      new_playlist_name: String::new(),
      pending_remove_track_uri: None,
      pending_remove_playlist_id: None,
      pending_remove_track_name: None,
      pending_remove_playlist_name: None,
      show_library: true,
      show_playlists: true,
      sidebar_minimized: false,
      sidebar_width_override: None,
      library_height_override: None,
      music_left_pct: None,
      music_lyrics_pct: None,
      music_cover_height: None,
      music_copied_until: None,
      for_you_paste_input: None,
      hovered_library_index: None,
      hovered_playlist_index: None,
      hovered_list_index: None,
      hidden_library_sections: vec![],
      config_theme: Theme::default(),
      theme_preset_index: None,
      dev_view: false,
      processes_suspended: false,
      show_process_suspend: false,
      desktop_notifications: true,
      mock_mode: false,
      last_notified_uri: None,
      visualizer: crate::visualizer::VizState::new(),
      saved_last_track: None,
      request_log: VecDeque::new(),
      request_log_gen: 0,
      startup_at: Instant::now(),
      first_draw_ms: None,
      request_log_index: None,
    }
  }
}

/// "title – artist, artist" for the OSC 7501 status message. Empty when idle.
pub fn now_playing_title(ctx: Option<&CurrentPlaybackContext>) -> String {
  let Some(item) = ctx.and_then(|c| c.item.clone()) else {
    return String::new();
  };
  match item {
    PlayableItem::Track(track) => {
      let artists = track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
      format!("{} – {artists}", track.name)
    }
    PlayableItem::Episode(episode) => episode.name,
    PlayableItem::Unknown(_) => String::new(),
  }
}

/// OSC 7501 program status: `working` (+ title) while playing, `idle` otherwise.
/// One escape sequence on stdout; unknown OSCs are ignored, so zero-risk.
pub fn emit_program_status(playing: bool, title: &str) {
  use std::io::Write as _;
  let body = if playing && !title.is_empty() {
    format!(
      "state=working:app=sptune:msg={}",
      base64::engine::general_purpose::STANDARD.encode(title)
    )
  } else if playing {
    "state=working:app=sptune".to_string()
  } else {
    "state=idle:app=sptune".to_string()
  };
  let _ = write!(std::io::stdout(), "\x1b]7501;{body}\x1b\\");
}

impl App {
  /// Log a request; repeats coalesce globally, keeping only latest timestamp (no tracetrail) — grouped display in tui.rs shows HH:MM header with counts.
  pub fn log_request(&mut self, text: String) {
    self.request_log_gen = self.request_log_gen.wrapping_add(1);
    let pos = self.request_log.iter().position(|e| e.text == text);
    if let Some(pos) = pos {
      let mut entry = self.request_log.remove(pos).unwrap();
      entry.count = entry.count.saturating_add(1);
      entry.instant = Instant::now();
      self.request_log.push_front(entry);
    } else {
      self.request_log.push_front(RequestLogEntry {
        text,
        count: 1,
        instant: Instant::now(),
      });
      self.request_log.truncate(100);
    }
  }

  pub fn new(
    io_tx: Sender<IoEvent>,
    user_config: UserConfig,
    spotify_token_expiry: SystemTime,
  ) -> App {
    App {
      io_tx: Some(io_tx),
      config_theme: user_config.theme,
      user_config,
      spotify_token_expiry,
      ..App::default()
    }
  }

  pub fn current_sort_key(&self) -> Option<String> {
    match self.track_table.context {
      Some(TrackTableContext::MyPlaylists)
      | Some(TrackTableContext::PlaylistSearch)
      | Some(TrackTableContext::MadeForYou) => self
        .active_playlist_id
        .as_ref()
        .map(|id| format!("playlist:{id}"))
        .or_else(|| {
          self
            .active_playlist_index
            .and_then(|idx| self.playlists.as_ref().and_then(|p| p.items.get(idx)))
            .map(|p| format!("playlist:{}", p.id.id()))
        }),
      Some(TrackTableContext::SavedTracks) => Some("saved_tracks".to_string()),
      _ => None,
    }
  }

  pub fn apply_saved_sort(&mut self, key: &str, is_playlist: bool) {
    let Some((column, desc)) = self.playlist_sorts.get(key).copied() else {
      self.track_table_sort = None;
      return;
    };
    self.track_table_sort = Some((column, desc));
    if is_playlist && column == TrackSortColumn::DateAdded {
      self.materialize_date_added();
    } else {
      self.sort_tracks();
    }
  }

  // Stable sort of track_table (and its parallel added_at vec). None dates always
  // sort last regardless of direction.
  pub fn sort_tracks(&mut self) {
    let Some((column, desc)) = self.track_table_sort else {
      return;
    };
    let selected_id = self.selected_track_id();
    let tracks = std::mem::take(&mut self.track_table.tracks);
    let added_at = std::mem::take(&mut self.track_table_added_at);
    let raw_index = std::mem::take(&mut self.track_table_raw_index);
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    order.sort_by(|&a, &b| {
      let cmp = match column {
        TrackSortColumn::Title => tracks[a].name.cmp(&tracks[b].name),
        TrackSortColumn::Artist => artist_key(&tracks[a]).cmp(&artist_key(&tracks[b])),
        TrackSortColumn::Album => tracks[a].album.name.cmp(&tracks[b].album.name),
        TrackSortColumn::Length => tracks[a]
          .duration
          .num_milliseconds()
          .cmp(&tracks[b].duration.num_milliseconds()),
        TrackSortColumn::DateAdded => match (added_at[a], added_at[b]) {
          (Some(x), Some(y)) => x.cmp(&y),
          (None, Some(_)) => Ordering::Greater,
          (Some(_), None) => Ordering::Less,
          (None, None) => Ordering::Equal,
        },
      };
      let cmp = if desc { cmp.reverse() } else { cmp };
      cmp.then(a.cmp(&b))
    });
    self.track_table.tracks = order.iter().map(|&i| tracks[i].clone()).collect();
    self.track_table_added_at = order.iter().map(|&i| added_at[i]).collect();
    self.track_table_raw_index = if raw_index.len() == tracks.len() {
      order.iter().map(|&i| raw_index[i]).collect()
    } else {
      vec![]
    };
    self.remap_selection_after_reorder(selected_id);
  }

  // A re-sort moves rows around; make the selection follow the same track
  // instead of staying at the same position (position-based selection was
  // highlighting the wrong song after sorting).
  fn selected_track_id(&self) -> Option<String> {
    self
      .track_table
      .tracks
      .get(self.track_table.selected_index)
      .and_then(|t| t.id.as_ref().map(|id| id.to_string()))
  }

  fn remap_selection_after_reorder(&mut self, selected_id: Option<String>) {
    let Some(selected_id) = selected_id else {
      return;
    };
    if let Some(pos) =
      self.track_table.tracks.iter().position(|t| {
        t.id.as_ref().map(|id| id.to_string()).as_deref() == Some(selected_id.as_str())
      })
    {
      self.track_table.selected_index = pos;
    }
  }

  // Materialize the Date Added sort: order the display by raw playlist
  // position (newest first when desc, original order when asc). raw_index
  // survives any prior comparator sort, so this always lands on the correct
  // state; only applied once the whole playlist is loaded.
  pub fn materialize_date_added(&mut self) {
    self.date_added_pending = false;
    let Some((column, desc)) = self.track_table_sort else {
      return;
    };
    if column != TrackSortColumn::DateAdded {
      return;
    }
    let selected_id = self.selected_track_id();
    let tracks = std::mem::take(&mut self.track_table.tracks);
    let added_at = std::mem::take(&mut self.track_table_added_at);
    let raw_index = std::mem::take(&mut self.track_table_raw_index);
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    if raw_index.len() == tracks.len() {
      order.sort_by(|&a, &b| raw_index[a].cmp(&raw_index[b]));
      if desc {
        order.reverse();
      }
    }
    self.track_table.tracks = order.iter().map(|&i| tracks[i].clone()).collect();
    self.track_table_added_at = order.iter().map(|&i| added_at[i]).collect();
    self.track_table_raw_index = if raw_index.len() == tracks.len() {
      order.iter().map(|&i| raw_index[i]).collect()
    } else {
      vec![]
    };
    self.remap_selection_after_reorder(selected_id);
  }

  // Send a network event to the network thread
  /// Clear the persistent search box (header input) when navigating away from
  /// search so the next `/` starts fresh.
  pub fn clear_search_input(&mut self) {
    self.input = vec![];
    self.input_idx = 0;
    self.input_cursor_position = 0;
  }

  /// True for "next page" continuation fetches (offset > 0 or inherently a
  /// continuation). Initial opens (offset 0) are not throttled. The
  /// first-open background fill is exempt too: it completes the visible
  /// list, and dropping it leaves a partial cache forever.
  fn is_load_more_event(action: &IoEvent) -> bool {
    match action {
      IoEvent::GetMoreSearchResults(_) | IoEvent::GetMoreRecentlyPlayed(_) => true,
      IoEvent::GetPlaylistItems(_, offset)
      | IoEvent::GetMadeForYouPlaylistItems(_, offset)
      | IoEvent::GetAlbumTracksMore(_, offset)
      | IoEvent::GetArtistTopTracksMore(_, _, offset) => *offset > 0,
      _ => false,
    }
  }

  pub fn mark_dirty(&mut self) {
    self.ui_dirty = true;
    if let Some(wake) = &self.ui_wake {
      wake.wake();
    }
  }

  pub fn dispatch(&mut self, action: IoEvent) {
    if Self::is_load_more_event(&action)
      && self
        .last_load_more
        .map(|t| t.elapsed() < Duration::from_secs(2))
        .unwrap_or(false)
    {
      return;
    }
    if Self::is_load_more_event(&action) {
      self.last_load_more = Some(Instant::now());
    }
    // `is_loading` will be set to false again after the async action has finished in network.rs
    // The 5s playback poll (and its saved-tracks check) is a silent background
    // refresh — it must not flash the loading indicator.
    self.is_loading = !matches!(
      action,
      IoEvent::GetCurrentPlayback
        | IoEvent::CurrentUserSavedTracksContains(_)
        | IoEvent::GetCoverArt(_, _)
        | IoEvent::SoloistWsWatch { .. }
    );
    if let Some(io_tx) = &self.io_tx {
      if let Err(e) = io_tx.send(action) {
        self.is_loading = false;
        self.push_api_error(format!("dispatch failed: {}", e));
      };
    }
  }

  fn apply_seek(&mut self, seek_ms: u32) {
    let duration_ms = match &self.current_playback_context {
      Some(CurrentPlaybackContext {
        item: Some(item), ..
      }) => match item {
        PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
        PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
        _ => 0,
      },
      _ => return,
    };

    // Equality is a valid end-of-track seek (last bar cell); only a seek
    // past the end advances to the next track.
    let event = if (seek_ms as u128) <= duration_ms {
      self.seek_pending = self.seek_pending.saturating_add(1);
      self.song_progress_ms = seek_ms as u128;
      if let Some(ctx) = self.current_playback_context.as_mut() {
        ctx.progress = Some(chrono::TimeDelta::milliseconds(seek_ms as i64));
      }
      self.instant_since_last_current_playback_poll = Instant::now();
      self.track_end_polls = 0;
      IoEvent::Seek(seek_ms)
    } else {
      IoEvent::NextTrack
    };

    self.dispatch(event);
  }

  /// Preview the seek position immediately and dispatch the seek right away;
  /// `seek_ms` is cleared once the final in-flight seek settles.
  pub fn seek_to(&mut self, seek_ms: u32) {
    self.seek_ms = Some(seek_ms as u128);
    self.apply_seek(seek_ms);
  }

  /// Preview a scrub position without dispatching (used while dragging);
  /// the draw code renders `seek_ms` live.
  pub fn preview_seek(&mut self, seek_ms: u32) {
    self.seek_ms = Some(seek_ms as u128);
  }

  /// Optimistically show a clicked track in the playbar right away so the UI
  /// doesn't wait for the Spotify start-playback + GET /me/player round-trip.
  pub fn play_track_optimistic(&mut self, track: FullTrack) {
    if let Some(ctx) = self.current_playback_context.as_mut() {
      ctx.item = Some(PlayableItem::Track(track));
      ctx.is_playing = true;
      ctx.progress = Some(chrono::TimeDelta::zero());
      self.song_progress_ms = 0;
      self.seek_ms = None;
      self.track_end_polls = 0;
      self.instant_since_last_current_playback_poll = Instant::now();
      self.ui_dirty = true;
      self.ensure_cover_art();
    }
  }

  fn poll_current_playback(&mut self) {
    // WS loop owns state while live; it maintains the poll anchor itself.
    if self.soloist_live {
      return;
    }
    // Poll while playing at the user's cadence (settings, default 5s);
    // while paused poll less often so a playback started externally
    // (e.g. in the Spotify app) gets picked up without hammering the API.
    // A pending scrub (seek_ms) still forces one poll so the seek commits.
    let mut poll_interval_ms = if self.seek_ms.is_some()
      || !matches!(
        self.current_playback_context,
        Some(CurrentPlaybackContext {
          is_playing: false,
          ..
        })
      ) {
      self.user_config.behavior.playback_poll_secs.clamp(1, 30) as u128 * 1_000
    } else {
      15_000
    };

    let elapsed = self
      .instant_since_last_current_playback_poll
      .elapsed()
      .as_millis();

    let mut at_track_end = false;
    if let Some(overrun_ms) = self.playback_overrun_ms() {
      // Track's calculated end has passed: trigger an immediate refresh right
      // after the boundary (150ms grace for Spotify to flip tracks), then up to
      // 3 fast retries (350ms) before falling back to normal cadence so an
      // ended/empty queue never spams the API.
      if self.track_end_polls == 0 && overrun_ms >= 150 {
        poll_interval_ms = 0;
        at_track_end = true;
      } else if self.track_end_polls > 0 && self.track_end_polls <= 3 {
        poll_interval_ms = poll_interval_ms.min(350);
        at_track_end = true;
      }
    } else if let Some(remaining_ms) = self.playback_remaining_ms() {
      // Near track end the next song is imminent: keep a fresh progress anchor
      // so the track-end boundary fires on time without up to 5s of lag.
      if remaining_ms < 10_000 {
        poll_interval_ms = poll_interval_ms.min(1_000);
      }
    }

    if !self.is_fetching_current_playback && elapsed >= poll_interval_ms {
      self.is_fetching_current_playback = true;
      if at_track_end {
        self.track_end_polls = self.track_end_polls.saturating_add(1);
      }
      // A pending seek was already dispatched by seek_to (click, release,
      // seek dialog); this tick only fetches, and the response clears
      // seek_ms. Re-applying the seek here duplicated it every tick.
      self.dispatch(IoEvent::GetCurrentPlayback);
    }
  }

  /// Milliseconds past the current item's duration while playing, or None if
  /// not playing / still before track end.
  fn playback_overrun_ms(&self) -> Option<u128> {
    let context = self.current_playback_context.as_ref()?;
    if !context.is_playing {
      return None;
    }
    let item = context.item.as_ref()?;
    let progress = context.progress.as_ref()?;
    let duration_ms: u128 = match item {
      PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
      PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
      _ => return None,
    };
    if duration_ms == 0 {
      return None;
    }
    let pos = progress.num_milliseconds().max(0) as u128
      + self
        .instant_since_last_current_playback_poll
        .elapsed()
        .as_millis();
    (pos >= duration_ms).then(|| pos - duration_ms)
  }

  /// Milliseconds left in the current item, or None when unknown (no
  /// context/item/progress/duration). Live progress advances between polls,
  /// so elapsed time since the last poll is added while playing.
  fn playback_remaining_ms(&self) -> Option<u128> {
    let context = self.current_playback_context.as_ref()?;
    let item = context.item.as_ref()?;
    let progress = context.progress.as_ref()?;
    let duration_ms: u128 = match item {
      PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
      PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
      _ => return None,
    };
    let elapsed = if context.is_playing {
      self
        .instant_since_last_current_playback_poll
        .elapsed()
        .as_millis()
    } else {
      0
    };
    Some(duration_ms.saturating_sub(progress.num_milliseconds() as u128 + elapsed))
  }

  pub fn ensure_cover_art_url(&mut self, item_id: String, url: String) {
    if let Some((id, u)) = self.prepare_cover_art_url(item_id, url) {
      self.dispatch(IoEvent::GetCoverArt(id, u));
    }
  }

  /// Populate cover art from disk cache synchronously when available, or mark
  /// the item as fetching and return `(item_id, url)` for background download.
  pub fn prepare_cover_art_url(
    &mut self,
    item_id: String,
    url: String,
  ) -> Option<(String, String)> {
    let is_artist = item_id.starts_with("artist:");
    if is_artist {
      if self
        .artist_cover
        .as_ref()
        .is_some_and(|(id, _)| id == &item_id)
        || self.artist_cover_fetching.as_deref() == Some(item_id.as_str())
      {
        return None;
      }
      if let Some(img) = crate::cover_art::load_cached(&url) {
        self.artist_cover = Some((item_id, img));
        self.ui_dirty = true;
        return None;
      }
      self.artist_cover_fetching = Some(item_id.clone());
    } else {
      if self
        .cover_art
        .as_ref()
        .is_some_and(|(id, _)| id == &item_id)
        || self.cover_art_fetching.as_deref() == Some(item_id.as_str())
      {
        return None;
      }
      if let Some(img) = crate::cover_art::load_cached(&url) {
        self.cover_art = Some((item_id, img));
        self.ui_dirty = true;
        return None;
      }
      self.cover_art_fetching = Some(item_id.clone());
    }
    Some((item_id, url))
  }

  pub fn prepare_cover_art(&mut self) -> Option<(String, String)> {
    let (item_id, url) = crate::cover_art::now_playing(
      self
        .current_playback_context
        .as_ref()
        .and_then(|c| c.item.as_ref()),
    )?;
    self.prepare_cover_art_url(item_id, url)
  }

  pub fn ensure_cover_art(&mut self) {
    if let Some((item_id, url)) = self.prepare_cover_art() {
      self.dispatch(IoEvent::GetCoverArt(item_id, url));
    }
  }

  pub fn update_on_tick(&mut self) -> bool {
    if self.get_current_route().active_block == ActiveBlock::MusicView {
      self.ensure_cover_art();
    }
    self.poll_current_playback();
    if let Some(CurrentPlaybackContext {
      item: Some(item),
      progress: Some(progress),
      is_playing,
      ..
    }) = &self.current_playback_context
    {
      // Update progress even when the song is not playing,
      // because seeking is possible while paused
      let progress_ms = progress.num_milliseconds() as u128;
      let elapsed = if *is_playing {
        self
          .instant_since_last_current_playback_poll
          .elapsed()
          .as_millis()
      } else {
        0u128
      } + progress_ms;

      let duration_ms: u128 = match item {
        PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
        PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
        _ => 0,
      };

      let new_progress = if elapsed < duration_ms {
        elapsed
      } else {
        duration_ms
      };
      let prev = self.song_progress_ms;
      self.song_progress_ms = new_progress;
      // playbar renders whole seconds — redraw only when the visible
      // second flips (1 Hz while playing, 0 when paused) instead of every 250ms tick
      return (new_progress / 1000) != (prev / 1000);
    }
    false
  }

  pub fn seek_forwards(&mut self) {
    if let Some(CurrentPlaybackContext {
      item: Some(item), ..
    }) = &self.current_playback_context
    {
      let duration_ms: u128 = match item {
        PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
        PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
        _ => 0,
      };

      let old_progress = match self.seek_ms {
        Some(seek_ms) => seek_ms,
        None => self.song_progress_ms,
      };

      let new_progress = min(
        old_progress + self.user_config.behavior.seek_milliseconds as u128,
        duration_ms,
      );

      self.seek_to(new_progress as u32);
    }
  }

  pub fn seek_backwards(&mut self) {
    let old_progress = match self.seek_ms {
      Some(seek_ms) => seek_ms,
      None => self.song_progress_ms,
    };
    let new_progress =
      (old_progress as u32).saturating_sub(self.user_config.behavior.seek_milliseconds);
    self.seek_to(new_progress);
  }

  pub fn increase_volume(&mut self) {
    if let Some(volume_percent) = self
      .current_playback_context
      .as_ref()
      .map(|c| c.device.volume_percent.unwrap_or(0) as u8)
    {
      // Chain on the optimistic preview so rapid presses step from the
      // pending value instead of collapsing onto the stale server snapshot.
      let current_volume = self.volume_preview.unwrap_or(volume_percent);
      let next_volume = min(
        current_volume + self.user_config.behavior.volume_increment,
        100,
      );

      if next_volume != current_volume {
        // Optimistic preview so the bar moves immediately; cleared on API response.
        self.volume_preview = Some(next_volume);
        self.dispatch(IoEvent::ChangeVolume(next_volume));
      }
    }
  }

  pub fn decrease_volume(&mut self) {
    if let Some(volume_percent) = self
      .current_playback_context
      .as_ref()
      .map(|c| c.device.volume_percent.unwrap_or(0) as i8)
    {
      // Chain on the optimistic preview so rapid presses step from the
      // pending value instead of collapsing onto the stale server snapshot.
      let current_volume = self
        .volume_preview
        .map(|v| v as i8)
        .unwrap_or(volume_percent);
      let next_volume = max(
        current_volume - self.user_config.behavior.volume_increment as i8,
        0,
      );

      if next_volume != current_volume {
        // Optimistic preview so the bar moves immediately; cleared on API response.
        self.volume_preview = Some(next_volume as u8);
        self.dispatch(IoEvent::ChangeVolume(next_volume as u8));
      }
    }
  }

  pub fn handle_error(&mut self, e: anyhow::Error) {
    // Debug keeps the full anyhow chain (rspotify's parsed Spotify message);
    // Display would reduce a 403 to a bare status code.
    let msg = format!("{e:?}");
    crate::log_file::error(&msg);
    self.push_api_error(msg);
    // Don't pile Error routes: repeated startup failures would trap Esc
    // behind a stack of identical error screens.
    if self.get_current_route().id != RouteId::Error {
      self.push_navigation_stack(RouteId::Error, ActiveBlock::Error);
    }
  }

  pub fn push_api_error(&mut self, msg: String) {
    // Repeats add no information; count them on the existing entry.
    if let Some(existing) = self.api_errors.iter_mut().find(|e| e.text.contains(&msg)) {
      existing.count += 1;
      return;
    }
    if self.api_errors.len() >= 20 {
      self.api_errors.remove(0);
    }
    self.api_errors.push(ApiError {
      text: msg,
      count: 1,
    });
  }

  pub fn has_api_error(&self, msg: &str) -> bool {
    self.api_errors.iter().any(|e| e.text.contains(msg))
  }

  pub fn api_errors_text(&self) -> String {
    self
      .api_errors
      .iter()
      .enumerate()
      .map(|(i, e)| {
        if e.count > 1 {
          format!("{}: {} (×{})", i + 1, e.text, e.count)
        } else {
          format!("{}: {}", i + 1, e.text)
        }
      })
      .collect::<Vec<_>>()
      .join("\n\n")
  }

  pub fn toggle_playback(&mut self) {
    let is_playing = matches!(
      self.current_playback_context,
      Some(CurrentPlaybackContext {
        is_playing: true,
        ..
      })
    );
    let target_playing = !is_playing;
    // Optimistic update: flip the playback state immediately so the UI reacts
    // without waiting for the API round-trip. Generation counter lets the
    // backend drop intermediate clicks when toggled rapidly.
    if let Some(context) = self.current_playback_context.as_mut() {
      context.is_playing = target_playing;
    }
    self.play_toggle_gen = self.play_toggle_gen.wrapping_add(1);
    self.play_toggle_pending = self.play_toggle_pending.saturating_add(1);
    // OSC 7501 program status: playing→working (with title), else idle.
    // Terminals that don't know the sequence ignore it: zero-risk.
    emit_program_status(
      target_playing,
      &now_playing_title(self.current_playback_context.as_ref()),
    );
    self.dispatch(IoEvent::SetPlaybackState(
      target_playing,
      self.play_toggle_gen,
    ));
  }

  pub fn previous_track(&mut self) {
    if self.song_progress_ms >= 3_000 {
      self.seek_to(0);
    } else {
      self.dispatch(IoEvent::PreviousTrack);
    }
  }

  // The navigation_stack actually only controls the large block to the right of `library` and
  // `playlists`
  pub fn push_navigation_stack(&mut self, next_route_id: RouteId, next_active_block: ActiveBlock) {
    // Leaving the current view (tabs, other pages) drops the in-playlist search.
    self.playlist_filter = None;
    self.playlist_filter_idx = 0;
    self.playlist_filter_cursor_position = 0;
    if !self
      .navigation_stack
      .last()
      .map(|last_route| last_route.id == next_route_id)
      .unwrap_or(false)
    {
      self.navigation_stack.push(Route {
        id: next_route_id,
        active_block: next_active_block,
        hovered_block: next_active_block,
      });
    }
  }

  pub fn pop_navigation_stack(&mut self) -> Option<Route> {
    self.playlist_filter = None;
    self.playlist_filter_idx = 0;
    self.playlist_filter_cursor_position = 0;
    if self.navigation_stack.len() == 1 {
      None
    } else {
      self.navigation_stack.pop()
    }
  }

  pub fn get_current_route(&self) -> &Route {
    // if for some reason there is no route return the default
    self.navigation_stack.last().unwrap_or(&DEFAULT_ROUTE)
  }

  /// Resolve the playlist uri backing the current track table, honoring the
  /// two entry points: the sidebar (MyPlaylists) and search results
  /// (PlaylistSearch). Returns None when no playlist context is active.
  pub fn track_table_playlist_uri(&self) -> Option<String> {
    if matches!(
      self.track_table.context.as_ref(),
      Some(
        TrackTableContext::MyPlaylists
          | TrackTableContext::PlaylistSearch
          | TrackTableContext::MadeForYou
      )
    ) {
      if let Some(id) = &self.active_playlist_id {
        return Some(if id.starts_with("spotify:") {
          id.clone()
        } else {
          format!("spotify:playlist:{id}")
        });
      }
    }
    let (playlists, index) = match self.track_table.context.as_ref()? {
      TrackTableContext::MyPlaylists => (
        &self.playlists,
        self.active_playlist_index.or(self.selected_playlist_index),
      ),
      TrackTableContext::PlaylistSearch => (
        &self.search_results.playlists,
        self.search_results.selected_playlists_index,
      ),
      _ => return None,
    };
    let playlists = playlists.as_ref()?;
    let index = index
      .unwrap_or(0)
      .min(playlists.items.len().saturating_sub(1));
    playlists.items.get(index).map(|item| item.id.uri())
  }

  /// Bare id of the playlist shown in the track table, if any. Falls back
  /// from the active to the selected sidebar index: keyboard navigation
  /// changes the selection without activating, but the panel is the same.
  pub fn viewed_playlist_id(&self) -> Option<String> {
    if self.track_table.context != Some(TrackTableContext::MyPlaylists) {
      return None;
    }
    let idx = self
      .active_playlist_index
      .or(self.selected_playlist_index)?;
    self
      .playlists
      .as_ref()?
      .items
      .get(idx)
      .map(|pl| pl.id.id().to_string())
  }

  /// Cut one row from the visible track table by full uri (unlike / remove
  /// from playlist). Parallel added_at / raw_index entries go too, so clicks
  /// keep playing the right song; raw indices above the removed raw position
  /// shift down. Returns true when a row was removed.
  pub fn remove_track_from_table(&mut self, uri: &str) -> bool {
    let pre_len = self.track_table.tracks.len();
    let Some(idx) = self
      .track_table
      .tracks
      .iter()
      .position(|t| t.id.as_ref().map(|id| id.uri()).as_deref() == Some(uri))
    else {
      return false;
    };
    let aligned =
      self.track_table_added_at.len() == pre_len && self.track_table_raw_index.len() == pre_len;
    let removed_raw = aligned.then(|| self.track_table_raw_index[idx]);
    self.track_table.tracks.remove(idx);
    if aligned {
      self.track_table_added_at.remove(idx);
      self.track_table_raw_index.remove(idx);
      if let Some(p) = removed_raw {
        for r in self.track_table_raw_index.iter_mut() {
          if *r > p {
            *r -= 1;
          }
        }
      }
    }
    let len = self.track_table.tracks.len();
    self.track_table.selected_index = if len == 0 {
      0
    } else {
      self.track_table.selected_index.min(len - 1)
    };
    true
  }

  fn get_current_route_mut(&mut self) -> &mut Route {
    self.navigation_stack.last_mut().unwrap()
  }

  pub fn set_current_route_state(
    &mut self,
    active_block: Option<ActiveBlock>,
    hovered_block: Option<ActiveBlock>,
  ) {
    if let Some(active_block) = active_block {
      // Leaving the track table (clicking another panel/tab) drops the
      // in-playlist search focus.
      if active_block != ActiveBlock::TrackTable {
        self.playlist_filter = None;
        self.playlist_filter_idx = 0;
        self.playlist_filter_cursor_position = 0;
      }
      // Engaging a sidebar panel latches its highlight: the row stays marked
      // while browsing the page opened from it, until the user engages
      // something outside the sidebar (search box, gear, another block).
      match active_block {
        ActiveBlock::Library | ActiveBlock::MyPlaylists => {
          self.sidebar_latched_block = Some(active_block);
        }
        _ => {}
      }
      self.get_current_route_mut().active_block = active_block;
    }
    if let Some(hovered_block) = hovered_block {
      self.get_current_route_mut().hovered_block = hovered_block;
    }
  }

  fn copy_text(&mut self, text: String) {
    if self.clipboard.is_none() {
      self.clipboard = Clipboard::new().ok();
    }
    if let Some(clipboard) = self.clipboard.as_mut() {
      if let Err(e) = clipboard.set_text(text) {
        self.handle_error(anyhow!("failed to set clipboard content: {}", e));
      }
    }
  }

  pub fn copy_song_url(&mut self) {
    let url = match &self.current_playback_context {
      Some(CurrentPlaybackContext {
        item: Some(PlayableItem::Track(track)),
        ..
      }) => track
        .id
        .as_ref()
        .map(|id| format!("https://open.spotify.com/track/{}", id.id())),
      Some(CurrentPlaybackContext {
        item: Some(PlayableItem::Episode(episode)),
        ..
      }) => Some(format!(
        "https://open.spotify.com/episode/{}",
        episode.id.id()
      )),
      _ => None,
    };
    if let Some(url) = url {
      self.copy_text(url);
    }
  }

  pub fn copy_error(&mut self) {
    if self.api_errors.is_empty() {
      return;
    }
    let text = self.api_errors_text();
    self.copy_text(text);
  }

  pub fn copy_request_log(&mut self) {
    if self.request_log.is_empty() {
      return;
    }
    let text = self
      .request_log
      .iter()
      .map(|entry| {
        if entry.count > 1 {
          format!("{} x{}", entry.text, entry.count)
        } else {
          entry.text.clone()
        }
      })
      .collect::<Vec<String>>()
      .join("\n");
    self.copy_text(text);
  }

  pub fn copy_album_url(&mut self) {
    let url = match &self.current_playback_context {
      Some(CurrentPlaybackContext {
        item: Some(PlayableItem::Track(track)),
        ..
      }) => track
        .album
        .id
        .as_ref()
        .map(|id| format!("https://open.spotify.com/album/{}", id.id())),
      Some(CurrentPlaybackContext {
        item: Some(PlayableItem::Episode(episode)),
        ..
      }) => Some(format!(
        "https://open.spotify.com/show/{}",
        episode.show.id.id()
      )),
      _ => None,
    };
    if let Some(url) = url {
      self.copy_text(url);
    }
  }

  pub fn set_saved_tracks_to_table(&mut self, saved_track_page: &Page<SavedTrack>) {
    self.dispatch(IoEvent::SetTracksToTable(
      saved_track_page
        .items
        .clone()
        .into_iter()
        .map(|item| item.track)
        .collect::<Vec<FullTrack>>(),
    ));
  }

  pub fn set_saved_artists_to_table(&mut self, saved_artists_page: &CursorBasedPage<FullArtist>) {
    self.dispatch(IoEvent::SetArtistsToTable(
      saved_artists_page
        .items
        .clone()
        .into_iter()
        .collect::<Vec<FullArtist>>(),
    ))
  }

  pub fn get_current_user_saved_artists_next(&mut self) {
    match self
      .library
      .saved_artists
      .get_results(Some(self.library.saved_artists.index + 1))
      .cloned()
    {
      Some(saved_artists) => {
        self.set_saved_artists_to_table(&saved_artists);
        self.library.saved_artists.index += 1
      }
      None => {
        let last_id = self
          .library
          .saved_artists
          .get_results(None)
          .and_then(|saved_artists| saved_artists.items.last())
          .map(|last_artist| last_artist.id.to_string());
        if let Some(id) = last_id {
          self.dispatch(IoEvent::GetFollowedArtists(Some(id)));
        }
      }
    }
  }

  pub fn get_current_user_saved_artists_previous(&mut self) {
    if self.library.saved_artists.index > 0 {
      self.library.saved_artists.index -= 1;
    }

    if let Some(saved_artists) = &self.library.saved_artists.get_results(None).cloned() {
      self.set_saved_artists_to_table(saved_artists);
    }
  }

  pub fn get_current_user_saved_tracks_next(&mut self) {
    // Before fetching the next tracks, check if we have already fetched them
    match self
      .library
      .saved_tracks
      .get_results(Some(self.library.saved_tracks.index + 1))
      .cloned()
    {
      Some(saved_tracks) => {
        self.set_saved_tracks_to_table(&saved_tracks);
        self.library.saved_tracks.index += 1;
        self.is_fetching_next_page = false;
      }
      None => {
        if let Some(saved_tracks) = &self.library.saved_tracks.get_results(None) {
          let offset = Some(saved_tracks.offset + saved_tracks.limit);
          self.dispatch(IoEvent::GetCurrentSavedTracks(offset));
        }
      }
    }
  }

  pub fn get_current_user_saved_tracks_previous(&mut self) {
    if self.library.saved_tracks.index > 0 {
      self.library.saved_tracks.index -= 1;
    }

    if let Some(saved_tracks) = &self.library.saved_tracks.get_results(None).cloned() {
      self.set_saved_tracks_to_table(saved_tracks);
    }
  }

  pub fn shuffle(&mut self) {
    // Plain toggle. The flip is optimistic so the icon paints on the next
    // tick; the backend confirms on Ok and the next poll corrects on Err.
    if let Some(live) = &mut self.current_playback_context {
      let was_shuffle = live.shuffle_state;
      live.shuffle_state = !was_shuffle;
      self.dispatch(IoEvent::Shuffle(was_shuffle));
    }
  }

  pub fn get_current_user_saved_albums_next(&mut self) {
    if self
      .library
      .saved_albums
      .get_results(Some(self.library.saved_albums.index + 1))
      .is_some()
    {
      self.library.saved_albums.index += 1;
    } else if let Some(offset) = self
      .library
      .saved_albums
      .get_results(None)
      .map(|saved_albums| saved_albums.offset + saved_albums.limit)
    {
      self.dispatch(IoEvent::GetCurrentUserSavedAlbums(Some(offset)));
    }
  }

  pub fn get_current_user_saved_albums_previous(&mut self) {
    if self.library.saved_albums.index > 0 {
      self.library.saved_albums.index -= 1;
    }
  }

  pub fn current_user_saved_album_delete(&mut self, block: ActiveBlock) {
    match block {
      ActiveBlock::SearchResultBlock => {
        if let Some(albums) = &self.search_results.albums {
          if let Some(selected_index) = self.search_results.selected_album_index {
            let selected_album = &albums.items[selected_index];
            if let Some(album_id) = selected_album.id.clone() {
              self.dispatch(IoEvent::CurrentUserSavedAlbumDelete(album_id.to_string()));
            }
          }
        }
      }
      ActiveBlock::AlbumList => {
        if let Some(albums) = self.library.saved_albums.get_results(None) {
          if let Some(selected_album) = albums.items.get(self.album_list_index) {
            let album_id = selected_album.album.id.to_string();
            self.dispatch(IoEvent::CurrentUserSavedAlbumDelete(album_id));
          }
        }
      }
      ActiveBlock::ArtistBlock => {
        if let Some(artist) = &self.artist {
          if let Some(selected_album) = artist.albums.items.get(artist.selected_album_index) {
            if let Some(album_id) = selected_album.id.clone() {
              self.dispatch(IoEvent::CurrentUserSavedAlbumDelete(album_id.to_string()));
            }
          }
        }
      }
      _ => (),
    }
  }

  pub fn current_user_saved_album_add(&mut self, block: ActiveBlock) {
    match block {
      ActiveBlock::SearchResultBlock => {
        if let Some(albums) = &self.search_results.albums {
          if let Some(selected_index) = self.search_results.selected_album_index {
            let selected_album = &albums.items[selected_index];
            if let Some(album_id) = selected_album.id.clone() {
              self.dispatch(IoEvent::CurrentUserSavedAlbumAdd(album_id.to_string()));
            }
          }
        }
      }
      ActiveBlock::ArtistBlock => {
        if let Some(artist) = &self.artist {
          if let Some(selected_album) = artist.albums.items.get(artist.selected_album_index) {
            if let Some(album_id) = selected_album.id.clone() {
              self.dispatch(IoEvent::CurrentUserSavedAlbumAdd(album_id.to_string()));
            }
          }
        }
      }
      _ => (),
    }
  }

  pub fn get_current_user_saved_shows_next(&mut self) {
    if self
      .library
      .saved_shows
      .get_results(Some(self.library.saved_shows.index + 1))
      .is_some()
    {
      self.library.saved_shows.index += 1;
    } else if let Some(offset) = self
      .library
      .saved_shows
      .get_results(None)
      .map(|saved_shows| saved_shows.offset + saved_shows.limit)
    {
      self.dispatch(IoEvent::GetCurrentUserSavedShows(Some(offset)));
    }
  }

  pub fn get_current_user_saved_shows_previous(&mut self) {
    if self.library.saved_shows.index > 0 {
      self.library.saved_shows.index -= 1;
    }
  }

  pub fn get_episode_table_next(&mut self, show_id: String) {
    if self
      .library
      .show_episodes
      .get_results(Some(self.library.show_episodes.index + 1))
      .is_some()
    {
      self.library.show_episodes.index += 1;
    } else if let Some(offset) = self
      .library
      .show_episodes
      .get_results(None)
      .map(|show_episodes| show_episodes.offset + show_episodes.limit)
    {
      self.dispatch(IoEvent::GetCurrentShowEpisodes(show_id, Some(offset)));
    }
  }

  pub fn get_episode_table_previous(&mut self) {
    if self.library.show_episodes.index > 0 {
      self.library.show_episodes.index -= 1;
    }
  }

  pub fn user_unfollow_artists(&mut self, block: ActiveBlock) {
    match block {
      ActiveBlock::SearchResultBlock => {
        if let Some(artists) = &self.search_results.artists {
          if let Some(selected_index) = self.search_results.selected_artists_index {
            let selected_artist: &FullArtist = &artists.items[selected_index];
            let artist_id = selected_artist.id.to_string();
            self.dispatch(IoEvent::UserUnfollowArtists(vec![artist_id]));
          }
        }
      }
      ActiveBlock::AlbumList => {
        if let Some(artists) = self.library.saved_artists.get_results(None) {
          if let Some(selected_artist) = artists.items.get(self.artists_list_index) {
            let artist_id = selected_artist.id.to_string();
            self.dispatch(IoEvent::UserUnfollowArtists(vec![artist_id]));
          }
        }
      }
      ActiveBlock::ArtistBlock => {
        if let Some(artist) = &self.artist {
          let selected_artis = &artist.related_artists[artist.selected_related_artist_index];
          let artist_id = selected_artis.id.to_string();
          self.dispatch(IoEvent::UserUnfollowArtists(vec![artist_id]));
        }
      }
      _ => (),
    };
  }

  pub fn user_follow_artists(&mut self, block: ActiveBlock) {
    match block {
      ActiveBlock::SearchResultBlock => {
        if let Some(artists) = &self.search_results.artists {
          if let Some(selected_index) = self.search_results.selected_artists_index {
            let selected_artist: &FullArtist = &artists.items[selected_index];
            let artist_id = selected_artist.id.to_string();
            self.dispatch(IoEvent::UserFollowArtists(vec![artist_id]));
          }
        }
      }
      ActiveBlock::ArtistBlock => {
        if let Some(artist) = &self.artist {
          let selected_artis = &artist.related_artists[artist.selected_related_artist_index];
          let artist_id = selected_artis.id.to_string();
          self.dispatch(IoEvent::UserFollowArtists(vec![artist_id]));
        }
      }
      _ => (),
    }
  }

  pub fn user_follow_playlist(&mut self) {
    if let SearchResult {
      playlists: Some(ref playlists),
      selected_playlists_index: Some(selected_index),
      ..
    } = self.search_results
    {
      let selected_playlist: &SimplifiedPlaylist = &playlists.items[selected_index];
      let selected_id = selected_playlist.id.clone();
      let selected_public = selected_playlist.public;
      let selected_owner_id = selected_playlist.owner.id.clone();
      self.dispatch(IoEvent::UserFollowPlaylist(
        selected_owner_id.to_string(),
        selected_id.to_string(),
        selected_public,
      ));
    }
  }

  pub fn user_unfollow_playlist(&mut self) {
    if let (Some(playlists), Some(selected_index), Some(user)) =
      (&self.playlists, self.selected_playlist_index, &self.user)
    {
      let selected_playlist = &playlists.items[selected_index];
      let selected_id = selected_playlist.id.clone();
      let user_id = user.id.clone();
      self.dispatch(IoEvent::UserUnfollowPlaylist(
        user_id.to_string(),
        selected_id.to_string(),
      ))
    }
  }

  /// URI of the track the add-to-playlist picker was opened for: the
  /// currently selected row of the active song list.
  pub fn selected_track_uri(&self) -> Option<String> {
    match self.get_current_route().active_block {
      ActiveBlock::TrackTable => self
        .track_table
        .tracks
        .get(self.track_table.selected_index)
        .and_then(|t| t.id.as_ref().map(|id| id.uri())),
      ActiveBlock::AlbumTracks => {
        if let Some(simplified) = &self.selected_album_simplified {
          simplified
            .tracks
            .items
            .get(simplified.selected_index)
            .and_then(|t| t.id.as_ref().map(|id| id.uri()))
        } else {
          self
            .selected_album_full
            .as_ref()
            .and_then(|full| full.album.tracks.items.get(full.selected_index))
            .and_then(|t| t.id.as_ref().map(|id| id.uri()))
        }
      }
      _ => None,
    }
  }

  /// Open the add-to-playlist picker for the selected track (no-op when no
  /// track is selected or the feature is disabled).
  pub fn open_add_to_playlist(&mut self) {
    if !self.user_config.behavior.enable_add_to_playlist {
      return;
    }
    let Some(uri) = self.selected_track_uri() else {
      return;
    };
    self.pending_track_uri = Some(uri);
    self.playlist_picker_index = 0;
    self.naming_playlist = false;
    self.new_playlist_name.clear();
    self.push_navigation_stack(
      RouteId::Dialog,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist),
    );
  }

  pub fn prompt_remove_selected_track_from_playlist(&mut self) {
    if !self.user_config.behavior.enable_remove_from_playlist {
      return;
    }
    let Some(track_uri) = self.selected_track_uri() else {
      return;
    };
    let Some(playlist_uri) = self.track_table_playlist_uri() else {
      return;
    };
    let track_name = self
      .track_table
      .tracks
      .get(self.track_table.selected_index)
      .map(|t| t.name.clone())
      .unwrap_or_default();
    let playlist_name = self
      .playlists
      .as_ref()
      .and_then(|p| {
        let idx = self
          .selected_playlist_index
          .or(self.active_playlist_index)?;
        p.items.get(idx).map(|pl| pl.name.clone())
      })
      .unwrap_or_default();
    self.pending_remove_track_uri = Some(track_uri);
    self.pending_remove_playlist_id = Some(playlist_uri);
    self.pending_remove_track_name = Some(track_name);
    self.pending_remove_playlist_name = Some(playlist_name);
    self.confirm = false;
    self.push_navigation_stack(
      RouteId::Dialog,
      ActiveBlock::Dialog(DialogContext::ConfirmRemoveFromPlaylist),
    );
  }

  pub fn confirm_pending_remove(&mut self) {
    if self
      .last_remove_time
      .map(|t| t.elapsed() < Duration::from_secs(5))
      .unwrap_or(false)
    {
      self.pending_remove_track_uri = None;
      self.pending_remove_playlist_id = None;
      self.pending_remove_track_name = None;
      self.pending_remove_playlist_name = None;
      return;
    }
    if let (Some(uri), Some(pid)) = (
      self.pending_remove_track_uri.take(),
      self.pending_remove_playlist_id.take(),
    ) {
      self.pending_remove_track_name = None;
      self.pending_remove_playlist_name = None;
      self.last_remove_time = Some(Instant::now());
      self.dispatch(IoEvent::RemoveTrackFromPlaylist(uri, pid));
    }
  }

  /// True when the uri appears in any cached playlist other than `exclude`
  /// (the playlist currently being viewed).
  pub fn playlist_contains(&self, uri: &str, exclude: Option<&str>) -> bool {
    self
      .playlist_uri_map
      .iter()
      .any(|(id, uris)| uris.contains(uri) && exclude.map(|e| e != id).unwrap_or(true))
  }

  pub fn is_in_any_playlist(&self, uri: &str) -> bool {
    self.playlist_contains(uri, None)
  }

  pub fn playlists_containing(&self, uri: &str) -> Vec<String> {
    let mut names: Vec<String> = self
      .playlist_uri_map
      .iter()
      .filter(|(_, uris)| uris.contains(uri))
      .filter_map(|(id, _)| {
        self
          .playlists
          .as_ref()?
          .items
          .iter()
          .find(|p| p.id.to_string() == *id || p.id.uri() == *id)
          .map(|p| p.name.clone())
      })
      .collect();
    names.sort();
    names.dedup();
    names
  }

  pub fn playing_track_uri(&self) -> Option<String> {
    match &self.current_playback_context.as_ref()?.item {
      Some(PlayableItem::Track(t)) => t.id.as_ref().map(|id| id.uri()),
      _ => None,
    }
  }

  /// Full uri of the now-playing track, if a track (not an episode) is playing.
  pub fn now_playing_track_id(&self) -> Option<String> {
    match self.current_playback_context.as_ref()?.item.as_ref()? {
      PlayableItem::Track(t) => t.id.as_ref().map(|id| id.to_string()),
      _ => None,
    }
  }

  pub fn open_add_to_playlist_for_uri(&mut self, uri: String) {
    if !self.user_config.behavior.enable_add_to_playlist {
      return;
    }
    self.pending_track_uri = Some(uri);
    self.playlist_picker_index = 0;
    self.naming_playlist = false;
    self.new_playlist_name.clear();
    self.push_navigation_stack(
      RouteId::Dialog,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist),
    );
  }

  pub fn user_unfollow_playlist_search_result(&mut self) {
    if let (Some(playlists), Some(selected_index), Some(user)) = (
      &self.search_results.playlists,
      self.search_results.selected_playlists_index,
      &self.user,
    ) {
      let selected_playlist = &playlists.items[selected_index];
      let selected_id = selected_playlist.id.clone();
      let user_id = user.id.clone();
      self.dispatch(IoEvent::UserUnfollowPlaylist(
        user_id.to_string(),
        selected_id.to_string(),
      ))
    }
  }

  pub fn user_follow_show(&mut self, block: ActiveBlock) {
    match block {
      ActiveBlock::SearchResultBlock => {
        if let Some(shows) = &self.search_results.shows {
          if let Some(selected_index) = self.search_results.selected_shows_index {
            if let Some(show_id) = shows.items.get(selected_index).map(|item| item.id.clone()) {
              self.dispatch(IoEvent::CurrentUserSavedShowAdd(show_id.to_string()));
            }
          }
        }
      }
      ActiveBlock::EpisodeTable => match self.episode_table_context {
        EpisodeTableContext::Full => {
          if let Some(selected_episode) = self.selected_show_full.clone() {
            let show_id = selected_episode.show.id;
            self.dispatch(IoEvent::CurrentUserSavedShowAdd(show_id.to_string()));
          }
        }
        EpisodeTableContext::Simplified => {
          if let Some(selected_episode) = self.selected_show_simplified.clone() {
            let show_id = selected_episode.show.id;
            self.dispatch(IoEvent::CurrentUserSavedShowAdd(show_id.to_string()));
          }
        }
      },
      _ => (),
    }
  }

  pub fn user_unfollow_show(&mut self, block: ActiveBlock) {
    match block {
      ActiveBlock::Podcasts => {
        if let Some(shows) = self.library.saved_shows.get_results(None) {
          if let Some(selected_show) = shows.items.get(self.shows_list_index) {
            let show_id = selected_show.show.id.to_string();
            self.dispatch(IoEvent::CurrentUserSavedShowDelete(show_id));
          }
        }
      }
      ActiveBlock::SearchResultBlock => {
        if let Some(shows) = &self.search_results.shows {
          if let Some(selected_index) = self.search_results.selected_shows_index {
            let show_id = shows.items[selected_index].id.to_string();
            self.dispatch(IoEvent::CurrentUserSavedShowDelete(show_id));
          }
        }
      }
      ActiveBlock::EpisodeTable => match self.episode_table_context {
        EpisodeTableContext::Full => {
          if let Some(selected_episode) = self.selected_show_full.clone() {
            let show_id = selected_episode.show.id;
            self.dispatch(IoEvent::CurrentUserSavedShowDelete(show_id.to_string()));
          }
        }
        EpisodeTableContext::Simplified => {
          if let Some(selected_episode) = self.selected_show_simplified.clone() {
            let show_id = selected_episode.show.id;
            self.dispatch(IoEvent::CurrentUserSavedShowDelete(show_id.to_string()));
          }
        }
      },
      _ => (),
    }
  }

  pub fn expand_made_for_you(&mut self, index: usize) {
    self.track_table.context = Some(TrackTableContext::MadeForYou);
    self.playlist_offset = 0;
    self.made_for_you_offset = 0;
    if let Some(playlist_id) = self.made_for_you_playlist_id(index) {
      self.dispatch(IoEvent::GetMadeForYouPlaylistItems(playlist_id, 0));
    }
  }

  /// Add the playlist the user pasted into the search box to "For you".
  /// If an entry with `id` already exists (e.g. placeholder) and `name` is
  /// non-empty, update its display name in place.
  pub fn add_pasted_playlist_to_for_you(&mut self, name: String, id: String) {
    if let Some(entry) = self
      .made_for_you_custom
      .iter_mut()
      .find(|(_, existing)| existing == &id)
    {
      if !name.is_empty() {
        entry.0 = name;
      }
      return;
    }
    let display = if name.is_empty() { id.clone() } else { name };
    self.made_for_you_custom.push((display, id));
  }

  /// Extract a Spotify playlist ID from a `https://open.spotify.com/playlist/<id>`
  /// URL or `spotify:playlist:<id>` URI.
  pub fn extract_playlist_id(raw: &str) -> Option<String> {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("spotify:playlist:") {
      let id = rest.split([':', '?', '#']).next()?.trim();
      return (!id.is_empty()).then(|| id.to_string());
    }
    if let Some(pos) = s.find("open.spotify.com/playlist/") {
      let after = &s[pos + "open.spotify.com/playlist/".len()..];
      let id = after.split(['/', '?', '#']).next()?.trim();
      return (!id.is_empty()).then(|| id.to_string());
    }
    None
  }

  /// Extract all unique Spotify playlist IDs from whitespace/comma/semicolon-separated text.
  pub fn extract_playlist_ids(raw: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for token in raw.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';')) {
      if let Some(id) = Self::extract_playlist_id(token) {
        if !ids.contains(&id) {
          ids.push(id);
        }
      }
    }
    ids
  }

  /// Parse one or more pasted playlist links/URIs and pin them into "For you".
  /// A single link also opens immediately; multiple links stay in "For you" and
  /// resolve each playlist's display name in the background.
  pub fn paste_playlist_to_for_you(&mut self, raw: &str) -> bool {
    let ids = Self::extract_playlist_ids(raw);
    if ids.is_empty() {
      return false;
    }
    for id in &ids {
      self.add_pasted_playlist_to_for_you(String::new(), id.clone());
    }
    if let Some(pos) = self
      .made_for_you_custom
      .iter()
      .position(|(_, existing)| existing == &ids[0])
    {
      self.made_for_you_index = pos;
    }
    if ids.len() == 1 {
      self.track_table.context = Some(TrackTableContext::MadeForYou);
      self.playlist_offset = 0;
      self.made_for_you_offset = 0;
      self.dispatch(IoEvent::GetMadeForYouPlaylistItems(ids[0].clone(), 0));
    } else {
      for id in ids {
        self.dispatch(IoEvent::ResolveForYouPlaylistName(id));
      }
    }
    true
  }

  /// Read the system clipboard and pin any Spotify playlist link into "For you".
  pub fn paste_clipboard_to_for_you(&mut self) {
    if self.clipboard.is_none() {
      self.clipboard = Clipboard::new().ok();
    }
    let text = self
      .clipboard
      .as_mut()
      .and_then(|cb| cb.get_text().ok())
      .unwrap_or_default();
    if !self.paste_playlist_to_for_you(&text) {
      self.handle_error(anyhow!(
        "Clipboard does not contain a Spotify playlist URL or URI (expected open.spotify.com/playlist/... or spotify:playlist:...)"
      ));
    }
  }

  /// Remove a pasted playlist from "For you".
  pub fn remove_pasted_playlist_from_for_you(&mut self, index: usize) {
    if index < self.made_for_you_custom.len() {
      self.made_for_you_custom.remove(index);
    }
  }

  /// Total number of rows in the "For you" list.
  pub fn made_for_you_len(&self) -> usize {
    self.made_for_you_custom.len()
  }

  pub fn made_for_you_name(&self, index: usize) -> Option<String> {
    self
      .made_for_you_custom
      .get(index)
      .map(|(name, _)| name.clone())
  }

  pub fn made_for_you_playlist_id(&self, index: usize) -> Option<String> {
    self
      .made_for_you_custom
      .get(index)
      .map(|(_, id)| id.clone())
  }

  /// Refresh all Music View data (cover art, lyrics, credits, queue) on view
  /// open or when the playing track changes while Music View is open.
  pub fn refresh_music_view_data(&mut self) {
    self.lyrics = None;
    self.track_credits = None;
    self.queue_next = None;
    self.get_panel_data();
  }

  pub fn get_panel_data(&mut self) {
    // Dispatch cover art first so it never queues behind lyrics/credits/queue.
    self.ensure_cover_art();
    let track_id = match &self.current_playback_context {
      Some(CurrentPlaybackContext {
        item: Some(PlayableItem::Track(track)),
        ..
      }) => track.id.as_ref().map(|id| id.id().to_string()),
      _ => None,
    };
    if let Some(id) = track_id {
      self.dispatch(IoEvent::GetLyrics);
      self.dispatch(IoEvent::GetTrackCredits(id));
    }
    self.dispatch(IoEvent::GetQueue);
  }

  pub fn repeat(&mut self) {
    if let Some(live) = &mut self.current_playback_context {
      let prev = live.repeat_state;
      live.repeat_state = match prev {
        RepeatState::Off => RepeatState::Context,
        RepeatState::Context => RepeatState::Track,
        RepeatState::Track => RepeatState::Off,
      };
      self.repeat_pending = self.repeat_pending.saturating_add(1);
      // Optimistic flip so the icon paints on the next tick; backend skips
      // superseded clicks so rapid presses never jiggle backwards.
      self.dispatch(IoEvent::Repeat(prev));
    }
  }

  pub fn get_artist(&mut self, artist_id: String, input_artist_name: String) {
    self.dispatch(IoEvent::GetArtist(artist_id.clone(), input_artist_name));
    // Albums load eagerly so the artist profile never shows an empty
    // Albums tab; the remaining tabs stay lazy (fetched on first visit).
    self.dispatch(IoEvent::GetArtistAlbumsMore(artist_id, 0));
  }

  /// Activate an artist tab, lazily fetching its data on first use.
  pub fn artist_select_tab(&mut self, tab: ArtistBlock) {
    let (needs_albums, needs_singles, needs_appears_on) = match &self.artist {
      Some(artist) => (
        tab == ArtistBlock::Albums && artist.albums.items.is_empty(),
        tab == ArtistBlock::Singles && artist.singles.items.is_empty(),
        tab == ArtistBlock::AppearsOn && artist.appears_on.items.is_empty(),
      ),
      None => return,
    };
    let artist_id = self.artist.as_ref().map(|artist| artist.artist_id.clone());
    if let Some(artist) = &mut self.artist {
      artist.artist_selected_block = tab;
      artist.artist_hovered_block = tab;
    }
    if needs_albums {
      if let Some(artist_id) = artist_id.clone() {
        self.dispatch(IoEvent::GetArtistAlbumsMore(artist_id, 0));
      }
    }
    if needs_singles {
      if let Some(artist_id) = artist_id.clone() {
        self.dispatch(IoEvent::GetArtistSinglesMore(artist_id, 0));
      }
    }
    if needs_appears_on {
      if let Some(artist_id) = artist_id.clone() {
        self.dispatch(IoEvent::GetArtistAppearsOnMore(artist_id, 0));
      }
    }
    if tab == ArtistBlock::EPs {
      // EPs are derived from singles; ensure the backing singles are loaded
      let singles_loaded = self
        .artist
        .as_ref()
        .map(|artist| !artist.singles.items.is_empty())
        .unwrap_or(false);
      if !singles_loaded {
        if let Some(artist_id) = artist_id {
          self.dispatch(IoEvent::GetArtistSinglesMore(artist_id, 0));
        }
      }
      // if singles not empty but eps empty, derive immediately
      if let Some(artist) = &mut self.artist {
        if artist.eps.items.is_empty() && !artist.singles.items.is_empty() {
          self.derive_eps_from_singles();
        }
      }
    }
  }

  fn derive_eps_from_singles(&mut self) {
    if let Some(artist) = &mut self.artist {
      // SimplifiedAlbum no longer has total_tracks in rspotify 0.16 (removed Feb 2026);
      // heuristic: name contains EP — fallback clone all singles and filter by name hint.
      // Tracks count unavailable without FullAlbum fetch.
      let eps_items = artist
        .singles
        .items
        .iter()
        .filter(|a| a.name.to_lowercase().contains("ep"))
        .cloned()
        .collect::<Vec<_>>();
      // If no EP-named items, leave EPs empty (not faking singles as EPs)
      let total = eps_items.len() as u32;
      artist.eps.items = eps_items;
      artist.eps.total = total;
      artist.eps.offset = 0;
      // Keep limit/next consistent with source page shape
      artist.eps.limit = artist.singles.limit;
      artist.eps.next = None;
      artist.eps.previous = None;
      artist.eps.href = artist.singles.href.clone();
    }
  }

  /// Continuation paging for lists: dispatch `event` (already carrying the
  /// next offset) only when more items exist past what we hold.
  fn load_more_page(&mut self, loaded: usize, total: usize, event: IoEvent) {
    if loaded < total {
      self.dispatch(event);
    }
  }

  pub fn load_more_albums(&mut self) {
    if let Some(artist) = &self.artist {
      let loaded = artist.albums.items.len() as u32;
      self.load_more_page(
        artist.albums.items.len(),
        artist.albums.total as usize,
        IoEvent::GetArtistAlbumsMore(artist.artist_id.clone(), loaded),
      );
    }
  }

  pub fn load_more_singles(&mut self) {
    if let Some(artist) = &self.artist {
      self.load_more_page(
        artist.singles.items.len(),
        artist.singles.total as usize,
        IoEvent::GetArtistSinglesMore(artist.artist_id.clone(), artist.singles.items.len() as u32),
      );
    }
  }

  pub fn load_more_appears_on(&mut self) {
    if let Some(artist) = &self.artist {
      self.load_more_page(
        artist.appears_on.items.len(),
        artist.appears_on.total as usize,
        IoEvent::GetArtistAppearsOnMore(
          artist.artist_id.clone(),
          artist.appears_on.items.len() as u32,
        ),
      );
    }
  }

  pub fn load_more_artist_top_tracks(&mut self) {
    if let Some(artist) = &self.artist {
      let loaded = artist.top_tracks.len() as u32;
      // No total gate: the search `total` under-reports for many artists,
      // so dispatch whenever the load-more row is visible. A short page on
      // the next fetch clears top_tracks_has_more and hides the row.
      self.dispatch(IoEvent::GetArtistTopTracksMore(
        artist.artist_id.clone(),
        artist.artist_name.clone(),
        loaded,
      ));
    }
  }

  pub fn load_more_album_tracks(&mut self) {
    if let Some(album) = &self.selected_album_simplified {
      if let Some(album_id) = &album.album.id {
        let loaded = album.tracks.items.len() as u32;
        self.load_more_page(
          album.tracks.items.len(),
          album.tracks.total as usize,
          IoEvent::GetAlbumTracksMore(album_id.to_string(), loaded),
        );
      }
    }
  }

  /// Load the next page of recently-played history (before-cursor paging).
  pub fn load_more_recently_played(&mut self) {
    if self.is_fetching_next_page {
      return;
    }
    let before = self
      .recently_played
      .result
      .as_ref()
      .and_then(|p| p.cursors.as_ref().and_then(|c| c.after.clone()));
    self.dispatch(IoEvent::GetMoreRecentlyPlayed(before));
  }

  /// Whether the recently-played list has more pages (full-page rule: the
  /// endpoint under-reports totals, so a full page means more exist).
  pub fn recently_played_has_more(&self) -> bool {
    self
      .recently_played
      .result
      .as_ref()
      .is_some_and(|p| p.limit > 0 && p.items.len() >= p.limit as usize)
  }

  /// Whether the current track-table context has more pages to load
  /// (playlist / saved tracks / made-for-you page past the loaded items).
  pub fn track_table_has_more(&self) -> bool {
    match self.track_table.context {
      Some(TrackTableContext::MyPlaylists) | Some(TrackTableContext::PlaylistSearch) => self
        .playlist_tracks
        .as_ref()
        .map(|p| p.items.len() < p.total as usize)
        .unwrap_or(false),
      Some(TrackTableContext::SavedTracks) => self
        .library
        .saved_tracks
        .get_results(None)
        .map(|p| self.track_table.tracks.len() < p.total as usize)
        .unwrap_or(false),
      Some(TrackTableContext::MadeForYou) => self
        .made_for_you_tracks
        .as_ref()
        .map(|p| p.items.len() < p.total as usize)
        .unwrap_or(false),
      _ => false,
    }
  }

  /// Remaining items past the loaded page for the current track-table context.
  pub fn track_table_remaining(&self) -> Option<usize> {
    match self.track_table.context {
      Some(TrackTableContext::MyPlaylists) | Some(TrackTableContext::PlaylistSearch) => self
        .playlist_tracks
        .as_ref()
        .map(|p| p.total as usize - p.items.len()),
      Some(TrackTableContext::SavedTracks) => self
        .library
        .saved_tracks
        .get_results(None)
        .map(|p| p.total as usize - self.track_table.tracks.len()),
      Some(TrackTableContext::MadeForYou) => self
        .made_for_you_tracks
        .as_ref()
        .map(|p| p.total as usize - p.items.len()),
      _ => None,
    }
  }

  /// The in-playlist search bar is only meaningful on a playlist track table.
  pub fn playlist_search_active(&self) -> bool {
    matches!(
      self.track_table.context,
      Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
    ) && self.playlist_filter.is_some()
  }

  /// Client-side filter predicate for the in-playlist search bar: a track
  /// matches when the (lowercased) query is a substring of title or any artist
  /// name. Empty query matches all.
  pub fn track_matches_needle(track: &FullTrack, needle: &str) -> bool {
    track.name.to_lowercase().contains(needle)
      || track
        .artists
        .iter()
        .any(|a| a.name.to_lowercase().contains(needle))
  }

  pub fn playlist_filter_matches(&self, track: &FullTrack) -> bool {
    let needle = match self.playlist_filter.as_deref() {
      Some(q) if !q.is_empty() => q.to_lowercase(),
      _ => return true,
    };
    Self::track_matches_needle(track, &needle)
  }

  /// Whether the given search block has more pages past what is loaded.
  /// A full last page means more can be fetched: the search `total`
  /// under-reports for many queries, so gating on it kills the load-more
  /// row early (same disease as the artist top tracks).
  pub fn search_block_has_more(&self, block: &SearchResultBlock) -> bool {
    match block {
      SearchResultBlock::AlbumSearch => self
        .search_results
        .albums
        .as_ref()
        .is_some_and(page_has_more),
      SearchResultBlock::SongSearch => self
        .search_results
        .tracks
        .as_ref()
        .is_some_and(page_has_more),
      SearchResultBlock::ArtistSearch => self
        .search_results
        .artists
        .as_ref()
        .is_some_and(page_has_more),
      SearchResultBlock::PlaylistSearch => self
        .search_results
        .playlists
        .as_ref()
        .is_some_and(page_has_more),
      SearchResultBlock::ShowSearch => self
        .search_results
        .shows
        .as_ref()
        .is_some_and(page_has_more),
      SearchResultBlock::Empty => false,
    }
  }

  /// Whether the given search block has a page loaded already.
  pub fn search_block_loaded(&self, block: &SearchResultBlock) -> bool {
    match block {
      SearchResultBlock::AlbumSearch => self.search_results.albums.is_some(),
      SearchResultBlock::SongSearch => self.search_results.tracks.is_some(),
      SearchResultBlock::ArtistSearch => self.search_results.artists.is_some(),
      SearchResultBlock::PlaylistSearch => self.search_results.playlists.is_some(),
      SearchResultBlock::ShowSearch => self.search_results.shows.is_some(),
      SearchResultBlock::Empty => false,
    }
  }

  /// Expand the given search tab: mark it selected and fetch its first page
  /// if it has not been loaded yet.
  pub fn load_search_block(&mut self, block: &SearchResultBlock) {
    self.search_results.selected_block = block.clone();
    if self.search_results.query.is_empty() || self.search_block_loaded(block) {
      return;
    }
    self.is_fetching_next_page = true;
    self.dispatch(IoEvent::GetMoreSearchResults(block.clone()));
  }

  /// Dispatch a fetch of the next page for the given search block. The next
  /// offset is derived backend-side from the block's current Page.offset.
  pub fn load_more_search_block(&mut self, block: &SearchResultBlock) {
    if self.is_fetching_next_page || self.search_results.query.is_empty() {
      return;
    }
    if !self.search_block_has_more(block) {
      return;
    }
    self.is_fetching_next_page = true;
    self.dispatch(IoEvent::GetMoreSearchResults(block.clone()));
  }

  /// Reset search state for a new search term.
  pub fn reset_search_results(&mut self) {
    self.search_results.albums = None;
    self.search_results.artists = None;
    self.search_results.playlists = None;
    self.search_results.tracks = None;
    self.search_results.shows = None;
    self.search_results.selected_album_index = None;
    self.search_results.selected_artists_index = None;
    self.search_results.selected_playlists_index = None;
    self.search_results.selected_tracks_index = None;
    self.search_results.selected_shows_index = None;
    self.search_results.selected_block = SearchResultBlock::Empty;
  }

  /// Fetch the next page of the current track-table context, mirroring the
  /// per-context paging in handlers/track_table.rs.
  pub fn load_more_tracks(&mut self) {
    match self.track_table.context {
      Some(TrackTableContext::MyPlaylists) => {
        if let (Some(playlists), Some(selected_playlist_index)) =
          (&self.playlists, &self.selected_playlist_index)
        {
          if let (Some(selected_playlist), Some(playlist_tracks)) = (
            playlists.items.get(selected_playlist_index.to_owned()),
            &self.playlist_tracks,
          ) {
            let offset = playlist_tracks.items.len() as u32;
            let playlist_id = selected_playlist.id.to_string();
            self.load_more_page(
              playlist_tracks.items.len(),
              playlist_tracks.total as usize,
              IoEvent::GetPlaylistItems(playlist_id, offset),
            );
          }
        }
      }
      Some(TrackTableContext::SavedTracks) => {
        self.get_current_user_saved_tracks_next();
      }
      Some(TrackTableContext::MadeForYou) => {
        if let (Some(selected_playlist_id), Some(playlist_tracks)) = (
          self.made_for_you_playlist_id(self.made_for_you_index),
          &self.made_for_you_tracks,
        ) {
          let offset = playlist_tracks.items.len() as u32;
          let playlist_id = selected_playlist_id;
          self.load_more_page(
            playlist_tracks.items.len(),
            playlist_tracks.total as usize,
            IoEvent::GetMadeForYouPlaylistItems(playlist_id, offset),
          );
        }
      }
      _ => {}
    }
  }

  pub fn calculate_help_menu_offset(&mut self) {
    let old_offset = self.help_scroll_offset;

    if self.help_menu_max_lines < self.help_docs_size {
      self.help_scroll_offset = self.help_menu_page * self.help_menu_max_lines;
    }
    if self.help_scroll_offset > self.help_docs_size {
      self.help_scroll_offset = old_offset;
      self.help_menu_page = self.help_menu_page.saturating_sub(1);
    }
  }

  /// Toggles one of the settings shown in the '?' menu by row index:
  /// 0 = black theme, 1 = library block, 2 = playlists block,
  /// 3 = volume ramp bar, 4 = mouse interactions, 5 = theme preset,
  /// 6 = seek by typing, 7 = restore settings, 8 = dev view,
  /// 9-12 = column visibility, 13 = add to playlist, 14 = remove from
  /// playlist, 15 = max name length, 17 = clear cache, 23 = suspend/resume.
  /// Manual suspend/resume of the Spotify client's helper processes. The
  /// state flips optimistically so the button label is instant; the backend
  /// reverts it when no Spotify process can be found (the Windows process
  /// scan alone can take a second).
  pub fn toggle_process_suspend(&mut self) {
    self.processes_suspended = !self.processes_suspended;
    self.dispatch(IoEvent::ToggleProcessSuspend);
  }

  /// Resume Last Listening: start the previous session's last track. Only
  /// meaningful when nothing is playing; the backend applies the saved
  /// volume/repeat/shuffle on top.
  pub fn resume_last_listening(&mut self) {
    if let Some(uri) = self.saved_last_track.clone() {
      self.dispatch(IoEvent::ResumeLastTrack(uri));
    }
  }

  pub fn toggle_setting(&mut self, index: usize) {
    match index {
      0 => {
        self.user_config.theme.background = match self.user_config.theme.background {
          Color::Rgb(0, 0, 0) => Color::Reset,
          _ => Color::Rgb(0, 0, 0),
        };
      }
      1 => {
        self.show_library = !self.show_library;
        self.clamp_library_selection();
      }
      2 => {
        self.show_playlists = !self.show_playlists;
        if let Some(playlists) = &self.playlists {
          let max = playlists.items.len().saturating_sub(1);
          if let Some(index) = self.selected_playlist_index {
            self.selected_playlist_index = Some(index.min(max));
          }
        }
      }
      4 => {
        self.user_config.behavior.enable_mouse = !self.user_config.behavior.enable_mouse;
        self.user_config.terminal.enable_mouse = self.user_config.behavior.enable_mouse;
      }
      5 => {
        let presets = theme_presets();
        self.theme_preset_index = match self.theme_preset_index {
          Some(i) if i + 1 < presets.len() => Some(i + 1),
          Some(_) => None,
          None => Some(0),
        };
        self.user_config.theme = match self.theme_preset_index {
          Some(i) => presets[i].1,
          None => self.config_theme,
        };
        if !self.user_config.terminal.truecolor {
          self.user_config.theme = self.user_config.theme.downgraded();
        }
      }
      6 => {
        self.user_config.behavior.seek_by_typing = !self.user_config.behavior.seek_by_typing;
      }
      7 => {}
      8 => {
        self.dev_view = !self.dev_view;
      }
      9 => {
        self.user_config.behavior.show_album_column = !self.user_config.behavior.show_album_column;
      }
      10 => {
        self.user_config.behavior.show_artist_column =
          !self.user_config.behavior.show_artist_column;
      }
      11 => {
        self.user_config.behavior.show_length_column =
          !self.user_config.behavior.show_length_column;
      }
      12 => {
        self.user_config.behavior.show_date_added_column =
          !self.user_config.behavior.show_date_added_column;
      }
      13 => {
        self.user_config.behavior.enable_add_to_playlist =
          !self.user_config.behavior.enable_add_to_playlist;
      }
      14 => {
        self.user_config.behavior.enable_remove_from_playlist =
          !self.user_config.behavior.enable_remove_from_playlist;
      }
      15 => {
        self.user_config.behavior.max_display_length =
          match self.user_config.behavior.max_display_length {
            0 => 15,
            15 => 20,
            20 => 25,
            25 => 30,
            30 => 40,
            40 => 50,
            _ => 0,
          };
      }
      16 => {
        self.user_config.behavior.enable_animations = !self.user_config.behavior.enable_animations;
      }
      17 => {
        self.dispatch(IoEvent::CleanCache);
      }
      18 => {
        self.user_config.spotify.auto_launch = !self.user_config.spotify.auto_launch;
      }
      19 => {
        self.user_config.spotify.use_chromium_flags = !self.user_config.spotify.use_chromium_flags;
      }
      20 => {
        self.user_config.spotify.suspend_children = !self.user_config.spotify.suspend_children;
      }
      21 => {
        self.user_config.spotify.trim_working_set = !self.user_config.spotify.trim_working_set;
      }
      22 => {
        self.user_config.spotify.memory_limit_mb = if self.user_config.spotify.memory_limit_mb == 0
        {
          150
        } else {
          0
        };
      }
      23 => {
        self.show_process_suspend = !self.show_process_suspend;
      }
      24 => {
        // Playback poll cadence while playing. Floor is 1s: sub-second
        // polling would eat the token bucket and risk Spotify 429s.
        self.user_config.behavior.playback_poll_secs =
          match self.user_config.behavior.playback_poll_secs {
            1 => 2,
            2 => 5,
            5 => 10,
            10 => 15,
            15 => 30,
            _ => 1,
          };
      }
      25 => {
        self.desktop_notifications = !self.desktop_notifications;
      }
      26 => {
        self.user_config.soloist.keep_running = !self.user_config.soloist.keep_running;
      }
      27 => {
        self.user_config.behavior.volume_increment =
          match self.user_config.behavior.volume_increment {
            1 => 2,
            2 => 5,
            5 => 10,
            _ => 1,
          };
      }
      28 => {
        self.user_config.soloist.enabled = !self.user_config.soloist.enabled;
        if !self.user_config.soloist.enabled && !self.user_config.soloist.keep_running {
          crate::platform::soloist::kill_spawned();
        }
      }
      _ => {}
    }
  }

  pub fn clamp_library_selection(&mut self) {
    let visible = visible_library_options(&self.hidden_library_sections);
    self.library.selected_index = self
      .library
      .selected_index
      .min(visible.len().saturating_sub(1));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn mock_track(i: usize) -> FullTrack {
    serde_json::from_value(json!({
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": format!("mocktrack{}", i),
      "is_local": false,
      "name": format!("Mock Song {}", i),
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    }))
    .unwrap()
  }

  fn track_page(count: usize, total: u32) -> Page<FullTrack> {
    Page {
      href: String::new(),
      items: (0..count).map(mock_track).collect(),
      limit: 10,
      next: None,
      offset: 0,
      previous: None,
      total,
    }
  }

  fn paused_playback() -> CurrentPlaybackContext {
    serde_json::from_value(json!({
      "device": {
        "id": "mock-device",
        "is_active": true,
        "is_private_session": false,
        "is_restricted": false,
        "name": "Mock Device",
        "type": "Computer",
        "volume_percent": 50,
      },
      "repeat_state": "off",
      "shuffle_state": false,
      "context": null,
      "timestamp": 0,
      "progress_ms": 0,
      "is_playing": false,
      "item": null,
      "currently_playing_type": "track",
      "actions": { "disallows": {} },
    }))
    .unwrap()
  }

  #[test]
  fn poll_current_playback_while_paused_uses_slower_interval() {
    use std::time::Duration;

    // While paused a poll within the slow interval is skipped.
    let mut app = App {
      current_playback_context: Some(paused_playback()),
      instant_since_last_current_playback_poll: Instant::now() - Duration::from_millis(6_000),
      ..Default::default()
    };
    app.update_on_tick();
    assert!(!app.is_fetching_current_playback);

    // After the slow interval a poll fires, so a playback started externally
    // (e.g. in the Spotify app) is picked up.
    app.instant_since_last_current_playback_poll = Instant::now() - Duration::from_millis(16_000);
    app.update_on_tick();
    assert!(app.is_fetching_current_playback);
  }

  fn playing_playback_with_progress(progress_ms: u64) -> CurrentPlaybackContext {
    // mock_track(0) runs 180_000 ms; progress near that means the track end
    // (and the next song) is imminent.
    let item = serde_json::to_value(mock_track(0)).unwrap();
    serde_json::from_value(json!({
      "device": {
        "id": "mock-device",
        "is_active": true,
        "is_private_session": false,
        "is_restricted": false,
        "name": "Mock Device",
        "type": "Computer",
        "volume_percent": 50,
      },
      "repeat_state": "off",
      "shuffle_state": false,
      "context": null,
      "timestamp": 0,
      "progress_ms": progress_ms,
      "is_playing": true,
      "item": item,
      "currently_playing_type": "track",
      "actions": { "disallows": {} },
    }))
    .unwrap()
  }

  #[test]
  fn poll_shortens_near_track_end() {
    use std::time::Duration;

    // 5s left of 180s, last poll 1.5s ago: base 5s interval would wait,
    // the adaptive 1s interval fires so the next song appears promptly.
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(175_000)),
      instant_since_last_current_playback_poll: Instant::now() - Duration::from_millis(1_500),
      ..Default::default()
    };
    app.update_on_tick();
    assert!(app.is_fetching_current_playback);

    // Same age poll mid-track: still quiet until the 5s base interval.
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(10_000)),
      instant_since_last_current_playback_poll: Instant::now() - Duration::from_millis(1_500),
      ..Default::default()
    };
    app.update_on_tick();
    assert!(!app.is_fetching_current_playback);

    // No item (e.g. paused mock): remaining unknown, no adaptive poll.
    let app = App {
      current_playback_context: Some(paused_playback()),
      ..Default::default()
    };
    assert_eq!(app.playback_remaining_ms(), None);
  }

  #[test]
  fn poll_fires_immediately_at_track_end() {
    use std::time::Duration;

    // Track is 180_000ms; progress=179_950ms + 250ms elapsed => 200ms overrun.
    // Even though elapsed (250ms) is far below 1s/5s, poll must fire right away.
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(179_950)),
      instant_since_last_current_playback_poll: Instant::now() - Duration::from_millis(250),
      ..Default::default()
    };
    app.update_on_tick();
    assert!(app.is_fetching_current_playback);
    assert_eq!(app.track_end_polls, 1);

    // After 4 track-end polls (e.g. empty queue), back off to the normal 5s cadence.
    app.is_fetching_current_playback = false;
    app.track_end_polls = 4;
    app.instant_since_last_current_playback_poll = Instant::now() - Duration::from_millis(500);
    app.update_on_tick();
    assert!(!app.is_fetching_current_playback);
  }

  #[test]
  fn play_track_optimistic_updates_playbar() {
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(90_000)),
      song_progress_ms: 90_000,
      seek_ms: Some(120_000),
      track_end_polls: 2,
      ..Default::default()
    };
    let next = mock_track(7);
    let expected_name = next.name.clone();
    app.play_track_optimistic(next);
    assert_eq!(app.song_progress_ms, 0);
    assert_eq!(app.seek_ms, None);
    assert_eq!(app.track_end_polls, 0);
    assert!(app.ui_dirty);
    let ctx = app.current_playback_context.as_ref().unwrap();
    assert!(ctx.is_playing);
    match ctx.item.as_ref().unwrap() {
      PlayableItem::Track(t) => assert_eq!(t.name, expected_name),
      _ => panic!("expected track"),
    }
  }

  #[test]
  fn refresh_music_view_data_clears_stale_and_fetches() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(0)),
      io_tx: Some(tx),
      lyrics: Some(vec![(0, "old".to_string())]),
      track_credits: Some(vec!["Producer: Old".to_string()]),
      queue_next: Some("Old Next".to_string()),
      ..Default::default()
    };
    app.refresh_music_view_data();
    assert!(app.lyrics.is_none());
    assert!(app.track_credits.is_none());
    assert!(app.queue_next.is_none());
    let events: Vec<IoEvent> = rx.try_iter().collect();
    assert!(events.contains(&IoEvent::GetLyrics));
    assert!(events.contains(&IoEvent::GetQueue));
    assert!(events
      .iter()
      .any(|e| matches!(e, IoEvent::GetTrackCredits(_))));
  }

  #[test]
  fn poll_tick_with_pending_seek_fetches_instead_of_reseeking() {
    use crate::backend::IoEvent;
    use std::time::Duration;

    // A click already dispatched the Seek via seek_to; the poll tick must
    // only fetch (the response clears seek_ms), never re-apply the seek.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(10_000)),
      io_tx: Some(tx),
      seek_ms: Some(90_000),
      instant_since_last_current_playback_poll: Instant::now() - Duration::from_millis(6_000),
      ..Default::default()
    };
    app.update_on_tick();
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::GetCurrentPlayback]);
  }

  #[test]
  fn repeat_flips_state_optimistically() {
    use crate::backend::IoEvent;
    use rspotify::model::RepeatState;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(paused_playback()),
      io_tx: Some(tx),
      ..Default::default()
    };

    app.repeat();

    assert_eq!(
      app.current_playback_context.as_ref().unwrap().repeat_state,
      RepeatState::Context
    );
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::Repeat(RepeatState::Off)]);
  }

  #[test]
  fn shuffle_toggles() {
    use crate::backend::IoEvent;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(paused_playback()),
      io_tx: Some(tx),
      ..Default::default()
    };

    // First press: off -> on, plain Shuffle event.
    app.shuffle();
    assert!(app.current_playback_context.as_ref().unwrap().shuffle_state);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::Shuffle(false)]);

    // Second press: on -> off.
    app.shuffle();
    assert!(!app.current_playback_context.as_ref().unwrap().shuffle_state);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::Shuffle(true)]);
  }

  #[test]
  fn log_request_coalesces_repeats() {
    let mut app = App::default();
    app.log_request("GetCurrentPlayback".to_string());
    app.log_request("GetCurrentPlayback".to_string());
    app.log_request("GetCurrentPlayback".to_string());
    app.log_request("GetPlaylists".to_string());
    assert_eq!(app.request_log.len(), 2);
    assert_eq!(app.request_log[0].text, "GetPlaylists");
    assert_eq!(app.request_log[0].count, 1);
    assert_eq!(app.request_log[1].text, "GetCurrentPlayback");
    assert_eq!(app.request_log[1].count, 3);
    // Interleaved event doesn't break the streak — same name always coalesces.
    app.log_request("GetCurrentPlayback".to_string());
    assert_eq!(app.request_log[0].text, "GetCurrentPlayback");
    assert_eq!(app.request_log[0].count, 4);
  }

  #[test]
  fn materialize_date_added_reverses_full_list_once() {
    let mut app = App::default();
    app.track_table.tracks = (0..5).map(mock_track).collect();
    // Raw order 0..5, displayed reversed (newest first) with matching
    // parallel vecs, as a previous materialize would leave them.
    app.track_table_raw_index = vec![4, 3, 2, 1, 0];
    app.track_table_added_at = (0..5).map(|_| None).collect();
    app.track_table_sort = Some((TrackSortColumn::DateAdded, true));
    app.date_added_pending = true;

    // desc=true: order by raw index ascending then reverse → newest first.
    app.materialize_date_added();
    assert!(!app.date_added_pending);
    assert_eq!(app.track_table_raw_index, vec![4, 3, 2, 1, 0]);

    // Toggle back to raw order.
    app.track_table_sort = Some((TrackSortColumn::DateAdded, false));
    app.materialize_date_added();
    assert_eq!(app.track_table_raw_index, vec![0, 1, 2, 3, 4]);

    // Non-DateAdded sort must not reorder the raw list.
    app.track_table_sort = Some((TrackSortColumn::Title, true));
    app.materialize_date_added();
    assert_eq!(app.track_table_raw_index, vec![0, 1, 2, 3, 4]);

    // No sort set → no-op.
    app.track_table_sort = None;
    app.materialize_date_added();
    assert_eq!(app.track_table_raw_index, vec![0, 1, 2, 3, 4]);
  }

  #[test]
  fn test_search_block_has_more() {
    let mut app = App::default();
    app.search_results.query = "test".to_string();

    // Full page (items == limit) -> more pages available, even if the
    // under-reported total says otherwise.
    app.search_results.tracks = Some(track_page(10, 20));
    assert!(app.search_block_has_more(&SearchResultBlock::SongSearch));

    // Short page (items < limit) -> no more pages
    app.search_results.tracks = Some(track_page(9, 20));
    assert!(!app.search_block_has_more(&SearchResultBlock::SongSearch));

    // not yet loaded -> no more
    app.search_results.tracks = None;
    assert!(!app.search_block_has_more(&SearchResultBlock::SongSearch));

    // untouched sibling blocks stay false
    assert!(!app.search_block_has_more(&SearchResultBlock::AlbumSearch));

    // Empty block never pages
    assert!(!app.search_block_has_more(&SearchResultBlock::Empty));
  }

  #[test]
  fn load_more_artist_top_tracks_dispatches_past_the_search_total() {
    let mut app = App::default();
    let empty = Page {
      href: String::new(),
      items: vec![],
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total: 0,
    };
    app.artist = Some(Artist {
      artist_id: "mockartist1".to_string(),
      artist_name: "Mock Artist".to_string(),
      albums: empty.clone(),
      singles: empty.clone(),
      eps: empty.clone(),
      appears_on: empty.clone(),
      related_artists: vec![],
      top_tracks: (0..10).map(mock_track).collect(),
      top_tracks_total: 26,
      top_tracks_has_more: true,
      selected_album_index: 0,
      selected_singles_index: 0,
      selected_eps_index: 0,
      selected_appears_on_index: 0,
      selected_related_artist_index: 0,
      selected_top_track_index: 0,
      artist_hovered_block: ArtistBlock::TopTracks,
      artist_selected_block: ArtistBlock::Empty,
      image_url: None,
      is_following: None,
      followers: None,
      genres: vec![],
      meta: None,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    app.load_more_artist_top_tracks();
    let events: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(events.len(), 1);
    assert_eq!(
      events[0],
      IoEvent::GetArtistTopTracksMore("mockartist1".to_string(), "Mock Artist".to_string(), 10)
    );

    // The search `total` under-reports for some artists: the dispatch must
    // still fire when total <= loaded, letting the backend page reveal the
    // true end (a short page clears top_tracks_has_more).
    app.artist.as_mut().unwrap().top_tracks_total = 10;
    app.load_more_artist_top_tracks();
    // A load-more right after another is throttled (min 2s between pages).
    assert_eq!(rx.try_iter().count(), 0);
    // After the throttle window passes, the next page still fires past the
    // under-reported total.
    app.last_load_more = None;
    app.load_more_artist_top_tracks();
    let events: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(events.len(), 1);
    assert_eq!(
      events[0],
      IoEvent::GetArtistTopTracksMore("mockartist1".to_string(), "Mock Artist".to_string(), 10)
    );
  }

  #[test]
  fn artist_select_tab_fetches_albums_and_related_only_on_first_use() {
    let mut app = App::default();
    let empty2 = Page {
      href: String::new(),
      items: vec![],
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total: 0,
    };
    app.artist = Some(Artist {
      artist_id: "mockartist1".to_string(),
      artist_name: "Mock Artist".to_string(),
      albums: empty2.clone(),
      singles: empty2.clone(),
      eps: empty2.clone(),
      appears_on: empty2.clone(),
      related_artists: vec![],
      top_tracks: (0..10).map(mock_track).collect(),
      top_tracks_total: 26,
      top_tracks_has_more: true,
      selected_album_index: 0,
      selected_singles_index: 0,
      selected_eps_index: 0,
      selected_appears_on_index: 0,
      selected_related_artist_index: 0,
      selected_top_track_index: 0,
      artist_hovered_block: ArtistBlock::TopTracks,
      artist_selected_block: ArtistBlock::TopTracks,
      image_url: None,
      is_following: None,
      followers: None,
      genres: vec![],
      meta: None,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    app.artist_select_tab(ArtistBlock::Albums);
    let events: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], IoEvent::GetArtistAlbumsMore(..)));

    // Data already fetched → no refetch on a second visit.
    app.artist.as_mut().unwrap().albums.items.push(
      serde_json::from_value(json!({
        "album_type": "album",
        "artists": [{ "external_urls": {}, "href": null, "id": "mockartist1", "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": "mockalbum0",
        "images": [],
        "name": "Mock Album",
        "release_date": "2020-01-01",
        "release_date_precision": "day",
        "total_tracks": 1,
        "type": "album",
      }))
      .unwrap(),
    );
    app.artist_select_tab(ArtistBlock::Albums);
    assert_eq!(rx.try_iter().count(), 0);
  }

  #[test]
  fn playlist_contains_excludes_the_viewed_playlist() {
    let mut app = App::default();
    app.playlist_uri_map.insert(
      "p1".to_string(),
      HashSet::from(["spotify:track:a".to_string(), "spotify:track:b".to_string()]),
    );
    app.playlist_uri_map.insert(
      "p2".to_string(),
      HashSet::from(["spotify:track:b".to_string()]),
    );
    assert!(app.playlist_contains("spotify:track:a", None));
    assert!(app.playlist_contains("spotify:track:a", Some("p2")));
    assert!(!app.playlist_contains("spotify:track:a", Some("p1")));
    assert!(!app.playlist_contains("spotify:track:z", None));
  }

  #[test]
  fn open_add_to_playlist_captures_the_selected_track() {
    let mut app = App::default();
    app.set_current_route_state(Some(ActiveBlock::TrackTable), None);
    app.track_table.tracks = vec![mock_track(3), mock_track(4)];
    app.track_table.selected_index = 1;
    app.open_add_to_playlist();
    assert_eq!(
      app.pending_track_uri.as_deref(),
      Some("spotify:track:mocktrack4")
    );
    assert_eq!(
      app.get_current_route().active_block,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist)
    );

    // No track selected (empty table) → nothing opens.
    let mut app = App::default();
    app.set_current_route_state(Some(ActiveBlock::TrackTable), None);
    app.open_add_to_playlist();
    assert!(app.pending_track_uri.is_none());
  }

  #[test]
  fn gear_toggle_13_flips_add_to_playlist() {
    let mut app = App::default();
    assert!(app.user_config.behavior.enable_add_to_playlist);
    app.toggle_setting(13);
    assert!(!app.user_config.behavior.enable_add_to_playlist);
    app.toggle_setting(13);
    assert!(app.user_config.behavior.enable_add_to_playlist);
  }

  #[test]
  fn gear_toggle_16_flips_animations() {
    let mut app = App::default();
    assert!(app.user_config.behavior.enable_animations);
    app.toggle_setting(16);
    assert!(!app.user_config.behavior.enable_animations);
    app.toggle_setting(16);
    assert!(app.user_config.behavior.enable_animations);
  }

  #[test]
  fn gear_toggle_24_cycles_playback_poll_secs() {
    let mut app = App::default();
    assert_eq!(app.user_config.behavior.playback_poll_secs, 5);
    for expected in [10, 15, 30, 1, 2, 5] {
      app.toggle_setting(24);
      assert_eq!(app.user_config.behavior.playback_poll_secs, expected);
    }
  }

  #[test]
  fn gear_toggle_27_cycles_volume_increment() {
    let mut app = App::default();
    app.user_config.behavior.volume_increment = 5;
    for expected in [10, 1, 2, 5] {
      app.toggle_setting(27);
      assert_eq!(app.user_config.behavior.volume_increment, expected);
    }
  }

  #[test]
  fn gear_toggle_28_flips_soloist_enabled() {
    let mut app = App::default();
    assert!(!app.user_config.soloist.enabled);
    app.toggle_setting(28);
    assert!(app.user_config.soloist.enabled);
    app.toggle_setting(28);
    assert!(!app.user_config.soloist.enabled);
  }

  #[test]
  fn sort_keeps_the_selection_on_the_same_track() {
    let mut app = App::default();
    app.track_table.tracks = vec![mock_track(0), mock_track(1), mock_track(2)];
    app.track_table_added_at = vec![None; 3];
    app.track_table.selected_index = 0;
    app.track_table_sort = Some((TrackSortColumn::Title, true));
    app.sort_tracks();
    // Descending: Mock Song 2, 1, 0 — the selected song 0 moved to the end.
    assert_eq!(app.track_table.selected_index, 2);
    assert_eq!(
      app.track_table.tracks[app.track_table.selected_index].name,
      "Mock Song 0"
    );
  }

  #[test]
  fn add_pasted_playlist_to_for_you_deduplicates() {
    let mut app = App::default();
    app.add_pasted_playlist_to_for_you("My Mix".to_string(), "mix1".to_string());
    assert_eq!(
      app.made_for_you_custom,
      vec![("My Mix".to_string(), "mix1".to_string())]
    );
    // Duplicates are ignored.
    app.add_pasted_playlist_to_for_you("My Mix".to_string(), "mix1".to_string());
    assert_eq!(app.made_for_you_custom.len(), 1);
    // A missing name falls back to the playlist id.
    app.add_pasted_playlist_to_for_you(String::new(), "mix2".to_string());
    assert_eq!(app.made_for_you_custom.len(), 2);
    assert_eq!(app.made_for_you_name(1), Some("mix2".to_string()));
    // The pasted playlists extend the For you list.
    assert_eq!(app.made_for_you_len(), 2);
    assert_eq!(app.made_for_you_playlist_id(0), Some("mix1".to_string()));
    assert_eq!(app.made_for_you_name(0), Some("My Mix".to_string()));
  }

  #[test]
  fn extract_playlist_id_and_paste_adds_to_for_you() {
    assert_eq!(
      App::extract_playlist_id(
        "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc123"
      ),
      Some("37i9dQZF1DXcBWIGoYBM5M".to_string())
    );
    assert_eq!(
      App::extract_playlist_id("spotify:playlist:1234567890"),
      Some("1234567890".to_string())
    );
    assert_eq!(App::extract_playlist_id("not a playlist"), None);

    let mut app = App::default();
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    assert!(app.paste_playlist_to_for_you("https://open.spotify.com/playlist/abc123?si=1"));
    assert_eq!(
      rx.try_iter().collect::<Vec<_>>(),
      vec![IoEvent::GetMadeForYouPlaylistItems("abc123".into(), 0)]
    );

    // Multiple links (newline/space/comma-separated, with duplicates) pin all and resolve names.
    assert!(app.paste_playlist_to_for_you(
      "https://open.spotify.com/playlist/p1?si=1\nspotify:playlist:p2, https://open.spotify.com/playlist/p1"
    ));
    assert_eq!(app.made_for_you_len(), 3);
    assert_eq!(
      rx.try_iter().collect::<Vec<_>>(),
      vec![
        IoEvent::ResolveForYouPlaylistName("p1".into()),
        IoEvent::ResolveForYouPlaylistName("p2".into()),
      ]
    );
  }

  #[test]
  fn expand_made_for_you_opens_the_pasted_playlist() {
    let mut app = App::default();
    app
      .made_for_you_custom
      .push(("My Mix".to_string(), "mix1".to_string()));
    app.expand_made_for_you(0);
    assert_eq!(app.track_table.context, Some(TrackTableContext::MadeForYou));
    assert_eq!(app.made_for_you_playlist_id(0), Some("mix1".to_string()));
  }

  fn mock_playlist(i: usize) -> SimplifiedPlaylist {
    serde_json::from_value(json!({
      "collaborative": false,
      "external_urls": {},
      "href": format!("spotify:playlist:mockplaylist{}", i),
      "id": format!("mockplaylist{}", i),
      "images": [],
      "name": format!("Mock Playlist {}", i),
      "owner": {
        "display_name": "Mock User",
        "external_urls": {},
        "followers": null,
        "href": "spotify:user:mockuser",
        "id": "mockuser",
        "images": [],
      },
      "public": null,
      "snapshot_id": format!("snap{}", i),
      "items": { "href": "", "total": 150 },
    }))
    .unwrap()
  }

  #[test]
  fn remove_track_from_table_cuts_row_and_parallel_vecs() {
    let mut app = App::default();
    app.track_table.tracks = (0..5).map(mock_track).collect();
    app.track_table_added_at = (0..5).map(|_| None).collect();
    app.track_table_raw_index = vec![4, 3, 2, 1, 0];
    app.track_table.selected_index = 4;
    // Displayed 0 is raw 4 (the last raw position): no shifting needed.
    assert!(app.remove_track_from_table("spotify:track:mocktrack0"));
    assert_eq!(app.track_table.tracks.len(), 4);
    assert_eq!(app.track_table_added_at.len(), 4);
    assert_eq!(app.track_table_raw_index, vec![3, 2, 1, 0]);
    assert_eq!(app.track_table.selected_index, 3);
    // Unknown uri changes nothing.
    assert!(!app.remove_track_from_table("spotify:track:nope"));
    assert_eq!(app.track_table.tracks.len(), 4);
  }

  #[test]
  fn remove_track_from_table_shifts_raw_indices_above_cut() {
    let mut app = App::default();
    app.track_table.tracks = (0..5).map(mock_track).collect();
    app.track_table_added_at = (0..5).map(|_| None).collect();
    app.track_table_raw_index = vec![0, 1, 2, 3, 4];
    // Displayed 1 is raw 1: raw 2..4 shift down one.
    assert!(app.remove_track_from_table("spotify:track:mocktrack1"));
    assert_eq!(app.track_table_raw_index, vec![0, 1, 2, 3]);
    assert_eq!(
      app.track_table.tracks[1].id.as_ref().unwrap().id(),
      "mocktrack2"
    );
  }

  #[test]
  fn remove_track_from_table_ignores_misaligned_parallel_vecs() {
    let mut app = App::default();
    app.track_table.tracks = (0..3).map(mock_track).collect();
    // Stale parallel vecs (e.g. mid-sort): only the visible row goes.
    assert!(app.remove_track_from_table("spotify:track:mocktrack0"));
    assert_eq!(app.track_table.tracks.len(), 2);
    assert!(app.track_table_added_at.is_empty());
    assert!(app.track_table_raw_index.is_empty());
  }

  #[test]
  fn viewed_playlist_id_prefers_active_falls_back_to_selected() {
    let mut app = App {
      playlists: Some(rspotify::model::Page {
        href: String::new(),
        items: vec![mock_playlist(0), mock_playlist(1)],
        limit: 2,
        next: None,
        offset: 0,
        previous: None,
        total: 2,
      }),
      selected_playlist_index: Some(1),
      ..Default::default()
    };
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    // Nothing open yet: selected alone identifies the panel.
    assert_eq!(app.viewed_playlist_id(), Some("mockplaylist1".to_string()));
    // Active wins once the playlist is opened.
    app.active_playlist_index = Some(0);
    assert_eq!(app.viewed_playlist_id(), Some("mockplaylist0".to_string()));
    // Other contexts never match.
    app.track_table.context = Some(TrackTableContext::SavedTracks);
    assert_eq!(app.viewed_playlist_id(), None);
  }
  #[test]
  fn now_playing_title_formats_track_and_empty() {
    assert_eq!(
      now_playing_title(Some(&playing_playback_with_progress(0))),
      "Mock Song 0 – Mock Artist"
    );
    assert_eq!(now_playing_title(None), "");
  }
  #[test]
  fn toggle_playback_flips_state_optimistically() {
    let (io_tx, io_rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(0)),
      io_tx: Some(io_tx),
      ..Default::default()
    };

    app.toggle_playback();
    assert!(matches!(
      app.current_playback_context,
      Some(CurrentPlaybackContext {
        is_playing: false,
        ..
      })
    ));
    assert_eq!(app.play_toggle_gen, 1);
    assert_eq!(app.play_toggle_pending, 1);
    assert_eq!(
      io_rx.try_recv().ok(),
      Some(IoEvent::SetPlaybackState(false, 1))
    );

    app.toggle_playback();
    assert!(matches!(
      app.current_playback_context,
      Some(CurrentPlaybackContext {
        is_playing: true,
        ..
      })
    ));
    assert_eq!(app.play_toggle_gen, 2);
    assert_eq!(app.play_toggle_pending, 2);
    assert_eq!(
      io_rx.try_recv().ok(),
      Some(IoEvent::SetPlaybackState(true, 2))
    );
  }

  #[test]
  fn rapid_repeat_cycles_and_tracks_pending() {
    use rspotify::model::RepeatState;
    let (io_tx, io_rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(0)),
      io_tx: Some(io_tx),
      ..Default::default()
    };
    app.repeat();
    app.repeat();
    app.repeat();
    assert_eq!(app.repeat_pending, 3);
    assert_eq!(
      app.current_playback_context.as_ref().unwrap().repeat_state,
      RepeatState::Off
    );
    let events: Vec<IoEvent> = io_rx.try_iter().collect();
    assert_eq!(
      events,
      vec![
        IoEvent::Repeat(RepeatState::Off),
        IoEvent::Repeat(RepeatState::Context),
        IoEvent::Repeat(RepeatState::Track),
      ]
    );
  }

  #[test]
  fn volume_keys_preview_optimistically_and_chain() {
    let (io_tx, io_rx) = std::sync::mpsc::channel();
    let mut app = App {
      current_playback_context: Some(playing_playback_with_progress(0)),
      io_tx: Some(io_tx),
      ..Default::default()
    };
    let step = app.user_config.behavior.volume_increment;

    app.increase_volume();
    assert_eq!(app.volume_preview, Some(50 + step));
    assert!(matches!(
      io_rx.try_recv(),
      Ok(IoEvent::ChangeVolume(v)) if v == 50 + step
    ));

    // A second press steps from the pending preview, not the stale snapshot.
    app.increase_volume();
    assert_eq!(app.volume_preview, Some(50 + step + step));
    assert!(matches!(
      io_rx.try_recv(),
      Ok(IoEvent::ChangeVolume(v)) if v == 50 + step + step
    ));
  }
}
