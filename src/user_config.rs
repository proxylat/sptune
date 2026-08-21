use crate::event::Key;
use anyhow::{anyhow, Result};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::{
  fs,
  path::{Path, PathBuf},
};

const FILE_NAME: &str = "config.yml";
const CONFIG_DIR: &str = ".config";
const APP_CONFIG_DIR: &str = "sptune";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UserTheme {
  pub active: Option<String>,
  pub banner: Option<String>,
  pub error_border: Option<String>,
  pub error_text: Option<String>,
  pub hint: Option<String>,
  pub hovered: Option<String>,
  pub inactive: Option<String>,
  pub load_more: Option<String>,
  pub playbar_background: Option<String>,
  pub playbar_progress: Option<String>,
  pub playbar_progress_text: Option<String>,
  pub playbar_text: Option<String>,
  pub selected: Option<String>,
  pub text: Option<String>,
  pub header: Option<String>,
  pub background: Option<String>,
}

#[derive(Copy, Clone, Debug)]
pub struct Theme {
  pub active: Color,
  pub banner: Color,
  pub error_border: Color,
  pub error_text: Color,
  pub hint: Color,
  pub hovered: Color,
  pub inactive: Color,
  pub load_more: Color,
  pub playbar_background: Color,
  pub playbar_progress: Color,
  pub playbar_progress_text: Color,
  pub playbar_text: Color,
  pub selected: Color,
  pub text: Color,
  pub header: Color,
  pub background: Color,
}

impl Default for Theme {
  fn default() -> Self {
    Theme {
      active: Color::Cyan,
      banner: Color::LightCyan,
      error_border: Color::Red,
      error_text: Color::LightRed,
      hint: Color::Yellow,
      hovered: Color::Magenta,
      inactive: Color::Gray,
      load_more: Color::Yellow,
      playbar_background: Color::Black,
      playbar_progress: Color::LightCyan,
      playbar_progress_text: Color::LightCyan,
      playbar_text: Color::Reset,
      selected: Color::LightCyan,
      text: Color::Reset,
      header: Color::Cyan,
      background: Color::Rgb(0, 0, 0),
    }
  }
}

