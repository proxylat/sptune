// Mock mode: demo data + a self-contained fake Spotify, so `sptune --mock`
// runs without an account. Split out of network.rs to keep the real client
// file focused on the live API path.
use super::*;

// Real sync-format lyrics for the mock player (public-domain "Amazing Grace",
// LINE_SYNCED timestamps) so the Music View shows realistic scrolling lyrics.
const MOCK_LYRICS: &[(u128, &str)] = &[
  (0, "Amazing grace! how sweet the sound,"),
  (8_000, "That saved a wretch like me!"),
  (16_000, "I once was lost, but now am found,"),
  (24_000, "Was blind, but now I see."),
  (36_000, "'Twas grace that taught my heart to fear,"),
  (44_000, "And grace my fears relieved;"),
  (52_000, "How precious did that grace appear"),
  (60_000, "The hour I first believed."),
  (72_000, "Through many dangers, toils and snares,"),
  (80_000, "I have already come;"),
  (88_000, "'Tis grace hath brought me safe thus far,"),
  (96_000, "And grace will lead me home."),
  (108_000, "The Lord has promised good to me,"),
  (116_000, "His word my hope secures;"),
  (124_000, "He will my shield and portion be,"),
  (132_000, "As long as life endures."),
  (144_000, "Yea, when this flesh and heart shall fail,"),
  (152_000, "And mortal life shall cease,"),
  (160_000, "I shall possess, within the veil,"),
  (168_000, "A life of joy and peace."),
];

#[derive(Clone)]
pub(crate) struct MockState {
  progress_ms: u32,
  is_playing: bool,
  volume: u8,
  shuffle: bool,
  repeat: RepeatState,
  track_index: usize,
  last_progress_update: Instant,
}

impl Default for MockState {
  fn default() -> Self {
    MockState {
      progress_ms: 0,
      is_playing: true,
      volume: 50,
      shuffle: false,
      repeat: RepeatState::Off,
      track_index: 0,
      last_progress_update: Instant::now(),
    }
  }
}

