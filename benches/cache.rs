use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rspotify::model::PlaylistItem;
use sptune::library_cache::{CachedList, LibraryCache};
use sptune::playlist_cache::{playlist_item_uri, CachedPlaylist, PlaylistCache};
use std::hint::black_box;

/// Point $HOME at a throwaway dir: both caches resolve paths via
/// dirs::home_dir(), and write ops spawn save-threads. Never touch real config.
fn isolate_home() {
  let dir = std::env::temp_dir().join(format!("sptune_bench_{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&dir);
  std::fs::create_dir_all(&dir).unwrap();
  std::env::set_var("HOME", &dir);
}

fn track_ids(n: usize) -> Vec<String> {
  (0..n).map(|i| format!("spotify:track:{i:06}")).collect()
}

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

fn playlist_items(n: usize) -> Vec<PlaylistItem> {
  (0..n)
    .map(|i| item(&format!("spotify:track:{i:06}")))
    .collect()
}

fn seeded_library(ids: &[String]) -> LibraryCache {
  let mut cache = LibraryCache::new();
  cache.map.insert(
    "tracks".to_string(),
    CachedList {
      fetched_at: 0,
      total: ids.len() as u32,
      items: serde_json::to_value(ids).unwrap(),
    },
  );
  cache
}

fn seeded_playlist(items: &[PlaylistItem]) -> PlaylistCache {
  let mut cache = PlaylistCache::new();
  cache.map.insert(
    "p1".to_string(),
    CachedPlaylist {
      snapshot: "snap1".to_string(),
      fetched_at: 0,
      items: items.to_vec(),
      total: items.len() as u32,
    },
  );
  cache
}

fn bench_library_put(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("library_put");
  for n in [100, 2000] {
    let ids = track_ids(n);
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter_batched(
        LibraryCache::new,
        |mut cache| cache.put("tracks", black_box(&ids), n as u32),
        criterion::BatchSize::SmallInput,
      );
    });
  }
  group.finish();
}

fn bench_library_get_typed(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("library_get_typed");
  for n in [585, 10_000] {
    let cache = seeded_library(&track_ids(n));
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter(|| black_box(cache.get_typed::<String>("tracks")));
    });
  }
  group.finish();
}

fn bench_library_append(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("library_append");
  for n in [100, 2000] {
    let ids = track_ids(n);
    let delta = track_ids(50);
    let total = (n + 50) as u32;
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter_batched(
        || seeded_library(&ids),
        |mut cache| cache.append("tracks", black_box(&delta), total),
        criterion::BatchSize::SmallInput,
      );
    });
  }
  group.finish();
}

fn bench_playlist_update(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("playlist_update");
  for n in [100, 585] {
    let items = playlist_items(n);
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter_batched(
        PlaylistCache::new,
        |mut cache| cache.update("p1", black_box(items.clone()), n as u32, 0, false),
        criterion::BatchSize::SmallInput,
      );
    });
  }
  group.finish();
}

fn bench_playlist_lookup(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("playlist_lookup");
  for n in [585, 10_000] {
    let cache = seeded_playlist(&playlist_items(n));
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter(|| black_box(cache.lookup("p1")));
    });
  }
  group.finish();
}

fn bench_playlist_is_polluted(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("playlist_is_polluted");
  for n in [585, 10_000] {
    let cache = seeded_playlist(&playlist_items(n));
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter(|| black_box(cache.is_polluted("p1")));
    });
  }
  group.finish();
}

fn bench_playlist_remove_item(c: &mut Criterion) {
  isolate_home();
  let mut group = c.benchmark_group("playlist_remove_item");
  for n in [100, 585] {
    let items = playlist_items(n);
    let victim = format!("spotify:track:{:06}", n / 2);
    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
      b.iter_batched(
        || seeded_playlist(&items),
        |mut cache| cache.remove_item("p1", black_box(&victim), "snap2".to_string()),
        criterion::BatchSize::SmallInput,
      );
    });
  }
  group.finish();
}

fn bench_playlist_item_uri(c: &mut Criterion) {
  let one = item("spotify:track:000001");
  c.bench_function("playlist_item_uri", |b| {
    b.iter(|| black_box(playlist_item_uri(black_box(&one))));
  });
}

criterion_group!(
  benches,
  bench_library_put,
  bench_library_get_typed,
  bench_library_append,
  bench_playlist_update,
  bench_playlist_lookup,
  bench_playlist_is_polluted,
  bench_playlist_remove_item,
  bench_playlist_item_uri,
);
criterion_main!(benches);
