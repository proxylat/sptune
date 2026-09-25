use super::super::app::{ActiveBlock, App, DialogContext};
use crate::event::Key;
use rspotify::model::PlayableItem;

pub fn handler(key: Key, app: &mut App) {
  let context = match app.get_current_route().active_block {
    ActiveBlock::Dialog(context) => context,
    _ => return,
  };

  if context == DialogContext::SeekTime {
    handle_seek_dialog(key, app);
    return;
  }

  if context == DialogContext::AddToPlaylist {
    handle_add_to_playlist_dialog(key, app);
    return;
  }

  if context == DialogContext::ConfirmRemoveFromPlaylist {
    handle_confirm_remove_dialog(key, app);
    return;
  }

  match key {
    Key::Enter => {
      if let Some(route) = app.pop_navigation_stack() {
        if app.confirm {
          if let ActiveBlock::Dialog(d) = route.active_block {
            match d {
              DialogContext::PlaylistWindow => handle_playlist_dialog(app),
              DialogContext::PlaylistSearch => handle_playlist_search_dialog(app),
              DialogContext::SeekTime => {}
              DialogContext::AddToPlaylist => {}
              DialogContext::ConfirmRemoveFromPlaylist => {}
            }
          }
        }
      }
    }
    Key::Char('q') => {
      app.pop_navigation_stack();
    }
    Key::Right => app.confirm = !app.confirm,
    Key::Left => app.confirm = !app.confirm,
    _ => {}
  }
}

// Digits are right-aligned: the last two are seconds, everything before them
// minutes. "5" -> 0:05, "60" -> 1:00, "200" -> 2:00, "1205" -> 12:05.
fn digits_to_millis(digits: &str) -> Option<u32> {
  if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
    return None;
  }
  let secs_end = digits.len();
  let secs_start = secs_end.saturating_sub(2);
  let secs: u32 = digits[secs_start..secs_end].parse().ok()?;
  if secs > 59 {
    return None;
  }
  let mins: u32 = if secs_start > 0 {
    digits[..secs_start].parse().ok()?
  } else {
    0
  };
  Some((mins * 60 + secs) * 1000)
}

fn millis_to_digits(ms: u32) -> String {
  let total_secs = ms / 1000;
  let (mins, secs) = (total_secs / 60, total_secs % 60);
  if mins == 0 {
    secs.to_string()
  } else {
    format!("{}{:02}", mins, secs)
  }
}

fn track_duration_ms(app: &App) -> Option<u32> {
  app
    .current_playback_context
    .as_ref()?
    .item
    .as_ref()
    .map(|item| match item {
      PlayableItem::Track(track) => track.duration.num_milliseconds() as u32,
      PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u32,
      _ => 0,
    })
}

fn handle_seek_dialog(key: Key, app: &mut App) {
  match key {
    Key::Enter => {
      let Some(duration_ms) = track_duration_ms(app) else {
        app.pop_navigation_stack();
        return;
      };
      let digits = app.dialog.as_deref().unwrap_or_default().to_string();
      app.pop_navigation_stack();
      // Overshoot is rejected: the dialog just closes, playback continues
      // where it was.
      if let Some(ms) = digits_to_millis(&digits) {
        if ms <= duration_ms {
          app.seek_to(ms);
        }
      }
    }
    Key::Backspace => {
      if let Some(digits) = app.dialog.as_mut() {
        digits.pop();
        if digits.is_empty() {
          app.pop_navigation_stack();
        }
      }
    }
    Key::Right | Key::Up => seek_dialog_delta(app, 10_000),
    Key::Left | Key::Down => seek_dialog_delta(app, -10_000),
    Key::Char('q') => {
      app.pop_navigation_stack();
    }
    _ => {}
  }
}

// Arrow keys step the typed value by 10s, clamped to 0..track length. The
// typed digits are re-rendered to reflect the stepped value.
fn seek_dialog_delta(app: &mut App, delta: i64) {
  let Some(duration_ms) = track_duration_ms(app) else {
    return;
  };
  let current_ms = app
    .dialog
    .as_deref()
    .and_then(digits_to_millis)
    .unwrap_or_else(|| app.seek_ms.unwrap_or(0) as u32);
  let next_ms = (current_ms as i64 + delta).clamp(0, duration_ms as i64) as u32;
  app.dialog = Some(millis_to_digits(next_ms));
}

fn handle_playlist_dialog(app: &mut App) {
  app.user_unfollow_playlist()
}

// Pick a playlist for the captured track; Enter adds it, arrows move the
// selection, q/Esc closes without adding. The last row is "+ New playlist":
// Enter there flips into name-entry mode (typed chars build the name,
// Enter creates it private and adds the track, Esc back to the list).
fn handle_add_to_playlist_dialog(key: Key, app: &mut App) {
  if app.naming_playlist {
    match key {
      Key::Enter => {
        let name = app.new_playlist_name.trim().to_string();
        if name.is_empty() {
          return;
        }
        let Some(uri) = app.pending_track_uri.take() else {
          app.pop_navigation_stack();
          return;
        };
        app.naming_playlist = false;
        app.new_playlist_name.clear();
        app.pop_navigation_stack();
        app.dispatch(crate::backend::IoEvent::CreatePlaylistThenAdd(name, uri));
      }
      Key::Esc => {
        app.naming_playlist = false;
        app.new_playlist_name.clear();
      }
      Key::Backspace => {
        app.new_playlist_name.pop();
      }
      Key::Char(c) => {
        if app.new_playlist_name.len() < 100 {
          app.new_playlist_name.push(c);
        }
      }
      _ => {}
    }
    return;
  }
  let items = app.playlists.as_ref().map_or(0, |p| p.items.len());
  let count = items + 1; // trailing "+ New playlist" row
  match key {
    Key::Down | Key::Char('j') => {
      app.playlist_picker_index = (app.playlist_picker_index + 1) % count;
    }
    Key::Up | Key::Char('k') => {
      app.playlist_picker_index = (app.playlist_picker_index + count - 1) % count;
    }
    Key::Enter => {
      if app.playlist_picker_index == items {
        app.naming_playlist = true;
        app.new_playlist_name.clear();
        return;
      }
      let Some(uri) = app.pending_track_uri.take() else {
        app.pop_navigation_stack();
        return;
      };
      let playlist_id = app
        .playlists
        .as_ref()
        .and_then(|p| p.items.get(app.playlist_picker_index))
        .map(|playlist| playlist.id.to_string());
      app.pop_navigation_stack();
      if let Some(playlist_id) = playlist_id {
        app.dispatch(crate::backend::IoEvent::AddTrackToPlaylist(
          uri,
          playlist_id,
        ));
      }
    }
    Key::Char('q') | Key::Esc => {
      app.pop_navigation_stack();
    }
    _ => {}
  }
}

