use super::{
  super::app::{App, RecommendationsContext, TrackTable, TrackTableContext},
  common_key_events,
};
use crate::backend::IoEvent;
use crate::event::Key;
use crate::lcg::rand_idx;
use crate::tui::layout::song_table_viewport;
use rspotify::model::Id;

pub fn handler(key: Key, app: &mut App) {
  // When the in-playlist search bar is active, capture ALL keys here so
  // typing doesn't move the cursor, play songs, or trigger other actions.
  if app.playlist_filter.is_some() {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let cw = |c: char| UnicodeWidthChar::width(c).unwrap_or(0) as u16;
    match key {
      k if Some(k) == app.user_config.keys.search_in_playlist => {
        app.playlist_filter = None;
        app.playlist_filter_idx = 0;
        app.playlist_filter_cursor_position = 0;
      }
      Key::Char(c) => {
        if let Some(query) = app.playlist_filter.as_mut() {
          let idx = app.playlist_filter_idx.min(query.chars().count());
          let byte_idx = query.chars().take(idx).map(|ch| ch.len_utf8()).sum::<usize>();
          query.insert(byte_idx, c);
          app.playlist_filter_idx += 1;
          app.playlist_filter_cursor_position += cw(c);
        }
      }
      Key::Backspace | Key::Ctrl('h') => {
        if app.playlist_filter_idx > 0 {
          if let Some(query) = app.playlist_filter.as_mut() {
            let idx = app.playlist_filter_idx - 1;
            let mut chars: Vec<char> = query.chars().collect();
            let removed = chars.remove(idx);
            *query = chars.into_iter().collect();
            app.playlist_filter_idx -= 1;
            app.playlist_filter_cursor_position = app.playlist_filter_cursor_position.saturating_sub(cw(removed));
          }
        }
      }
      Key::Delete | Key::Ctrl('d') => {
        if let Some(query) = app.playlist_filter.as_mut() {
          let idx = app.playlist_filter_idx;
          if idx < query.chars().count() {
            let mut chars: Vec<char> = query.chars().collect();
            chars.remove(idx);
            *query = chars.into_iter().collect();
          }
        }
      }
      Key::Left | Key::Ctrl('b') => {
        if app.playlist_filter_idx > 0 {
          if let Some(query) = app.playlist_filter.as_deref() {
            let ch = query.chars().nth(app.playlist_filter_idx - 1).unwrap();
            app.playlist_filter_idx -= 1;
            app.playlist_filter_cursor_position = app.playlist_filter_cursor_position.saturating_sub(cw(ch));
          }
        }
      }
      Key::Right | Key::Ctrl('f') => {
        if let Some(query) = app.playlist_filter.as_deref() {
          if app.playlist_filter_idx < query.chars().count() {
            let ch = query.chars().nth(app.playlist_filter_idx).unwrap();
            app.playlist_filter_idx += 1;
            app.playlist_filter_cursor_position += cw(ch);
          }
        }
      }
      Key::Home | Key::Ctrl('a') => {
        app.playlist_filter_idx = 0;
        app.playlist_filter_cursor_position = 0;
      }
      Key::End | Key::Ctrl('e') => {
        if let Some(query) = app.playlist_filter.as_deref() {
          app.playlist_filter_idx = query.chars().count();
          app.playlist_filter_cursor_position = UnicodeWidthStr::width(query) as u16;
        }
      }
      Key::Esc => {
        app.playlist_filter = None;
        app.playlist_filter_idx = 0;
        app.playlist_filter_cursor_position = 0;
      }
      _ => {}
    }
    return;
  }
  if common_key_events::down_event(key)
    || common_key_events::up_event(key)
    || common_key_events::high_event(key)
    || common_key_events::middle_event(key)
    || common_key_events::low_event(key)
  {
    app.selection_engaged = true;
  }
  match key {
    k if k == app.user_config.keys.add_to_playlist => {
      app.open_add_to_playlist();
    }
    k if Some(k) == app.user_config.keys.remove_from_playlist => {
      app.remove_selected_track_from_playlist();
    }
    k if common_key_events::left_event(k) => common_key_events::handle_left_event(app),
    k if common_key_events::down_event(k) => {
      let len = app.track_table.tracks.len();
      if app.track_table_has_more() && app.track_table.selected_index == len {
        // stay on the "Load more songs..." row
      } else if app.track_table_has_more() && app.track_table.selected_index + 1 == len {
        app.track_table.selected_index = len;
      } else if app.track_table.selected_index + 1 >= len {
        try_load_next_page(app);
      } else {
        app.track_table.selected_index += 1;
      }
    }
    k if common_key_events::up_event(k) => {
      let next_index = common_key_events::on_up_press_handler(
        &app.track_table.tracks,
        Some(app.track_table.selected_index),
      );
      app.track_table.selected_index = next_index;
    }
    k if common_key_events::high_event(k) => {
      let next_index = common_key_events::on_high_press_handler();
      app.track_table.selected_index = next_index;
    }
    k if common_key_events::middle_event(k) => {
      let next_index = common_key_events::on_middle_press_handler(&app.track_table.tracks);
      app.track_table.selected_index = next_index;
    }
    k if common_key_events::low_event(k) => {
      let next_index = common_key_events::on_low_press_handler(&app.track_table.tracks);
      app.track_table.selected_index = next_index;
    }
    Key::Enter => {
      if app.track_table.selected_index >= app.track_table.tracks.len() {
        if app.track_table_has_more() {
          app.load_more_tracks();
        }
      } else {
        on_enter(app);
      }
    }
    k if Some(k) == app.user_config.keys.refresh => match app.track_table.context {
      Some(TrackTableContext::SavedTracks) => app.dispatch(IoEvent::RefreshSavedTracks),
      Some(TrackTableContext::MyPlaylists) => {
        if let (Some(playlists), Some(index)) = (
          &app.playlists,
          &app.active_playlist_index.or(app.selected_playlist_index),
        ) {
          if let Some(selected_playlist) = playlists.items.get(*index) {
            app.dispatch(IoEvent::RefreshPlaylistTracks(
              selected_playlist.id.to_string(),
            ));
          }
        }
      }
      _ => {}
    },
    k if Some(k) == app.user_config.keys.search_in_playlist => {
      // Toggle the in-playlist search bar. Only valid on a playlist page.
      if matches!(
        app.track_table.context,
        Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
      ) {
        app.playlist_filter = Some(String::new());
        app.playlist_filter_idx = 0;
        app.playlist_filter_cursor_position = 0;
      }
    }
    // Scroll down
    k if k == app.user_config.keys.next_page => {
      match &app.track_table.context {
        Some(context) => match context {
          TrackTableContext::MyPlaylists => {
            if let (Some(playlists), Some(selected_playlist_index)) =
              (&app.playlists, &app.selected_playlist_index)
            {
              if let Some(selected_playlist) =
                playlists.items.get(selected_playlist_index.to_owned())
              {
                if let Some(playlist_tracks) = &app.playlist_tracks {
                  if app.playlist_offset + app.large_search_limit < playlist_tracks.total {
                    app.playlist_offset += app.large_search_limit;
                    let playlist_id = selected_playlist.id.to_string();
                    app.dispatch(IoEvent::GetPlaylistItems(playlist_id, app.playlist_offset));
                  }
                }
              }
            };
          }
          TrackTableContext::RecommendedTracks => {}
          TrackTableContext::SavedTracks => {
            app.get_current_user_saved_tracks_next();
          }
          TrackTableContext::AlbumSearch => {}
          TrackTableContext::PlaylistSearch => {}
          TrackTableContext::MadeForYou => {
            if let (Some(selected_playlist_id), Some(playlist_tracks)) = (
              app
                .made_for_you_playlist_id(app.made_for_you_index)
                .as_deref(),
              &app.made_for_you_tracks,
            ) {
              if app.made_for_you_offset + app.large_search_limit < playlist_tracks.total {
                app.made_for_you_offset += app.large_search_limit;
                app.dispatch(IoEvent::GetMadeForYouPlaylistItems(
                  selected_playlist_id.to_string(),
                  app.made_for_you_offset,
                ));
              }
            }
          }
        },
        None => {}
      };
    }
    // Scroll up
    k if k == app.user_config.keys.previous_page => {
      match &app.track_table.context {
        Some(context) => match context {
          TrackTableContext::MyPlaylists => {
            if let (Some(playlists), Some(selected_playlist_index)) =
              (&app.playlists, &app.selected_playlist_index)
            {
              if app.playlist_offset >= app.large_search_limit {
                app.playlist_offset -= app.large_search_limit;
              };
              if let Some(selected_playlist) =
                playlists.items.get(selected_playlist_index.to_owned())
              {
                let playlist_id = selected_playlist.id.to_string();
                app.dispatch(IoEvent::GetPlaylistItems(playlist_id, app.playlist_offset));
              }
            };
          }
          TrackTableContext::RecommendedTracks => {}
          TrackTableContext::SavedTracks => {
            app.get_current_user_saved_tracks_previous();
          }
          TrackTableContext::AlbumSearch => {}
          TrackTableContext::PlaylistSearch => {}
          TrackTableContext::MadeForYou => {
            if app.made_for_you_offset >= app.large_search_limit {
              app.made_for_you_offset -= app.large_search_limit;
            }
            if let Some(selected_playlist_id) = app
              .made_for_you_playlist_id(app.made_for_you_index)
              .as_deref()
            {
              app.dispatch(IoEvent::GetMadeForYouPlaylistItems(
                selected_playlist_id.to_string(),
                app.made_for_you_offset,
              ));
            }
          }
        },
        None => {}
      };
    }
    Key::Char('s') => handle_save_track_event(app),
    Key::Char('S') => play_random_song(app),
    k if k == app.user_config.keys.jump_to_end => jump_to_end(app),
    k if k == app.user_config.keys.jump_to_start => jump_to_start(app),
    //recommended song radio
    Key::Char('R') => {
      handle_recommended_tracks(app);
    }
    _ if key == app.user_config.keys.add_item_to_queue => on_queue(app),
    _ => {}
  }
  keep_selection_visible(app);
}