/// Built-in palettes cycled from the gear settings menu. Only the visually
/// dominant slots are set; the rest keep the default theme.
pub fn theme_presets() -> [(&'static str, Theme); 2] {
  [
    (
      "Spotify",
      Theme {
        active: Color::Rgb(29, 185, 84),
        banner: Color::Rgb(30, 215, 96),
        hovered: Color::Rgb(30, 215, 96),
        load_more: Color::Rgb(155, 240, 180),
        playbar_progress: Color::Rgb(29, 185, 84),
        playbar_progress_text: Color::Rgb(29, 185, 84),
        selected: Color::White,
        header: Color::Rgb(29, 185, 84),
        hint: Color::Rgb(155, 240, 180),
        background: Color::Rgb(18, 18, 18),
        ..Default::default()
      },
    ),
    (
      "Dracula",
      Theme {
        active: Color::Rgb(189, 147, 249),
        banner: Color::Rgb(189, 147, 249),
        hovered: Color::Rgb(255, 121, 198),
        load_more: Color::Rgb(255, 184, 108),
        playbar_progress: Color::Rgb(80, 250, 123),
        playbar_progress_text: Color::Rgb(80, 250, 123),
        selected: Color::White,
        header: Color::Rgb(139, 233, 253),
        hint: Color::Rgb(255, 184, 108),
        background: Color::Rgb(40, 42, 54),
        ..Default::default()
      },
    ),
  ]
}

fn parse_key(key: String) -> Result<Key> {
  fn get_single_char(string: &str) -> char {
    match string.chars().next() {
      Some(c) => c,
      None => panic!(),
    }
  }

  match key.len() {
    1 => Ok(Key::Char(get_single_char(key.as_str()))),
    _ => {
      let sections: Vec<&str> = key.split('-').collect();

      if sections.len() > 2 {
        return Err(anyhow!(
          "Shortcut can only have 2 keys, \"{}\" has {}",
          key,
          sections.len()
        ));
      }

      match sections[0].to_lowercase().as_str() {
        "ctrl" => Ok(Key::Ctrl(get_single_char(sections[1]))),
        "alt" => Ok(Key::Alt(get_single_char(sections[1]))),
        "left" => Ok(Key::Left),
        "right" => Ok(Key::Right),
        "up" => Ok(Key::Up),
        "down" => Ok(Key::Down),
        "backspace" | "delete" => Ok(Key::Backspace),
        "del" => Ok(Key::Delete),
        "esc" | "escape" => Ok(Key::Esc),
        "pageup" => Ok(Key::PageUp),
        "pagedown" => Ok(Key::PageDown),
        "space" => Ok(Key::Char(' ')),
        "tab" => Ok(Key::Tab),
        _ => Err(anyhow!("The key \"{}\" is unknown.", sections[0])),
      }
    }
  }
}

fn check_reserved_keys(key: Key) -> Result<()> {
  let reserved = [
    Key::Char('h'),
    Key::Char('j'),
    Key::Char('k'),
    Key::Char('l'),
    Key::Char('H'),
    Key::Char('M'),
    Key::Char('L'),
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::Backspace,
    Key::Enter,
  ];
  for item in reserved.iter() {
    if key == *item {
      // TODO: Add pretty print for key
      return Err(anyhow!(
        "The key {:?} is reserved and cannot be remapped",
        key
      ));
    }
  }
  Ok(())
}

#[derive(Clone)]
pub struct UserConfigPaths {
  pub config_file_path: PathBuf,
}

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyBindingsString {
  back: Option<String>,
  next_page: Option<String>,
  previous_page: Option<String>,
  jump_to_start: Option<String>,
  jump_to_end: Option<String>,
  jump_to_album: Option<String>,
  jump_to_artist_album: Option<String>,
  jump_to_context: Option<String>,
  manage_devices: Option<String>,
  decrease_volume: Option<String>,
  increase_volume: Option<String>,
  toggle_playback: Option<String>,
  seek_backwards: Option<String>,
  seek_forwards: Option<String>,
  next_track: Option<String>,
  previous_track: Option<String>,
  help: Option<String>,
  shuffle: Option<String>,
  repeat: Option<String>,
  search: Option<String>,
  submit: Option<String>,
  copy_song_url: Option<String>,
  copy_album_url: Option<String>,
  copy_error: Option<String>,
  music_view: Option<String>,
  add_item_to_queue: Option<String>,
  add_to_playlist: Option<String>,
  refresh: Option<String>,
  clear_cache: Option<String>,
  search_in_playlist: Option<String>,
  remove_from_playlist: Option<String>,
  toggle_sidebar: Option<String>,
}

#[derive(Clone)]
pub struct KeyBindings {
  pub back: Key,
  pub next_page: Key,
  pub previous_page: Key,
  pub jump_to_start: Key,
  pub jump_to_end: Key,
  pub jump_to_album: Key,
  pub jump_to_artist_album: Key,
  pub jump_to_context: Key,
  pub manage_devices: Key,
  pub decrease_volume: Key,
  pub increase_volume: Key,
  pub toggle_playback: Key,
  pub seek_backwards: Key,
  pub seek_forwards: Key,
  pub next_track: Key,
  pub previous_track: Key,
  pub help: Key,
  pub shuffle: Key,
  pub repeat: Key,
  pub search: Key,
  pub submit: Key,
  pub copy_song_url: Key,
  pub copy_album_url: Key,
  pub copy_error: Key,
  pub music_view: Key,
  pub add_item_to_queue: Key,
  pub add_to_playlist: Key,
  pub refresh: Option<Key>,
  pub clear_cache: Option<Key>,
  pub search_in_playlist: Option<Key>,
  pub remove_from_playlist: Option<Key>,
  pub toggle_sidebar: Option<Key>,
}

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BehaviorConfigString {
  pub seek_milliseconds: Option<u32>,
  pub volume_increment: Option<u8>,
  pub tick_rate_milliseconds: Option<u64>,
  pub enable_text_emphasis: Option<bool>,
  pub volume_ramp_bar: Option<bool>,
  pub seek_by_typing: Option<bool>,
  pub resume_track: Option<bool>,
  pub restore_settings: Option<bool>,
  pub enforce_wide_search_bar: Option<bool>,
  pub liked_icon: Option<String>,
  pub shuffle_icon: Option<String>,
  pub repeat_track_icon: Option<String>,
  pub repeat_context_icon: Option<String>,
  pub playing_icon: Option<String>,
  pub paused_icon: Option<String>,
  pub set_window_title: Option<bool>,
  pub enable_mouse: Option<bool>,
  pub show_album_column: Option<bool>,
  pub show_artist_column: Option<bool>,
  pub show_length_column: Option<bool>,
  pub show_date_added_column: Option<bool>,
  pub enable_add_to_playlist: Option<bool>,
  pub in_playlist_icon: Option<String>,
  pub show_liked_icon: Option<bool>,
  pub enable_remove_from_playlist: Option<bool>,
  pub max_display_length: Option<u16>,
  pub enable_animations: Option<bool>,
}

#[derive(Clone)]
pub struct BehaviorConfig {
  pub seek_milliseconds: u32,
  pub volume_increment: u8,
  pub tick_rate_milliseconds: u64,
  pub enable_text_emphasis: bool,
  pub volume_ramp_bar: bool,
  pub seek_by_typing: bool,
  pub resume_track: bool,
  pub restore_settings: bool,
  pub enforce_wide_search_bar: bool,
  pub liked_icon: String,
  pub shuffle_icon: String,
  pub repeat_track_icon: String,
  pub repeat_context_icon: String,
  pub playing_icon: String,
  pub paused_icon: String,
  pub set_window_title: bool,
  pub enable_mouse: bool,
  pub show_album_column: bool,
  pub show_artist_column: bool,
  pub show_length_column: bool,
  pub show_date_added_column: bool,
  pub enable_add_to_playlist: bool,
  pub in_playlist_icon: String,
  pub show_liked_icon: bool,
  pub enable_remove_from_playlist: bool,
  /// Maximum characters for name columns (song, artist, album). 0 = no limit.
  pub max_display_length: u16,
  pub enable_animations: bool,
}

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserConfigString {
  keybindings: Option<KeyBindingsString>,
  behavior: Option<BehaviorConfigString>,
  theme: Option<UserTheme>,
}

#[derive(Clone)]
pub struct UserConfig {
  pub keys: KeyBindings,
  pub theme: Theme,
  pub behavior: BehaviorConfig,
  pub path_to_config: Option<UserConfigPaths>,
}

impl UserConfig {
  pub fn new() -> UserConfig {
    UserConfig {
      theme: Default::default(),
      keys: KeyBindings {
        back: Key::Char('q'),
        next_page: Key::Ctrl('d'),
        previous_page: Key::Ctrl('u'),
        jump_to_start: Key::Ctrl('a'),
        jump_to_end: Key::Ctrl('e'),
        jump_to_album: Key::Char('a'),
        jump_to_artist_album: Key::Char('A'),
        jump_to_context: Key::Char('o'),
        manage_devices: Key::Char('d'),
        decrease_volume: Key::Char('-'),
        increase_volume: Key::Char('+'),
        toggle_playback: Key::Char(' '),
        seek_backwards: Key::Char('<'),
        seek_forwards: Key::Char('>'),
        next_track: Key::Char('n'),
        previous_track: Key::Char('p'),
        help: Key::Char('?'),
        shuffle: Key::Ctrl('s'),
        repeat: Key::Ctrl('r'),
        search: Key::Char('/'),
        submit: Key::Enter,
        copy_song_url: Key::Char('c'),
        copy_album_url: Key::Char('C'),
        copy_error: Key::Char('y'),
        music_view: Key::Tab,
        add_item_to_queue: Key::Char('z'),
        add_to_playlist: Key::Char('a'),
        refresh: None,
        clear_cache: None,
        search_in_playlist: Some(Key::Char('f')),
        remove_from_playlist: None,
        toggle_sidebar: Some(Key::Char('\\')),
      },
      behavior: BehaviorConfig {
        seek_milliseconds: 1000,
        volume_increment: 10,
        tick_rate_milliseconds: 250,
        enable_text_emphasis: true,
        volume_ramp_bar: false,
        seek_by_typing: false,
        resume_track: false,
        restore_settings: false,
        enforce_wide_search_bar: false,
        liked_icon: "♥".to_string(),
        shuffle_icon: "🔀".to_string(),
        repeat_track_icon: "🔂".to_string(),
        repeat_context_icon: "🔁".to_string(),
        playing_icon: "▶".to_string(),
        paused_icon: "⏸".to_string(),
        set_window_title: true,
        enable_mouse: true,
        show_album_column: true,
        show_artist_column: true,
        show_length_column: true,
        show_date_added_column: true,
        enable_add_to_playlist: true,
        in_playlist_icon: "✓".to_string(),
        show_liked_icon: true,
        enable_remove_from_playlist: false,
        max_display_length: 0,
        enable_animations: true,
      },
      path_to_config: None,
    }
  }

  pub fn get_or_build_paths(&mut self) -> Result<()> {
    match dirs::home_dir() {
      Some(home) => {
        let path = Path::new(&home);
        let home_config_dir = path.join(CONFIG_DIR);
        let app_config_dir = home_config_dir.join(APP_CONFIG_DIR);

        if !home_config_dir.exists() {
          fs::create_dir(&home_config_dir)?;
        }

        if !app_config_dir.exists() {
          fs::create_dir(&app_config_dir)?;
        }

        let config_file_path = &app_config_dir.join(FILE_NAME);

        let paths = UserConfigPaths {
          config_file_path: config_file_path.to_path_buf(),
        };
        self.path_to_config = Some(paths);
        Ok(())
      }
      None => Err(anyhow!("No $HOME directory found for client config")),
    }
  }

  pub fn load_keybindings(&mut self, keybindings: KeyBindingsString) -> Result<()> {
    macro_rules! to_keys {
      ($name: ident) => {
        if let Some(key_string) = keybindings.$name {
          self.keys.$name = parse_key(key_string)?;
          check_reserved_keys(self.keys.$name)?;
        }
      };
    }

    to_keys!(back);
    to_keys!(next_page);
    to_keys!(previous_page);
    to_keys!(jump_to_start);
    to_keys!(jump_to_end);
    to_keys!(jump_to_album);
    to_keys!(jump_to_artist_album);
    to_keys!(jump_to_context);
    to_keys!(manage_devices);
    to_keys!(decrease_volume);
    to_keys!(increase_volume);
    to_keys!(toggle_playback);
    to_keys!(seek_backwards);
    to_keys!(seek_forwards);
    to_keys!(next_track);
    to_keys!(previous_track);
    to_keys!(help);
    to_keys!(shuffle);
    to_keys!(repeat);
    to_keys!(search);
    to_keys!(submit);
    to_keys!(copy_song_url);
    to_keys!(copy_album_url);
    to_keys!(copy_error);
    to_keys!(music_view);
    to_keys!(add_item_to_queue);
    to_keys!(add_to_playlist);

    macro_rules! to_optional_keys {
      ($name: ident) => {
        if let Some(key_string) = keybindings.$name {
          let key = parse_key(key_string)?;
          check_reserved_keys(key)?;
          self.keys.$name = Some(key);
        }
      };
    }

    to_optional_keys!(refresh);
    to_optional_keys!(clear_cache);
    to_optional_keys!(search_in_playlist);
    to_optional_keys!(remove_from_playlist);
    to_optional_keys!(toggle_sidebar);

    Ok(())
  }

  pub fn load_theme(&mut self, theme: UserTheme) -> Result<()> {
    macro_rules! to_theme_item {
      ($name: ident) => {
        if let Some(theme_item) = theme.$name {
          self.theme.$name = parse_theme_item(&theme_item)?;
        }
      };
    }

    to_theme_item!(active);
    to_theme_item!(banner);
    to_theme_item!(error_border);
    to_theme_item!(error_text);
    to_theme_item!(hint);
    to_theme_item!(hovered);
    to_theme_item!(inactive);
    to_theme_item!(load_more);
    to_theme_item!(playbar_background);
    to_theme_item!(playbar_progress);
    to_theme_item!(playbar_progress_text);
    to_theme_item!(playbar_text);
    to_theme_item!(selected);
    to_theme_item!(text);
    to_theme_item!(header);
    to_theme_item!(background);
    Ok(())
  }

  pub fn load_behaviorconfig(&mut self, behavior_config: BehaviorConfigString) -> Result<()> {
    if let Some(behavior_string) = behavior_config.seek_milliseconds {
      self.behavior.seek_milliseconds = behavior_string;
    }

    if let Some(behavior_string) = behavior_config.volume_increment {
      if behavior_string > 100 {
        return Err(anyhow!(
          "Volume increment must be between 0 and 100, is {}",
          behavior_string,
        ));
      }
      self.behavior.volume_increment = behavior_string;
    }

    if let Some(tick_rate) = behavior_config.tick_rate_milliseconds {
      if tick_rate >= 1000 {
        return Err(anyhow!("Tick rate must be below 1000"));
      } else {
        self.behavior.tick_rate_milliseconds = tick_rate;
      }
    }

    if let Some(text_emphasis) = behavior_config.enable_text_emphasis {
      self.behavior.enable_text_emphasis = text_emphasis;
    }

    if let Some(volume_ramp) = behavior_config.volume_ramp_bar {
      self.behavior.volume_ramp_bar = volume_ramp;
    }

    if let Some(seek_by_typing) = behavior_config.seek_by_typing {
      self.behavior.seek_by_typing = seek_by_typing;
    }

    if let Some(resume_track) = behavior_config.resume_track {
      self.behavior.resume_track = resume_track;
    }

    if let Some(restore_settings) = behavior_config.restore_settings {
      self.behavior.restore_settings = restore_settings;
    }

    if let Some(wide_search_bar) = behavior_config.enforce_wide_search_bar {
      self.behavior.enforce_wide_search_bar = wide_search_bar;
    }

    if let Some(liked_icon) = behavior_config.liked_icon {
      self.behavior.liked_icon = liked_icon;
    }

    if let Some(in_playlist_icon) = behavior_config.in_playlist_icon {
      self.behavior.in_playlist_icon = in_playlist_icon;
    }

    if let Some(enable_add_to_playlist) = behavior_config.enable_add_to_playlist {
      self.behavior.enable_add_to_playlist = enable_add_to_playlist;
    }

    if let Some(show_liked_icon) = behavior_config.show_liked_icon {
      self.behavior.show_liked_icon = show_liked_icon;
    }

    if let Some(enable_remove_from_playlist) = behavior_config.enable_remove_from_playlist {
      self.behavior.enable_remove_from_playlist = enable_remove_from_playlist;
    }

    if let Some(max_display_length) = behavior_config.max_display_length {
      self.behavior.max_display_length = max_display_length;
    }

    if let Some(paused_icon) = behavior_config.paused_icon {
      self.behavior.paused_icon = paused_icon;
    }

    if let Some(playing_icon) = behavior_config.playing_icon {
      self.behavior.playing_icon = playing_icon;
    }

    if let Some(shuffle_icon) = behavior_config.shuffle_icon {
      self.behavior.shuffle_icon = shuffle_icon;
    }

    if let Some(repeat_track_icon) = behavior_config.repeat_track_icon {
      self.behavior.repeat_track_icon = repeat_track_icon;
    }

    if let Some(repeat_context_icon) = behavior_config.repeat_context_icon {
      self.behavior.repeat_context_icon = repeat_context_icon;
    }

    if let Some(set_window_title) = behavior_config.set_window_title {
      self.behavior.set_window_title = set_window_title;
    }

    if let Some(enable_mouse) = behavior_config.enable_mouse {
      self.behavior.enable_mouse = enable_mouse;
    }
    if let Some(show_album) = behavior_config.show_album_column {
      self.behavior.show_album_column = show_album;
    }
    if let Some(show_artist) = behavior_config.show_artist_column {
      self.behavior.show_artist_column = show_artist;
    }
    if let Some(show_length) = behavior_config.show_length_column {
      self.behavior.show_length_column = show_length;
    }
    if let Some(show_date) = behavior_config.show_date_added_column {
      self.behavior.show_date_added_column = show_date;
    }
    if let Some(enable_animations) = behavior_config.enable_animations {
      self.behavior.enable_animations = enable_animations;
    }

    Ok(())
  }

  pub fn load_config(&mut self) -> Result<()> {
    let paths = match &self.path_to_config {
      Some(path) => path,
      None => {
        self.get_or_build_paths()?;
        self.path_to_config.as_ref().unwrap()
      }
    };
    if paths.config_file_path.exists() {
      let config_string = fs::read_to_string(&paths.config_file_path)?;
      // serde fails if file is empty
      if config_string.trim().is_empty() {
        return Ok(());
      }

      let config_yml: UserConfigString = serde_yml::from_str(&config_string)?;

      if let Some(keybindings) = config_yml.keybindings.clone() {
        self.load_keybindings(keybindings)?;
      }

      if let Some(behavior) = config_yml.behavior {
        self.load_behaviorconfig(behavior)?;
      }
      if let Some(theme) = config_yml.theme {
        self.load_theme(theme)?;
      }

      Ok(())
    } else {
      Ok(())
    }
  }

  pub fn padded_liked_icon(&self) -> String {
    format!("{} ", &self.behavior.liked_icon)
  }

  pub fn padded_in_playlist_icon(&self) -> String {
    format!("{} ", &self.behavior.in_playlist_icon)
  }
}

fn parse_theme_item(theme_item: &str) -> Result<Color> {
  let color = match theme_item {
    "Reset" => Color::Reset,
    "Black" => Color::Black,
    "Red" => Color::Red,
    "Green" => Color::Green,
    "Yellow" => Color::Yellow,
    "Blue" => Color::Blue,
    "Magenta" => Color::Magenta,
    "Cyan" => Color::Cyan,
    "Gray" => Color::Gray,
    "DarkGray" => Color::DarkGray,
    "LightRed" => Color::LightRed,
    "LightGreen" => Color::LightGreen,
    "LightYellow" => Color::LightYellow,
    "LightBlue" => Color::LightBlue,
    "LightMagenta" => Color::LightMagenta,
    "LightCyan" => Color::LightCyan,
    "White" => Color::White,
    _ => {
      let colors = theme_item.split(',').collect::<Vec<&str>>();
      if let (Some(r), Some(g), Some(b)) = (colors.get(0), colors.get(1), colors.get(2)) {
        Color::Rgb(
          r.trim().parse::<u8>()?,
          g.trim().parse::<u8>()?,
          b.trim().parse::<u8>()?,
        )
      } else {
        println!("Unexpected color {}", theme_item);
        Color::Black
      }
    }
  };

  Ok(color)
}

#[cfg(test)]
mod tests {
  use super::theme_presets;

  #[test]
  fn preset_selected_differs_from_active() {
    // The selected row is rendered with theme.selected + BOLD and the playing
    // row with theme.active + BOLD; equal colors would make row 0 look like it
    // is playing even when nothing is.
    for (_, theme) in theme_presets() {
      assert_ne!(theme.selected, theme.active);
    }
  }

  #[test]
  fn test_parse_key() {
    use super::parse_key;
    use crate::event::Key;
    assert_eq!(parse_key(String::from("j")).unwrap(), Key::Char('j'));
    assert_eq!(parse_key(String::from("J")).unwrap(), Key::Char('J'));
    assert_eq!(parse_key(String::from("ctrl-j")).unwrap(), Key::Ctrl('j'));
    assert_eq!(parse_key(String::from("ctrl-J")).unwrap(), Key::Ctrl('J'));
    assert_eq!(parse_key(String::from("-")).unwrap(), Key::Char('-'));
    assert_eq!(parse_key(String::from("esc")).unwrap(), Key::Esc);
    assert_eq!(parse_key(String::from("del")).unwrap(), Key::Delete);
  }

  #[test]
  fn parse_theme_item_test() {
    use super::parse_theme_item;
    use ratatui::style::Color;
    assert_eq!(parse_theme_item("Reset").unwrap(), Color::Reset);
    assert_eq!(parse_theme_item("Black").unwrap(), Color::Black);
    assert_eq!(parse_theme_item("Red").unwrap(), Color::Red);
    assert_eq!(parse_theme_item("Green").unwrap(), Color::Green);
    assert_eq!(parse_theme_item("Yellow").unwrap(), Color::Yellow);
    assert_eq!(parse_theme_item("Blue").unwrap(), Color::Blue);
    assert_eq!(parse_theme_item("Magenta").unwrap(), Color::Magenta);
    assert_eq!(parse_theme_item("Cyan").unwrap(), Color::Cyan);
    assert_eq!(parse_theme_item("Gray").unwrap(), Color::Gray);
    assert_eq!(parse_theme_item("DarkGray").unwrap(), Color::DarkGray);
    assert_eq!(parse_theme_item("LightRed").unwrap(), Color::LightRed);
    assert_eq!(parse_theme_item("LightGreen").unwrap(), Color::LightGreen);
    assert_eq!(parse_theme_item("LightYellow").unwrap(), Color::LightYellow);
    assert_eq!(parse_theme_item("LightBlue").unwrap(), Color::LightBlue);
    assert_eq!(
      parse_theme_item("LightMagenta").unwrap(),
      Color::LightMagenta
    );
    assert_eq!(parse_theme_item("LightCyan").unwrap(), Color::LightCyan);
    assert_eq!(parse_theme_item("White").unwrap(), Color::White);
    assert_eq!(
      parse_theme_item("23, 43, 45").unwrap(),
      Color::Rgb(23, 43, 45)
    );
  }

  #[test]
  fn test_reserved_key() {
    use super::check_reserved_keys;
    use crate::event::Key;

    assert!(
      check_reserved_keys(Key::Enter).is_err(),
      "Enter key should be reserved"
    );
  }
}