impl<'a> Network<'a> {
pub(crate) async fn handle_mock_event(&mut self, io_event: IoEvent) {
    match io_event {
      IoEvent::GetPlaylists => {
        let mut app = self.app.lock().await;
        app.playlists = Some(self.mock_playlist_page(0));
      }
      IoEvent::GetUser => {
        let mut app = self.app.lock().await;
        app.user = Some(
          serde_json::from_value(json!({
            "country": null,
            "display_name": "Mock User",
            "email": null,
            "explicit_content": null,
            "external_urls": {},
            "followers": null,
            "href": "spotify:user:mockuser",
            "id": "mockuser",
            "images": null,
            "product": null,
          }))
          .unwrap(),
        );
      }
      IoEvent::RefreshUser => {}
      IoEvent::GetDevices => {
        let mut app = self.app.lock().await;
        app.push_navigation_stack(RouteId::SelectedDevice, ActiveBlock::SelectDevice);
        app.devices = Some(vec![self.mock_device()]);
        app.selected_device_index = Some(0);
      }
      IoEvent::GetCurrentPlayback => {
        self.mock_set_playback().await;
      }
      IoEvent::GetPlaylistItems(_playlist_id, offset) => {
        let page = self.mock_playlist_items_page(offset);
        self.set_playlist_tracks_to_table(&page, offset > 0).await;
        let mut app = self.app.lock().await;
        app.playlist_tracks = Some(page);
        app.is_fetching_next_page = false;
        app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
      }
      IoEvent::LoadAllPlaylistItems(_playlist_id) => {
        // Continue from the already-loaded items (cache-serve or prior pages)
        // so the remaining pages don't duplicate them.
        let mut offset = self.app.lock().await.playlist_tracks.as_ref().map_or(0, |p| {
          p.items.len() as u32
        });
        loop {
          let page = self.mock_playlist_items_page(offset);
          let total = page.total;
          self.set_playlist_tracks_to_table(&page, offset > 0).await;
          let mut app = self.app.lock().await;
          if let Some(existing) = app.playlist_tracks.as_mut() {
            existing.items.extend(page.items);
            existing.total = page.total;
          }
          app.is_fetching_next_page = false;
          offset += self.large_search_limit;
          if offset >= total {
            break;
          }
        }
      }
      IoEvent::ReconcilePlaylistTracks(_playlist_id) => {
        // Mock playlists never change; nothing to reconcile.
      }
      IoEvent::ResumeState(saved) => {
        self.mock_state.shuffle = saved.shuffle.unwrap_or(false);
        if let Some(repeat) = &saved.repeat {
          self.mock_state.repeat = match repeat.as_str() {
            "track" => RepeatState::Track,
            "context" => RepeatState::Context,
            _ => RepeatState::Off,
          };
        }
        if let Some(last_page) = &saved.last_page {
          if let Some(_playlist_id) = last_page.strip_prefix("playlist:") {
            let page = self.mock_playlist_items_page(0);
            self.set_playlist_tracks_to_table(&page, false).await;
            let mut app = self.app.lock().await;
            app.playlist_tracks = Some(page);
            app.playlist_offset = 0;
            app.track_table.context = Some(TrackTableContext::MyPlaylists);
            app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
            if let Some((name, desc)) = &saved.track_sort {
              if let Some(column) = sort_column_from_name(name) {
                app.track_table_sort = Some((column, *desc));
                app.sort_tracks();
              }
            }
          }
        }
      }
      IoEvent::GetMadeForYouPlaylistItems(_playlist_id, offset) => {
        let page = self.mock_playlist_items_page(offset);
        self.set_playlist_tracks_to_table(&page, offset > 0).await;
        let mut app = self.app.lock().await;
        app.made_for_you_tracks = Some(page);
        app.is_fetching_next_page = false;
        if app.get_current_route().id != RouteId::TrackTable {
          app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
        }
      }
      IoEvent::MadeForYouExpand(_name, slot) => {
        let mut app = self.app.lock().await;
        app.made_for_you_ids[slot] = Some(format!("mockplaylist{}", slot));
        app.track_table.context = Some(TrackTableContext::MadeForYou);
        app.playlist_offset = 0;
        app.made_for_you_offset = 0;
        let page = self.mock_playlist_items_page(0);
        self.set_playlist_tracks_to_table(&page, false).await;
        app.made_for_you_tracks = Some(page);
        app.is_fetching_next_page = false;
        if app.get_current_route().id != RouteId::TrackTable {
          app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
        }
      }
      IoEvent::SaveState => {
        let app = self.app.lock().await;
        self.save_settings_from_app(&app);
      }
      IoEvent::GetCurrentSavedTracks(offset) => {
        let offset = offset.unwrap_or(0);
        let page = self.mock_saved_tracks_page(offset);
        let mut app = self.app.lock().await;
        let append = offset > 0;
        if append {
          app
            .track_table
            .tracks
            .extend(page.items.iter().map(|item| item.track.clone()));
          app
            .track_table_added_at
            .extend(page.items.iter().map(|item| Some(item.added_at)));
          if app.track_table_sort.is_some() {
            app.sort_tracks();
          }
        } else {
          app.track_table.tracks = page
            .items
            .clone()
            .into_iter()
            .map(|item| item.track)
            .collect::<Vec<FullTrack>>();
          app.track_table_added_at = page.items.iter().map(|item| Some(item.added_at)).collect();
          app.track_table_sort = None;
        }
        page.items.iter().for_each(|item| {
          if let Some(track_id) = &item.track.id {
            app.liked_song_ids_set.insert(track_id.to_string());
          }
        });
        app.library.saved_tracks.add_pages(page);
        app.track_table.context = Some(TrackTableContext::SavedTracks);
        app.is_fetching_next_page = false;
      }
      IoEvent::GetSearchResults(term, _country) => {
        let mut app = self.app.lock().await;
        app.reset_search_results();
        app.search_results.query = term;
      }
      IoEvent::GetMoreSearchResults(block) => {
        let mut app = self.app.lock().await;
        app.is_fetching_next_page = false;
        match block {
          SearchResultBlock::SongSearch => {
            let base = app
              .search_results
              .tracks
              .as_ref()
              .map(|p| p.items.len() as u32)
              .unwrap_or(0);
            let page = self.mock_track_page(4, base);
            if let Some(old) = &mut app.search_results.tracks {
              old.items.extend(page.items);
              old.limit = 4;
              old.offset = base;
              old.total = old.items.len() as u32;
            } else {
              app.search_results.tracks = Some(page);
            }
          }
          SearchResultBlock::ArtistSearch => {
            let base = app
              .search_results
              .artists
              .as_ref()
              .map(|p| p.items.len() as u32)
              .unwrap_or(0);
            let page = self.mock_artist_page(3, base);
            if let Some(old) = &mut app.search_results.artists {
              old.items.extend(page.items);
              old.limit = 3;
              old.offset = base;
              old.total = old.items.len() as u32;
            } else {
              app.search_results.artists = Some(page);
            }
          }
          SearchResultBlock::AlbumSearch => {
            let base = app
              .search_results
              .albums
              .as_ref()
              .map(|p| p.items.len() as u32)
              .unwrap_or(0);
            let page = self.mock_album_page(3, base);
            if let Some(old) = &mut app.search_results.albums {
              old.items.extend(page.items);
              old.limit = 3;
              old.offset = base;
              old.total = old.items.len() as u32;
            } else {
              app.search_results.albums = Some(page);
            }
          }
          SearchResultBlock::PlaylistSearch => {
            let base = app
              .search_results
              .playlists
              .as_ref()
              .map(|p| p.items.len() as u32)
              .unwrap_or(0);
            let page = self.mock_playlist_page(base);
            if let Some(old) = &mut app.search_results.playlists {
              old.items.extend(page.items);
              old.offset = base;
              old.total = old.items.len() as u32;
            } else {
              app.search_results.playlists = Some(page);
            }
          }
          SearchResultBlock::ShowSearch => {}
          SearchResultBlock::Empty => {}
        }
      }
      IoEvent::GetRecentlyPlayed => {
        let mut app = self.app.lock().await;
        app.recently_played.result = Some(self.mock_history_page());
      }
      IoEvent::GetMoreRecentlyPlayed(_before) => {
        let mut app = self.app.lock().await;
        if let Some(page) = &mut app.recently_played.result {
          let base = page.items.len();
          let more = (0..3)
            .map(|i| {
              serde_json::from_value(json!({
                "track": self.mock_track_json(base + i),
                "played_at": "2024-01-01T00:00:00Z",
                "context": null,
              }))
              .unwrap()
            })
            .collect::<Vec<_>>();
          page.items.extend(more);
          page.total = Some(page.items.len() as u32);
        }
      }
      IoEvent::GetFollowedArtists(_after) => {
        let page = self.mock_artist_page(3, 0);
        let mut app = self.app.lock().await;
        app.artists = page.items.clone();
        app.library.saved_artists.add_pages(CursorBasedPage {
          href: String::new(),
          items: page.items,
          limit: 3,
          next: None,
          cursors: None,
          total: Some(3),
        });
      }
      IoEvent::StartPlayback(context_uri, uris, offset) => {
        self.mock_state.is_playing = true;
        let start_new = context_uri.is_some() || uris.is_some() || offset.is_some();
        if start_new {
          self.mock_state.progress_ms = 0;
          let mut app = self.app.lock().await;
          app.song_progress_ms = 0;
          drop(app);
          // Honor the requested track so clicking a song starts THAT song, not
          // always the first one. No modulo: albums are 60 tracks, so the raw
          // offset must index the album's own rows (a % 20 wrap restarted the
          // album from the top, re-marking song 0).
          if let Some(offset) = offset {
            self.mock_state.track_index = offset;
          }
        }
        self.mock_state.last_progress_update = Instant::now();
        self.mock_set_playback().await;
      }
      IoEvent::Seek(position_ms) => {
        self.mock_state.progress_ms = position_ms;
        self.mock_state.last_progress_update = Instant::now();
        self.mock_set_playback().await;
      }
      IoEvent::NextTrack => {
        self.mock_state.track_index = (self.mock_state.track_index + 1) % 20;
        self.mock_state.progress_ms = 0;
        self.mock_state.last_progress_update = Instant::now();
        self.mock_set_playback().await;
      }
      IoEvent::PreviousTrack => {
        self.mock_state.track_index = self.mock_state.track_index.saturating_sub(1);
        self.mock_state.progress_ms = 0;
        self.mock_state.last_progress_update = Instant::now();
        self.mock_set_playback().await;
      }
      IoEvent::Shuffle(_) => {
        self.mock_state.shuffle = !self.mock_state.shuffle;
        self.mock_set_playback().await;
      }
      IoEvent::Repeat(_) => {
        self.mock_state.repeat = match self.mock_state.repeat {
          RepeatState::Off => RepeatState::Context,
          RepeatState::Context => RepeatState::Track,
          RepeatState::Track => RepeatState::Off,
        };
        self.mock_set_playback().await;
      }
      IoEvent::PausePlayback => {
        self.mock_state.is_playing = false;
        self.mock_set_playback().await;
      }
      IoEvent::ChangeVolume(volume) => {
        self.mock_state.volume = volume;
        self.mock_set_playback().await;
      }
      IoEvent::UpdateSearchLimits(_large, _small) => {
        // Mock mode: page at 30 so the first page roughly fits the viewport
        // with a little room to scroll; the rest loads via "Load more songs...".
        self.large_search_limit = 30;
        self.small_search_limit = 4;
        let mut app = self.app.lock().await;
        app.large_search_limit = self.large_search_limit;
      }
      IoEvent::SetTracksToTable(tracks) => {
        let count = tracks.len();
        self
          .set_tracks_to_table(tracks, vec![None; count], (0..count).collect(), false)
          .await;
      }
      IoEvent::SetArtistsToTable(artists) => {
        self.set_artists_to_table(artists).await;
      }
      IoEvent::GetAlbumTracks(album) => {
        let total = 60u32;
        let limit = self.large_search_limit;
        let count = limit.min(total);
        let tracks = Page {
          href: String::new(),
          items: (0..count)
            .map(|i| serde_json::from_value(self.mock_track_json(i as usize)).unwrap())
            .collect(),
          limit,
          next: if count < total { Some("next".into()) } else { None },
          offset: 0,
          previous: None,
          total,
        };
        let mut app = self.app.lock().await;
        app.selected_album_simplified = Some(SelectedAlbum {
          album: *album,
          tracks,
          selected_index: 0,
        });
        app.album_table_context = AlbumTableContext::Simplified;
        app.push_navigation_stack(RouteId::AlbumTracks, ActiveBlock::AlbumTracks);
      }
      IoEvent::GetAlbumTracksMore(_, offset) => {
        let mut app = self.app.lock().await;
        if let Some(album) = &mut app.selected_album_simplified {
          let total = album.tracks.total as usize;
          let end = (offset as usize + self.large_search_limit as usize).min(total);
          let new_items: Vec<_> = (album.tracks.items.len()..end)
            .map(|i| serde_json::from_value(self.mock_track_json(i)).unwrap())
            .collect();
          album.tracks.items.extend(new_items);
          album.tracks.offset = end as u32;
        }
      }
      IoEvent::RefreshAuthentication
      | IoEvent::GetRecommendationsForSeed(..)
      | IoEvent::GetRecommendationsForTrackId(..)
      | IoEvent::CurrentUserSavedAlbumsContains(_)
      | IoEvent::GetCurrentUserSavedAlbums(_)
      | IoEvent::CurrentUserSavedAlbumDelete(_)
      | IoEvent::CurrentUserSavedAlbumAdd(_)
      | IoEvent::UserUnfollowArtists(_)
      | IoEvent::UserFollowArtists(_)
      | IoEvent::UserFollowPlaylist(..)
      | IoEvent::UserUnfollowPlaylist(..)
      | IoEvent::GetAudioAnalysis(_)
      | IoEvent::GetLyrics => {
        // Real-looking synced lyrics (public-domain "Amazing Grace") so the
        // Music View in mock mode shows how actual scrolled lyrics behave.
        let mut app = self.app.lock().await;
        app.lyrics = Some(
          MOCK_LYRICS
            .iter()
            .map(|(ms, words)| (*ms, words.to_string()))
            .collect(),
        );
      }
      IoEvent::GetAudioFeatures(uri) => {
        let mut app = self.app.lock().await;
        app.audio_features = Some((
          uri,
          rspotify::model::audio::AudioFeatures {
            acousticness: 0.2,
            analysis_url: String::new(),
            danceability: 0.6,
            duration: chrono::Duration::minutes(3),
            energy: 0.8,
            id: rspotify::model::TrackId::from_uri(
              "spotify:track:mocktrack0",
            )
            .unwrap(),
            instrumentalness: 0.0,
            key: 7,
            liveness: 0.1,
            loudness: -8.0,
            mode: rspotify::model::Modality::Major,
            speechiness: 0.03,
            tempo: 120.0,
            time_signature: 4,
            track_href: String::new(),
            valence: 0.5,
          },
        ));
      }
      IoEvent::GetMonthlyListeners(_) | IoEvent::GetTrackCredits(_) | IoEvent::GetQueue => {
        let mut app = self.app.lock().await;
        app.monthly_listeners = Some(12_400_000);
        app.track_credits = Some(vec![
          "Lead Vocals: Mock Singer One".to_string(),
          "Songwriter: Mock Songwriter".to_string(),
        ]);
        app.queue_next = if self.mock_state.track_index < 19 {
          Some(format!("Mock Song {}", self.mock_state.track_index + 1))
        } else {
          None
        };
      }
      IoEvent::GetArtist(..) => {
        let mut app = self.app.lock().await;
        app.artist = Some(self.mock_artist());
      }
      IoEvent::GetArtistAlbumsMore(_, offset) => {
        let mut app = self.app.lock().await;
        if let Some(artist) = &mut app.artist {
          let end = ((offset + 10) as usize).min(artist.albums.total as usize);
          for i in artist.albums.items.len()..end {
            artist
              .albums
              .items
              .push(Self::mock_album_json(i));
          }
          artist.albums.offset = end as u32;
        }
      }
      IoEvent::GetArtistTopTracksMore(_, _, offset) => {
        let mut app = self.app.lock().await;
        if let Some(artist) = &mut app.artist {
          let end = ((offset + 10) as usize).min(artist.top_tracks_total);
          for i in artist.top_tracks.len()..end {
            artist.top_tracks.push(self.mock_track(i));
          }
          artist.top_tracks_has_more = end < artist.top_tracks_total;
        }
      }
      | IoEvent::ToggleSaveTrack(_)
      | IoEvent::UserArtistFollowCheck(_)
      | IoEvent::GetAlbum(_)
      | IoEvent::TransferPlaybackToDevice(_)
      | IoEvent::GetAlbumForTrack(_)
      | IoEvent::CurrentUserSavedTracksContains(_)
      | IoEvent::GetCurrentUserSavedShows(_)
      | IoEvent::CurrentUserSavedShowsContains(_)
      | IoEvent::CurrentUserSavedShowDelete(_)
      | IoEvent::CurrentUserSavedShowAdd(_)
      | IoEvent::GetShowEpisodes(_)
      | IoEvent::GetShow(_)
      | IoEvent::GetCurrentShowEpisodes(..)
      | IoEvent::AddItemToQueue(_)
      | IoEvent::AddTrackToPlaylist(..)
      | IoEvent::CleanCache
      | IoEvent::RefreshPlaylists
      | IoEvent::RefreshSavedTracks
      | IoEvent::RefreshSavedAlbums
      | IoEvent::RefreshSavedShows
      | IoEvent::RefreshPlaylistTracks(_) => {}
    }
  }

