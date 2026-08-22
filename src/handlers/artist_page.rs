use super::common_key_events;
use crate::app::{ActiveBlock, App, ArtistBlock, RecommendationsContext, TrackTableContext};
use crate::backend::IoEvent;
use crate::event::Key;
use rspotify::model::Id;

fn handle_down_press_on_selected_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        let max = if artist.top_tracks_has_more {
          artist.top_tracks.len()
        } else {
          artist.top_tracks.len().saturating_sub(1)
        };
        artist.selected_top_track_index = (artist.selected_top_track_index + 1).min(max);
      }
      ArtistBlock::Albums => {
        let max_selectable = if artist.albums.items.len() < artist.albums.total as usize {
          artist.albums.items.len()
        } else {
          artist.albums.items.len().saturating_sub(1)
        };
        artist.selected_album_index = (artist.selected_album_index + 1).min(max_selectable);
      }
      ArtistBlock::Singles => {
        let max_selectable = if artist.singles.items.len() < artist.singles.total as usize {
          artist.singles.items.len()
        } else {
          artist.singles.items.len().saturating_sub(1)
        };
        artist.selected_singles_index = (artist.selected_singles_index + 1).min(max_selectable);
      }
      ArtistBlock::EPs => {
        let max_selectable = if artist.eps.items.len() < artist.eps.total as usize {
          artist.eps.items.len()
        } else {
          artist.eps.items.len().saturating_sub(1)
        };
        artist.selected_eps_index = (artist.selected_eps_index + 1).min(max_selectable);
      }
      ArtistBlock::AppearsOn => {
        let max_selectable = if artist.appears_on.items.len() < artist.appears_on.total as usize {
          artist.appears_on.items.len()
        } else {
          artist.appears_on.items.len().saturating_sub(1)
        };
        artist.selected_appears_on_index = (artist.selected_appears_on_index + 1).min(max_selectable);
      }
      ArtistBlock::DiscoveredOn => {
        let max_selectable = if artist.discovered_on.items.len() < artist.discovered_on.total as usize {
          artist.discovered_on.items.len()
        } else {
          artist.discovered_on.items.len().saturating_sub(1)
        };
        artist.selected_discovered_on_index = (artist.selected_discovered_on_index + 1).min(max_selectable);
      }
      _ => {}
    }
  }
}

fn next_artist_tab(current: ArtistBlock) -> ArtistBlock {
  match current {
    ArtistBlock::About => ArtistBlock::TopTracks,
    ArtistBlock::TopTracks => ArtistBlock::Albums,
    ArtistBlock::Albums => ArtistBlock::Singles,
    ArtistBlock::Singles => ArtistBlock::EPs,
    ArtistBlock::EPs => ArtistBlock::AppearsOn,
    ArtistBlock::AppearsOn => ArtistBlock::DiscoveredOn,
    ArtistBlock::DiscoveredOn => ArtistBlock::About,
    ArtistBlock::Empty => ArtistBlock::About,
  }
}
fn prev_artist_tab(current: ArtistBlock) -> ArtistBlock {
  match current {
    ArtistBlock::About => ArtistBlock::DiscoveredOn,
    ArtistBlock::TopTracks => ArtistBlock::About,
    ArtistBlock::Albums => ArtistBlock::TopTracks,
    ArtistBlock::Singles => ArtistBlock::Albums,
    ArtistBlock::EPs => ArtistBlock::Singles,
    ArtistBlock::AppearsOn => ArtistBlock::EPs,
    ArtistBlock::DiscoveredOn => ArtistBlock::AppearsOn,
    ArtistBlock::Empty => ArtistBlock::About,
  }
}
fn handle_down_press_on_hovered_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    artist.artist_hovered_block = next_artist_tab(artist.artist_hovered_block);
  }
}

fn handle_up_press_on_selected_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        let next_index = common_key_events::on_up_press_handler(
          &artist.top_tracks,
          Some(artist.selected_top_track_index),
        );
        let max = if artist.top_tracks_has_more {
          artist.top_tracks.len()
        } else {
          artist.top_tracks.len().saturating_sub(1)
        };
        artist.selected_top_track_index = next_index.min(max);
      }
      ArtistBlock::Albums => {
        let next_index = artist.selected_album_index.saturating_sub(1);
        artist.selected_album_index = if next_index >= artist.albums.items.len() {
          artist.albums.items.len().saturating_sub(1)
        } else {
          next_index
        };
      }
      ArtistBlock::Singles => {
        let next_index = artist.selected_singles_index.saturating_sub(1);
        artist.selected_singles_index = next_index.min(artist.singles.items.len().saturating_sub(1));
      }
      ArtistBlock::EPs => {
        let next_index = artist.selected_eps_index.saturating_sub(1);
        artist.selected_eps_index = next_index.min(artist.eps.items.len().saturating_sub(1));
      }
      ArtistBlock::AppearsOn => {
        let next_index = artist.selected_appears_on_index.saturating_sub(1);
        artist.selected_appears_on_index = next_index.min(artist.appears_on.items.len().saturating_sub(1));
      }
      ArtistBlock::DiscoveredOn => {
        let next_index = artist.selected_discovered_on_index.saturating_sub(1);
        artist.selected_discovered_on_index = next_index.min(artist.discovered_on.items.len().saturating_sub(1));
      }
      _ => {}
    }
  }
}

