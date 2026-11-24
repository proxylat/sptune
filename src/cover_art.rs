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

/// Disk cache TTL: temporary 7-day cache for cover art (Spotify Terms §IV.3.b).
pub const COVER_TTL_SECS: u64 = 7 * 86400;

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
/// Direct CDN fetch + fast target-size PNG encoding is already fast enough
/// that a separate 64px LQIP pre-pass is unnecessary.
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
  let mut dir = dirs::home_dir()
    .map(|h| h.join(".config").join("sptune"))
    .unwrap_or_default();
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

/// Download, decode, and asynchronously cache a cover image.
pub async fn fetch(url: &str) -> Result<DynamicImage> {
  let bytes = crate::backend::shared_http_client()
    .get(url)
    .send()
    .await
    .with_context(|| format!("cover download failed for {url}"))?
    .error_for_status()?
    .bytes()
    .await?;
  let owned = bytes.to_vec();
  let for_disk = owned.clone();
  let cache_path = cache_file(url);
  // Write raw CDN bytes to disk in the background so display never waits on disk I/O or re-encoding.
  tokio::spawn(async move {
    let _ = tokio::task::spawn_blocking(move || {
      if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
      }
      let _ = std::fs::write(cache_path, &for_disk);
    })
    .await;
  });
  let img = tokio::task::spawn_blocking(move || -> Result<DynamicImage> {
    let img = image::load_from_memory(&owned)?;
    if img.width() > COVER_MAX_PX || img.height() > COVER_MAX_PX {
      Ok(img.resize(
        COVER_MAX_PX,
        COVER_MAX_PX,
        image::imageops::FilterType::Triangle,
      ))
    } else {
      Ok(img)
    }
  })
  .await??;
  Ok(img)
}

const KITTY_COVER_ID: u32 = 1;
pub const KITTY_PLAYBAR_COVER_ID: u32 = 2;

fn kitty_apc(payload: &str, is_tmux: bool) -> String {
  if is_tmux {
    format!("\x1bPtmux;\x1b\x1b_G{payload}\x1b\x1b\\\x1b\\")
  } else {
    format!("\x1b_G{payload}\x1b\\")
  }
}

/// Delete a placed Kitty image by ID without touching text cells.
pub fn kitty_delete_id_seq(id: u32, is_tmux: bool) -> String {
  kitty_apc(&format!("a=d,d=i,i={id},q=2"), is_tmux)
}

/// Delete the placed Kitty cover image without touching text cells.
pub fn kitty_delete_seq(is_tmux: bool) -> String {
  kitty_delete_id_seq(KITTY_COVER_ID, is_tmux)
}

/// Resize `img` to target terminal pixel bounds (when larger), fast-encode as PNG,
/// and build chunked Kitty direct-placement sequences for `id`.
pub fn kitty_place_id_seq(
  id: u32,
  img: &DynamicImage,
  cols: u16,
  rows: u16,
  font_px: (u16, u16),
  is_tmux: bool,
) -> Result<String> {
  use base64::Engine as _;
  use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
  use image::ImageEncoder as _;
  use std::fmt::Write as _;

  let target_w = (cols as u32 * font_px.0.max(1) as u32).max(1);
  let target_h = (rows as u32 * font_px.1.max(1) as u32).max(1);
  let resized = if img.width() > target_w || img.height() > target_h {
    std::borrow::Cow::Owned(img.resize(target_w, target_h, image::imageops::FilterType::Triangle))
  } else {
    std::borrow::Cow::Borrowed(img)
  };
  let rgb = resized.to_rgb8();
  let mut png = Vec::with_capacity((rgb.width() * rgb.height()) as usize);
  let enc = PngEncoder::new_with_quality(&mut png, CompressionType::Fast, PngFilter::Sub);
  enc.write_image(
    rgb.as_raw(),
    rgb.width(),
    rgb.height(),
    image::ExtendedColorType::Rgb8,
  )?;
  let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
  let chunks: Vec<&str> = b64
    .as_bytes()
    .chunks(4096)
    .map(|c| std::str::from_utf8(c).unwrap_or_default())
    .collect();
  let mut out = String::with_capacity(b64.len() + chunks.len() * 48);
  let total = chunks.len();
  for (idx, chunk) in chunks.into_iter().enumerate() {
    let more = u8::from(idx + 1 < total);
    let body = if idx == 0 {
      format!("a=T,f=100,t=d,i={id},c={cols},r={rows},C=1,q=2,m={more};{chunk}")
    } else {
      format!("m={more};{chunk}")
    };
    let _ = write!(out, "{}", kitty_apc(&body, is_tmux));
  }
  Ok(out)
}

/// Resize `img` to target terminal pixel bounds (when larger), fast-encode as PNG,
/// and build chunked Kitty direct-placement sequences
/// (`a=T,f=100,t=d,i=1,c=cols,r=rows,C=1,q=2`).
pub fn kitty_place_seq(
  img: &DynamicImage,
  cols: u16,
  rows: u16,
  font_px: (u16, u16),
  is_tmux: bool,
) -> Result<String> {
  kitty_place_id_seq(KITTY_COVER_ID, img, cols, rows, font_px, is_tmux)
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

  #[test]
  fn kitty_delete_seq_formats_plain_and_tmux() {
    assert_eq!(kitty_delete_seq(false), "\x1b_Ga=d,d=i,i=1,q=2\x1b\\");
    assert_eq!(
      kitty_delete_seq(true),
      "\x1bPtmux;\x1b\x1b_Ga=d,d=i,i=1,q=2\x1b\x1b\\\x1b\\"
    );
  }

  #[test]
  fn kitty_place_seq_chunks_png_and_sets_cell_bounds() {
    let img = DynamicImage::new_rgb8(64, 64);
    let seq = kitty_place_seq(&img, 24, 12, (9, 18), false).unwrap();
    assert!(seq.starts_with("\x1b_Ga=T,f=100,t=d,i=1,c=24,r=12,C=1,q=2,m="));
    assert!(seq.ends_with("\x1b\\"));
    assert!(seq.contains("m=0;"));
  }
}
