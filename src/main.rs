mod app;
mod backend;
mod cli;
mod client_creds;
mod event;
mod handlers;
mod lcg;
mod library_cache;
mod playlist_cache;
mod redirect_uri;
mod tui;
mod user_config;

use crate::app::RouteId;
use crate::event::Key;
use anyhow::{anyhow, Result};
use app::{ActiveBlock, App};
use backend::{get_spotify, IoEvent, Network, SavedState};
use clap::{Arg, Command};
use clap_complete::{generate, Shell};
use client_creds::ClientConfig;
use crossterm::{
  cursor::MoveTo,
  event::{DisableMouseCapture, EnableMouseCapture},
  execute,
  terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
  },
};
use ratatui::{
  backend::{Backend, CrosstermBackend},
  layout::Rect,
  style::{Color, Style},
  widgets::Block,
  Terminal,
};
use redirect_uri::redirect_uri_web_server;
use rspotify::{
  clients::{BaseClient, OAuthClient},
  model::Token,
  AuthCodeSpotify, Credentials, OAuth,
};
use std::backtrace::Backtrace;
use std::{
  io::{self, stdout},
  panic::{self, PanicHookInfo},
  path::PathBuf,
  sync::Arc,
  time::SystemTime,
};
use tokio::sync::Mutex;
use user_config::{theme_presets, UserConfig, UserConfigPaths};

const SCOPES: [&str; 14] = [
  "playlist-read-collaborative",
  "playlist-read-private",
  "playlist-modify-private",
  "playlist-modify-public",
  "user-follow-read",
  "user-follow-modify",
  "user-library-modify",
  "user-library-read",
  "user-modify-playback-state",
  "user-read-currently-playing",
  "user-read-playback-state",
  "user-read-playback-position",
  "user-read-private",
  "user-read-recently-played",
];

/// get token automatically with local webserver
pub async fn get_token_auto(spotify: &mut AuthCodeSpotify, port: u16) -> Option<Token> {
  if let Ok(Some(token)) = spotify.read_token_cache(false).await {
    return Some(token);
  }

  // ponytail: expired cached token is still usable — refresh_authentication
  // refetches via the stored refresh_token on the next event-loop tick.
  if let Ok(token) = Token::from_cache(spotify.get_config().cache_path.clone()) {
    return Some(token);
  }

  let auth_url = spotify.get_authorize_url(false).ok()?;
  println!("{}", auth_url);

  match redirect_uri_web_server(port) {
    Ok(path) => {
      let full_url = format!("http://127.0.0.1:{}{}", port, path);
      get_code_from_url(spotify, full_url).await
    }
    Err(()) => {
      println!("Starting webserver failed. Continuing with manual authentication");
      println!("Enter the URL you were redirected to: ");
      let mut input = String::new();
      match io::stdin().read_line(&mut input) {
        Ok(_) => get_code_from_url(spotify, input).await,
        Err(_) => None,
      }
    }
  }
}

async fn get_code_from_url(spotify: &mut AuthCodeSpotify, url: String) -> Option<Token> {
  let code = spotify.parse_response_code(url.as_str())?;
  spotify.request_token(code.as_str()).await.ok()?;
  spotify.get_token().lock().await.unwrap().clone()
}

fn close_application() -> Result<()> {
  disable_raw_mode()?;
  let mut stdout = io::stdout();
  execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
  Ok(())
}

fn panic_hook(info: &PanicHookInfo<'_>) {
  let location = info.location().map(|l| l.to_string()).unwrap_or_default();

  let msg = match info.payload().downcast_ref::<&'static str>() {
    Some(s) => *s,
    None => match info.payload().downcast_ref::<String>() {
      Some(s) => &s[..],
      None => "Box<Any>",
    },
  };

  eprintln!("thread '<unnamed>' panicked at '{}' ({})", msg, location);
  eprintln!("{:?}", Backtrace::capture());
}