// The drawer renders scroll_offset verbatim (the wheel scrolls the view
// independently), so moving the selection with the keyboard must nudge the
// view to keep the selected row on screen.
fn keep_selection_visible(app: &mut App) {
  let viewport = song_table_viewport(app);
  let selected = app.track_table.selected_index;
  let offset = &mut app.track_table.scroll_offset;
  if selected < *offset {
    *offset = selected;
  } else if selected >= *offset + viewport {
    *offset = selected + 1 - viewport;
  }
}

fn play_random_song(app: &mut App) {
  if let Some(context) = &app.track_table.context {
    match context {
      TrackTableContext::MyPlaylists => {
        let (context_uri, track_json) = match (&app.selected_playlist_index, &app.playlists) {
          (Some(selected_playlist_index), Some(playlists)) => {
            if let Some(selected_playlist) = playlists.items.get(selected_playlist_index.to_owned())
            {
              (
                Some(selected_playlist.id.uri()),
                Some(selected_playlist.items.total),
              )
            } else {
              (None, None)
            }
          }
          _ => (None, None),
        };

        if let Some(num_tracks) = track_json {
          app.dispatch(IoEvent::StartPlayback(
            context_uri,
            None,
            Some(rand_idx(num_tracks as usize)),
          ));
        }
      }
      TrackTableContext::RecommendedTracks => {}
      TrackTableContext::SavedTracks => {
        if let Some(saved_tracks) = &app.library.saved_tracks.get_results(None) {
          let track_uris: Vec<String> = saved_tracks
            .items
            .iter()
            .map(|item| {
              item
                .track
                .id
                .as_ref()
                .map(|id| id.uri())
                .unwrap_or_default()
            })
            .collect();
          let pick = rand_idx(track_uris.len());
          app.dispatch(IoEvent::StartPlayback(None, Some(track_uris), Some(pick)))
        }
      }
      TrackTableContext::AlbumSearch => {}
      TrackTableContext::PlaylistSearch => {
        let (context_uri, playlist_track_json) = match (
          &app.search_results.selected_playlists_index,
          &app.search_results.playlists,
        ) {
          (Some(selected_playlist_index), Some(playlist_result)) => {
            if let Some(selected_playlist) = playlist_result
              .items
              .get(selected_playlist_index.to_owned())
            {
              (
                Some(selected_playlist.id.uri()),
                Some(selected_playlist.items.total),
              )
            } else {
              (None, None)
            }
          }
          _ => (None, None),
        };
        if let Some(num_tracks) = playlist_track_json {
          app.dispatch(IoEvent::StartPlayback(
            context_uri,
            None,
            Some(rand_idx(num_tracks as usize)),
          ))
        }
      }
      TrackTableContext::MadeForYou => {
        if let (Some(selected_playlist_id), Some(playlist_tracks)) = (
          app
            .made_for_you_playlist_id(app.made_for_you_index)
            .as_deref(),
          &app.made_for_you_tracks,
        ) {
          let num_tracks = playlist_tracks.total;
          let uri = Some(format!("spotify:playlist:{}", selected_playlist_id));
          app.dispatch(IoEvent::StartPlayback(
            uri,
            None,
            Some(rand_idx(num_tracks as usize)),
          ));
        }
      }
    }
  };
}