  async fn mock_set_playback(&mut self) {
    if self.mock_state.is_playing {
      let elapsed = self.mock_state.last_progress_update.elapsed().as_millis() as u32;
      let ended = self.mock_state.progress_ms + elapsed >= 180_000 && self.mock_state.progress_ms < 180_000;
      self.mock_state.progress_ms = (self.mock_state.progress_ms + elapsed).min(180_000);
      // Track ended: honor repeat/shuffle like Spotify would. Progress is
      // reset so the transition fires exactly once.
      if ended {
        match self.mock_state.repeat {
          RepeatState::Track => self.mock_state.progress_ms = 0,
          RepeatState::Context | RepeatState::Off => {
            self.mock_state.track_index = if self.mock_state.shuffle {

              // non-sequential pick is enough for a mock player.
              (self.mock_state.track_index * 7 + 3) % 20
            } else {
              (self.mock_state.track_index + 1) % 20
            };
            self.mock_state.progress_ms = 0;
          }
        }
      }
    }
    self.mock_state.last_progress_update = Instant::now();
    let mut app = self.app.lock().await;
    app.current_playback_context = Some(self.mock_playback());
    app.instant_since_last_current_playback_poll = Instant::now();
    app.seek_ms.take();
    app.is_fetching_current_playback = false;
    self.save_state_from_app(&app);
  }

