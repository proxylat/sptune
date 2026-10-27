use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use ratatui::{backend::TestBackend, Terminal};
use rspotify::model::FullTrack;
use sptune::app::{ActiveBlock, App, RouteId, TrackTableContext};
use sptune::tui::{draw_main_layout, layout::display_track_progress};
use std::hint::black_box;

/// One realistic track: same JSON shape the API returns (and the cache
/// benches use), deserialized into the type the song table renders.
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
/// actually stares at (585 = the reported playlist size).
fn track_table_app(n: usize) -> App {
  let mut app = App::default();
  app.track_table.tracks = (0..n).map(full_track).collect();
  app.track_table.context = Some(TrackTableContext::MyPlaylists);
  app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
  app
}

fn draw(app: &App) {
  let backend = TestBackend::new(200, 50);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_main_layout(f, black_box(app)))
    .unwrap();
}

fn bench_render(c: &mut Criterion) {
  let mut group = c.benchmark_group("frame");
  // Fresh terminal per iteration: measures one full draw, not buffer reuse.
  group.bench_function("empty", |b| {
    let app = App::default();
    b.iter(|| draw(black_box(&app)))
  });
  for n in [585usize, 2000] {
    group.bench_with_input(BenchmarkId::new("track_table", n), &n, |b, &n| {
      let app = track_table_app(n);
      b.iter(|| draw(black_box(&app)))
    });
  }
  group.finish();

  c.bench_function("progress_label", |b| {
    b.iter(|| display_track_progress(black_box(95_000), black_box(180_000)))
  });
}

criterion_group!(benches, bench_render);
criterion_main!(benches);
