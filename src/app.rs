use super::user_config::{theme_presets, Theme, UserConfig};
use crate::backend::IoEvent;
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::Color;
use rspotify::{
  model::Country,
  model::{
    album::{FullAlbum, SavedAlbum, SimplifiedAlbum},
    artist::FullArtist,
    audio::{AudioAnalysis, AudioFeatures},
    context::CurrentPlaybackContext,
    device::Device,
    page::{CursorBasedPage, Page},
    playing::PlayHistory,
    playlist::{PlaylistItem, SimplifiedPlaylist},
    show::{FullShow, Show, SimplifiedEpisode, SimplifiedShow},
    track::{FullTrack, SavedTrack, SimplifiedTrack},
    user::PrivateUser,
    Id, PlayableItem,
  },
};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;
use std::{
  cmp::{max, min, Ordering},
  collections::HashSet,
  time::{Instant, SystemTime},
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

pub const MADE_FOR_YOU_NAMES: [&str; 5] = [
  "Discover Weekly",
  "Release Radar",
  "On Repeat",
  "Repeat Rewind",
  "Daily Drive",
];

pub const LIBRARY_OPTIONS: [&str; 6] = [
  "For you",
  "Recently Played",
  "Liked Songs",
  "Albums",
  "Artists",
  "Podcasts",
];

/// Library sections not hidden via the '?' toggles (1-6).
pub fn visible_library_options(hidden: &[String]) -> Vec<&'static str> {
  LIBRARY_OPTIONS
    .iter()
    .copied()
    .filter(|option| !hidden.iter().any(|h| h == option))
    .collect()
}

const DEFAULT_ROUTE: Route = Route {
  id: RouteId::MadeForYou,
  active_block: ActiveBlock::Empty,
  hovered_block: ActiveBlock::Library,
};

