mod app;
mod backend;
mod cli;
mod client_creds;
mod cover_art;
mod handlers;
mod metadata;
mod platform;
mod redirect_uri;
mod soloist_setup;
mod soloist_ws;
mod tui;
mod visualizer;

use sptune::{event, lcg, library_cache, playlist_cache, user_config};

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
  AuthCodePkceSpotify, Credentials, OAuth,
};
use std::backtrace::Backtrace;
use std::{
  io::{self, stdout},
  panic::{self, PanicHookInfo},
  path::PathBuf,
  sync::Arc,
  time::{Duration, SystemTime},
};
use tokio::sync::Mutex;
use user_config::{theme_presets, UserConfig, UserConfigPaths};

const SCOPES: [&str; 13] = [
  "playlist-read-collaborative",
  "playlist-read-private",
  "playlist-modify-private",
  "playlist-modify-public",
  "user-follow-read",
  "user-follow-modify",
  "user-library-modify",
  "user-library-read",
  "user-modify-playback-state",
  "user-read-playback-state",
  "user-read-playback-position",
  "user-read-private",
  "user-read-recently-played",
];

/// get token automatically with local webserver (PKCE, no client secret)
pub async fn get_token_auto(spotify: &mut AuthCodePkceSpotify, port: u16) -> Option<Token> {
  // A cached token without a refresh token is a poisoned leftover (older
  // builds saved refresh responses that omitted it): the session could
  // never refresh, so every launch ended in Ok(None) + the restart
  // message. Ignore it and re-authenticate through the browser once; the
  // fresh grant rewrites the file with a refresh token. Healthy grants
  // always carry one, so this only ever fires on poisoned files.
  let usable = |token: Token| {
    token
      .refresh_token
      .as_deref()
      .is_some_and(|s| !s.is_empty())
      .then_some(token)
  };
  if let Ok(Some(token)) = spotify.read_token_cache(false).await {
    if let Some(token) = usable(token) {
      return Some(token);
    }
  }

  // expired cached token is still usable — refresh_authentication
  // refetches via the stored refresh_token on the next event-loop tick.
  if let Ok(token) = Token::from_cache(spotify.get_config().cache_path.clone()) {
    if let Some(token) = usable(token) {
      return Some(token);
    }
  }

  let auth_url = spotify.get_authorize_url(None).ok()?;
  println!("\nOpening browser for Spotify authorization...");
  // On Windows cmd.exe splits '&' — tell user to paste into browser address bar, not terminal
  println!(
    "If it doesn't open, copy this URL into your browser:\n{}\n",
    auth_url
  );
  #[cfg(windows)]
  println!("(On Windows, paste into browser — don't run it in cmd)\n");
  println!(
    "Waiting for callback on http://127.0.0.1:{}/callback ...",
    port
  );
  open_url(&auth_url);

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

fn open_url(url: &str) {
  #[cfg(target_os = "linux")]
  let _ = std::process::Command::new("xdg-open").arg(url).spawn();
  #[cfg(target_os = "macos")]
  let _ = std::process::Command::new("open").arg(url).spawn();
  #[cfg(target_os = "windows")]
  {
    // avoid cmd.exe '&' splitting — explorer/rundll32 take the URL as a single arg
    if std::process::Command::new("rundll32")
      .args(["url.dll,FileProtocolHandler", url])
      .spawn()
      .is_err()
    {
      let _ = std::process::Command::new("explorer").arg(url).spawn();
    }
  }
  #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
  let _ = url;
}

async fn get_code_from_url(spotify: &mut AuthCodePkceSpotify, url: String) -> Option<Token> {
  let code = spotify.parse_response_code(url.trim())?;
  if let Err(e) = spotify.request_token(code.as_str()).await {
    eprintln!("Failed to exchange code for token: {}", e);
    return None;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
      &spotify.get_config().cache_path,
      std::fs::Permissions::from_mode(0o600),
    );
  }
  spotify.get_token().lock().await.unwrap().clone()
}