fn handle_save_track_event(app: &mut App) {
  let (selected_index, tracks) = (&app.track_table.selected_index, &app.track_table.tracks);
  if let Some(track) = tracks.get(*selected_index) {
    if let Some(id) = &track.id {
      let id = id.to_string();
      app.dispatch(IoEvent::ToggleSaveTrack(id));
    };
  };
}

fn handle_recommended_tracks(app: &mut App) {
  let (selected_index, tracks) = (&app.track_table.selected_index, &app.track_table.tracks);
  if let Some(track) = tracks.get(*selected_index) {
    let first_track = track.clone();
    let track_id_list = track.id.as_ref().map(|id| vec![id.to_string()]);

    app.recommendations_context = Some(RecommendationsContext::Song);
    app.recommendations_seed = first_track.name.clone();
    app.get_recommendations_for_seed(None, track_id_list, Some(first_track));
  };
}

fn jump_to_end(app: &mut App) {
  match &app.track_table.context {
    Some(context) => match context {
      TrackTableContext::MyPlaylists => {
        if let (Some(playlists), Some(selected_playlist_index)) =
          (&app.playlists, &app.selected_playlist_index)
        {
          if let Some(selected_playlist) = playlists.items.get(selected_playlist_index.to_owned()) {
            let total_tracks = selected_playlist.items.total;

            if app.large_search_limit < total_tracks {
              app.playlist_offset = total_tracks - (total_tracks % app.large_search_limit);
              let playlist_id = selected_playlist.id.to_string();
              app.dispatch(IoEvent::GetPlaylistItems(playlist_id, app.playlist_offset));
            }
          }
        }
      }
      TrackTableContext::RecommendedTracks => {}
      TrackTableContext::SavedTracks => {}
      TrackTableContext::AlbumSearch => {}
      TrackTableContext::PlaylistSearch => {}
      TrackTableContext::MadeForYou => {}
    },
    None => {}
  }
}

