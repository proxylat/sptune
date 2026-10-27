use super::runner::CliApp;

use anyhow::{anyhow, Result};
use rspotify::{
  clients::{BaseClient, OAuthClient},
  model::{Id, Market, PlayableItem, PlaylistId},
};
use std::{collections::HashSet, path::PathBuf, time::Duration};

const HEADER: &str = "Track URI,Track Name,Album Name,Artist Name(s),Release Date,Duration (ms),Explicit,Added By,Added At";

/// One exported playlist item, in both its CSV (flat strings) and JSON
/// (typed values) shapes. Built by [`track_fields_row`].
struct BackupRow {
  csv: [String; 10],
  json: serde_json::Value,
}

/// The ten fields of one playlist item. Bundled so `track_fields_row` stays
/// under clippy's `too_many_arguments` threshold.
struct TrackFields {
  uri: String,
  name: String,
  album: String,
  artists: String,
  release_date: String,
  duration_ms: i64,
  explicit: bool,
  added_by: String,
  added_at: String,
}

impl<'a> CliApp<'a> {
  /// Export every playlist + Liked Songs to CSV (and optionally JSON).
  /// Metadata only, never audio. Progress goes to stderr so stdout stays
  /// a single summary line for scripts.
  pub async fn backup(&mut self, dir: PathBuf, write_json: bool) -> Result<String> {
    std::fs::create_dir_all(&dir)
      .map_err(|e| anyhow!("cannot create dir '{}': {}", dir.display(), e))?;
    let mut taken: HashSet<String> = HashSet::new();
    let mut playlists_done = 0u32;
    let mut tracks_total = 0u64;
    let mut failed: Vec<String> = Vec::new();

    let mut all = Vec::new();
    loop {
      let page = self
        .net
        .spotify
        .current_user_playlists_manual(Some(50), Some(all.len() as u32))
        .await
        .map_err(|e| anyhow!("failed to list playlists: {}", e))?;
      let total = page.total;
      all.extend(page.items);
      if all.len() as u32 >= total {
        break;
      }
      tokio::time::sleep(Duration::from_millis(200)).await;
    }

    match self.fetch_all_saved_tracks().await {
      Ok(rows) if !rows.is_empty() => {
        let n = rows.len() as u64;
        let path = self.write_playlist(&dir, "Liked Songs", &rows, write_json, &mut taken)?;
        eprintln!("Saved {} ({} tracks)", path.display(), n);
        playlists_done += 1;
        tracks_total += n;
      }
      Ok(_) => {}
      Err(e) => {
        eprintln!("Couldn't export Liked Songs: {}", e);
        failed.push("Liked Songs".to_string());
      }
    }

    for p in &all {
      let rows = match self.fetch_all_playlist_items(&p.id.uri()).await {
        Ok(r) => r,
        Err(e) if e.to_string().contains("404") => {
          eprintln!(
            "Skipped '{}': algorithmic mixes aren't exposed by the Web API (404)",
            p.name
          );
          continue;
        }
        Err(e) => {
          eprintln!("Couldn't export {}: {}", p.name, e);
          failed.push(p.name.clone());
          continue;
        }
      };
      let n = rows.len() as u64;
      let path = self.write_playlist(&dir, &p.name, &rows, write_json, &mut taken)?;
      eprintln!("Saved {} ({} tracks)", path.display(), n);
      playlists_done += 1;
      tracks_total += n;
    }

    let mut summary = format!(
      "Backup complete: {} playlists, {} tracks in {}",
      playlists_done,
      tracks_total,
      dir.display()
    );
    if !failed.is_empty() {
      summary.push_str(&format!(
        " ({} failed: {})",
        failed.len(),
        failed.join(", ")
      ));
    }
    Ok(summary)
  }