fn close_application(alt_screen: bool, enable_mouse: bool) -> Result<()> {
  disable_raw_mode()?;
  let mut stdout = io::stdout();
  if alt_screen {
    execute!(stdout, LeaveAlternateScreen)?;
  }
  if enable_mouse {
    execute!(stdout, DisableMouseCapture)?;
  }
  Ok(())
}

/// Shutdown question for the soloist child. True = keep it running as an
/// independent speaker; anything but y/yes (Enter, EOF, Ctrl-C) = kill.
fn prompt_keep_soloist() -> bool {
  use std::io::Write as _;
  print!("Soloist is still playing. Keep it running after sptune exits? [y/N] ");
  let _ = std::io::stdout().flush();
  let mut line = String::new();
  std::io::stdin().read_line(&mut line).is_ok()
    && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
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
  // Panic = sptune is going down: take Spotify with it only if sptune launched it.
  if crate::platform::spotify::was_spawned() {
    crate::platform::spotify::kill_all_spotify();
  }
  crate::platform::soloist::kill_spawned();
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
  let startup_at = std::time::Instant::now();
  panic::set_hook(Box::new(|info| {
    panic_hook(info);
  }));

  let mut clap_app = Command::new(env!("CARGO_PKG_NAME"))
    .version(env!("CARGO_PKG_VERSION"))
    .author(env!("CARGO_PKG_AUTHORS"))
    .about(env!("CARGO_PKG_DESCRIPTION"))
    .after_help(
      "Your Spotify Client ID is stored in $HOME/.config/sptune/client.yml\nPowered by Spotify — music data from the Spotify Web API. Spotify is a trademark of Spotify AB.\nHeadless playback (Linux): enable [soloist] in your config and set SOLOIST_API_KEY — get your own key at https://developer.spotify.com/dashboard/soloist (Premium required). First run needs a one-time pair: select the sptune device in the official Spotify desktop or mobile app (web player can't).",
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
      Arg::new("setup-soloist")
        .long("setup-soloist")
        .help("Run the interactive Soloist headless-playback setup, then continue startup.")
        .action(clap::ArgAction::SetTrue),
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
    .subcommand(cli::clean_cache_subcommand())
    .arg(
      Arg::new("mini-popup")
        .long("mini-popup")
        .action(clap::ArgAction::SetTrue)
        .help("Open or toggle the floating mini-player popup window"),
    );

  let matches = clap_app.clone().get_matches();

  if matches.get_flag("mini-popup") {
    crate::platform::tray::toggle_integrated_popover(None);
    return Ok(());
  }

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
  let had_config_file = user_config
    .path_to_config
    .as_ref()
    .is_some_and(|p| p.config_file_path.exists());

  if let Some(tick_rate) = matches.get_one::<u64>("tick-rate") {
    if *tick_rate >= 1000 {
      panic!("Tick rate must be below 1000");
    } else {
      user_config.behavior.tick_rate_milliseconds = *tick_rate;
    }
  }

  let mut client_config = ClientConfig::new();
  client_config.load_config()?;
  let config_paths = client_config.get_or_build_paths()?;

  // Soloist setup: explicit flag on any OS, or first run on Linux (fresh
  // config.yml). Runs before auth so startup launches Soloist directly.
  if matches.get_flag("setup-soloist") || (!had_config_file && cfg!(target_os = "linux")) {
    crate::soloist_setup::run_setup(&mut client_config, &mut user_config).await?;
  }

  // Start authorization with spotify
  let oauth = OAuth {
    redirect_uri: client_config.get_redirect_uri(),
    scopes: SCOPES.iter().map(|s| s.to_string()).collect(),
    ..Default::default()
  };
  // PKCE-only: no client secret (Spotify TOS forbids secrets in native apps).
  let creds = Credentials::new_pkce(&client_config.client_id);
  let config = rspotify::Config {
    cache_path: config_paths.token_cache_path,
    token_cached: true,
    // rspotify's auto_reauth calls PKCE refetch_token, which drops
    // refresh_token (0.16.1) and poisons the in-memory token to Ok(None) on
    // the next refresh. sptune owns refresh via refresh_authentication
    // (carry-over + write-back) instead.
    token_refreshing: false,
    ..Default::default()
  };
  let mut spotify = AuthCodePkceSpotify::with_config(creds, oauth, config);

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
  let mut restored_sidebar_width = None;
  let mut restored_library_height = None;
  let mut restored_dev_view = None;
  let mut saved_state = SavedState::load();
  if let Some(saved) = saved_state.as_ref() {
    if let Some(enabled) = saved.mouse_enabled {
      user_config.behavior.enable_mouse = enabled;
      user_config.terminal.enable_mouse = enabled;
    }
    if let Some(name) = saved.theme_preset.as_deref() {
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
    user_config.apply_truecolor_downgrade();
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
    if let Some(restore_settings) = saved.restore_settings {
      user_config.behavior.restore_settings = restore_settings;
    }
    if let Some(enable) = saved.enable_add_to_playlist {
      user_config.behavior.enable_add_to_playlist = enable;
    }
    if let Some(enable) = saved.enable_remove_from_playlist {
      user_config.behavior.enable_remove_from_playlist = enable;
    }
    if let Some(max_len) = saved.max_display_length {
      user_config.behavior.max_display_length = max_len;
    }
    if let Some(poll_secs) = saved.playback_poll_secs {
      user_config.behavior.playback_poll_secs = poll_secs.clamp(1, 30);
    }
    if let Some(dev_view) = saved.dev_view {
      restored_dev_view = Some(dev_view);
    }
    if let Some(enable_animations) = saved.enable_animations {
      user_config.behavior.enable_animations = enable_animations;
    }
    restored_show_library = saved.show_library;
    restored_show_playlists = saved.show_playlists;
    restored_sidebar_minimized = saved.sidebar_minimized;
    restored_sidebar_width = saved.sidebar_width;
    restored_library_height = saved.library_height;
    if let Some(v) = saved.spotify_auto_launch {
      user_config.spotify.auto_launch = v;
    }
    if let Some(v) = saved.spotify_use_flags {
      user_config.spotify.use_chromium_flags = v;
    }
    if let Some(v) = saved.spotify_suspend {
      user_config.spotify.suspend_children = v;
    }
    if let Some(v) = saved.spotify_trim_ws {
      user_config.spotify.trim_working_set = v;
    }
    if let Some(v) = saved.spotify_mem_limit {
      user_config.spotify.memory_limit_mb = v;
    }
  }

  // Initialise app state
  let app = Arc::new(Mutex::new(App::new(
    sync_io_tx.clone(),
    user_config.clone(),
    token_expiry,
  )));
  let mut app_guard = app.lock().await;
  app_guard.startup_at = startup_at;
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
  if let Some(w) = restored_sidebar_width {
    app_guard.sidebar_width_override = Some(w);
  }
  if let Some(h) = restored_library_height {
    app_guard.library_height_override = Some(h);
  }
  if let Some(dev) = restored_dev_view {
    app_guard.dev_view = dev;
  }
  if let Some(saved) = saved_state.as_mut() {
    app_guard.music_left_pct = saved.music_left_pct;
    app_guard.music_lyrics_pct = saved.music_lyrics_pct;
    app_guard.music_cover_height = saved.music_cover_height;
    if let Some(show) = saved.show_process_suspend {
      app_guard.show_process_suspend = show;
    }
    if let Some(notif) = saved.desktop_notifications {
      app_guard.desktop_notifications = notif;
    }
    if let Some(sorts) = saved.playlist_sorts.take() {
      for (key, (name, desc)) in sorts {
        if let Some(col) = backend::sort_column_from_name(&name) {
          app_guard.playlist_sorts.insert(key, (col, desc));
        }
      }
    }
    if let Some(custom) = saved.made_for_you_custom.take() {
      app_guard.made_for_you_custom = custom;
    }
    if let Some(uri) = saved.last_played_track.take() {
      app_guard.saved_last_track = Some(uri);
    }
  }
  app_guard.clamp_library_selection();
  let should_auto_launch = app_guard.user_config.spotify.auto_launch;
  let spotify_cfg = app_guard.user_config.spotify.clone();
  let soloist_cfg = app_guard.user_config.soloist.clone();
  drop(app_guard);
  if should_auto_launch {
    let _ = crate::platform::spotify::launch(&spotify_cfg);
  }
  if soloist_cfg.enabled {
    let api_key = std::env::var("SOLOIST_API_KEY")
      .ok()
      .filter(|k| !k.trim().is_empty())
      .or_else(|| client_config.soloist_api_key.clone());
    match api_key {
      Some(key) => {
        let app_dir: PathBuf = config_paths
          .config_file_path
          .parent()
          .map(|p| p.to_path_buf())
          .or_else(|| dirs::config_dir().map(|d| d.join("sptune")))
          .unwrap_or_else(|| PathBuf::from(".").join("sptune"));
        let soloist_data_dir = crate::platform::soloist::data_dir_under(&app_dir);
        match crate::platform::soloist::launch(&soloist_cfg, &key, &soloist_data_dir) {
          Ok(()) => crate::platform::soloist::mark_started(
            soloist_data_dir,
            soloist_cfg.device_name.clone(),
          ),
          Err(e) => eprintln!("soloist: {}", e),
        }
      }
      None => eprintln!(
        "soloist: enabled but no API key — generate your own (Premium required) at https://developer.spotify.com/dashboard/soloist and set SOLOIST_API_KEY or soloist_api_key in client.yml"
      ),
    }
  }

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
    // Periodic sweeper: keeps the suspend/trim/mem-limit policy applied
    // while the UI runs. Spotify keeps spawning helper processes for several
    // seconds after the browser starts, and Spotify may not have been
    // launched by sptune at all — a one-shot launch sweep can't cover either.
    // The registry in platform::spotify keeps every pid suspend-once.
    {
      let sweeper_app = Arc::clone(&app);
      std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(4));
        let (auto_suspend, trim, mem_limit, manual_suspended) = {
          let guard = sweeper_app.blocking_lock();
          (
            guard.user_config.spotify.suspend_children,
            guard.user_config.spotify.trim_working_set,
            guard.user_config.spotify.memory_limit_mb,
            guard.processes_suspended,
          )
        };
        let suspend = auto_suspend || manual_suspended;
        if !(suspend || trim || mem_limit != 0) {
          continue;
        }
        let Some(root) = crate::platform::spotify::find_root_pid() else {
          continue;
        };
        if mem_limit != 0 {
          #[cfg(target_os = "linux")]
          {
            // cgroup.procs writes are idempotent, so re-adding known pids is fine
            let kids = crate::platform::linux::descendants_of(root);
            if kids.is_empty() {
              crate::platform::linux::apply_memory_limits_pids(&[root]);
            } else {
              crate::platform::linux::apply_memory_limits(&kids);
            }
          }
        }
        crate::platform::spotify::sweep_processes(root, suspend, trim);
      });
    }
    std::thread::spawn(move || {
      let mut network = Network::new(spotify, client_config, &app);
      start_tokio(sync_io_rx, &mut network);
    });
    // OS media events (GSMTC/MPRIS): track changes wake the playbar without
    // spending API tokens. Dies silently where unsupported; the poll below
    // and the 5s cadence keep working regardless.
    crate::platform::media_watch::spawn_media_watcher(sync_io_tx.clone());
    crate::platform::tray_ipc::spawn(Arc::clone(&cloned_app));
    crate::platform::tray::spawn_tray(sync_io_tx.clone(), Arc::clone(&cloned_app));
    // Restore the last session: volume/shuffle/repeat apply only when
    // nothing is already playing (gated inside AutoRestore), last
    // page/sort always. One event instead of the old ResumeState + ChangeVolume pair.
    // GetCurrentPlayback goes first so AutoRestore (and the first paint)
    // can read the live state instead of paying a second fetch.
    if SystemTime::now() >= token_expiry {
      let _ = sync_io_tx.send(IoEvent::RefreshAuthentication);
    }
    let _ = sync_io_tx.send(IoEvent::GetCurrentPlayback);
    if let Some(saved) = saved_state {
      if user_config.behavior.restore_settings
        && (saved.volume.is_some()
          || saved.shuffle.is_some()
          || saved.repeat.is_some()
          || saved.track_sort.is_some()
          || saved.last_page.is_some())
      {
        let _ = sync_io_tx.send(IoEvent::AutoRestore(Box::new(saved)));
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
  // Pre-warm the shared HTTP pool on the network runtime while UI starts.
  rt.spawn(async {
    let _ = crate::backend::shared_http_client()
      .get("https://api.spotify.com/v1/")
      .send()
      .await;
  });
  while let Ok(io_event) = io_rx.recv() {
    // catch one handler panic so the thread (and the channel) survives;
    // a panicking handler is skipped and the next event is processed.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      rt.block_on(network.handle_network_event(io_event));
    }));
  }
}

