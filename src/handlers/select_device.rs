use super::{
  super::app::{ActiveBlock, App},
  common_key_events,
};
use crate::backend::IoEvent;
use crate::event::Key;

pub fn handler(key: Key, app: &mut App) {
  match key {
    Key::Esc => {
      app.set_current_route_state(Some(ActiveBlock::Library), None);
    }
    k if common_key_events::down_event(k) => {
      match &app.devices {
        Some(p) => {
          if let Some(selected_device_index) = app.selected_device_index {
            let next_index =
              common_key_events::on_down_press_handler(p, Some(selected_device_index));
            app.selected_device_index = Some(next_index);
          }
        }
        None => {}
      };
    }
    k if common_key_events::up_event(k) => {
      match &app.devices {
        Some(p) => {
          if let Some(selected_device_index) = app.selected_device_index {
            let next_index = common_key_events::on_up_press_handler(p, Some(selected_device_index));
            app.selected_device_index = Some(next_index);
          }
        }
        None => {}
      };
    }
    k if common_key_events::high_event(k) => {
      match &app.devices {
        Some(_p) => {
          if let Some(_selected_device_index) = app.selected_device_index {
            let next_index = common_key_events::on_high_press_handler();
            app.selected_device_index = Some(next_index);
          }
        }
        None => {}
      };
    }
    k if common_key_events::middle_event(k) => {
      match &app.devices {
        Some(p) => {
          if let Some(_selected_device_index) = app.selected_device_index {
            let next_index = common_key_events::on_middle_press_handler(p);
            app.selected_device_index = Some(next_index);
          }
        }
        None => {}
      };
    }
    k if common_key_events::low_event(k) => {
      match &app.devices {
        Some(p) => {
          if let Some(_selected_device_index) = app.selected_device_index {
            let next_index = common_key_events::on_low_press_handler(p);
            app.selected_device_index = Some(next_index);
          }
        }
        None => {}
      };
    }
    Key::Enter => {
      if let (Some(devices), Some(index)) = (app.devices.clone(), app.selected_device_index) {
        if let Some(device) = devices.get(index) {
          // Clicking the device we are already on must not re-fire the
          // transfer: with play=true it restarts playback on the same
          // device. Only an actual change dispatches. Both ids must be
          // known and equal to skip; otherwise the dialog stays open and
          // nothing is sent.
          let already_here = match (
            device.id.as_deref(),
            app
              .current_playback_context
              .as_ref()
              .and_then(|c| c.device.id.as_deref()),
          ) {
            (Some(clicked), Some(current)) => clicked == current,
            _ => false,
          };
          if already_here {
            // No request, but still leave the screen — otherwise a
            // single-device setup has no way out (Enter used to exit via
            // the transfer-success pop).
            app.pop_navigation_stack();
          } else {
            app.dispatch(IoEvent::TransferPlaybackToDevice(
              device.id.clone().unwrap_or_default(),
            ));
          }
        }
      };
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::RouteId;
  use serde_json::json;

  fn mock_device(id: &str) -> rspotify::model::Device {
    serde_json::from_value(json!({
      "id": id,
      "is_active": false,
      "is_private_session": false,
      "is_restricted": false,
      "name": id,
      "type": "Computer",
      "volume_percent": 50,
    }))
    .unwrap()
  }

  fn app_with_playback_on(device_id: Option<&str>) -> App {
    let mut app = App::default();
    app.devices = Some(vec![mock_device("d-active"), mock_device("d-other")]);
    if let Some(id) = device_id {
      let playback: rspotify::model::CurrentPlaybackContext = serde_json::from_value(json!({
        "device": {
          "id": id,
          "is_active": true,
          "is_private_session": false,
          "is_restricted": false,
          "name": "Mock Device",
          "type": "Computer",
          "volume_percent": 50,
        },
        "repeat_state": "off",
        "shuffle_state": false,
        "context": null,
        "timestamp": 0,
        "is_playing": true,
        "item": null,
        "currently_playing_type": "track",
        "actions": { "disallows": {} },
      }))
      .unwrap();
      app.current_playback_context = Some(playback);
    }
    let (tx, _rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    app
  }

  #[test]
  fn enter_on_the_current_device_dispatches_nothing() {
    let mut app = app_with_playback_on(Some("d-active"));
    app.selected_device_index = Some(0);
    app.push_navigation_stack(RouteId::SelectedDevice, ActiveBlock::SelectDevice);
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    handler(Key::Enter, &mut app);

    assert!(rx.try_iter().collect::<Vec<_>>().is_empty());
    // …but still leaves the screen: a single-device setup must have a way out.
    assert_ne!(
      app.get_current_route().id,
      RouteId::SelectedDevice
    );
  }

  #[test]
  fn enter_on_a_different_device_dispatches_transfer() {
    let mut app = app_with_playback_on(Some("d-active"));
    app.selected_device_index = Some(1);
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    handler(Key::Enter, &mut app);

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::TransferPlaybackToDevice("d-other".to_string())]
    );
  }

  #[test]
  fn enter_without_playback_context_dispatches() {
    // No live device id to compare against → cannot prove "same", so send.
    let mut app = app_with_playback_on(None);
    app.selected_device_index = Some(0);
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    handler(Key::Enter, &mut app);

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::TransferPlaybackToDevice("d-active".to_string())]
    );
  }
}
