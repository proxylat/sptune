//! Optional external music metadata enrichment via MusicBrainz (zero-config)
//! and Last.fm (when `behavior.lastfm_api_key` is configured).

use serde::Deserialize;

const USER_AGENT: &str = "sptune/2.0.0 (https://github.com/proxy-works/sptune)";

#[derive(Clone, Debug, Default)]
pub struct ArtistMeta {
  pub country: Option<String>,
  pub kind: Option<String>,
  pub tags: Vec<String>,
  pub bio: Option<String>,
  pub listeners: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
  pub listeners: Option<u64>,
  pub playcount: Option<u64>,
}

#[derive(Deserialize)]
struct MbArtistSearch {
  #[serde(default)]
  artists: Vec<MbArtist>,
}

#[derive(Deserialize)]
struct MbArtist {
  name: String,
  country: Option<String>,
  #[serde(rename = "type")]
  kind: Option<String>,
  #[serde(default)]
  tags: Vec<MbTag>,
}

#[derive(Deserialize)]
struct MbTag {
  name: String,
}

#[derive(Deserialize)]
struct LfmArtistResp {
  artist: Option<LfmArtist>,
}

#[derive(Deserialize)]
struct LfmArtist {
  stats: Option<LfmStats>,
  tags: Option<LfmTags>,
  bio: Option<LfmBio>,
}

#[derive(Deserialize)]
struct LfmStats {
  listeners: Option<String>,
}

#[derive(Deserialize)]
struct LfmTags {
  #[serde(default)]
  tag: Vec<MbTag>,
}

#[derive(Deserialize)]
struct LfmBio {
  summary: Option<String>,
}

#[derive(Deserialize)]
struct LfmTrackResp {
  track: Option<LfmTrack>,
}

#[derive(Deserialize)]
struct LfmTrack {
  listeners: Option<String>,
  playcount: Option<String>,
}

fn strip_html_tags(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut in_tag = false;
  for ch in s.chars() {
    match ch {
      '<' => in_tag = true,
      '>' => in_tag = false,
      _ if !in_tag => out.push(ch),
      _ => {}
    }
  }
  out
    .split("Read more on Last.fm")
    .next()
    .unwrap_or(&out)
    .trim()
    .to_string()
}

pub fn format_compact_count(n: u64) -> String {
  if n >= 1_000_000 {
    format!("{:.1}M", n as f64 / 1_000_000.0)
  } else if n >= 1_000 {
    format!("{:.1}K", n as f64 / 1_000.0)
  } else {
    n.to_string()
  }
}

pub async fn fetch_artist_meta(name: &str, lastfm_key: Option<&str>) -> Option<ArtistMeta> {
  if name.trim().is_empty() {
    return None;
  }
  let client = crate::backend::shared_http_client();
  if let Some(key) = lastfm_key.filter(|k| !k.trim().is_empty()) {
    let resp = client
      .get("https://ws.audioscrobbler.com/2.0/")
      .query(&[
        ("method", "artist.getinfo"),
        ("artist", name),
        ("api_key", key),
        ("format", "json"),
      ])
      .header("User-Agent", USER_AGENT)
      .send()
      .await
      .ok()?
      .json::<LfmArtistResp>()
      .await
      .ok()?;
    if let Some(a) = resp.artist {
      return Some(ArtistMeta {
        country: None,
        kind: None,
        tags: a
          .tags
          .map(|t| t.tag.into_iter().take(4).map(|x| x.name).collect())
          .unwrap_or_default(),
        bio: a
          .bio
          .and_then(|b| b.summary)
          .as_deref()
          .map(strip_html_tags)
          .filter(|s| !s.is_empty()),
        listeners: a
          .stats
          .and_then(|s| s.listeners)
          .and_then(|v| v.parse::<u64>().ok()),
      });
    }
  }

  let query = format!("artist:\"{name}\"");
  let resp = client
    .get("https://musicbrainz.org/ws/2/artist/")
    .query(&[("query", query.as_str()), ("limit", "5"), ("fmt", "json")])
    .header("User-Agent", USER_AGENT)
    .send()
    .await
    .ok()?
    .json::<MbArtistSearch>()
    .await
    .ok()?;
  let first = resp
    .artists
    .into_iter()
    .find(|a| a.name.eq_ignore_ascii_case(name.trim()))?;
  Some(ArtistMeta {
    country: first.country,
    kind: first.kind,
    tags: first.tags.into_iter().take(4).map(|t| t.name).collect(),
    bio: None,
    listeners: None,
  })
}

pub async fn fetch_track_meta(
  artist: &str,
  track: &str,
  lastfm_key: Option<&str>,
) -> Option<TrackMeta> {
  let key = lastfm_key.filter(|k| !k.trim().is_empty())?;
  let client = crate::backend::shared_http_client();
  let resp = client
    .get("https://ws.audioscrobbler.com/2.0/")
    .query(&[
      ("method", "track.getinfo"),
      ("artist", artist),
      ("track", track),
      ("api_key", key),
      ("format", "json"),
    ])
    .header("User-Agent", USER_AGENT)
    .send()
    .await
    .ok()?
    .json::<LfmTrackResp>()
    .await
    .ok()?;
  let t = resp.track?;
  Some(TrackMeta {
    listeners: t.listeners.and_then(|v| v.parse::<u64>().ok()),
    playcount: t.playcount.and_then(|v| v.parse::<u64>().ok()),
  })
}