  async fn fetch_all_saved_tracks(&mut self) -> Result<Vec<BackupRow>> {
    let mut rows = Vec::new();
    let mut skipped = 0u32;
    loop {
      let page = self
        .net
        .spotify
        .current_user_saved_tracks_manual(
          Some(Market::FromToken),
          Some(50),
          Some(rows.len() as u32 + skipped),
        )
        .await
        .map_err(|e| anyhow!(e))?;
      let total = page.total;
      for item in &page.items {
        let t = &item.track;
        let Some(uri) = t.id.as_ref().map(|id| id.uri()) else {
          skipped += 1;
          continue;
        };
        rows.push(track_fields_row(TrackFields {
          uri,
          name: t.name.clone(),
          album: t.album.name.clone(),
          artists: t
            .artists
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(";"),
          release_date: t.album.release_date.clone().unwrap_or_default(),
          duration_ms: t.duration.num_milliseconds(),
          explicit: t.explicit,
          added_by: String::new(),
          added_at: item.added_at.to_rfc3339(),
        }));
      }
      if rows.len() as u32 + skipped >= total {
        break;
      }
      tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if skipped > 0 {
      eprintln!("  Liked Songs: {} items without metadata skipped", skipped);
    }
    Ok(rows)
  }

  async fn fetch_all_playlist_items(&mut self, uri: &str) -> Result<Vec<BackupRow>> {
    let id = PlaylistId::from_id_or_uri(uri).map_err(|e| anyhow!(e))?;
    let mut rows = Vec::new();
    let mut skipped = 0u32;
    loop {
      let page = self
        .net
        .spotify
        .playlist_items_manual(
          id.clone(),
          None,
          Some(Market::FromToken),
          Some(50),
          Some(rows.len() as u32 + skipped),
        )
        .await
        .map_err(|e| anyhow!(e))?;
      let total = page.total;
      for item in &page.items {
        let added_by = item
          .added_by
          .as_ref()
          .map(|u| u.id.id().to_string())
          .unwrap_or_default();
        let added_at = item.added_at.map(|d| d.to_rfc3339()).unwrap_or_default();
        match item.item.as_ref() {
          Some(PlayableItem::Track(t)) => {
            let Some(uri) = t.id.as_ref().map(|id| id.uri()) else {
              skipped += 1;
              continue;
            };
            rows.push(track_fields_row(TrackFields {
              uri,
              name: t.name.clone(),
              album: t.album.name.clone(),
              artists: t
                .artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join(";"),
              release_date: t.album.release_date.clone().unwrap_or_default(),
              duration_ms: t.duration.num_milliseconds(),
              explicit: t.explicit,
              added_by,
              added_at,
            }));
          }
          Some(PlayableItem::Episode(e)) => {
            rows.push(track_fields_row(TrackFields {
              uri: e.id.uri(),
              name: e.name.clone(),
              album: e.show.name.clone(),
              artists: String::new(),
              release_date: e.release_date.clone(),
              duration_ms: e.duration.num_milliseconds(),
              explicit: e.explicit,
              added_by,
              added_at,
            }));
          }
          _ => skipped += 1,
        }
      }
      if rows.len() as u32 + skipped >= total {
        break;
      }
      tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(rows)
  }

  fn write_playlist(
    &self,
    dir: &PathBuf,
    name: &str,
    rows: &[BackupRow],
    write_json: bool,
    taken: &mut HashSet<String>,
  ) -> Result<PathBuf> {
    let stem = unique_stem(name, taken);
    let path = dir.join(format!("{}.csv", stem));
    let mut out = String::with_capacity(rows.len() * 128 + HEADER.len() + 2);
    out.push('\u{FEFF}');
    out.push_str(HEADER);
    out.push('\n');
    let mut lines = String::with_capacity(rows.len() * 128);
    for r in rows {
      let escaped: Vec<String> = r.csv.iter().map(|f| escape_csv(f)).collect();
      lines.push_str(&escaped.join(","));
      lines.push('\n');
    }
    out.push_str(&lines);
    std::fs::write(&path, out).map_err(|e| anyhow!("cannot write '{}': {}", path.display(), e))?;
    if write_json {
      let jpath = dir.join(format!("{}.json", stem));
      let doc = serde_json::json!({
        "playlist": name,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "total": rows.len(),
        "tracks": rows.iter().map(|r| &r.json).collect::<Vec<_>>(),
      });
      let text = serde_json::to_string_pretty(&doc).map_err(|e| anyhow!(e))?;
      std::fs::write(&jpath, text)
        .map_err(|e| anyhow!("cannot write '{}': {}", jpath.display(), e))?;
    }
    Ok(path)
  }
}

fn track_fields_row(f: TrackFields) -> BackupRow {
  let csv = [
    f.uri.clone(),
    f.name,
    f.album,
    f.artists.clone(),
    f.release_date.clone(),
    f.duration_ms.to_string(),
    if f.explicit {
      "True".to_string()
    } else {
      "False".to_string()
    },
    f.added_by.clone(),
    f.added_at.clone(),
  ];
  let json = serde_json::json!({
    "uri": f.uri,
    "name": f.name,
    "album": f.album,
    "artists": f.artists.split(';').filter(|s| !s.is_empty()).collect::<Vec<_>>(),
    "release_date": f.release_date,
    "duration_ms": f.duration_ms,
    "explicit": f.explicit,
    "added_by": f.added_by,
    "added_at": f.added_at,
  });
  BackupRow { csv, json }
}

fn escape_csv(field: &str) -> String {
  if field.contains(['"', ',', '\n', '\r']) {
    format!("\"{}\"", field.replace('"', "\"\""))
  } else {
    field.to_string()
  }
}

fn sanitize_stem(name: &str) -> String {
  let mut s: String = name
    .chars()
    .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    .filter(|c| !c.is_control())
    .collect();
  while s.ends_with(['.', ' ']) {
    s.pop();
  }
  s = s.replace(' ', "_");
  if s.is_empty() {
    s = "playlist".to_string();
  }
  if s.chars().count() > 100 {
    s = s.chars().take(100).collect();
  }
  s
}

fn unique_stem(name: &str, taken: &mut HashSet<String>) -> String {
  let base = sanitize_stem(name);
  if taken.insert(base.clone()) {
    return base;
  }
  let mut n = 2u32;
  loop {
    let cand = format!("{}_{}", base, n);
    if taken.insert(cand.clone()) {
      return cand;
    }
    n += 1;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sanitize_strips_illegal_chars() {
    assert_eq!(sanitize_stem("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
    assert_eq!(sanitize_stem("My Playlist"), "My_Playlist");
    assert_eq!(sanitize_stem("trailing. "), "trailing");
    assert_eq!(sanitize_stem(""), "playlist");
    assert_eq!(sanitize_stem("???"), "playlist");
  }

  #[test]
  fn unique_stem_dedups() {
    let mut taken = HashSet::new();
    assert_eq!(unique_stem("Mix", &mut taken), "Mix");
    assert_eq!(unique_stem("Mix", &mut taken), "Mix_2");
    assert_eq!(unique_stem("Mix", &mut taken), "Mix_3");
  }

  #[test]
  fn csv_escapes_quotes_commas() {
    assert_eq!(escape_csv("plain"), "plain");
    assert_eq!(escape_csv("a,b"), "\"a,b\"");
    assert_eq!(escape_csv("say \"hi\""), "\"say \"\"hi\"\"\"");
  }

  #[test]
  fn explicit_matches_python_capitalization() {
    let row = track_fields_row(TrackFields {
      uri: "spotify:track:x".to_string(),
      name: "t".to_string(),
      album: "a".to_string(),
      artists: "art".to_string(),
      release_date: "2020-01-01".to_string(),
      duration_ms: 1000,
      explicit: true,
      added_by: String::new(),
      added_at: String::new(),
    });
    assert_eq!(row.csv[6], "True");
  }
}
