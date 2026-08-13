use super::common_key_events;
use crate::{app::App, event::Key};

#[derive(PartialEq)]
enum Direction {
  Up,
  Down,
}

pub fn handler(key: Key, app: &mut App) {
  match key {
    k if common_key_events::down_event(k) => {
      move_page(Direction::Down, app);
    }
    k if common_key_events::up_event(k) => {
      move_page(Direction::Up, app);
    }
    Key::Ctrl('d') => {
      move_page(Direction::Down, app);
    }
    Key::Ctrl('u') => {
      move_page(Direction::Up, app);
    }
    Key::Char('T') => app.toggle_setting(0),
    Key::Char('0') => app.toggle_setting(1),
    Key::Char('m') => app.toggle_setting(4),
    Key::Char('P') => app.toggle_setting(5),
    Key::Char('c') => app.toggle_setting(10),
    Key::Char('V') => app.toggle_setting(15),
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
