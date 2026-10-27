//! Now-playing cover art: fetch (Spotify CDN URLs), decode, disk-cache.
//!
//! The image bytes come from URLs the Web API already returns in
//! `album.images` — no new Spotify endpoint, artwork shown unmodified.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result};
use image::DynamicImage;

/// Disk cache TTL: covers are content-addressed by URL and effectively
/// immutable, but bound the disk use like the other caches.
pub const COVER_TTL_SECS: u64 = 30 * 86400;

/// Largest dimension fetched images are downscaled to before caching.
/// 640px matches the biggest `album.images` entry and is plenty for a
/// terminal box; keeps the disk cache small.
const COVER_MAX_PX: u32 = 640;

/// Pick the cover URL to fetch: the largest image available (Spotify serves
/// 640/300/64 — take the 640; Soloist supplies one undimensioned cover).
pub fn pick_url(images: &[rspotify::model::Image]) -> Option<String> {
  images
    .iter()
    .max_by_key(|i| i.width.unwrap_or(0))
    .map(|i| i.url.clone())
}

/// (item id, cover url) for the now-playing item, if it has both.
pub fn now_playing(item: Option<&rspotify::model::PlayableItem>) -> Option<(String, String)> {
  match item {
    Some(rspotify::model::PlayableItem::Track(t)) => {
      let id = t.id.as_ref().map(|id| id.to_string())?;
      let url = pick_url(&t.album.images)?;
      Some((format!("track:{id}"), url))
    }
    Some(rspotify::model::PlayableItem::Episode(e)) => {
      let url = pick_url(&e.images)?;
      Some((format!("episode:{}", e.id), url))
    }
    _ => None,
  }
}

/// Content-addressed cache file for a cover URL (raw downloaded bytes).
pub fn cache_file(url: &str) -> PathBuf {
  let mut h = DefaultHasher::new();
  url.hash(&mut h);
  let mut dir = crate::client_creds::app_config_dir();
  dir.push("covers");
  dir.push(format!("{:016x}.img", h.finish()));
  dir
}

/// Load a fresh-enough cached cover, if present.
pub fn load_cached(url: &str) -> Option<DynamicImage> {
  let path = cache_file(url);
  let bytes = std::fs::read(&path).ok()?;
  let fresh = path
    .metadata()
    .and_then(|m| m.modified())
    .ok()
    .and_then(|t| SystemTime::now().duration_since(t).ok())
    .is_some_and(|age| age.as_secs() < COVER_TTL_SECS);
  if !fresh {
    return None;
  }
  image::load_from_memory(&bytes).ok()
}

/// Download, downscale, cache, and decode a cover image.
pub async fn fetch(url: &str) -> Result<DynamicImage> {
  // ponytail: one shared client (connection pool) instead of a fresh
  // Client per fetch — Client::new() spawns a connection pool each call.
  static COVER_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
  let client = COVER_CLIENT.get_or_init(reqwest::Client::new);
  let bytes = client
    .get(url)
    .send()
    .await
    .with_context(|| format!("cover download failed for {url}"))?
    .error_for_status()?
    .bytes()
    .await?;
  let owned = bytes.to_vec();
  let img = tokio::task::spawn_blocking(move || -> Result<DynamicImage> {
    let img = image::load_from_memory(&owned)?;
    let img = img.resize(
      COVER_MAX_PX,
      COVER_MAX_PX,
      image::imageops::FilterType::Lanczos3,
    );
    Ok(img)
  })
  .await??;
  // Cache best-effort: a failed write must not fail the display.
  if let Some(parent) = cache_file(url).parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  if let Ok(f) = std::fs::File::create(cache_file(url)) {
    // Re-encode as JPEG (covers are photos; smaller than PNG on disk).
    let rgb = img.to_rgb8();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(f, 85);
    let _ = enc.encode_image(&rgb);
  }
  Ok(img)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn img(w: Option<u32>, url: &str) -> rspotify::model::Image {
    rspotify::model::Image {
      height: None,
      width: w,
      url: url.to_string(),
    }
  }

  #[test]
  fn picks_largest_width() {
    let images = vec![
      img(Some(64), "small"),
      img(Some(640), "large"),
      img(Some(300), "medium"),
    ];
    assert_eq!(pick_url(&images).as_deref(), Some("large"));
  }

  #[test]
  fn picks_first_cover_when_no_dimensions() {
    let images = vec![img(None, "a"), img(None, "b")];
    assert!(pick_url(&images).is_some());
  }

  #[test]
  fn empty_images_pick_none() {
    assert_eq!(pick_url(&[]), None);
  }

  #[test]
  fn cache_file_is_stable_and_hex() {
    let a = cache_file("https://i.scdn.co/image/abc");
    let b = cache_file("https://i.scdn.co/image/abc");
    assert_eq!(a, b);
    assert_ne!(a, cache_file("https://i.scdn.co/image/abd"));
    assert_eq!(a.extension().and_then(|e| e.to_str()), Some("img"));
  }

  #[test]
  fn missing_cache_loads_none() {
    assert!(load_cached("https://i.scdn.co/image/does-not-exist-12345").is_none());
  }
}
