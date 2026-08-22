mod album_list;
mod album_tracks;
mod artist_list;
mod artist_page;
mod common_key_events;
mod dialog;
mod episode_table;
mod help_keys;
mod library;
mod made_for_you;
mod mouse;
mod playbar;
mod playlist;
mod podcasts;
mod recently_played;
mod request_log;
mod search_input;
mod search_results;
mod select_device;
mod track_table;
mod unfocused_keys;

use super::app::{ActiveBlock, App, ArtistBlock, DialogContext, RouteId, SearchResultBlock};
use crate::backend::IoEvent;
use crate::event::Key;
use rspotify::model::{context::CurrentPlaybackContext, PlayableItem};

pub use mouse::handle_mouse;
pub use search_input::handler as input_handler;

pub fn handle_app(key: Key, app: &mut App) {
  // When the in-playlist search bar has text (playlist_filter Some),
  // capture every key in the filter handler so typing never triggers
  // global shortcuts. playlist_search_active checks ActiveBlock but the
  // filter outlives the block switch, so check is_some() directly.
  if app.playlist_filter.is_some() {
    track_table::handler(key, app);
    return;
  }
  // First handle any global event and then move to block event
  match key {
    Key::Esc => {
      handle_escape(app);
    }
    _ if key == app.user_config.keys.jump_to_album => {
      handle_jump_to_album(app);
    }
    _ if key == app.user_config.keys.jump_to_artist_album => {
      handle_jump_to_artist_album(app);
    }
    _ if key == app.user_config.keys.jump_to_context => {
      handle_jump_to_context(app);
    }
    _ if key == app.user_config.keys.manage_devices => {
      app.dispatch(IoEvent::GetDevices);
    }
    _ if key == app.user_config.keys.decrease_volume => {
      app.decrease_volume();
    }
    _ if key == app.user_config.keys.increase_volume => {
      app.increase_volume();
    }
    // Press space to toggle playback
    _ if key == app.user_config.keys.toggle_playback => {
      app.toggle_playback();
    }
    _ if key == app.user_config.keys.seek_backwards => {
      app.seek_backwards();
    }
    _ if key == app.user_config.keys.seek_forwards => {
      app.seek_forwards();
    }
    _ if key == app.user_config.keys.next_track => {
      app.dispatch(IoEvent::NextTrack);
    }
    _ if key == app.user_config.keys.previous_track => {
      app.previous_track();
    }
    _ if key == app.user_config.keys.help => {
      app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    }

    _ if key == app.user_config.keys.shuffle => {
      app.shuffle();
    }
    _ if key == app.user_config.keys.repeat => {
      app.repeat();
    }
    _ if key == app.user_config.keys.search => {
      app.set_current_route_state(Some(ActiveBlock::Input), Some(ActiveBlock::Input));
    }
    _ if Some(key) == app.user_config.keys.toggle_sidebar => {
      app.sidebar_minimized = !app.sidebar_minimized;
      app.dispatch(IoEvent::SaveState);
    }
    _ if key == app.user_config.keys.copy_song_url => {
      app.copy_song_url();
    }
    _ if key == app.user_config.keys.copy_album_url => {
      app.copy_album_url();
    }
    _ if key == app.user_config.keys.copy_error => {
      app.copy_error();
    }
    _ if key == app.user_config.keys.music_view => {
      app.get_panel_data();
      app.push_navigation_stack(RouteId::MusicView, ActiveBlock::MusicView);
    }
    Key::Char(c) if c.is_ascii_digit() && app.user_config.behavior.seek_by_typing => {
      handle_digit(app, c);
    }
    _ => handle_block_events(key, app),
  }
}

// Seek-by-typing digit entry. Opens the dialog on the first digit when a track
// is playing; appends while the seek dialog is open. Never steals digits from
// the search input or other dialogs.
fn handle_digit(app: &mut App, c: char) {
  match app.get_current_route().active_block {
    ActiveBlock::Input => {}
    ActiveBlock::Dialog(DialogContext::SeekTime) => {
      let digits = app.dialog.get_or_insert_with(String::new);
      if digits.len() < 6 {
        digits.push(c);
      }
    }
    ActiveBlock::Dialog(_) => {}
    _ => {
      if app.current_playback_context.is_some() {
        app.dialog = Some(c.to_string());
        app.confirm = false;
        app.push_navigation_stack(
          RouteId::Dialog,
          ActiveBlock::Dialog(DialogContext::SeekTime),
        );
      }
    }
  }
}

