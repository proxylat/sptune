use super::{
  super::app::{App, MADE_FOR_YOU_NAMES},
  common_key_events,
};
use crate::event::Key;

pub fn handler(key: Key, app: &mut App) {
  match key {
    k if common_key_events::left_event(k) => common_key_events::handle_left_event(app),
    k if common_key_events::up_event(k) => {
      let next_index = common_key_events::on_up_press_handler(
        &MADE_FOR_YOU_NAMES,
        Some(app.made_for_you_index),
      );
      app.made_for_you_index = next_index;
    }
    k if common_key_events::down_event(k) => {
      let next_index = common_key_events::on_down_press_handler(
        &MADE_FOR_YOU_NAMES,
        Some(app.made_for_you_index),
      );
      app.made_for_you_index = next_index;
    }
    k if common_key_events::high_event(k) => {
      let next_index = common_key_events::on_high_press_handler();
      app.made_for_you_index = next_index;
    }
    k if common_key_events::middle_event(k) => {
      let next_index = common_key_events::on_middle_press_handler(&MADE_FOR_YOU_NAMES[..]);
      app.made_for_you_index = next_index;
    }
    k if common_key_events::low_event(k) => {
      let next_index = common_key_events::on_low_press_handler(&MADE_FOR_YOU_NAMES[..]);
      app.made_for_you_index = next_index;
    }
    Key::Enter => {
      app.expand_made_for_you(app.made_for_you_index);
    }
    _ => {}
  }
}