  fn mock_device(&self) -> Device {
    serde_json::from_value(json!({
      "id": "mock-device",
      "is_active": true,
      "is_private_session": false,
      "is_restricted": false,
      "name": "Mock Device",
      "type": "Computer",
      "volume_percent": self.mock_state.volume,
    }))
    .unwrap()
  }

  fn mock_album_json(i: usize) -> SimplifiedAlbum {
    serde_json::from_value(json!({
      "album_type": "album",
      "artists": [{ "external_urls": {}, "href": null, "id": "mockartist1", "name": "Mock Artist" }],
      "external_urls": {},
      "href": null,
      "id": format!("mockalbum{}", i),
      "images": [],
      "name": format!("Mock Album {}", i + 1),
      "release_date": format!("20{:02}-01-01", i),
      "release_date_precision": "day",
      "total_tracks": 12,
      "type": "album",
    }))
    .unwrap()
  }

  fn mock_track(&self, i: usize) -> FullTrack {
    serde_json::from_value(self.mock_track_json(i)).unwrap()
  }

pub(crate) fn mock_track_json(&self, i: usize) -> serde_json::Value {
    json!({
"album": {
          "artists": [{ "external_urls": {}, "href": null, "id": "mockartist1", "name": "Mock Artist" }],
          "external_urls": {},
          "href": null,
          "id": format!("mockalbum{}", i % 6),
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": "mockartist1", "name": "Mock Artist" }],
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
      "track_number": (i as u32) + 1,
      "type": "track",
    })
  }

  fn mock_playback(&self) -> CurrentPlaybackContext {
    let repeat_state = match self.mock_state.repeat {
      RepeatState::Off => "off",
      RepeatState::Context => "context",
      RepeatState::Track => "track",
    };
    serde_json::from_value(json!({
      "device": self.mock_device(),
      "repeat_state": repeat_state,
      "shuffle_state": self.mock_state.shuffle,
      "context": null,
      "timestamp": chrono::Utc::now().timestamp_millis(),
      "progress_ms": self.mock_state.progress_ms,
      "is_playing": self.mock_state.is_playing,
      "item": self.mock_track_json(self.mock_state.track_index),
      "currently_playing_type": "track",
      "actions": { "disallows": {} },
    }))
    .unwrap()
  }

  fn mock_artist(&self) -> Artist {
    let artist_json = |i: usize| {
      json!({
        "external_urls": {},
        "followers": { "href": null, "total": 12_400_000 },
        "genres": ["mock genre"],
        "href": format!("https://api.spotify.com/v1/artists/mockartist{}", i),
        "id": format!("mockartist{}", i),
        "images": [],
        "name": if i == 1 { "Mock Artist" } else { "Mock Related Artist" },
        "popularity": 87,
        "type": "artist",
      })
    };
    let albums_page = Page {
      href: String::new(),
      items: (0..5).map(Self::mock_album_json).collect(),
      limit: 5,
      next: None,
      offset: 0,
      previous: None,
      total: 26,
    };
    let top_tracks = (0..6).map(|i| self.mock_track(i)).collect();
    let related_artists = (2..5)
      .map(|i| serde_json::from_value(artist_json(i)).unwrap())
      .collect();
    Artist {
      artist_id: "mockartist1".to_string(),
      artist_name: "Mock Artist".to_string(),
      albums: albums_page,
      related_artists,
      top_tracks,
      top_tracks_total: 26,
      top_tracks_has_more: true,
      selected_album_index: 0,
      selected_related_artist_index: 0,
      selected_top_track_index: 0,
      artist_hovered_block: ArtistBlock::TopTracks,
      artist_selected_block: ArtistBlock::Empty,
    }
  }

  fn mock_playlist_page(&self, offset: u32) -> Page<SimplifiedPlaylist> {
    let items = (offset..offset + 3)
      .map(|i| {
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
      })
      .collect();
    Page {
      href: String::new(),
      items,
      limit: 3,
      next: None,
      offset: 0,
      previous: None,
      total: 3,
    }
  }

  fn mock_playlist_items_page(&self, offset: u32) -> Page<PlaylistItem> {
    let end = (offset + self.large_search_limit).min(150);
    let items = (offset..end)
      .map(|i| {
        serde_json::from_value(json!({
          "added_at": mock_date(i),
          "added_by": null,
          "is_local": false,
          "item": self.mock_track_json(i as usize),
        }))
        .unwrap()
      })
      .collect();
    Page {
      href: String::new(),
      items,
      limit: self.large_search_limit,
      next: if end < 150 { Some("next".into()) } else { None },
      offset,
      previous: None,
      total: 150,
    }
  }

pub(crate) fn mock_saved_tracks_page(&self, offset: u32) -> Page<SavedTrack> {

    // high id range (130..150) that no other mock list covers (playlists,
    // albums and search all use ids 0..59), so hearts show up ONLY in Liked
    // Songs instead of on rows the user never liked (liked_song_ids_set is
    // global, like in the real client).
    let saved: Vec<u32> = (130..150).collect();
    let start = (offset as usize).min(saved.len());
    let end = (start + self.large_search_limit as usize).min(saved.len());
    let items = saved[start..end]
      .iter()
      .map(|&i| {
        serde_json::from_value(json!({
          "added_at": mock_date(i),
          "track": self.mock_track_json(i as usize),
        }))
        .unwrap()
      })
      .collect();
    Page {
      href: String::new(),
      items,
      limit: self.large_search_limit,
      next: if end < saved.len() { Some("next".into()) } else { None },
      offset,
      previous: None,
      total: saved.len() as u32,
    }
  }

  fn mock_track_page(&self, count: u32, offset: u32) -> Page<FullTrack> {
    Page {
      href: String::new(),
      items: (offset..offset + count).map(|i| self.mock_track(i as usize)).collect(),
      limit: count,
      next: None,
      offset,
      previous: None,
      total: count,
    }
  }

  fn mock_artist_page(&self, count: u32, offset: u32) -> Page<FullArtist> {
    let items = (offset..offset + count)
      .map(|i| {
        serde_json::from_value(json!({
          "external_urls": {},
          "href": format!("spotify:artist:mockartist{}", i),
          "id": format!("mockartist{}", i),
          "images": [],
          "name": format!("Mock Artist {}", i),
        }))
        .unwrap()
      })
      .collect();
    Page {
      href: String::new(),
      items,
      limit: count,
      next: None,
      offset,
      previous: None,
      total: count,
    }
  }

  fn mock_album_page(&self, count: u32, offset: u32) -> Page<SimplifiedAlbum> {
    let items = (offset..offset + count)
      .map(|i| {
        serde_json::from_value(json!({
          "artists": [{ "external_urls": {}, "href": null, "id": "mockartist1", "name": "Mock Artist" }],
          "external_urls": {},
          "href": null,
          "id": null,
          "images": [],
          "name": format!("Mock Album {}", i),
        }))
        .unwrap()
      })
      .collect();
    Page {
      href: String::new(),
      items,
      limit: count,
      next: None,
      offset,
      previous: None,
      total: count,
    }
  }

  fn mock_history_page(&self) -> CursorBasedPage<PlayHistory> {
    let items = (0..3)
      .map(|i| {
        serde_json::from_value(json!({
          "track": self.mock_track_json(i),
          "played_at": "2024-01-01T00:00:00Z",
          "context": null,
        }))
        .unwrap()
      })
      .collect();
    CursorBasedPage {
      href: String::new(),
      items,
      limit: 3,
      next: None,
      cursors: None,
      total: Some(3),
    }
  }
}
