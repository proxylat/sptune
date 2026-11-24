use super::common_key_events;
use crate::{app::App, backend::IoEvent, event::Key};
use rspotify::model::PlayableItem;
use rspotify::prelude::Id;

pub fn handler(key: Key, app: &mut App) {
  match key {
    k if common_key_events::left_event(k) => common_key_events::handle_left_event(app),
    k if common_key_events::down_event(k) => {
      if !app.queue_items.is_empty() {
        let next = common_key_events::on_down_press_handler(
          &app.queue_items,
          Some(app.queue_selected_index),
        );
        app.queue_selected_index = next;
      }
    }
    k if common_key_events::up_event(k) => {
      if !app.queue_items.is_empty() {
        let next =
          common_key_events::on_up_press_handler(&app.queue_items, Some(app.queue_selected_index));
        app.queue_selected_index = next;
      }
    }
    k if Some(k) == app.user_config.keys.refresh => {
      app.dispatch(IoEvent::GetQueue);
    }
    Key::Enter => {
      if let Some(item) = app.queue_items.get(app.queue_selected_index) {
        let uri = match item {
          PlayableItem::Track(t) => t.id.as_ref().map(|id| id.uri()),
          PlayableItem::Episode(e) => Some(e.id.uri()),
          PlayableItem::Unknown(_) => None,
        };
        if let Some(uri) = uri {
          app.dispatch(IoEvent::StartPlayback(None, Some(vec![uri]), Some(0)));
        }
      }
    }
    _ => {}
  }
}