fn handle_up_press_on_hovered_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    artist.artist_hovered_block = prev_artist_tab(artist.artist_hovered_block);
  }
}

fn handle_high_press_on_selected_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        artist.selected_top_track_index = common_key_events::on_high_press_handler();
      }
      ArtistBlock::Albums => artist.selected_album_index = common_key_events::on_high_press_handler(),
      ArtistBlock::Singles => artist.selected_singles_index = common_key_events::on_high_press_handler(),
      ArtistBlock::EPs => artist.selected_eps_index = common_key_events::on_high_press_handler(),
      ArtistBlock::AppearsOn => artist.selected_appears_on_index = common_key_events::on_high_press_handler(),
      ArtistBlock::DiscoveredOn => artist.selected_discovered_on_index = common_key_events::on_high_press_handler(),
      _ => {}
    }
  }
}

fn handle_middle_press_on_selected_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        artist.selected_top_track_index = common_key_events::on_middle_press_handler(&artist.top_tracks)
      }
      ArtistBlock::Albums => {
        artist.selected_album_index = common_key_events::on_middle_press_handler(&artist.albums.items)
      }
      ArtistBlock::Singles => {
        artist.selected_singles_index = common_key_events::on_middle_press_handler(&artist.singles.items)
      }
      ArtistBlock::EPs => {
        artist.selected_eps_index = common_key_events::on_middle_press_handler(&artist.eps.items)
      }
      ArtistBlock::AppearsOn => {
        artist.selected_appears_on_index = common_key_events::on_middle_press_handler(&artist.appears_on.items)
      }
      ArtistBlock::DiscoveredOn => {
        artist.selected_discovered_on_index =
          common_key_events::on_middle_press_handler(&artist.discovered_on.items)
      }
      _ => {}
    }
  }
}

fn handle_low_press_on_selected_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        artist.selected_top_track_index = common_key_events::on_low_press_handler(&artist.top_tracks)
      }
      ArtistBlock::Albums => {
        artist.selected_album_index = common_key_events::on_low_press_handler(&artist.albums.items)
      }
      ArtistBlock::Singles => {
        artist.selected_singles_index = common_key_events::on_low_press_handler(&artist.singles.items)
      }
      ArtistBlock::EPs => {
        artist.selected_eps_index = common_key_events::on_low_press_handler(&artist.eps.items)
      }
      ArtistBlock::AppearsOn => {
        artist.selected_appears_on_index = common_key_events::on_low_press_handler(&artist.appears_on.items)
      }
      ArtistBlock::DiscoveredOn => {
        artist.selected_discovered_on_index =
          common_key_events::on_low_press_handler(&artist.discovered_on.items)
      }
      _ => {}
    }
  }
}

fn handle_recommend_event_on_selected_block(app: &mut App) {
  //recommendations.
  if let Some(artist) = &mut app.artist.clone() {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        let selected_index = artist.selected_top_track_index;
        if let Some(track) = artist.top_tracks.get(selected_index) {
          let track_id_list: Option<Vec<String>> = track.id.as_ref().map(|id| vec![id.to_string()]);
          app.recommendations_context = Some(RecommendationsContext::Song);
          app.recommendations_seed = track.name.clone();
          app.get_recommendations_for_seed(None, track_id_list, Some(track.clone()));
        }
      }
      _ => {}
    }
  }
}

fn handle_enter_event_on_selected_block(app: &mut App) {
  if let Some(artist) = &mut app.artist.clone() {
    match artist.artist_selected_block {
      ArtistBlock::TopTracks => {
        if artist.selected_top_track_index >= artist.top_tracks.len() {
          app.load_more_artist_top_tracks();
          return;
        }
        let selected_index = artist.selected_top_track_index;
        let top_tracks = artist
          .top_tracks
          .iter()
          .map(|track| track.id.as_ref().map(|id| id.uri()).unwrap_or_default())
          .collect();
        app.dispatch(IoEvent::StartPlayback(
          None,
          Some(top_tracks),
          Some(selected_index),
        ));
      }
      ArtistBlock::Albums => {
        if artist.selected_album_index >= artist.albums.items.len() {
          app.load_more_albums();
        } else if let Some(selected_album) = artist
          .albums
          .items
          .get(artist.selected_album_index)
          .cloned()
        {
          app.track_table.context = Some(TrackTableContext::AlbumSearch);
          app.dispatch(IoEvent::GetAlbumTracks(Box::new(selected_album)));
        }
      }
      ArtistBlock::Singles => {
        if artist.selected_singles_index >= artist.singles.items.len() {
          app.load_more_singles();
        } else if let Some(al) = artist.singles.items.get(artist.selected_singles_index).cloned() {
          app.track_table.context = Some(TrackTableContext::AlbumSearch);
          app.dispatch(IoEvent::GetAlbumTracks(Box::new(al)));
        }
      }
      ArtistBlock::EPs => {
        if artist.selected_eps_index >= artist.eps.items.len() {
          app.load_more_singles();
        } else if let Some(al) = artist.eps.items.get(artist.selected_eps_index).cloned() {
          app.track_table.context = Some(TrackTableContext::AlbumSearch);
          app.dispatch(IoEvent::GetAlbumTracks(Box::new(al)));
        }
      }
      ArtistBlock::AppearsOn => {
        if artist.selected_appears_on_index >= artist.appears_on.items.len() {
          app.load_more_appears_on();
        } else if let Some(al) = artist
          .appears_on
          .items
          .get(artist.selected_appears_on_index)
          .cloned()
        {
          app.track_table.context = Some(TrackTableContext::AlbumSearch);
          app.dispatch(IoEvent::GetAlbumTracks(Box::new(al)));
        }
      }
      ArtistBlock::DiscoveredOn => {
        if artist.selected_discovered_on_index >= artist.discovered_on.items.len() {
          app.load_more_discovered_on();
        } else if let Some(pl) = artist
          .discovered_on
          .items
          .get(artist.selected_discovered_on_index)
          .cloned()
        {
          app.dispatch(IoEvent::GetPlaylistItems(pl.id.to_string(), 0));
        }
      }
      _ => {}
    }
  }
}