async fn start_ui(user_config: UserConfig, app: &Arc<Mutex<App>>) -> Result<()> {
  // Terminal initialization
  let mut stdout = stdout();
  if user_config.terminal.alt_screen {
    execute!(stdout, EnterAlternateScreen)?;
  }
  if user_config.terminal.enable_mouse {
    execute!(stdout, EnableMouseCapture)?;
  }
  enable_raw_mode()?;
  {
    let picker = ratatui_image::picker::Picker::from_query_stdio()
      .unwrap_or_else(|_| ratatui_image::picker::Picker::halfblocks());
    app.lock().await.cover_picker = Some(picker);
  }

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

  #[cfg(windows)]
  crate::platform::windows::disable_quick_edit();

  let events = event::Events::new(
    user_config.behavior.tick_rate_milliseconds,
    user_config.terminal.mouse_y_offset,
  );

  // play music on, if not send them to the device selection view

  let mut is_first_render = true;
  let mut last_refresh_attempt = SystemTime::UNIX_EPOCH;
  let mut last_active_block: Option<ActiveBlock> = None;
  let mut kitty_placed: Option<(String, Rect)> = None;
  let is_tmux = std::env::var_os("TMUX").is_some_and(|v| !v.is_empty());

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
    let mut needs_redraw = is_first_render;
    // Backend mutations (add/remove/unlike) flag the screen dirty so the
    // change paints on the next iteration without waiting for input.
    if app.ui_dirty {
      app.ui_dirty = false;
      needs_redraw = true;
    }
    // Get the size of the screen on each loop to account for resize event
    if let Ok(size) = terminal.backend().size() {
      let size = Rect::from(size);
      // Reset the help menu is the terminal was resized
      if is_first_render || app.size != size {
        app.help_menu_max_lines = 0;
        app.help_scroll_offset = 0;
        app.help_menu_page = 0;

        app.size = size;
        needs_redraw = true;

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

        // Shortcuts page fills help_full_rect (height-4); table reserves 3 for borders+header
        if app.size.height > 7 {
          app.help_menu_max_lines = (app.size.height as u32).saturating_sub(7);
        } else {
          app.help_menu_max_lines = 0;
        }
      }
    };

    let mut quit = false;
    for event in batch {
      // cooldown 60s so a failed refetch (main.rs:505 every 250ms) doesn't tight-loop
      let now = SystemTime::now();
      if !app.auth_reauth_required
        && now > app.spotify_token_expiry
        && now
          .duration_since(last_refresh_attempt)
          .unwrap_or(Duration::ZERO)
          > Duration::from_secs(60)
      {
        last_refresh_attempt = now;
        app.dispatch(IoEvent::RefreshAuthentication);
      }

      match event {
        event::Event::Input(input) => {
          needs_redraw = true;
          match input {
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
          }
        }
        event::Event::Tick => {
          if app.update_on_tick() {
            needs_redraw = true;
          }
          if app.get_current_route().active_block == ActiveBlock::MusicView {
            app.visualizer.poll();
            app.visualizer.tick();
            if app.visualizer.mode == visualizer::VizMode::Mosaic {
              let rows = (app.size.height as usize * 30 / 100)
                .saturating_sub(2)
                .max(1);
              let cols = (app.size.width as usize * 70 / 100)
                .saturating_sub(2)
                .max(1);
              app.visualizer.mosaic_tick(rows, cols);
            }
            needs_redraw = true;
          }
        }
      }
    }
    if quit {
      break;
    }
    // idle throttle — skip full redraw when paused and no input; cuts 4 fps wake-ups to 0 when idle
    if !needs_redraw {
      drop(app);
      continue;
    }

    let active_block = app.get_current_route().active_block;
    events.set_fast_tick(
      active_block == ActiveBlock::MusicView && app.visualizer.mode != visualizer::VizMode::Off,
    );
    if active_block == ActiveBlock::MusicView && last_active_block != Some(ActiveBlock::MusicView) {
      app.visualizer.poll();
    }
    let is_fullscreen = |b: ActiveBlock| {
      matches!(
        b,
        ActiveBlock::HelpMenu
          | ActiveBlock::Error
          | ActiveBlock::SelectDevice
          | ActiveBlock::MusicView
      )
    };
    if last_active_block.is_some_and(|prev| {
      prev != active_block && (is_fullscreen(prev) || is_fullscreen(active_block))
    }) {
      if last_active_block == Some(ActiveBlock::MusicView) {
        app.visualizer.stop();
      }
      let pb_was_placed = PLAYBAR_KITTY
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .is_some();
      if kitty_placed.take().is_some() || pb_was_placed {
        use std::io::Write as _;
        let _ = write!(
          terminal.backend_mut(),
          "{}{}",
          cover_art::kitty_delete_seq(is_tmux),
          cover_art::kitty_delete_id_seq(cover_art::KITTY_PLAYBAR_COVER_ID, is_tmux),
        );
      }
      terminal.clear()?;
    }
    last_active_block = Some(active_block);

    terminal.draw(|f| {
      // Paint the theme background so "black theme" is actually black
      f.render_widget(
        Block::default().style(Style::default().bg(app.user_config.theme.background)),
        f.area(),
      );
      match active_block {
        ActiveBlock::HelpMenu => {
          tui::draw_help_menu(f, &app);
        }
        ActiveBlock::Error => {
          tui::draw_error_screen(f, &app);
        }
        ActiveBlock::SelectDevice => {
          tui::draw_device_list(f, &app);
        }
        ActiveBlock::MusicView => {
          tui::draw_music_view(f, &app);
        }
        _ => {
          tui::draw_main_layout(f, &app);
        }
      }
    })?;

    sync_kitty_cover(
      terminal.backend_mut(),
      &app,
      active_block,
      is_tmux,
      &mut kitty_placed,
    )?;

    if active_block == ActiveBlock::Input {
      terminal.show_cursor()?;
      // Put the cursor back inside the search box (replicated header geometry,
      // matching what the drawer renders, so it tracks layout changes).
      let margin = tui::layout::get_main_layout_margin(&app);
      let input_box = ratatui::layout::Rect::new(
        margin,
        margin,
        app.size.width.saturating_sub(margin * 2),
        tui::layout::HEADER_HEIGHT,
      );
      let search_box = tui::layout::search_box_rect(&app, input_box);
      let w = terminal.backend_mut();
      // Scroll the input so the cursor stays in view; drawer uses the same
      // viewport (inner width minus the clear ✕ reserve).
      let input_string: String = app.input.iter().collect();
      let inner_w = search_box.width.saturating_sub(2) as usize;
      let reserve = if app.input.is_empty() { 0 } else { 3 };
      let viewport_w = inner_w.saturating_sub(reserve);
      let (_, vis_cursor) = tui::layout::search_input_visible(
        &input_string,
        app.input_cursor_position as usize,
        viewport_w,
      );
      let cursor_col = search_box.x + 1 + vis_cursor as u16;
      execute!(w, MoveTo(cursor_col, search_box.y + 1))?;
    } else {
      terminal.hide_cursor()?;
    }

    // Delay spotify request until first render, will have the effect of improving
    // startup speed
    if is_first_render {
      app.first_draw_ms = Some(app.startup_at.elapsed().as_millis() as u64);
      app.dispatch(IoEvent::GetPlaylists);
      app.dispatch(IoEvent::GetUser);
      let soloist_transfer = app
        .user_config
        .soloist
        .enabled
        .then(|| app.user_config.soloist.device_name.clone());
      if let Some(name) = soloist_transfer {
        app.dispatch(IoEvent::TransferPlaybackToDeviceName(name));
      }
      if let Some((data_dir, device_name)) = crate::platform::soloist::take_runtime() {
        app.dispatch(IoEvent::SoloistWsWatch {
          data_dir,
          device_name,
        });
      }
      // No GetCurrentPlayback here: startup already queued one before
      // AutoRestore, so this would be a duplicate roundtrip ahead of
      // the playlist/user responses.
      app.help_docs_size = tui::help::get_help_docs(&app.user_config.keys).len() as u32;

      is_first_render = false;
    }
  }

  crate::platform::tray_ipc::cleanup();
  // Only close Spotify desktop on exit if sptune itself launched it.
  if crate::platform::spotify::was_spawned() {
    crate::platform::spotify::kill_all_spotify();
  }
  // Soloist is our own child, so ask first: restore the terminal so the
  // question is visible (close_application is idempotent — the final
  // teardown below repeats it harmlessly). Anything but y/yes kills it,
  // preserving the old die-with-parent default on Enter/EOF.
  if crate::platform::soloist::is_spawned() {
    close_application(
      user_config.terminal.alt_screen,
      user_config.terminal.enable_mouse,
    )?;
    if !prompt_keep_soloist() {
      crate::platform::soloist::kill_spawned();
    }
  } else {
    crate::platform::soloist::kill_spawned();
  }

  {
    use std::io::Write as _;
    let _ = write!(
      terminal.backend_mut(),
      "{}{}",
      cover_art::kitty_delete_seq(is_tmux),
      cover_art::kitty_delete_id_seq(cover_art::KITTY_PLAYBAR_COVER_ID, is_tmux),
    );
    let _ = Backend::flush(terminal.backend_mut());
  }
  let _ = kitty_placed.take();
  terminal.show_cursor()?;

  // The single save point: the full snapshot (settings + playback state +
  // last track) is written here and nowhere else — nothing is saved during
  // the session.
  {
    let guard = app.lock().await;
    SavedState::snapshot_from_app(&guard).save();
  }

  close_application(
    user_config.terminal.alt_screen,
    user_config.terminal.enable_mouse,
  )?;

  Ok(())
}