#[tokio::main]
async fn main() -> Result<()> {
  panic::set_hook(Box::new(|info| {
    panic_hook(info);
  }));

  let mut clap_app = Command::new(env!("CARGO_PKG_NAME"))
    .version(env!("CARGO_PKG_VERSION"))
    .author(env!("CARGO_PKG_AUTHORS"))
    .about(env!("CARGO_PKG_DESCRIPTION"))
    .after_help(
      "Your spotify Client ID and Client Secret are stored in $HOME/.config/sptune/client.yml",
    )
    .arg(
      Arg::new("tick-rate")
        .short('t')
        .long("tick-rate")
        .help("Set the tick rate (milliseconds): the lower the number the higher the FPS.")
        .long_help(
          "Specify the tick rate in milliseconds: the lower the number the \
higher the FPS. It can be nicer to have a lower value when you want to use the audio analysis view \
of the app. Beware that this comes at a CPU cost!",
        )
        .value_parser(clap::value_parser!(u64)),
    )
    .arg(
      Arg::new("config")
        .short('c')
        .long("config")
        .help("Specify configuration file path.")
        .value_parser(clap::value_parser!(PathBuf)),
    )
    .arg(
      Arg::new("completions")
        .long("completions")
        .help("Generates completions for your preferred shell")
        .value_parser(["bash", "zsh", "fish", "power-shell", "elvish"])
        .value_name("SHELL"),
    )
    .arg(
      Arg::new("no-cache")
        .long("no-cache")
        .help("Run without reading or writing the on-disk caches")
        .action(clap::ArgAction::SetTrue),
    )
    // Control spotify from the command line
    .subcommand(cli::playback_subcommand())
    .subcommand(cli::play_subcommand())
    .subcommand(cli::list_subcommand())
    .subcommand(cli::search_subcommand())
    .subcommand(cli::clean_cache_subcommand());

  let matches = clap_app.clone().get_matches();

  if matches.get_flag("no-cache") {
    crate::library_cache::CACHE_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
  }

  // Shell completions don't need any spotify work
  if let Some(s) = matches.get_one::<String>("completions") {
    let shell = match s.as_str() {
      "fish" => Shell::Fish,
      "bash" => Shell::Bash,
      "zsh" => Shell::Zsh,
      "power-shell" => Shell::PowerShell,
      "elvish" => Shell::Elvish,
      _ => return Err(anyhow!("no completions avaible for '{}'", s)),
    };
    generate(shell, &mut clap_app, "sptune", &mut io::stdout());
    return Ok(());
  }

  // Cache cleaning needs no spotify work either
  if matches.subcommand_name() == Some("clean-cache") {
    crate::library_cache::LibraryCache::new().clear();
    crate::playlist_cache::PlaylistCache::new().clear();
    println!("Cache cleared.");
    return Ok(());
  }

  let mut user_config = UserConfig::new();
  if let Some(config_file_path) = matches.get_one::<PathBuf>("config") {
    let path = UserConfigPaths {
      config_file_path: config_file_path.clone(),
    };
    user_config.path_to_config.replace(path);
  }
  user_config.load_config()?;

  if let Some(tick_rate) = matches.get_one::<u64>("tick-rate") {
    if *tick_rate >= 1000 {
      panic!("Tick rate must be below 1000");
    } else {
      user_config.behavior.tick_rate_milliseconds = *tick_rate as u64;
    }
  }

  let mut client_config = ClientConfig::new();
  client_config.load_config()?;
  let config_paths = client_config.get_or_build_paths()?;

  // Start authorization with spotify
  let oauth = OAuth {
    redirect_uri: client_config.get_redirect_uri(),
    scopes: SCOPES.iter().map(|s| s.to_string()).collect(),
    ..Default::default()
  };
  let creds = Credentials::new(&client_config.client_id, &client_config.client_secret);
  let config = rspotify::Config {
    cache_path: config_paths.token_cache_path,
    token_cached: true,
    ..Default::default()
  };
  let mut spotify = AuthCodeSpotify::with_config(creds, oauth, config);

  let config_port = client_config.get_port();
  let (spotify, token_expiry) = match get_token_auto(&mut spotify, config_port).await {
    Some(token) => get_spotify(token, &client_config),
    None => {
      println!("\nSpotify auth failed");
      return Ok(());
    }
  };

  let (sync_io_tx, sync_io_rx) = std::sync::mpsc::channel::<IoEvent>();

  // Restore gear-menu settings (mouse interactions, theme preset, sidebar
  // visibility, black background, volume ramp) before the app and terminal are
  // built so both pick up the last session's choices.
  let mut restored_theme_index = None;
  let mut restored_show_library = None;
  let mut restored_show_playlists = None;
  let mut restored_sidebar_minimized = None;
  if let Some(saved) = SavedState::load() {
    if let Some(enabled) = saved.mouse_enabled {
      user_config.behavior.enable_mouse = enabled;
    }
    if let Some(name) = saved.theme_preset {
      if let Some(index) = theme_presets().iter().position(|(n, _)| *n == name) {
        user_config.theme = theme_presets()[index].1;
        restored_theme_index = Some(index);
      }
    }
    // Only the "black" override is stored explicitly; when the setting is off
    // the theme's own background (preset or config) is the intended value, so
    // nothing is forced here (Color::Reset would white-out light terminals).
    if saved.black_background == Some(true) {
      user_config.theme.background = Color::Rgb(0, 0, 0);
    }
    if let Some(seek_by_typing) = saved.seek_by_typing {
      user_config.behavior.seek_by_typing = seek_by_typing;
    }
    if let Some(show_album) = saved.show_album_column {
      user_config.behavior.show_album_column = show_album;
    }
    if let Some(show_artist) = saved.show_artist_column {
      user_config.behavior.show_artist_column = show_artist;
    }
    if let Some(show_length) = saved.show_length_column {
      user_config.behavior.show_length_column = show_length;
    }
    if let Some(show_date) = saved.show_date_added_column {
      user_config.behavior.show_date_added_column = show_date;
    }
    if let Some(resume_track) = saved.resume_track {
      user_config.behavior.resume_track = resume_track;
    }
    if let Some(restore_settings) = saved.restore_settings {
      user_config.behavior.restore_settings = restore_settings;
    }
    if let Some(ramp) = saved.volume_ramp_bar {
      user_config.behavior.volume_ramp_bar = ramp;
    }
    if let Some(enable) = saved.enable_add_to_playlist {
      user_config.behavior.enable_add_to_playlist = enable;
    }
    if let Some(show_liked_icon) = saved.show_liked_icon {
      user_config.behavior.show_liked_icon = show_liked_icon;
    }
    restored_show_library = saved.show_library;
    restored_show_playlists = saved.show_playlists;
    restored_sidebar_minimized = saved.sidebar_minimized;
  }

  // Initialise app state
  let app = Arc::new(Mutex::new(App::new(
    sync_io_tx.clone(),
    user_config.clone(),
    token_expiry,
  )));
  let mut app_guard = app.lock().await;
  app_guard.theme_preset_index = restored_theme_index;
  if let Some(show) = restored_show_library {
    app_guard.show_library = show;
  }
  if let Some(show) = restored_show_playlists {
    app_guard.show_playlists = show;
  }
  if let Some(minimized) = restored_sidebar_minimized {
    app_guard.sidebar_minimized = minimized;
  }
  if let Some(saved) = SavedState::load() {
    if let Some(custom) = saved.made_for_you_custom {
      app_guard.made_for_you_custom = custom;
    }
  }
  app_guard.clamp_library_selection();
  drop(app_guard);

  // Work with the cli (not really async)
  if let Some(cmd) = matches.subcommand_name() {
    // Save, because we checked if the subcommand is present at runtime
    let m = matches.subcommand_matches(cmd).unwrap();
    let network = Network::new(spotify, client_config, &app);
    println!(
      "{}",
      cli::handle_matches(m, cmd.to_string(), network, user_config).await?
    );
  // Launch the UI (async)
  } else {
    let cloned_app = Arc::clone(&app);
    std::thread::spawn(move || {
      let mut network = Network::new(spotify, client_config, &app);
      start_tokio(sync_io_rx, &mut network);
    });
    // Resume the volume and track from the last session
    if let Some(saved) = SavedState::load() {
      if user_config.behavior.restore_settings {
        if saved.shuffle.is_some()
          || saved.repeat.is_some()
          || saved.track_sort.is_some()
          || saved.last_page.is_some()
        {
          let _ = sync_io_tx.send(IoEvent::ResumeState(saved.clone()));
        }
        if let Some(volume) = saved.volume {
          let _ = sync_io_tx.send(IoEvent::ChangeVolume(volume));
        }
      }
      if user_config.behavior.resume_track {
        if let Some(uri) = saved.track_uri {
          if saved.is_playing != Some(false) {
            let _ = sync_io_tx.send(IoEvent::StartPlayback(None, Some(vec![uri]), Some(0)));
          }
        }
      }
    }
    // The UI must run in the "main" thread
    start_ui(user_config, &cloned_app).await?;
  }

  Ok(())
}

