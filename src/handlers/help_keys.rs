use super::common_key_events;
use crate::{app::App, event::Key};

#[derive(PartialEq)]
enum Direction {
  Up,
  Down,
}

pub fn handler(key: Key, app: &mut App) {
  match key {
    Key::Enter => {
      if !app.help_show_shortcuts {
        app.help_show_shortcuts = true;
        app.help_scroll_offset = 0;
        app.help_menu_page = 0;
        // viewport for fullscreen shortcuts page
        let vh = app.size.height.saturating_sub(4).saturating_sub(3) as u32;
        app.help_menu_max_lines = vh.max(1);
        app.calculate_help_menu_offset();
      }
    }
    k if common_key_events::down_event(k) => {
      if app.help_show_shortcuts {
        move_page(Direction::Down, app);
      } else {
        scroll_settings(Direction::Down, app);
      }
    }
    k if common_key_events::up_event(k) => {
      if app.help_show_shortcuts {
        move_page(Direction::Up, app);
      } else {
        scroll_settings(Direction::Up, app);
      }
    }
    Key::Ctrl('d') => {
      if app.help_show_shortcuts {
        move_page(Direction::Down, app);
      } else {
        scroll_settings(Direction::Down, app);
      }
    }
    Key::Ctrl('u') => {
      if app.help_show_shortcuts {
        move_page(Direction::Up, app);
      } else {
        scroll_settings(Direction::Up, app);
      }
    }
    Key::Char('T') => app.toggle_setting(0),
    Key::Char('0') => app.toggle_setting(1),
    Key::Char('m') => app.toggle_setting(4),
    Key::Char('P') => app.toggle_setting(5),
    k if Some(k) == app.user_config.keys.clear_cache => app.toggle_setting(19),
    _ => {}
  };
}

fn move_page(direction: Direction, app: &mut App) {
  if direction == Direction::Up {
    if app.help_menu_page > 0 {
      app.help_menu_page -= 1;
    }
  } else if direction == Direction::Down {
    app.help_menu_page += 1;
  }
  app.calculate_help_menu_offset();
}

fn scroll_settings(dir: Direction, app: &mut App) {
  let viewport = app.size.height.saturating_sub(4).saturating_sub(2) as usize;
  let count = crate::tui::layout::SETTINGS_ROW_COUNT as usize;
  let max = count.saturating_sub(viewport.min(count));
  let offset = app.help_scroll_offset as usize;
  app.help_scroll_offset = match dir {
    Direction::Up => offset.saturating_sub(1) as u32,
    Direction::Down => offset.saturating_add(1).min(max) as u32,
  };
}
