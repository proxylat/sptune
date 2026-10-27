//! Instruction-count regression gate (valgrind/callgrind).
//! READ-ONLY ops only: no save-threads, no disk I/O, fully deterministic Ir.
//! Fixtures are built in `#[bench::]` args (evaluated once at collection,
//! excluded from counts). CI fails the PR when Ir regresses more than 2%.

use iai_callgrind::{
  library_benchmark, library_benchmark_group, main, Callgrind, EventKind, LibraryBenchmarkConfig,
};
use ratatui::{backend::TestBackend, Terminal};
use rspotify::model::{FullTrack, PlaylistItem};
use sptune::app::{ActiveBlock, App, RouteId, TrackTableContext};
use sptune::lcg::rand_idx;
use sptune::library_cache::{CachedList, LibraryCache};
use sptune::playlist_cache::{playlist_item_uri, CachedPlaylist, PlaylistCache};
use sptune::user_config::{
  theme_presets, BehaviorConfigString, KeyBindingsString, UserConfig, UserTheme,
};
use std::hint::black_box;

fn item(uri: &str) -> PlaylistItem {
  serde_json::from_value(serde_json::json!({
    "added_at": null,
    "added_by": null,
    "is_local": false,
    "item": {
      "album": {
        "album_type": "album",
        "artists": [],
        "available_markets": [],
        "external_urls": {},
        "href": null,
        "id": "album1",
        "images": [],
        "name": "Album",
        "release_date": "2020-01-01",
        "release_date_precision": "day",
        "total_tracks": 1,
        "type": "album",
        "uri": "spotify:album:album1"
      },
      "artists": [{
        "external_urls": {},
        "href": null,
        "id": "artist1",
        "name": "Artist",
        "type": "artist",
        "uri": "spotify:artist:artist1"
      }],
      "available_markets": [],
      "disc_number": 1,
      "duration_ms": 1000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": uri.rsplit(':').next().unwrap(),
      "is_local": false,
      "is_playable": true,
      "name": "Track",
      "popularity": 0,
      "preview_url": null,
      "track_number": 1,
      "type": "track",
      "uri": uri
    }
  }))
  .unwrap()
}

fn seeded_library(n: usize) -> LibraryCache {
  let ids: Vec<String> = (0..n).map(|i| format!("spotify:track:{i:06}")).collect();
  let mut cache = LibraryCache::new();
  cache.map.insert(
    "tracks".to_string(),
    CachedList {
      fetched_at: 0,
      total: ids.len() as u32,
      items: serde_json::to_value(&ids).unwrap(),
    },
  );
  cache
}

fn seeded_playlist(n: usize) -> PlaylistCache {
  let items: Vec<PlaylistItem> = (0..n)
    .map(|i| item(&format!("spotify:track:{i:06}")))
    .collect();
  let mut cache = PlaylistCache::new();
  cache.map.insert(
    "p1".to_string(),
    CachedPlaylist {
      snapshot: "snap1".to_string(),
      fetched_at: 0,
      items,
      total: n as u32,
    },
  );
  cache
}

fn keybindings_fixture() -> KeyBindingsString {
  serde_json::from_value(serde_json::json!({
    "back": "z",
    "next_page": "ctrl-j",
    "previous_page": "ctrl-k",
  }))
  .unwrap()
}

fn theme_fixture() -> UserTheme {
  serde_json::from_value(serde_json::json!({
    "active": "Green",
    "banner": "29,185,84",
    "error_border": "Red",
    "selected": "White",
  }))
  .unwrap()
}

fn behavior_fixture() -> BehaviorConfigString {
  BehaviorConfigString {
    seek_milliseconds: Some(5000),
    volume_increment: Some(5),
    tick_rate_milliseconds: Some(250),
    ..Default::default()
  }
}