// Handle event for the current active block
fn handle_block_events(key: Key, app: &mut App) {
  let current_route = app.get_current_route();
  match current_route.active_block {
    ActiveBlock::ArtistBlock => {
      artist_page::handler(key, app);
    }
    ActiveBlock::Input => {
      search_input::handler(key, app);
    }
    ActiveBlock::MyPlaylists => {
      playlist::handler(key, app);
    }
    ActiveBlock::TrackTable => {
      track_table::handler(key, app);
    }
    ActiveBlock::EpisodeTable => {
      episode_table::handler(key, app);
    }
    ActiveBlock::HelpMenu => {
      help_keys::handler(key, app);
    }
    ActiveBlock::Error => {}
    ActiveBlock::SelectDevice => {
      select_device::handler(key, app);
    }
    ActiveBlock::SearchResultBlock => {
      search_results::handler(key, app);
    }
    ActiveBlock::AlbumList => {
      album_list::handler(key, app);
    }
    ActiveBlock::AlbumTracks => {
      album_tracks::handler(key, app);
    }
    ActiveBlock::Library => {
      library::handler(key, app);
    }
    ActiveBlock::Empty => {
      unfocused_keys::handler(key, app);
    }
    ActiveBlock::RecentlyPlayed => {
      recently_played::handler(key, app);
    }
    ActiveBlock::Artists => {
      artist_list::handler(key, app);
    }
    ActiveBlock::MadeForYou => {
      made_for_you::handler(key, app);
    }
    ActiveBlock::Podcasts => {
      podcasts::handler(key, app);
    }
    ActiveBlock::RequestLog => {
      request_log::handler(key, app);
    }
    ActiveBlock::PlayBar => {
      playbar::handler(key, app);
    }
    ActiveBlock::MusicView => {
      // Music view is a global overlay; no per-block keys.
    }
    ActiveBlock::Dialog(_) => {
      dialog::handler(key, app);
    }
  }
}

fn handle_escape(app: &mut App) {
  match app.get_current_route().active_block {
    ActiveBlock::SearchResultBlock => {
      app.search_results.selected_block = SearchResultBlock::Empty;
    }
    ActiveBlock::ArtistBlock => {
      if let Some(artist) = &mut app.artist {
        artist.artist_selected_block = ArtistBlock::Empty;
      }
    }
    ActiveBlock::Error => {
      app.pop_navigation_stack();
    }
    ActiveBlock::Dialog(_) => {
      app.pop_navigation_stack();
    }
    // Music view is a fullscreen overlay pushed on top of the dashboard;
    // leaving restores the route that was active before it was opened.
    ActiveBlock::MusicView => {
      app.pop_navigation_stack();
    }
    ActiveBlock::HelpMenu => {
      if app.help_show_shortcuts {
        app.help_show_shortcuts = false;
        app.help_scroll_offset = 0;
        app.help_menu_page = 0;
      } else {
        app.set_current_route_state(Some(ActiveBlock::Empty), None);
      }
    }
    // These are global views that have no active/inactive distinction so do nothing
    ActiveBlock::SelectDevice => {}
    _ => {
      app.set_current_route_state(Some(ActiveBlock::Empty), None);
    }
  }
}

fn handle_jump_to_context(app: &mut App) {
  if let Some(current_playback_context) = &app.current_playback_context {
    if let Some(play_context) = current_playback_context.context.clone() {
      match play_context._type {
        rspotify::model::Type::Album => handle_jump_to_album(app),
        rspotify::model::Type::Artist => handle_jump_to_artist_album(app),
        rspotify::model::Type::Playlist => {
          app.dispatch(IoEvent::GetPlaylistItems(play_context.uri.clone(), 0))
        }
        _ => {}
      }
    }
  }
}

fn handle_jump_to_album(app: &mut App) {
  if let Some(CurrentPlaybackContext {
    item: Some(item), ..
  }) = app.current_playback_context.to_owned()
  {
    match item {
      PlayableItem::Track(track) => {
        app.dispatch(IoEvent::GetAlbumTracks(Box::new(track.album)));
      }
      PlayableItem::Episode(episode) => {
        app.dispatch(IoEvent::GetShowEpisodes(Box::new(episode.show)));
      }
      _ => {}
    };
  }
}

// NOTE: this only finds the first artist of the song and jumps to their albums
fn handle_jump_to_artist_album(app: &mut App) {
  if let Some(CurrentPlaybackContext {
    item: Some(item), ..
  }) = app.current_playback_context.to_owned()
  {
    match item {
      PlayableItem::Track(track) => {
        if let Some(artist) = track.artists.first() {
          if let Some(artist_id) = artist.id.as_ref() {
            app.get_artist(artist_id.to_string(), artist.name.clone());
            app.push_navigation_stack(RouteId::Artist, ActiveBlock::ArtistBlock);
          }
        }
      }
      PlayableItem::Episode(_episode) => {
        // Do nothing for episode (yet!)
      }
      _ => {}
    }
  };
}
