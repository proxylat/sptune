use std::{
  collections::HashMap,
  fs,
  path::PathBuf,
  time::{SystemTime, UNIX_EPOCH},
};

use crate::library_cache::CACHE_ENABLED;
use rspotify::model::{Id, PlaylistItem};
use serde::{Deserialize, Serialize};

use std::sync::atomic::Ordering;

const CONFIG_DIR: &str = ".config";
const APP_CONFIG_DIR: &str = "sptune";
// v2: entries written before the added_at parse fix have no dates; bumping
// the filename drops them so every playlist refetches with dates.
const CACHE_FILE: &str = "playlist_cache_v2.json";
const MAX_CACHED_PLAYLISTS: usize = 20;

/// Stable identity of a playlist item (track or episode uri).
pub fn playlist_item_uri(item: &PlaylistItem) -> Option<String> {
  match &item.item {
    Some(rspotify::model::PlayableItem::Track(t)) => t.id.as_ref().map(|id| id.uri()),
    Some(rspotify::model::PlayableItem::Episode(e)) => Some(e.id.uri()),
    _ => None,
  }
}

/// Cached track list of a playlist, keyed by playlist id. Entries never
/// expire; the screen is served from cache and merged with a delta probe
// ponytail: terms — only PlaylistItem/FullTrack metadata, no audio features; reconciled via snapshot_id tail probe (backend.rs:1787); SAVED_CHECK_TTL 300s (backend.rs:643) for ❤️
/// (tail fetch) on open, so a changed playlist picks up new tracks only.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CachedPlaylist {
  pub snapshot: String,
  pub fetched_at: u64,
  #[serde(default)]
  pub items: Vec<PlaylistItem>,
  #[serde(default)]
  pub total: u32,
}

#[derive(Clone, Debug)]
pub struct PlaylistCache {
  path: PathBuf,
  pub map: HashMap<String, CachedPlaylist>,
  loaded: bool,
}

impl PlaylistCache {
  pub fn new() -> Self {
    Self {
      path: dirs::home_dir()
        .map(|home| home.join(CONFIG_DIR).join(APP_CONFIG_DIR).join(CACHE_FILE))
        .unwrap_or_default(),
      map: HashMap::new(),
      loaded: false,
    }
  }

  /// Redirect the backing file (used by tests; the field is private).
  #[cfg(test)]
  pub(crate) fn set_path(&mut self, path: std::path::PathBuf) {
    self.path = path;
    self.loaded = false;
  }