fn start_tokio(io_rx: std::sync::mpsc::Receiver<IoEvent>, network: &mut Network) {
  let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .expect("failed to build network runtime");
  while let Ok(io_event) = io_rx.recv() {
    // ponytail: catch one handler panic so the thread (and the channel) survives;
    // a panicking handler is skipped and the next event is processed.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      rt.block_on(network.handle_network_event(io_event));
    }));
  }
}

async fn start_ui(user_config: UserConfig, app: &Arc<Mutex<App>>) -> Result<()> {
  // Terminal initialization
  let mut stdout = stdout();
  execute!(stdout, EnterAlternateScreen)?;
  if user_config.behavior.enable_mouse {
    execute!(stdout, EnableMouseCapture)?;
  }
  enable_raw_mode()?;

  let mut backend = CrosstermBackend::new(stdout);

  if user_config.behavior.set_window_title {
    execute!(&mut backend, SetTitle("sptune"))?;
  }

  let mut terminal = Terminal::new(backend)?;
  // Clear the alternate screen so no scroll offset from the regular screen
  // survives into the TUI (a leftover offset hides the top rows of the app,
  // which is where the figlet banner starts).
  terminal.clear()?;
  terminal.hide_cursor()?;

  let events = event::Events::new(user_config.behavior.tick_rate_milliseconds);

  // play music on, if not send them to the device selection view

  let mut is_first_render = true;

  loop {
    // Wait for the next event WITHOUT holding the app lock, so the network
    // thread can keep processing (playback polls, page loads, likes) while
    // the UI idles; holding the lock here starved the network thread and the
    // whole program got laggier the longer it ran.
    let event = events.next()?;

    // Apply every event already queued behind this one before drawing a
    // single frame: wheel/drag floods would otherwise cost one full redraw
    // per event and fall behind the input by seconds.
    let mut batch = vec![event];
    while let Ok(tail) = events.try_next() {
      batch.push(tail);
    }

    let mut app = app.lock().await;
    // Get the size of the screen on each loop to account for resize event
    if let Ok(size) = terminal.backend().size() {
      let size = Rect::from(size);
      // Reset the help menu is the terminal was resized
      if is_first_render || app.size != size {
        app.help_menu_max_lines = 0;
        app.help_scroll_offset = 0;
        app.help_menu_page = 0;

        app.size = size;

        // Page size is the API max regardless of terminal size; the screen
        // renders a viewport of what fits, so bigger pages stay invisible.
        // Only dispatch when the limit actually changes (default is already
        // the API max), so the first-render no-op stays out of the request log.
        if app.large_search_limit != backend::API_MAX_LIMIT {
          app.dispatch(IoEvent::UpdateSearchLimits(
            backend::API_MAX_LIMIT,
            backend::API_MAX_LIMIT,
          ));
        }

        // Based on the size of the terminal, adjust how many lines are
        // displayed in the settings menu (8 = margins + borders + header,
        // plus the fixed settings section on top)
        if app.size.height > 8 {
          app.help_menu_max_lines = (app.size.height as u32)
            .saturating_sub(8 + crate::tui::layout::SETTINGS_ROW_COUNT as u32 + 2);
        } else {
          app.help_menu_max_lines = 0;
        }
      }
    };

    let mut quit = false;
    for event in batch {
      // Handle authentication refresh
      if SystemTime::now() > app.spotify_token_expiry {
        app.dispatch(IoEvent::RefreshAuthentication);
      }

      match event {
        event::Event::Input(input) => match input {
          event::InputEvent::Key(key) => {
            if key == Key::Ctrl('c') {
              quit = true;
              break;
            }

            let current_active_block = app.get_current_route().active_block;

            // To avoid swallowing the global key presses `q` and `-` make a special
            // case for the input handler
            if current_active_block == ActiveBlock::Input {
              handlers::input_handler(key, &mut app);
            } else if key == app.user_config.keys.back {
              if app.get_current_route().active_block != ActiveBlock::Input {
                // Go back through navigation stack when not in search input mode and exit the app if there are no more places to back to

                let pop_result = match app.pop_navigation_stack() {
                  Some(ref x) if x.id == RouteId::Search => app.pop_navigation_stack(),
                  Some(x) => Some(x),
                  None => None,
                };
                if pop_result.is_none() {
                  quit = true;
                  break;
                }
              }
            } else {
              handlers::handle_app(key, &mut app);
            }
          }
          event::InputEvent::Mouse(mouse) => {
            handlers::handle_mouse(mouse, &mut app);
          }
        },
        event::Event::Tick => {
          app.update_on_tick();
        }
      }
    }
    if quit {
      break;
    }

    let current_route = app.get_current_route();
    terminal.draw(|mut f| {
      // Paint the theme background so "black theme" is actually black
      f.render_widget(
        Block::default().style(Style::default().bg(app.user_config.theme.background)),
        f.area(),
      );
      match current_route.active_block {
        ActiveBlock::HelpMenu => {
          tui::draw_help_menu(&mut f, &app);
        }
        ActiveBlock::Error => {
          tui::draw_error_screen(&mut f, &app);
        }
        ActiveBlock::SelectDevice => {
          tui::draw_device_list(&mut f, &app);
        }
        ActiveBlock::MusicView => {
          tui::draw_music_view(&mut f, &app);
        }
        _ => {
          tui::draw_main_layout(&mut f, &app);
        }
      }
    })?;

    if current_route.active_block == ActiveBlock::Input {
      terminal.show_cursor()?;
    } else {
      terminal.hide_cursor()?;
    }

    // Put the cursor back inside the search box (replicated header geometry,
    // matching what the drawer renders, so it tracks layout changes).
    let margin = tui::layout::get_main_layout_margin(&app);
    let input_box = ratatui::layout::Rect::new(
      margin,
      margin,
      app.size.width.saturating_sub(margin * 2),
      tui::layout::header_height(&app),
    );
    let search_box = tui::layout::search_box_rect(&app, input_box);
    let w = terminal.backend_mut();
    // Clamp the cursor to the search box (the input string may be longer
    // than the box; the drawer ellipsizes the tail). The ellipsized text
    // ends at x+width-5 (3 dots; the ✕ button starts at x+width-4), so the
    // cursor sits one cell past the text, before the ✕.
    let cursor_col = (search_box.x + 1 + app.input_cursor_position)
      .min(search_box.x + search_box.width.saturating_sub(4));
    execute!(w, MoveTo(cursor_col, search_box.y + 1))?;

    // Delay spotify request until first render, will have the effect of improving
    // startup speed
    if is_first_render {
      app.dispatch(IoEvent::GetPlaylists);
      app.dispatch(IoEvent::GetUser);
      app.dispatch(IoEvent::GetCurrentPlayback);
      app.help_docs_size = tui::help::get_help_docs(&app.user_config.keys).len() as u32;

      is_first_render = false;
    }
  }

  terminal.show_cursor()?;
  close_application()?;

  Ok(())
}
