use super::{super::app::App, common_key_events};
use crate::{
  backend::{original_track_id, IoEvent},
  event::Key,
  tui::group_consecutive_plays,
};
use anyhow::anyhow;
use rspotify::model::{Id, PlayHistory};

/// Resolve a grouped-row index to its first raw history entry.
fn grouped_selected_item(items: &[PlayHistory], group_index: usize) -> Option<&PlayHistory> {
  let groups = group_consecutive_plays(items);
  items.get(groups.get(group_index)?.first_raw)
}

pub fn handler(key: Key, app: &mut App) {
  if common_key_events::down_event(key)
    || common_key_events::up_event(key)
    || common_key_events::high_event(key)
    || common_key_events::middle_event(key)
    || common_key_events::low_event(key)
  {
    app.selection_engaged = true;
  }
  match key {
    k if common_key_events::left_event(k) => common_key_events::handle_left_event(app),
    k if common_key_events::down_event(k) => {
      if let Some(recently_played_result) = &app.recently_played.result {
        let groups = group_consecutive_plays(&recently_played_result.items);
        let next_index =
          common_key_events::on_down_press_handler(&groups, Some(app.recently_played.index));
        app.recently_played.index = next_index;
      }
    }
    k if common_key_events::up_event(k) => {
      if let Some(recently_played_result) = &app.recently_played.result {
        let groups = group_consecutive_plays(&recently_played_result.items);
        let next_index =
          common_key_events::on_up_press_handler(&groups, Some(app.recently_played.index));
        app.recently_played.index = next_index;
      }
    }
    k if common_key_events::high_event(k) => {
      if let Some(_recently_played_result) = &app.recently_played.result {
        let next_index = common_key_events::on_high_press_handler();
        app.recently_played.index = next_index;
      }
    }
    k if common_key_events::middle_event(k) => {
      if let Some(recently_played_result) = &app.recently_played.result {
        let groups = group_consecutive_plays(&recently_played_result.items);
        let next_index = common_key_events::on_middle_press_handler(&groups);
        app.recently_played.index = next_index;
      }
    }
    k if common_key_events::low_event(k) => {
      if let Some(recently_played_result) = &app.recently_played.result {
        let groups = group_consecutive_plays(&recently_played_result.items);
        let next_index = common_key_events::on_low_press_handler(&groups);
        app.recently_played.index = next_index;
      }
    }
    Key::Char('s') => {
      if let Some(recently_played_result) = &app.recently_played.result.clone() {
        if let Some(selected_track) =
          grouped_selected_item(&recently_played_result.items, app.recently_played.index)
        {
          // Relinking: save/remove by original id, not the relinked one.
          let id = original_track_id(&selected_track.track);
          if !id.is_empty() {
            app.dispatch(IoEvent::ToggleSaveTrack(id));
          };
        };
      };
    }
    Key::Enter => {
      if let Some(recently_played_result) = &app.recently_played.result.clone() {
        let items = &recently_played_result.items;
        // The view shows one row per consecutive-play group, so the queue
        // hands over one copy per group — repeats are not queued.
        let groups = group_consecutive_plays(items);
        if app.recently_played.index >= groups.len() {
          if app.recently_played_has_more() {
            app.load_more_recently_played();
          }
          return;
        }
        let track_uris: Vec<String> = groups
          .iter()
          .filter_map(|group| {
            items
              .get(group.first_raw)?
              .track
              .id
              .as_ref()
              .map(|id| id.uri())
          })
          .collect();
        if track_uris.is_empty() {
          app.handle_error(anyhow!(
            "Nothing playable in recently played history (all episodes or local tracks)"
          ));
          return;
        }
        // Episode/local tracks carry no playable uri, so keep the queue offset
        // aligned with the playable subset.
        let offset = groups[..app.recently_played.index]
          .iter()
          .filter(|group| {
            items
              .get(group.first_raw)
              .is_some_and(|item| item.track.id.is_some())
          })
          .count();

        app.dispatch(IoEvent::StartPlayback(None, Some(track_uris), Some(offset)));
      };
    }
    _ if key == app.user_config.keys.add_item_to_queue => {
      if let Some(recently_played_result) = &app.recently_played.result.clone() {
        if let Some(history) =
          grouped_selected_item(&recently_played_result.items, app.recently_played.index)
        {
          app.dispatch(IoEvent::AddItemToQueue(
            history
              .track
              .id
              .as_ref()
              .map(|id| id.uri())
              .unwrap_or_default(),
          ))
        }
      };
    }
    _ => {}
  };
}

#[cfg(test)]
mod tests {
  use super::{super::super::app::ActiveBlock, *};
  use serde_json::json;

  fn mock_history(id: &str, name: &str) -> rspotify::model::PlayHistory {
    serde_json::from_value(json!({
      "track": {
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
        "id": id,
        "is_local": false,
        "name": name,
        "preview_url": null,
        "track_number": 1,
        "type": "track",
      },
      "played_at": "2024-01-01T00:00:00Z",
      "context": null,
    }))
    .unwrap()
  }

  #[test]
  fn enter_hands_over_one_copy_per_group() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::default();
    app.io_tx = Some(tx);
    app.recently_played.result = Some(rspotify::model::CursorBasedPage {
      href: String::new(),
      items: vec![
        mock_history("a", "A"),
        mock_history("a", "A"),
        mock_history("b", "B"),
      ],
      limit: 3,
      next: None,
      cursors: None,
      total: Some(3),
    });
    // Group 1 = B (second row); the A×2 group collapses to a single uri.
    app.recently_played.index = 1;

    handler(Key::Enter, &mut app);

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched.len(), 1);
    match &dispatched[0] {
      IoEvent::StartPlayback(_, Some(uris), Some(offset)) => {
        assert_eq!(uris.len(), 2);
        assert!(uris[0].ends_with("track:a"));
        assert!(uris[1].ends_with("track:b"));
        assert_eq!(*offset, 1);
      }
      other => panic!("unexpected event: {:?}", other),
    }
  }

  #[test]
  fn on_left_press() {
    let mut app = App::default();
    app.set_current_route_state(
      Some(ActiveBlock::AlbumTracks),
      Some(ActiveBlock::AlbumTracks),
    );

    handler(Key::Left, &mut app);
    let current_route = app.get_current_route();
    assert_eq!(current_route.active_block, ActiveBlock::Empty);
    assert_eq!(current_route.hovered_block, ActiveBlock::Library);
  }

  #[test]
  fn on_esc() {
    let mut app = App::default();

    handler(Key::Esc, &mut app);

    let current_route = app.get_current_route();
    assert_eq!(current_route.active_block, ActiveBlock::Empty);
  }
}