  pub fn ensure_loaded(&mut self) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      self.loaded = true;
      return;
    }
    if self.loaded {
      return;
    }
    if let Ok(contents) = fs::read_to_string(&self.path) {
      if let Ok(map) = serde_json::from_str::<HashMap<String, CachedPlaylist>>(&contents) {
        self.map = map;
      }
    }
    self.loaded = true;
  }

  pub fn lookup(&self, playlist_id: &str) -> Option<&CachedPlaylist> {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return None;
    }
    self.map.get(playlist_id)
  }

  /// Accumulate a fetched page. `append` is true for load-more/delta pages.
  pub fn update(&mut self, playlist_id: &str, items: Vec<PlaylistItem>, total: u32, append: bool) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    // ponytail: cap playlists (LRU by fetched_at), prevents unbounded HashMap growth
    if !self.map.contains_key(playlist_id) && self.map.len() >= MAX_CACHED_PLAYLISTS {
      if let Some(oldest) = self
        .map
        .iter()
        .min_by_key(|(_, v)| v.fetched_at)
        .map(|(k, _)| k.clone())
      {
        self.map.remove(&oldest);
      }
    }
    let entry = self.map.entry(playlist_id.to_string()).or_default();
    entry.fetched_at = now_secs();
    if append {
      entry.items.extend(items);
      if total > entry.total {
        entry.total = total;
      }
    } else {
      entry.items = items;
      entry.total = total;
    }
    self.save();
  }

  /// Record the playlist's snapshot id (changes on any edit; the open-path
  /// reconcile uses it to detect stale entries without a full re-fetch).
  pub fn set_snapshot(&mut self, playlist_id: &str, snapshot: String) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    if let Some(entry) = self.map.get_mut(playlist_id) {
      entry.snapshot = snapshot;
      self.save();
    }
  }

  pub fn remove(&mut self, playlist_id: &str) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    self.map.remove(playlist_id);
    self.save();
  }

  /// Drop a single item (by uri) from a cached playlist and record the new
  /// snapshot id. Used after a successful remove-from-playlist so the local
  /// view updates without a full refetch.
  pub fn remove_item(&mut self, playlist_id: &str, uri: &str, snapshot: String) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    if let Some(entry) = self.map.get_mut(playlist_id) {
      entry.snapshot = snapshot;
      entry.total = entry.total.saturating_sub(1);
      entry
        .items
        .retain(|item| playlist_item_uri(item).as_deref() != Some(uri));
      self.save();
    }
  }

  /// A cache entry is polluted when the same track/episode appears more than
  /// once (older sessions double-appended pages). Polluted entries must not
  /// be served: the raw_index mapping (displayed row -> playlist position)
  /// breaks, so clicks play the wrong song.
  pub fn is_polluted(&self, playlist_id: &str) -> bool {
    let Some(entry) = self.lookup(playlist_id) else {
      return false;
    };
    let mut seen = std::collections::HashSet::new();
    for item in &entry.items {
      let key = playlist_item_uri(item);
      if let Some(key) = key {
        if !seen.insert(key) {
          return true;
        }
      }
    }
    false
  }

  /// Delete everything: drop the in-memory map and remove the file from disk.
  /// Not affected by the no-cache flag — a disabled cache can still be
  /// wiped when the user asks for it.
  pub fn clear(&mut self) {
    self.map.clear();
    self.loaded = true;
    let _ = fs::remove_file(&self.path);
  }

  fn save(&self) {
    if let Ok(json) = serde_json::to_string(&self.map) {
      // ponytail: async disk to not block Network thread; sync in tests for determinism
      if cfg!(test) {
        let _ = fs::write(&self.path, json);
      } else {
        let path = self.path.clone();
        std::thread::spawn(move || {
          let _ = fs::write(path, json);
        });
      }
    }
  }
}

fn now_secs() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::*;

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

  #[test]
  fn cache_roundtrip_and_accumulate() {
    let dir = std::env::temp_dir().join(format!("sptune_cache_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut cache = PlaylistCache::new();
    cache.path = dir.join(CACHE_FILE);

    cache.update("p1", vec![item("spotify:track:t1")], 1, false);
    cache.update("p1", vec![item("spotify:track:t2")], 2, true);
    let entry = cache.lookup("p1").unwrap();
    assert_eq!(entry.items.len(), 2);

    let mut reloaded = PlaylistCache::new();
    reloaded.path = dir.join(CACHE_FILE);
    reloaded.ensure_loaded();
    let entry = reloaded.lookup("p1").unwrap();
    assert_eq!(entry.items.len(), 2);

    assert!(cache.lookup("p2").is_none());
    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn is_polluted_detects_duplicate_items() {
    let mut cache = PlaylistCache::new();
    cache.map.insert(
      "p1".to_string(),
      CachedPlaylist {
        items: vec![
          item("spotify:track:t1"),
          item("spotify:track:t2"),
          item("spotify:track:t1"),
        ],
        ..Default::default()
      },
    );
    cache.map.insert(
      "p2".to_string(),
      CachedPlaylist {
        items: vec![item("spotify:track:t1"), item("spotify:track:t2")],
        ..Default::default()
      },
    );
    assert!(cache.is_polluted("p1"));
    assert!(!cache.is_polluted("p2"));
    assert!(!cache.is_polluted("missing"));
  }
}
