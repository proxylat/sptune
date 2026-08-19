use super::{super::app::App, common_key_events};
use crate::event::Key;

pub fn handler(key: Key, app: &mut App) {
  let count = app.request_log.len();
  let select = |app: &mut App, index: usize| {
    app.request_log_index = Some(index.min(count.saturating_sub(1)));
  };
  match key {
    k if common_key_events::down_event(k) => {
      let index = app.request_log_index.unwrap_or(0);
      select(app, index + 1);
    }
    k if common_key_events::up_event(k) => {
      let index = app.request_log_index.unwrap_or(0);
      select(app, index.saturating_sub(1));
    }
    k if common_key_events::high_event(k) => select(app, 0),
    k if common_key_events::middle_event(k) => select(app, count / 2),
    k if common_key_events::low_event(k) => select(app, count),
    k if k == app.user_config.keys.copy_error => app.copy_request_log(),
    _ => {}
  }
}
