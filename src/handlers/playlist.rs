use super::{
  super::app::{App, DialogContext, TrackTableContext},
  common_key_events,
};
use crate::app::{ActiveBlock, RouteId};
use crate::backend::IoEvent;
use crate::event::Key;

pub fn handler(key: Key, app: &mut App) {
  match key {
    k if common_key_events::right_event(k) => common_key_events::handle_right_event(app),
    k if common_key_events::down_event(k) => {
      match &app.playlists {
        Some(p) => {
          if let Some(selected_playlist_index) = app.selected_playlist_index {
            let next_index =
              common_key_events::on_down_press_handler(&p.items, Some(selected_playlist_index));
            app.selected_playlist_index = Some(next_index);
          }
        }
        None => {}
      };
    }
    k if common_key_events::up_event(k) => {
      match &app.playlists {
        Some(p) => {
          let next_index =
            common_key_events::on_up_press_handler(&p.items, app.selected_playlist_index);
          app.selected_playlist_index = Some(next_index);
        }
        None => {}
      };
    }
    k if common_key_events::high_event(k) => {
      match &app.playlists {
        Some(_p) => {
          let next_index = common_key_events::on_high_press_handler();
          app.selected_playlist_index = Some(next_index);
        }
        None => {}
      };
    }
    k if common_key_events::middle_event(k) => {
      match &app.playlists {
        Some(p) => {
          let next_index = common_key_events::on_middle_press_handler(&p.items);
          app.selected_playlist_index = Some(next_index);
        }
        None => {}
      };
    }
    k if common_key_events::low_event(k) => {
      match &app.playlists {
        Some(p) => {
          let next_index = common_key_events::on_low_press_handler(&p.items);
          app.selected_playlist_index = Some(next_index);
        }
        None => {}
      };
    }
    k if Some(k) == app.user_config.keys.refresh => app.dispatch(IoEvent::RefreshPlaylists),
    Key::Enter => {
      app.clear_search_input();
      if let (Some(playlists), Some(selected_playlist_index)) =
        (&app.playlists, &app.selected_playlist_index)
      {
        // Re-opening the playlist that is already open must not re-fetch.
        if app.track_table.context == Some(TrackTableContext::MyPlaylists)
          && app.active_playlist_index == Some(*selected_playlist_index)
        {
          return;
        }
        app.active_playlist_index = Some(selected_playlist_index.to_owned());
        app.track_table.context = Some(TrackTableContext::MyPlaylists);
        app.playlist_offset = 0;
        if let Some(selected_playlist) = playlists.items.get(selected_playlist_index.to_owned()) {
          let playlist_id = selected_playlist.id.to_string();
          app.dispatch(IoEvent::GetPlaylistItems(playlist_id, app.playlist_offset));
        }
      };
    }
    Key::Char('D') => {
      if let (Some(playlists), Some(selected_index)) = (&app.playlists, app.selected_playlist_index)
      {
        let selected_playlist = &playlists.items[selected_index].name;
        app.dialog = Some(selected_playlist.clone());
        app.confirm = false;

        app.push_navigation_stack(
          RouteId::Dialog,
          ActiveBlock::Dialog(DialogContext::PlaylistWindow),
        );
      }
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn mock_playlist_page(n: usize) -> rspotify::model::Page<rspotify::model::SimplifiedPlaylist> {
    let items = (0..n)
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
          "tracks": { "href": "", "total": 1 },
          "type": "playlist",
        }))
        .unwrap()
      })
      .collect();
    rspotify::model::Page {
      href: String::new(),
      items,
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total: n as u32,
    }
  }

  #[test]
  fn enter_on_the_open_playlist_does_not_refetch() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::default();
    app.io_tx = Some(tx);
    app.playlists = Some(mock_playlist_page(3));
    app.selected_playlist_index = Some(1);
    app.active_playlist_index = Some(1);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);

    handler(Key::Enter, &mut app);

    assert!(rx.try_iter().collect::<Vec<_>>().is_empty());
  }

  #[test]
  fn enter_on_a_different_playlist_dispatches() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::default();
    app.io_tx = Some(tx);
    app.playlists = Some(mock_playlist_page(3));
    app.selected_playlist_index = Some(2);
    app.active_playlist_index = Some(1);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);

    handler(Key::Enter, &mut app);

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::GetPlaylistItems(
        "spotify:playlist:mockplaylist2".to_string(),
        0
      )]
    );
    assert_eq!(app.active_playlist_index, Some(2));
  }
}