fn full_track(i: usize) -> FullTrack {
  let uri = format!("spotify:track:{i:06}");
  serde_json::from_value(serde_json::json!({
    "album": {
      "album_type": "album",
      "artists": [],
      "available_markets": [],
      "external_urls": {},
      "href": null,
      "id": "album1",
      "images": [],
      "name": "Album",
      "release_date": "2020-01-01",
      "release_date_precision": "day",
      "total_tracks": 1,
      "type": "album",
      "uri": "spotify:album:album1"
    },
    "artists": [{
      "external_urls": {},
      "href": null,
      "id": "artist1",
      "name": "Artist",
      "type": "artist",
      "uri": "spotify:artist:artist1"
    }],
    "available_markets": [],
    "disc_number": 1,
    "duration_ms": 180_000,
    "explicit": false,
    "external_ids": {},
    "external_urls": {},
    "href": null,
    "id": uri.rsplit(':').next().unwrap(),
    "is_local": false,
    "is_playable": true,
    "name": format!("Track {i}"),
    "popularity": 50,
    "preview_url": null,
    "track_number": 1,
    "type": "track",
    "uri": uri
  }))
  .unwrap()
}

/// App sitting on an opened playlist with `n` tracks — the frame the user
/// actually stares at. Built in `#[bench::]` args, excluded from counts.
fn track_table_app(n: usize) -> App {
  let mut app = App::default();
  app.track_table.tracks = (0..n).map(full_track).collect();
  app.track_table.context = Some(TrackTableContext::MyPlaylists);
  app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
  app
}

/// One full draw into a headless terminal. Returns painted cells so the
/// frame can't be dead-code-eliminated out from under callgrind.
fn draw_frame(app: &App) -> usize {
  let backend = TestBackend::new(200, 50);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| sptune::tui::draw_main_layout(f, black_box(app)))
    .unwrap();
  terminal.backend().buffer().content.len()
}

#[library_benchmark]
#[bench::empty(App::default())]
#[bench::tracks585(track_table_app(585))]
fn render_frame(app: App) {
  black_box(draw_frame(black_box(&app)));
}

#[library_benchmark]
#[bench::n100(seeded_library(100))]
#[bench::n2000(seeded_library(2000))]
fn library_get_typed(cache: LibraryCache) {
  black_box(cache.get_typed::<String>(black_box("tracks")));
}

#[library_benchmark]
#[bench::n585(seeded_playlist(585))]
#[bench::n10000(seeded_playlist(10_000))]
fn playlist_lookup(cache: PlaylistCache) {
  black_box(cache.lookup(black_box("p1")));
}

#[library_benchmark]
#[bench::n585(seeded_playlist(585))]
#[bench::n10000(seeded_playlist(10_000))]
fn playlist_is_polluted(cache: PlaylistCache) {
  black_box(cache.is_polluted(black_box("p1")));
}

#[library_benchmark]
fn playlist_item_uri_one() {
  let one = item("spotify:track:000001");
  black_box(playlist_item_uri(black_box(&one)));
}

#[library_benchmark]
fn lcg_rand_idx() {
  black_box(rand_idx(black_box(10_000)));
}

#[library_benchmark]
#[bench::keys(keybindings_fixture())]
fn config_load_keybindings(kb: KeyBindingsString) {
  let mut cfg = UserConfig::new();
  cfg.load_keybindings(black_box(kb)).unwrap();
}

#[library_benchmark]
#[bench::theme(theme_fixture())]
fn config_load_theme(theme: UserTheme) {
  let mut cfg = UserConfig::new();
  cfg.load_theme(black_box(theme)).unwrap();
}

#[library_benchmark]
#[bench::behavior(behavior_fixture())]
fn config_load_behavior(behavior: BehaviorConfigString) {
  let mut cfg = UserConfig::new();
  cfg.load_behaviorconfig(black_box(behavior)).unwrap();
}

#[library_benchmark]
fn config_theme_presets() {
  black_box(theme_presets());
}

library_benchmark_group!(
  name = cache_reads;
  benchmarks =
    library_get_typed,
    playlist_lookup,
    playlist_is_polluted,
    playlist_item_uri_one,
    lcg_rand_idx
);

library_benchmark_group!(
  name = config_loads;
  benchmarks =
    config_load_keybindings,
    config_load_theme,
    config_load_behavior,
    config_theme_presets
);

library_benchmark_group!(
  name = render;
  benchmarks = render_frame
);

main!(
  config = LibraryBenchmarkConfig::default()
    // iai clears the env for runs by default; valgrind needs VALGRIND_LIB
    // to locate its tool binaries on machines with a non-/usr install.
    .env_clear(false)
    // PLAN.md CI rule: PR must not regress Ir by more than 2%.
    .tool(Callgrind::default().soft_limits([(EventKind::Ir, 2.0)]));
  library_benchmark_groups = cache_reads, config_loads, render);