fn on_enter(app: &mut App) {
  let TrackTable {
    context,
    selected_index,
    tracks,
    ..
  } = &app.track_table;
  match &context {
    Some(context) => match context {
      TrackTableContext::MyPlaylists => {
        if let Some(track) = tracks.get(*selected_index) {
          let context_uri = app.track_table_playlist_uri();
          let track_uri = track.id.as_ref().map(|id| id.uri());
          app.dispatch(IoEvent::StartPlaybackAt(context_uri, track_uri));
        };
      }
      TrackTableContext::RecommendedTracks => {
        app.dispatch(IoEvent::StartPlayback(
          None,
          Some(
            app
              .recommended_tracks
              .iter()
              .map(|x| x.id.as_ref().map(|id| id.uri()).unwrap_or_default())
              .collect::<Vec<String>>(),
          ),
          Some(app.track_table.selected_index),
        ));
      }
      TrackTableContext::SavedTracks => {
        if let Some(saved_tracks) = &app.library.saved_tracks.get_results(None) {
          let track_uris: Vec<String> = saved_tracks
            .items
            .iter()
            .map(|item| {
              item
                .track
                .id
                .as_ref()
                .map(|id| id.uri())
                .unwrap_or_default()
            })
            .collect();

          app.dispatch(IoEvent::StartPlayback(
            None,
            Some(track_uris),
            Some(app.track_table.selected_index),
          ));
        };
      }
      TrackTableContext::AlbumSearch => {}
      TrackTableContext::PlaylistSearch => {
        let TrackTable {
          selected_index,
          tracks,
          ..
        } = &app.track_table;
        if let Some(track) = tracks.get(*selected_index) {
          let context_uri = app.track_table_playlist_uri();
          let track_uri = track.id.as_ref().map(|id| id.uri());
          app.dispatch(IoEvent::StartPlaybackAt(context_uri, track_uri));
        };
      }
      TrackTableContext::MadeForYou => {
        if let Some(track) = tracks.get(*selected_index) {
          let context_uri = app
            .made_for_you_playlist_id(app.made_for_you_index)
            .map(|id| format!("spotify:playlist:{}", id));
          let track_uri = track.id.as_ref().map(|id| id.uri());
          app.dispatch(IoEvent::StartPlaybackAt(context_uri, track_uri));
        }
      }
    },
    None => {}
  };
}

