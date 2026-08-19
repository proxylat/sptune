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
// selection, q/Esc closes without adding.
fn handle_add_to_playlist_dialog(key: Key, app: &mut App) {
  let count = app.playlists.as_ref().map_or(0, |p| p.items.len());
  match key {
    Key::Down | Key::Char('j') => {
      if count > 0 {
        app.playlist_picker_index = (app.playlist_picker_index + 1) % count;
      }
    }
    Key::Up | Key::Char('k') => {
      if count > 0 {
        app.playlist_picker_index = (app.playlist_picker_index + count - 1) % count;
      }
    }
    Key::Enter => {
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