fn handle_playlist_search_dialog(app: &mut App) {
  app.user_unfollow_playlist_search_result()
}

fn handle_confirm_remove_dialog(key: Key, app: &mut App) {
  match key {
    Key::Left | Key::Right => app.confirm = !app.confirm,
    Key::Char('q') | Key::Esc => {
      app.pending_remove_track_uri = None;
      app.pending_remove_playlist_id = None;
      app.pending_remove_track_name = None;
      app.pending_remove_playlist_name = None;
      app.pop_navigation_stack();
    }
    Key::Enter => {
      let yes = app.confirm;
      app.pop_navigation_stack();
      if yes {
        app.confirm_pending_remove();
      } else {
        app.pending_remove_track_uri = None;
        app.pending_remove_playlist_id = None;
        app.pending_remove_track_name = None;
        app.pending_remove_playlist_name = None;
      }
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::RouteId;
  use crate::backend::IoEvent;
  use serde_json::json;

  fn picker_app() -> App {
    let mut app = App::default();
    app.playlists = Some(rspotify::model::Page {
      href: String::new(),
      items: (0..2)
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
            "items": { "href": "", "total": 10 },
          }))
          .unwrap()
        })
        .collect(),
      limit: 2,
      next: None,
      offset: 0,
      previous: None,
      total: 2,
    });
    app.pending_track_uri = Some("spotify:track:mocktrack0".to_string());
    app.playlist_picker_index = 0;
    app.push_navigation_stack(
      RouteId::Dialog,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist),
    );
    let (tx, _rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    app
  }

  #[test]
  fn enter_on_new_row_starts_naming() {
    let mut app = picker_app();
    app.playlist_picker_index = 2; // trailing "+ New playlist" row
    handler(Key::Enter, &mut app);
    assert!(app.naming_playlist);
    // Dialog stays open, track still pending.
    assert!(matches!(
      app.get_current_route().active_block,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist)
    ));
    assert!(app.pending_track_uri.is_some());
  }

  #[test]
  fn naming_enter_dispatches_create_then_add() {
    let mut app = picker_app();
    app.naming_playlist = true;
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    for c in "Gym".chars() {
      handler(Key::Char(c), &mut app);
    }
    handler(Key::Enter, &mut app);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::CreatePlaylistThenAdd(
        "Gym".to_string(),
        "spotify:track:mocktrack0".to_string(),
      )]
    );
    assert!(!matches!(
      app.get_current_route().active_block,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist)
    ));
  }

  #[test]
  fn naming_empty_enter_stays_and_esc_cancels() {
    let mut app = picker_app();
    app.naming_playlist = true;
    handler(Key::Enter, &mut app); // empty name: no-op
    assert!(app.naming_playlist);
    handler(Key::Char('x'), &mut app);
    handler(Key::Backspace, &mut app);
    assert!(app.new_playlist_name.is_empty());
    handler(Key::Esc, &mut app);
    assert!(!app.naming_playlist);
    assert!(matches!(
      app.get_current_route().active_block,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist)
    ));
  }

  #[test]
  fn picker_nav_wraps_over_new_row() {
    let mut app = picker_app();
    handler(Key::Up, &mut app); // 0 -> 2 (New row)
    assert_eq!(app.playlist_picker_index, 2);
    handler(Key::Down, &mut app); // 2 -> 0
    assert_eq!(app.playlist_picker_index, 0);
  }

  // Typing the name must go through the real entry point: handle_app runs
  // global shortcuts (space, d, digits...) before block handlers, so these
  // tests fail if the naming capture in handle_app regresses.
  #[test]
  fn naming_typing_through_handle_app_fires_no_shortcuts() {
    let mut app = picker_app();
    app.naming_playlist = true;
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    // 'd' opens devices, space toggles playback — both must land in the name.
    for c in "a d".chars() {
      crate::handlers::handle_app(Key::Char(c), &mut app);
    }
    assert_eq!(app.new_playlist_name, "a d");
    assert!(rx.try_iter().collect::<Vec<_>>().is_empty());
  }

  #[test]
  fn naming_esc_through_handle_app_cancels_without_closing() {
    let mut app = picker_app();
    app.naming_playlist = true;
    crate::handlers::handle_app(Key::Esc, &mut app);
    assert!(!app.naming_playlist);
    // Dialog stays open (back to the list), not popped.
    assert!(matches!(
      app.get_current_route().active_block,
      ActiveBlock::Dialog(DialogContext::AddToPlaylist)
    ));
  }
}