#[derive(Clone, Debug)]
pub struct RequestLogEntry {
  pub text: String,
  pub count: u32,
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

/// (items_len, total) of a page, for has-more checks.
fn page_len_total<T: serde::de::DeserializeOwned>(page: Option<&Page<T>>) -> Option<(usize, u32)> {
  page.map(|p| (p.items.len(), p.total))
}

#[derive(PartialEq, Debug, Clone)]
pub enum ArtistBlock {
  TopTracks,
  Albums,
  RelatedArtists,
  Empty,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DialogContext {
  PlaylistWindow,
  PlaylistSearch,
  SeekTime,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ActiveBlock {
  Analysis,
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
  Analysis,
  AlbumTracks,
  AlbumList,
  Artist,
  MusicView,
  Error,
  RecentlyPlayed,
  Search,
  SelectedDevice,
  TrackTable,
  MadeForYou,
  Artists,
  Podcasts,
  PodcastEpisodes,
  Recommendations,
  Dialog,
}

#[derive(Debug)]
pub struct Route {
  pub id: RouteId,
  pub active_block: ActiveBlock,
  pub hovered_block: ActiveBlock,
}

// Is it possible to compose enums?
#[derive(PartialEq, Debug)]
pub enum TrackTableContext {
  MyPlaylists,
  AlbumSearch,
  PlaylistSearch,
  SavedTracks,
  RecommendedTracks,
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

#[derive(Clone, PartialEq, Debug)]
pub enum RecommendationsContext {
  Artist,
  Song,
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
  pub related_artists: Vec<FullArtist>,
  pub top_tracks: Vec<FullTrack>,
  pub selected_album_index: usize,
  pub selected_related_artist_index: usize,
  pub selected_top_track_index: usize,
  pub artist_hovered_block: ArtistBlock,
  pub artist_selected_block: ArtistBlock,
}

pub struct App {
  pub instant_since_last_current_playback_poll: Instant,
  navigation_stack: Vec<Route>,
  pub audio_analysis: Option<AudioAnalysis>,
  // Track uri + normalized (0..1) loudness envelope over 512 song-time
  // slots, derived from the audio analysis; drives the visualizer.
  pub audio_envelope: Option<(String, Vec<f32>)>,
  // Track uri + audio features (energy/tempo) driving the visualizer in
  // real mode (the analysis endpoint is deprecated).
  pub audio_features: Option<(String, AudioFeatures)>,
  pub lyrics: Option<Vec<(u128, String)>>,
  pub monthly_listeners: Option<u64>,
  pub track_credits: Option<Vec<String>>,
  pub queue_next: Option<String>,
  pub top_track_playcounts: HashMap<String, u64>,
  pub user_config: UserConfig,
  pub artists: Vec<FullArtist>,
  pub artist: Option<Artist>,
  pub album_table_context: AlbumTableContext,
  pub saved_album_tracks_index: usize,
  pub api_error: String,
  pub current_playback_context: Option<CurrentPlaybackContext>,
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
  pub followed_artist_ids_set: HashSet<String>,
  pub saved_album_ids_set: HashSet<String>,
  pub saved_show_ids_set: HashSet<String>,
  pub large_search_limit: u32,
  pub library: Library,
  pub playlist_offset: u32,
  pub made_for_you_offset: u32,
  pub playlist_tracks: Option<Page<PlaylistItem>>,
  pub made_for_you_tracks: Option<Page<PlaylistItem>>,
  pub made_for_you_ids: Vec<Option<String>>,
  pub playlists: Option<Page<SimplifiedPlaylist>>,
  pub recently_played: SpotifyResultAndSelectedIndex<Option<CursorBasedPage<PlayHistory>>>,
  pub recommended_tracks: Vec<FullTrack>,
  pub recommendations_seed: String,
  pub recommendations_context: Option<RecommendationsContext>,
  pub search_results: SearchResult,
  pub selected_album_simplified: Option<SelectedAlbum>,
  pub selected_album_full: Option<SelectedFullAlbum>,
  pub selected_device_index: Option<usize>,
  pub selected_playlist_index: Option<usize>,
  pub active_playlist_index: Option<usize>,
  pub size: Rect,
  pub song_progress_ms: u128,
  pub seek_ms: Option<u128>,
  pub volume_preview: Option<u8>,
  pub track_table: TrackTable,
  pub track_table_sort: Option<(TrackSortColumn, bool)>,
  pub track_table_added_at: Vec<Option<DateTime<Utc>>>,
  // Date Added was requested while the playlist was only partially loaded;
  // the sort materializes once the remaining pages arrive.
  pub date_added_pending: bool,
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
  pub is_loading: bool,
  pub io_tx: Option<Sender<IoEvent>>,
  pub is_fetching_current_playback: bool,
  pub is_fetching_next_page: bool,
  pub spotify_token_expiry: SystemTime,
  pub dialog: Option<String>,
  pub confirm: bool,
  pub show_library: bool,
  pub show_playlists: bool,
  pub hidden_library_sections: Vec<String>,
  pub mock: bool,
  pub config_theme: Theme,
  pub theme_preset_index: Option<usize>,
  pub dev_view: bool,
  pub request_log: VecDeque<RequestLogEntry>,
  pub request_log_index: Option<usize>,
}

impl Default for App {
  fn default() -> Self {
    App {
      audio_analysis: None,
      audio_envelope: None,
      audio_features: None,
      lyrics: None,
      monthly_listeners: None,
      track_credits: None,
      queue_next: None,
      top_track_playcounts: HashMap::new(),
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
      followed_artist_ids_set: HashSet::new(),
      saved_album_ids_set: HashSet::new(),
      saved_show_ids_set: HashSet::new(),
      navigation_stack: vec![DEFAULT_ROUTE],
      large_search_limit: 20,
      api_error: String::new(),
      current_playback_context: None,
      devices: None,
      input: vec![],
      input_idx: 0,
      input_cursor_position: 0,
      playlist_offset: 0,
      made_for_you_offset: 0,
      playlist_tracks: None,
      made_for_you_tracks: None,
      made_for_you_ids: vec![None; 5],
      playlists: None,
      recommended_tracks: vec![],
      recommendations_context: None,
      recommendations_seed: "".to_string(),
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
      volume_preview: None,
      selected_device_index: None,
      selected_playlist_index: None,
      active_playlist_index: None,
      track_table: Default::default(),
      track_table_sort: None,
      track_table_added_at: vec![],
      date_added_pending: false,
      track_table_raw_index: vec![],
      episode_table_context: EpisodeTableContext::Full,
      selected_show_simplified: None,
      selected_show_full: None,
      user: None,
      instant_since_last_current_playback_poll: Instant::now(),
      clipboard: Clipboard::new().ok(),
      help_docs_size: 0,
      help_menu_page: 0,
      help_menu_max_lines: 0,
      help_scroll_offset: 0,
      is_loading: false,
      io_tx: None,
      is_fetching_current_playback: false,
      is_fetching_next_page: false,
      spotify_token_expiry: SystemTime::now(),
      dialog: None,
      confirm: false,
      mock: false,
      show_library: true,
      show_playlists: true,
      hidden_library_sections: vec![],
      config_theme: Theme::default(),
      theme_preset_index: None,
      dev_view: false,
      request_log: VecDeque::new(),
      request_log_index: None,
    }
  }
}

impl App {
  /// Log a request; consecutive repeats of the same event coalesce into one
  /// entry with a counter (the playback poll would otherwise flood the log).
  pub fn log_request(&mut self, text: String) {
    match self.request_log.front_mut() {
      Some(head) if head.text == text => head.count = head.count.saturating_add(1),
      _ => {
        self.request_log.push_front(RequestLogEntry { text, count: 1 });
        self.request_log.truncate(100);
      }
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

  // Stable sort of track_table (and its parallel added_at vec). None dates always
  // sort last regardless of direction.
  pub fn sort_tracks(&mut self) {
    let Some((column, desc)) = self.track_table_sort else {
      return;
    };
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
  }

  // Send a network event to the network thread
  pub fn dispatch(&mut self, action: IoEvent) {
    // `is_loading` will be set to false again after the async action has finished in network.rs
    // The 5s playback poll (and its saved-tracks check) is a silent background
    // refresh — it must not flash the loading indicator.
    self.is_loading = !matches!(
      action,
      IoEvent::GetCurrentPlayback | IoEvent::CurrentUserSavedTracksContains(_)
    );
    if let Some(io_tx) = &self.io_tx {
      if let Err(e) = io_tx.send(action) {
        self.is_loading = false;
        self.api_error = format!("dispatch failed: {}", e);
      };
    }
  }

  fn apply_seek(&mut self, seek_ms: u32) {
    if let Some(CurrentPlaybackContext {
      item: Some(item), ..
    }) = &self.current_playback_context
    {
      let duration_ms: u128 = match item {
        PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
        PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
        _ => 0,
      };

      // Equality is a valid end-of-track seek (last bar cell); only a seek
      // past the end advances to the next track.
      let event = if (seek_ms as u128) <= duration_ms {
        IoEvent::Seek(seek_ms)
      } else {
        IoEvent::NextTrack
      };

      self.dispatch(event);
    }
  }

  /// Preview the seek position immediately and dispatch the seek right away;
  /// `seek_ms` is cleared when the next playback fetch confirms the position.
  pub fn seek_to(&mut self, seek_ms: u32) {
    self.seek_ms = Some(seek_ms as u128);
    self.apply_seek(seek_ms);
  }

  /// Preview a scrub position without dispatching (used while dragging);
  /// the draw code renders `seek_ms` live.
  pub fn preview_seek(&mut self, seek_ms: u32) {
    self.seek_ms = Some(seek_ms as u128);
  }

  fn poll_current_playback(&mut self) {
    // ponytail: no polling while paused (no progress to track); resumes
    // automatically once playback flips is_playing back on. A pending scrub
    // (seek_ms) still forces one poll so the seek commits.
    if self.seek_ms.is_none()
      && matches!(
        self.current_playback_context,
        Some(CurrentPlaybackContext {
          is_playing: false,
          ..
        })
      )
    {
      return;
    }

    // Poll every 5 seconds
    let poll_interval_ms = 5_000;

    let elapsed = self
      .instant_since_last_current_playback_poll
      .elapsed()
      .as_millis();

    if !self.is_fetching_current_playback && elapsed >= poll_interval_ms {
      self.is_fetching_current_playback = true;
      // Trigger the seek if the user has set a new position
      match self.seek_ms {
        Some(seek_ms) => self.apply_seek(seek_ms as u32),
        None => self.dispatch(IoEvent::GetCurrentPlayback),
      }
    }
  }

  pub fn update_on_tick(&mut self) {
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
      } + u128::from(progress_ms);

      let duration_ms: u128 = match item {
        PlayableItem::Track(track) => track.duration.num_milliseconds() as u128,
        PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u128,
        _ => 0,
      };

      if elapsed < duration_ms {
        self.song_progress_ms = elapsed;
      } else {
        self.song_progress_ms = duration_ms.into();
      }
    }
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
        old_progress as u128 + self.user_config.behavior.seek_milliseconds as u128,
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
    let new_progress = if old_progress as u32 > self.user_config.behavior.seek_milliseconds {
      old_progress as u32 - self.user_config.behavior.seek_milliseconds
    } else {
      0u32
    };
    self.seek_to(new_progress as u32);
  }

  pub fn get_recommendations_for_seed(
    &mut self,
    seed_artists: Option<Vec<String>>,
    seed_tracks: Option<Vec<String>>,
    first_track: Option<FullTrack>,
  ) {
    let user_country = self.get_user_country();
    self.dispatch(IoEvent::GetRecommendationsForSeed(
      seed_artists,
      seed_tracks,
      Box::new(first_track),
      user_country,
    ));
  }

  pub fn get_recommendations_for_track_id(&mut self, id: String) {
    let user_country = self.get_user_country();
    self.dispatch(IoEvent::GetRecommendationsForTrackId(id, user_country));
  }

  pub fn increase_volume(&mut self) {
    if let Some(context) = self.current_playback_context.clone() {
      let current_volume = context.device.volume_percent.unwrap_or(0) as u8;
      let next_volume = min(
        current_volume + self.user_config.behavior.volume_increment,
        100,
      );

      if next_volume != current_volume {
        self.dispatch(IoEvent::ChangeVolume(next_volume));
      }
    }
  }

  pub fn decrease_volume(&mut self) {
    if let Some(context) = self.current_playback_context.clone() {
      let current_volume = context.device.volume_percent.unwrap_or(0) as i8;
      let next_volume = max(
        current_volume - self.user_config.behavior.volume_increment as i8,
        0,
      );

      if next_volume != current_volume {
        self.dispatch(IoEvent::ChangeVolume(next_volume as u8));
      }
    }
  }

  pub fn handle_error(&mut self, e: anyhow::Error) {
    self.push_navigation_stack(RouteId::Error, ActiveBlock::Error);
    self.api_error = e.to_string();
  }

  pub fn toggle_playback(&mut self) {
    if let Some(CurrentPlaybackContext {
      is_playing: true, ..
    }) = &self.current_playback_context
    {
      self.dispatch(IoEvent::PausePlayback);
    } else {
      // When no offset or uris are passed, spotify will resume current playback
      self.dispatch(IoEvent::StartPlayback(None, None, None));
    }
  }

  pub fn previous_track(&mut self) {
    if self.song_progress_ms >= 3_000 {
      self.dispatch(IoEvent::Seek(0));
    } else {
      self.dispatch(IoEvent::PreviousTrack);
    }
  }

  // The navigation_stack actually only controls the large block to the right of `library` and
  // `playlists`
  pub fn push_navigation_stack(&mut self, next_route_id: RouteId, next_active_block: ActiveBlock) {
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
    let (playlists, index) = match self.track_table.context.as_ref()? {
      TrackTableContext::MyPlaylists => (&self.playlists, self.active_playlist_index),
      TrackTableContext::PlaylistSearch => {
        (&self.search_results.playlists, self.search_results.selected_playlists_index)
      }
      _ => return None,
    };
    let playlists = playlists.as_ref()?;
    let index = index.unwrap_or(0).min(playlists.items.len().saturating_sub(1));
    playlists.items.get(index).map(|item| item.id.uri())
  }

  fn get_current_route_mut(&mut self) -> &mut Route {
    self.navigation_stack.last_mut().unwrap()
  }

  pub fn set_current_route_state(
    &mut self,
    active_block: Option<ActiveBlock>,
    hovered_block: Option<ActiveBlock>,
  ) {
    let current_route = self.get_current_route_mut();
    if let Some(active_block) = active_block {
      current_route.active_block = active_block;
    }
    if let Some(hovered_block) = hovered_block {
      current_route.hovered_block = hovered_block;
    }
  }

  pub fn copy_song_url(&mut self) {
    let clipboard = match &mut self.clipboard {
      Some(ctx) => ctx,
      None => return,
    };

    if let Some(CurrentPlaybackContext {
      item: Some(item), ..
    }) = &self.current_playback_context
    {
      match item {
        PlayableItem::Track(track) => {
          if let Err(e) = clipboard.set_text(format!(
            "https://open.spotify.com/track/{}",
            track
              .id
              .clone()
              .map(|id| id.to_string())
              .unwrap_or_default()
          )) {
            self.handle_error(anyhow!("failed to set clipboard content: {}", e));
          }
        }
        PlayableItem::Episode(episode) => {
          if let Err(e) = clipboard.set_text(format!(
            "https://open.spotify.com/episode/{}",
            episode.id.to_owned()
          )) {
            self.handle_error(anyhow!("failed to set clipboard content: {}", e));
          }
        }
        _ => {}
      }
    }
  }

  pub fn copy_album_url(&mut self) {
    let clipboard = match &mut self.clipboard {
      Some(ctx) => ctx,
      None => return,
    };

    if let Some(CurrentPlaybackContext {
      item: Some(item), ..
    }) = &self.current_playback_context
    {
      match item {
        PlayableItem::Track(track) => {
          if let Err(e) = clipboard.set_text(format!(
            "https://open.spotify.com/album/{}",
            track
              .album
              .id
              .clone()
              .map(|id| id.to_string())
              .unwrap_or_default()
          )) {
            self.handle_error(anyhow!("failed to set clipboard content: {}", e));
          }
        }
        PlayableItem::Episode(episode) => {
          if let Err(e) = clipboard.set_text(format!(
            "https://open.spotify.com/show/{}",
            episode.show.id.to_owned()
          )) {
            self.handle_error(anyhow!("failed to set clipboard content: {}", e));
          }
        }
        _ => {}
      }
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
        if let Some(saved_artists) = &self.library.saved_artists.clone().get_results(None) {
          if let Some(last_artist) = saved_artists.items.last() {
            self.dispatch(IoEvent::GetFollowedArtists(Some(
              last_artist.id.to_string(),
            )));
          }
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
    if let Some(context) = &self.current_playback_context.clone() {
      self.dispatch(IoEvent::Shuffle(context.shuffle_state));
    };
  }

  pub fn get_current_user_saved_albums_next(&mut self) {
    match self
      .library
      .saved_albums
      .get_results(Some(self.library.saved_albums.index + 1))
      .cloned()
    {
      Some(_) => self.library.saved_albums.index += 1,
      None => {
        if let Some(saved_albums) = &self.library.saved_albums.get_results(None) {
          let offset = Some(saved_albums.offset + saved_albums.limit);
          self.dispatch(IoEvent::GetCurrentUserSavedAlbums(offset));
        }
      }
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
    match self
      .library
      .saved_shows
      .get_results(Some(self.library.saved_shows.index + 1))
      .cloned()
    {
      Some(_) => self.library.saved_shows.index += 1,
      None => {
        if let Some(saved_shows) = &self.library.saved_shows.get_results(None) {
          let offset = Some(saved_shows.offset + saved_shows.limit);
          self.dispatch(IoEvent::GetCurrentUserSavedShows(offset));
        }
      }
    }
  }

  pub fn get_current_user_saved_shows_previous(&mut self) {
    if self.library.saved_shows.index > 0 {
      self.library.saved_shows.index -= 1;
    }
  }

  pub fn get_episode_table_next(&mut self, show_id: String) {
    match self
      .library
      .show_episodes
      .get_results(Some(self.library.show_episodes.index + 1))
      .cloned()
    {
      Some(_) => self.library.show_episodes.index += 1,
      None => {
        if let Some(show_episodes) = &self.library.show_episodes.get_results(None) {
          let offset = Some(show_episodes.offset + show_episodes.limit);
          self.dispatch(IoEvent::GetCurrentShowEpisodes(show_id, offset));
        }
      }
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
    if let Some(Some(playlist_id)) = self.made_for_you_ids.get(index) {
      self.dispatch(IoEvent::GetMadeForYouPlaylistItems(playlist_id.clone(), 0));
    } else {
      self.dispatch(IoEvent::MadeForYouExpand(
        MADE_FOR_YOU_NAMES.get(index).unwrap_or(&"").to_string(),
        index,
      ));
    }
  }

  pub fn get_audio_analysis(&mut self) {
    if let Some(CurrentPlaybackContext {
      item: Some(item), ..
    }) = &self.current_playback_context
    {
      match item {
        PlayableItem::Track(track) => {
          if self.get_current_route().id != RouteId::Analysis {
            let uri = track.id.as_ref().map(|id| id.uri()).unwrap_or_default();
            self.dispatch(IoEvent::GetAudioAnalysis(uri));
            self.push_navigation_stack(RouteId::Analysis, ActiveBlock::Analysis);
          }
        }
        PlayableItem::Episode(_episode) => {
          // No audio analysis available for podcast uris, so just default to the empty analysis
          // view to avoid a 400 error code
          self.push_navigation_stack(RouteId::Analysis, ActiveBlock::Analysis);
        }
        _ => {}
      }
    }
  }

  pub fn get_panel_data(&mut self) {
    let (track_id, artist_id) = match &self.current_playback_context {
      Some(CurrentPlaybackContext {
        item: Some(PlayableItem::Track(track)),
        ..
      }) => (
        track.id.as_ref().map(|id| id.id().to_string()),
        track
          .artists
          .first()
          .and_then(|artist| artist.id.as_ref().map(|id| id.id().to_string())),
      ),
      _ => (None, None),
    };
    if let Some(id) = track_id {
      self.dispatch(IoEvent::GetLyrics(id.clone()));
      self.dispatch(IoEvent::GetTrackCredits(id));
    }
    if let Some(id) = artist_id {
      self.dispatch(IoEvent::GetMonthlyListeners(id));
    }
    self.dispatch(IoEvent::GetQueue);
  }

  pub fn repeat(&mut self) {
    if let Some(context) = &self.current_playback_context.clone() {
      self.dispatch(IoEvent::Repeat(context.repeat_state));
    }
  }

  pub fn get_artist(&mut self, artist_id: String, input_artist_name: String) {
    let user_country = self.get_user_country();
    self.dispatch(IoEvent::GetArtist(
      artist_id,
      input_artist_name,
      user_country,
    ));
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

  /// Whether the current track-table context has more pages to load
  /// (playlist / saved tracks / made-for-you page past the loaded items).
  pub fn track_table_has_more(&self) -> bool {
    match self.track_table.context {
      Some(TrackTableContext::MyPlaylists) => {
        self
          .playlist_tracks
          .as_ref()
          .map(|p| p.items.len() < p.total as usize)
          .unwrap_or(false)
      }
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

  /// Whether the given search block has more pages past what is loaded
  /// (the page total exceeds the items we hold, so more can be fetched).
  pub fn search_block_has_more(&self, block: &SearchResultBlock) -> bool {
    let page = match block {
      SearchResultBlock::AlbumSearch => page_len_total(self.search_results.albums.as_ref()),
      SearchResultBlock::SongSearch => page_len_total(self.search_results.tracks.as_ref()),
      SearchResultBlock::ArtistSearch => page_len_total(self.search_results.artists.as_ref()),
      SearchResultBlock::PlaylistSearch => page_len_total(self.search_results.playlists.as_ref()),
      SearchResultBlock::ShowSearch => page_len_total(self.search_results.shows.as_ref()),
      SearchResultBlock::Empty => return false,
    };
    page.map(|(items_len, total)| items_len < total as usize).unwrap_or(false)
  }

  /// Whether the given search block has a page loaded already.
  pub fn search_block_loaded(&self, block: &SearchResultBlock) -> bool {
    let page = match block {
      SearchResultBlock::AlbumSearch => self.search_results.albums.is_some(),
      SearchResultBlock::SongSearch => self.search_results.tracks.is_some(),
      SearchResultBlock::ArtistSearch => self.search_results.artists.is_some(),
      SearchResultBlock::PlaylistSearch => self.search_results.playlists.is_some(),
      SearchResultBlock::ShowSearch => self.search_results.shows.is_some(),
      SearchResultBlock::Empty => false,
    };
    page
  }

  /// Expand the given search tab: mark it selected and fetch its first page
  /// if it has not been loaded yet.
  pub fn load_search_block(&mut self, block: &SearchResultBlock) {
    self.search_results.selected_block = block.clone();
    if self.is_fetching_next_page || self.search_results.query.is_empty() {
      return;
    }
    if self.search_block_loaded(block) {
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
          self
            .made_for_you_ids
            .get(self.made_for_you_index)
            .and_then(|ids| ids.as_deref()),
          &self.made_for_you_tracks,
        ) {
          let offset = playlist_tracks.items.len() as u32;
          let playlist_id = selected_playlist_id.to_string();
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

  pub fn get_user_country(&self) -> Option<Country> {
    // keep their market=None default instead of a dead field read
    None
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
  /// 6 = seek by typing, 7 = resume last song, 8 = restore settings,
  /// 9 = dev view, 10 = clear disk cache (not a toggle, one-shot),
  /// 11-14 = column visibility, 15 = visualizer style.
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
      3 => {
        self.user_config.behavior.volume_ramp_bar = !self.user_config.behavior.volume_ramp_bar;
      }
      4 => {
        self.user_config.behavior.enable_mouse = !self.user_config.behavior.enable_mouse;
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
      }
      6 => {
        self.user_config.behavior.seek_by_typing = !self.user_config.behavior.seek_by_typing;
      }
      7 => {
        self.user_config.behavior.resume_track = !self.user_config.behavior.resume_track;
      }
      // Restore settings on start: re-sends the last saved playback/volume state at launch.
      8 => {
        self.user_config.behavior.restore_settings = !self.user_config.behavior.restore_settings;
      }
      // Dev view: sidebar shows the request log instead of the library.
      9 => {
        self.dev_view = !self.dev_view;
      }
      // Clear the on-disk playlist/library caches.
      10 => {
        self.dispatch(IoEvent::CleanCache);
      }
      11 => {
        self.user_config.behavior.show_album_column = !self.user_config.behavior.show_album_column;
      }
      12 => {
        self.user_config.behavior.show_artist_column = !self.user_config.behavior.show_artist_column;
      }
      13 => {
        self.user_config.behavior.show_length_column = !self.user_config.behavior.show_length_column;
      }
       14 => {
        self.user_config.behavior.show_date_added_column =
          !self.user_config.behavior.show_date_added_column;
      }
      // Visualizer style: CAVA-style bars or an oscilloscope trace.
      15 => {
        self.user_config.behavior.visualizer_style =
          self.user_config.behavior.visualizer_style.next();
      }
      _ => {}
    }
    if index <= 15 {
      self.dispatch(IoEvent::SaveState);
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

  #[test]
  fn log_request_coalesces_consecutive_repeats() {
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
    // An interleaved event breaks the run: a new burst starts a new entry.
    app.log_request("GetCurrentPlayback".to_string());
    assert_eq!(app.request_log[0].text, "GetCurrentPlayback");
    assert_eq!(app.request_log[0].count, 1);
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

    // total == loaded -> no more pages
    app.search_results.tracks = Some(track_page(3, 3));
    assert!(!app.search_block_has_more(&SearchResultBlock::SongSearch));

    // total > loaded -> more pages available
    app.search_results.tracks = Some(track_page(3, 20));
    assert!(app.search_block_has_more(&SearchResultBlock::SongSearch));

    // not yet loaded -> no more
    app.search_results.tracks = None;
    assert!(!app.search_block_has_more(&SearchResultBlock::SongSearch));

    // untouched sibling blocks stay false
    assert!(!app.search_block_has_more(&SearchResultBlock::AlbumSearch));

    // Empty block never pages
    assert!(!app.search_block_has_more(&SearchResultBlock::Empty));
  }

  #[test]
  fn toggle_visualizer_style_cycles_bars_scope() {
    let mut app = App::default();
    assert_eq!(
      app.user_config.behavior.visualizer_style,
      crate::user_config::VisualizerStyle::Bars
    );
    app.toggle_setting(15);
    assert_eq!(
      app.user_config.behavior.visualizer_style,
      crate::user_config::VisualizerStyle::Oscilloscope
    );
    app.toggle_setting(15);
    assert_eq!(
      app.user_config.behavior.visualizer_style,
      crate::user_config::VisualizerStyle::Bars
    );
  }
}