fn handle_enter_event_on_hovered_block(app: &mut App) {
  if let Some(artist) = &mut app.artist {
    let tab = artist.artist_hovered_block;
    app.artist_select_tab(tab);
  }
}

pub fn handler(key: Key, app: &mut App) {
  if let Some(artist) = &mut app.artist {
    match key {
      Key::Esc => {
        artist.artist_selected_block = ArtistBlock::Empty;
      }
      k if common_key_events::down_event(k) => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_down_press_on_selected_block(app);
        } else if artist.artist_hovered_block == ArtistBlock::TopTracks {
          let max = if artist.top_tracks_has_more {
            artist.top_tracks.len()
          } else {
            artist.top_tracks.len().saturating_sub(1)
          };
          artist.selected_top_track_index = (artist.selected_top_track_index + 1).min(max);
        } else {
          handle_down_press_on_hovered_block(app);
        }
      }
      k if common_key_events::up_event(k) => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_up_press_on_selected_block(app);
        } else if artist.artist_hovered_block == ArtistBlock::TopTracks {
          let max = if artist.top_tracks_has_more {
            artist.top_tracks.len()
          } else {
            artist.top_tracks.len().saturating_sub(1)
          };
          artist.selected_top_track_index =
            artist.selected_top_track_index.saturating_sub(1).min(max);
        } else {
          handle_up_press_on_hovered_block(app);
        }
      }
      k if common_key_events::left_event(k) => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          artist.artist_selected_block = ArtistBlock::Empty;
        } else if artist.artist_hovered_block == ArtistBlock::About {
          // leftmost tab → leave artist page
          common_key_events::handle_left_event(app);
        } else {
          artist.artist_hovered_block = prev_artist_tab(artist.artist_hovered_block);
        }
      }
      k if common_key_events::right_event(k) => {
        artist.artist_selected_block = ArtistBlock::Empty;
        handle_down_press_on_hovered_block(app);
      }
      k if common_key_events::high_event(k) => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_high_press_on_selected_block(app);
        }
      }
      k if common_key_events::middle_event(k) => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_middle_press_on_selected_block(app);
        }
      }
      k if common_key_events::low_event(k) => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_low_press_on_selected_block(app);
        }
      }
      Key::Enter => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_enter_event_on_selected_block(app);
        } else {
          handle_enter_event_on_hovered_block(app);
        }
      }
      Key::Char('r') => {
        if artist.artist_selected_block != ArtistBlock::Empty {
          handle_recommend_event_on_selected_block(app);
        }
      }
      Key::Char('w') => match artist.artist_selected_block {
        ArtistBlock::Albums
        | ArtistBlock::Singles
        | ArtistBlock::EPs
        | ArtistBlock::AppearsOn => app.current_user_saved_album_add(ActiveBlock::ArtistBlock),
        _ => (),
      },
      Key::Char('D') => match artist.artist_selected_block {
        ArtistBlock::Albums
        | ArtistBlock::Singles
        | ArtistBlock::EPs
        | ArtistBlock::AppearsOn => app.current_user_saved_album_delete(ActiveBlock::ArtistBlock),
        _ => (),
      },
      _ if key == app.user_config.keys.add_item_to_queue => {
        if let ArtistBlock::TopTracks = artist.artist_selected_block {
          if let Some(track) = artist.top_tracks.get(artist.selected_top_track_index) {
            let uri = track.id.as_ref().map(|id| id.uri()).unwrap_or_default();
            app.dispatch(IoEvent::AddItemToQueue(uri));
          };
        }
      }
      _ => {}
    };
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::ActiveBlock;

  #[test]
  fn on_esc() {
    let mut app = App::default();

    handler(Key::Esc, &mut app);

    let current_route = app.get_current_route();
    assert_eq!(current_route.active_block, ActiveBlock::Empty);
  }
}
