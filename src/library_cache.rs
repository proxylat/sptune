use std::{
  collections::HashMap,
  fs,
  path::PathBuf,
  sync::atomic::{AtomicBool, Ordering},
  time::{SystemTime, UNIX_EPOCH},
};

/// Set false by `sptune --no-cache`: every cache read/write is skipped for
/// the run, so the app behaves as if the cache did not exist.
pub static CACHE_ENABLED: AtomicBool = AtomicBool::new(true);

use serde::{Deserialize, Serialize};

const CONFIG_DIR: &str = ".config";
const APP_CONFIG_DIR: &str = "sptune";
const CACHE_FILE: &str = "library_cache.json";

/// Disk cache of whole "my library" lists (playlists, saved
/// tracks/albums/shows), keyed by endpoint. Entries never expire; the
/// screen is served from cache and merged with a delta probe on open, so a
/// throttled endpoint never blanks the UI.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CachedList {
  pub fetched_at: u64,
  #[serde(default)]
  pub total: u32,
  pub items: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct LibraryCache {
  path: PathBuf,
  map: HashMap<String, CachedList>,
  loaded: bool,
}

impl LibraryCache {
  pub fn new() -> Self {
    Self {
      path: dirs::home_dir()
        .map(|home| home.join(CONFIG_DIR).join(APP_CONFIG_DIR).join(CACHE_FILE))
        .unwrap_or_default(),
      map: HashMap::new(),
      loaded: false,
    }
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
      if let Ok(map) = serde_json::from_str::<HashMap<String, CachedList>>(&contents) {
        self.map = map;
      }
    }
    self.loaded = true;
  }

  pub fn get(&self, key: &str) -> Option<&CachedList> {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return None;
    }
    self.map.get(key)
  }

  /// Cached items deserialized to `T`, plus the stored total.
  pub fn get_typed<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<(Vec<T>, u32)> {
    let entry = self.map.get(key)?;
    serde_json::from_value::<Vec<T>>(entry.items.clone())
      .ok()
      .map(|items| (items, entry.total))
  }

  pub fn put<T: Serialize>(&mut self, key: &str, items: &[T], total: u32) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    let Ok(items) = serde_json::to_value(items) else {
      return;
    };
    let entry = self.map.entry(key.to_string()).or_default();
    entry.fetched_at = now_secs();
    entry.items = items;
    entry.total = total;
    self.save();
  }

  pub fn append<T: Serialize>(&mut self, key: &str, items: &[T], total: u32) {
    self.merge(key, items, total, true);
  }

  /// Prepend a delta page (newest-first lists) in place.
  pub fn prepend<T: Serialize>(&mut self, key: &str, items: &[T], total: u32) {
    self.merge(key, items, total, false);
  }

  fn merge<T: Serialize>(&mut self, key: &str, items: &[T], total: u32, append: bool) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    let entry = self.map.entry(key.to_string()).or_default();
    let mut merged: Vec<serde_json::Value> = match entry.items {
      serde_json::Value::Array(ref arr) => arr.clone(),
      _ => Vec::new(),
    };
    if let Ok(added) = serde_json::to_value(items) {
      if let Ok(mut added) = serde_json::from_value::<Vec<serde_json::Value>>(added) {
        if append {
          merged.extend(added);
        } else {
          added.extend(merged);
          merged = added;
        }
      }
    }
    entry.items = serde_json::to_value(merged).unwrap_or(entry.items.clone());
    entry.fetched_at = now_secs();
    entry.total = total;
    self.save();
  }

  pub fn remove(&mut self, key: &str) {
    if !CACHE_ENABLED.load(Ordering::Relaxed) {
      return;
    }
    self.ensure_loaded();
    self.map.remove(key);
    self.save();
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
      let _ = fs::write(&self.path, json);
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

  #[test]
  fn cache_roundtrip_merge_and_remove() {
    let dir = std::env::temp_dir().join(format!("sptune_libcache_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut cache = LibraryCache::new();
    cache.path = dir.join(CACHE_FILE);

    assert!(cache.get_typed::<u32>("likes").is_none());
    cache.put("likes", &[3u32, 4, 5], 3);
    cache.prepend("likes", &[1, 2], 5);
    cache.append("likes", &[6], 6);
    assert_eq!(
      cache.get_typed::<u32>("likes"),
      Some((vec![1, 2, 3, 4, 5, 6], 6))
    );

    cache.remove("likes");
    assert!(cache.get_typed::<u32>("likes").is_none());

    let mut reloaded = LibraryCache::new();
    reloaded.path = dir.join(CACHE_FILE);
    reloaded.ensure_loaded();
    assert!(reloaded.get_typed::<u32>("likes").is_none());

    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn clear_wipes_disk_and_memory() {
    let dir = std::env::temp_dir().join(format!("sptune_libcache_clear_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut cache = LibraryCache::new();
    cache.path = dir.join(CACHE_FILE);

    cache.put("likes", &[1u32, 2], 2);
    cache.clear();
    assert!(cache.get_typed::<u32>("likes").is_none());
    assert!(!cache.path.exists());

    let mut reloaded = LibraryCache::new();
    reloaded.path = dir.join(CACHE_FILE);
    reloaded.ensure_loaded();
    assert!(reloaded.get_typed::<u32>("likes").is_none());

    let _ = fs::remove_dir_all(&dir);
  }
}