static PLAYBAR_KITTY: std::sync::Mutex<Option<(String, Rect)>> = std::sync::Mutex::new(None);

/// Emit direct Kitty graphics placement (`a=T,f=100,t=d,i=1,c=cols,r=rows,C=1`)
/// outside ratatui's cell buffer so Ghostty/Kitty render the cover cleanly on
/// the first frame without Unicode-placeholder alignment or cursor glitches.
fn sync_kitty_cover<W: io::Write>(
  w: &mut W,
  app: &App,
  active_block: ActiveBlock,
  is_tmux: bool,
  placed: &mut Option<(String, Rect)>,
) -> Result<()> {
  let Some(picker) = app.cover_picker.as_ref() else {
    return Ok(());
  };
  if picker.protocol_type() != ratatui_image::picker::ProtocolType::Kitty {
    return Ok(());
  }
  let route = app.get_current_route();
  let (want_id, target_inner, src_cover) = if active_block == ActiveBlock::MusicView {
    let id = cover_art::now_playing(
      app
        .current_playback_context
        .as_ref()
        .and_then(|c| c.item.as_ref()),
    )
    .map(|(id, _)| id);
    (
      id,
      Some(tui::music_view::cover_inner_rect_for(app, app.size)),
      app.cover_art.as_ref(),
    )
  } else if route.id == RouteId::Artist
    && !matches!(
      active_block,
      ActiveBlock::HelpMenu | ActiveBlock::Error | ActiveBlock::SelectDevice
    )
  {
    let id = app
      .artist
      .as_ref()
      .filter(|a| a.image_url.is_some())
      .map(|a| format!("artist:{}", a.artist_id));
    let inner = tui::layout::main_body_content_rect(app)
      .and_then(tui::layout::artist_header_rect)
      .map(tui::layout::artist_cover_rect);
    (id, inner, app.artist_cover.as_ref())
  } else {
    (None, None, None)
  };
  let next = match (want_id, target_inner, src_cover) {
    (Some(want), Some(inner), Some((have, img))) if &want == have => {
      let target =
        tui::music_view::fit_cover_rect(inner, (img.width(), img.height()), picker.font_size());
      (target.width > 0 && target.height > 0).then_some((want, img, target))
    }
    _ => None,
  };
  match next {
    Some((id, img, target)) => {
      if placed.as_ref() != Some(&(id.clone(), target)) {
        let seq = cover_art::kitty_place_seq(
          img,
          target.width,
          target.height,
          picker.font_size(),
          is_tmux,
        )?;
        write!(w, "{}", cover_art::kitty_delete_seq(is_tmux))?;
        execute!(w, MoveTo(target.x, target.y))?;
        w.write_all(seq.as_bytes())?;
        w.flush()?;
        *placed = Some((id, target));
      }
    }
    None if placed.is_some() => {
      write!(w, "{}", cover_art::kitty_delete_seq(is_tmux))?;
      w.flush()?;
      *placed = None;
    }
    None => {}
  }

  let pb_next = if !matches!(
    active_block,
    ActiveBlock::MusicView | ActiveBlock::HelpMenu | ActiveBlock::Error | ActiveBlock::SelectDevice
  ) && app.dialog.is_none()
  {
    let want_id = cover_art::now_playing(
      app
        .current_playback_context
        .as_ref()
        .and_then(|c| c.item.as_ref()),
    )
    .map(|(id, _)| id);
    match (
      want_id,
      tui::layout::playbar_cover_rect(app),
      &app.cover_art,
    ) {
      (Some(want), Some(inner), Some((have, img))) if &want == have => {
        let target =
          tui::music_view::fit_cover_rect(inner, (img.width(), img.height()), picker.font_size());
        (target.width > 0 && target.height > 0).then_some((want, img, target))
      }
      _ => None,
    }
  } else {
    None
  };
  if let Ok(mut pb_placed) = PLAYBAR_KITTY.lock() {
    match pb_next {
      Some((id, img, target)) => {
        if pb_placed.as_ref() != Some(&(id.clone(), target)) {
          let seq = cover_art::kitty_place_id_seq(
            cover_art::KITTY_PLAYBAR_COVER_ID,
            img,
            target.width,
            target.height,
            picker.font_size(),
            is_tmux,
          )?;
          write!(
            w,
            "{}",
            cover_art::kitty_delete_id_seq(cover_art::KITTY_PLAYBAR_COVER_ID, is_tmux)
          )?;
          execute!(w, MoveTo(target.x, target.y))?;
          w.write_all(seq.as_bytes())?;
          w.flush()?;
          *pb_placed = Some((id, target));
        }
      }
      None if pb_placed.is_some() => {
        write!(
          w,
          "{}",
          cover_art::kitty_delete_id_seq(cover_art::KITTY_PLAYBAR_COVER_ID, is_tmux)
        )?;
        w.flush()?;
        *pb_placed = None;
      }
      None => {}
    }
  }
  Ok(())
}