fn on_queue(app: &mut App) {
  let TrackTable {
    context,
    selected_index,
    tracks,
    ..
  } = &app.track_table;
  match &context {
    Some(context) => match context {
      TrackTableContext::MyPlaylists => {
        if let Some(track) = tracks.get(*selected_index) {
          let uri = track.id.as_ref().map(|id| id.uri()).unwrap_or_default();
          app.dispatch(IoEvent::AddItemToQueue(uri));
        };
      }
      TrackTableContext::RecommendedTracks => {
        if let Some(full_track) = app.recommended_tracks.get(app.track_table.selected_index) {
          let uri = full_track
            .id
            .as_ref()
            .map(|id| id.uri())
            .unwrap_or_default();
          app.dispatch(IoEvent::AddItemToQueue(uri));
        }
      }
      TrackTableContext::SavedTracks => {
        if let Some(page) = app.library.saved_tracks.get_results(None) {
          if let Some(saved_track) = page.items.get(app.track_table.selected_index) {
            let uri = saved_track
              .track
              .id
              .as_ref()
              .map(|id| id.uri())
              .unwrap_or_default();
            app.dispatch(IoEvent::AddItemToQueue(uri));
          }
        }
      }
      TrackTableContext::AlbumSearch => {}
      TrackTableContext::PlaylistSearch => {
        let TrackTable {
          selected_index,
          tracks,
          ..
        } = &app.track_table;
        if let Some(track) = tracks.get(*selected_index) {
          let uri = track.id.as_ref().map(|id| id.uri()).unwrap_or_default();
          app.dispatch(IoEvent::AddItemToQueue(uri));
        };
      }
      TrackTableContext::MadeForYou => {
        if let Some(track) = tracks.get(*selected_index) {
          let uri = track.id.as_ref().map(|id| id.uri()).unwrap_or_default();
          app.dispatch(IoEvent::AddItemToQueue(uri));
        }
      }
    },
    None => {}
  };
}

fn jump_to_start(app: &mut App) {
  match &app.track_table.context {
    Some(context) => match context {
      TrackTableContext::MyPlaylists => {
        if let (Some(playlists), Some(selected_playlist_index)) =
          (&app.playlists, &app.selected_playlist_index)
        {
          if let Some(selected_playlist) = playlists.items.get(selected_playlist_index.to_owned()) {
            app.playlist_offset = 0;
            let playlist_id = selected_playlist.id.to_string();
            app.dispatch(IoEvent::GetPlaylistItems(playlist_id, app.playlist_offset));
          }
        }
      }
      TrackTableContext::RecommendedTracks => {}
      TrackTableContext::SavedTracks => {}
      TrackTableContext::AlbumSearch => {}
      TrackTableContext::PlaylistSearch => {}
      TrackTableContext::MadeForYou => {}
    },
    None => {}
  }
}

fn try_load_next_page(app: &mut App) {
  if app.is_fetching_next_page {
    return;
  }
  match &app.track_table.context {
    Some(context) => match context {
      TrackTableContext::MyPlaylists => {
        if let (Some(playlists), Some(selected_playlist_index)) =
          (&app.playlists, &app.selected_playlist_index)
        {
          if let Some(selected_playlist) = playlists.items.get(selected_playlist_index.to_owned()) {
            if let Some(playlist_tracks) = &app.playlist_tracks {
              if app.playlist_offset + app.large_search_limit < playlist_tracks.total {
                app.playlist_offset += app.large_search_limit;
                app.is_fetching_next_page = true;
                let playlist_id = selected_playlist.id.to_string();
                app.dispatch(IoEvent::GetPlaylistItems(playlist_id, app.playlist_offset));
              }
            }
          }
        }
      }
      TrackTableContext::SavedTracks => {
        app.is_fetching_next_page = true;
        app.get_current_user_saved_tracks_next();
      }
      TrackTableContext::MadeForYou => {
        if let Some(playlist_id) = app
          .made_for_you_playlist_id(app.made_for_you_index)
          .as_deref()
        {
          if let Some(playlist_tracks) = &app.made_for_you_tracks {
            if app.made_for_you_offset + app.large_search_limit < playlist_tracks.total {
              app.made_for_you_offset += app.large_search_limit;
              app.is_fetching_next_page = true;
              app.dispatch(IoEvent::GetMadeForYouPlaylistItems(
                playlist_id.to_string(),
                app.made_for_you_offset,
              ));
            }
          }
        }
      }
      _ => {}
    },
    None => {}
  }
}
