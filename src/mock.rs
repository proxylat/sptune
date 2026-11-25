//! Offline showcase data for `--mock` mode (screenshots / demos).
//! Contains zero personal data, makes zero network calls, and never touches
//! saved state or token caches.

use crate::app::{ActiveBlock, App, RouteId, TrackTableContext};
use chrono::TimeDelta;
use image::{DynamicImage, RgbImage};
use rspotify::model::{
  CurrentPlaybackContext, CurrentlyPlayingType, Device, DeviceType, FullTrack, Page, PlayableItem,
  PrivateUser, RepeatState, SimplifiedPlaylist,
};
use serde_json::json;

fn mock_cover_image() -> DynamicImage {
  let (w, h) = (256u32, 256u32);
  let mut img = RgbImage::new(w, h);
  for y in 0..h {
    let fy = y as f32 / h as f32;
    for x in 0..w {
      let fx = x as f32 / w as f32;
      let dx = fx - 0.5;
      let dy = fy - 0.42;
      let dist = (dx * dx + dy * dy).sqrt();
      let (r, g, b) = if dist < 0.24 && (y % 10 < 7 || fy < 0.40) {
        let t = (fy / 0.65).clamp(0.0, 1.0);
        (255, (190.0 - 110.0 * t) as u8, (90.0 + 70.0 * t) as u8)
      } else if fy > 0.64 {
        let grid = (x % 24 == 0) || (y % 16 == 0);
        if grid {
          (180, 60, 220)
        } else {
          (18, 12, 38)
        }
      } else {
        (
          (24.0 + 48.0 * fy) as u8,
          (16.0 + 18.0 * fy) as u8,
          (64.0 + 80.0 * (1.0 - fy)) as u8,
        )
      };
      img.put_pixel(x, y, image::Rgb([r, g, b]));
    }
  }
  DynamicImage::ImageRgb8(img)
}

fn mock_track(
  track_id: &str,
  album_id: &str,
  artist_id: &str,
  name: &str,
  artist: &str,
  album: &str,
  duration_ms: i64,
) -> FullTrack {
  serde_json::from_value(json!({
    "album": {
      "album_type": "album",
      "artists": [{ "id": artist_id, "name": artist, "type": "artist", "uri": format!("spotify:artist:{artist_id}"), "href": "", "external_urls": {} }],
      "available_markets": [],
      "external_urls": { "spotify": format!("https://open.spotify.com/album/{album_id}") },
      "href": "",
      "id": album_id,
      "images": [],
      "name": album,
      "release_date": "2023-06-15",
      "release_date_precision": "day",
      "type": "album",
      "uri": format!("spotify:album:{album_id}")
    },
    "artists": [{
      "external_urls": {},
      "href": "",
      "id": artist_id,
      "name": artist,
      "type": "artist",
      "uri": format!("spotify:artist:{artist_id}")
    }],
    "available_markets": [],
    "disc_number": 1,
    "duration_ms": duration_ms,
    "explicit": false,
    "external_ids": {},
    "external_urls": { "spotify": format!("https://open.spotify.com/track/{track_id}") },
    "href": "",
    "id": track_id,
    "is_local": false,
    "is_playable": true,
    "name": name,
    "popularity": 78,
    "preview_url": null,
    "track_number": 1,
    "type": "track",
    "uri": format!("spotify:track:{track_id}")
  }))
  .expect("valid mock track json")
}

fn mock_playlist(id: &str, name: &str, total: u32) -> SimplifiedPlaylist {
  serde_json::from_value(json!({
    "collaborative": false,
    "external_urls": {},
    "href": "",
    "id": id,
    "images": [],
    "name": name,
    "owner": {
      "display_name": "Alex Mercer",
      "external_urls": {},
      "href": "",
      "id": "mock_user",
      "type": "user",
      "uri": "spotify:user:mock_user"
    },
    "public": true,
    "snapshot_id": "mock_snap",
    "tracks": { "href": "", "total": total },
    "type": "playlist",
    "uri": format!("spotify:playlist:{id}")
  }))
  .expect("valid mock playlist json")
}

fn showcase_tracks() -> Vec<FullTrack> {
  vec![
    mock_track(
      "2222222222222222222201",
      "3333333333333333333301",
      "4444444444444444444401",
      "Midnight City",
      "M83",
      "Hurry Up, We're Dreaming",
      243_000,
    ),
    mock_track(
      "2222222222222222222202",
      "3333333333333333333302",
      "4444444444444444444402",
      "Resonance",
      "HOME",
      "Odyssey",
      212_000,
    ),
    mock_track(
      "2222222222222222222203",
      "3333333333333333333303",
      "4444444444444444444403",
      "Nightcall",
      "Kavinsky",
      "OutRun",
      258_000,
    ),
    mock_track(
      "2222222222222222222204",
      "3333333333333333333304",
      "4444444444444444444404",
      "Intro",
      "The xx",
      "xx",
      127_000,
    ),
    mock_track(
      "2222222222222222222205",
      "3333333333333333333305",
      "4444444444444444444405",
      "Sunset Lover",
      "Petit Biscuit",
      "Presence",
      237_000,
    ),
    mock_track(
      "2222222222222222222206",
      "3333333333333333333306",
      "4444444444444444444406",
      "Weightless",
      "Marconi Union",
      "Weightless",
      480_000,
    ),
    mock_track(
      "2222222222222222222207",
      "3333333333333333333307",
      "4444444444444444444407",
      "Tech Noir",
      "GUNSHIP",
      "GUNSHIP",
      297_000,
    ),
    mock_track(
      "2222222222222222222208",
      "3333333333333333333308",
      "4444444444444444444408",
      "A Real Hero",
      "College, Electric Youth",
      "Drive OST",
      267_000,
    ),
    mock_track(
      "2222222222222222222209",
      "3333333333333333333309",
      "4444444444444444444409",
      "Genesis",
      "Grimes",
      "Visions",
      255_000,
    ),
    mock_track(
      "2222222222222222222210",
      "3333333333333333333310",
      "4444444444444444444410",
      "Veridis Quo",
      "Daft Punk",
      "Discovery",
      345_000,
    ),
  ]
}

pub fn tracks_for_playlist(playlist_id: &str) -> Option<Vec<FullTrack>> {
  let id = playlist_id
    .strip_prefix("spotify:playlist:")
    .unwrap_or(playlist_id);
  let mut all = showcase_tracks();
  match id {
    "1111111111111111111101" => Some(all),
    "1111111111111111111102" => {
      all.rotate_left(1);
      Some(all)
    }
    "1111111111111111111103" => {
      all.rotate_left(3);
      Some(all)
    }
    "1111111111111111111104" => {
      all.rotate_left(4);
      Some(all)
    }
    "1111111111111111111105" => {
      all.rotate_left(6);
      Some(all)
    }
    "1111111111111111111106" => {
      all.reverse();
      Some(all)
    }
    _ => None,
  }
}

pub fn populate_mock_app(app: &mut App) {
  app.mock_mode = true;
  let user: PrivateUser = serde_json::from_value(json!({
    "country": "US",
    "display_name": "Alex Mercer",
    "email": null,
    "external_urls": {},
    "followers": { "href": null, "total": 42 },
    "href": "",
    "id": "mock_user",
    "images": [],
    "product": "premium",
    "type": "user",
    "uri": "spotify:user:mock_user"
  }))
  .expect("valid mock user");
  app.user = Some(user);

  let playlists = vec![
    mock_playlist("1111111111111111111101", "Late Night Drive", 42),
    mock_playlist("1111111111111111111102", "Synthwave Essentials", 38),
    mock_playlist("1111111111111111111103", "Focus & Deep Work", 64),
    mock_playlist("1111111111111111111104", "Sunday Morning Coffee", 29),
    mock_playlist("1111111111111111111105", "Indie Roadtrip", 51),
    mock_playlist("1111111111111111111106", "Lo-Fi Beats", 80),
  ];
  app.playlists = Some(Page {
    href: String::new(),
    items: playlists,
    limit: 50,
    next: None,
    offset: 0,
    previous: None,
    total: 6,
  });
  app.selected_playlist_index = Some(0);
  app.active_playlist_index = Some(0);

  let tracks = showcase_tracks();
  let now_track = tracks[0].clone();
  for t in &tracks {
    if let Some(id) = &t.id {
      app.liked_song_ids_set.insert(id.to_string());
    }
  }
  app.track_table.tracks = tracks.clone();
  app.track_table.context = Some(TrackTableContext::MyPlaylists);
  app.track_table.selected_index = 0;

  app.queue_items = tracks[1..5]
    .iter()
    .cloned()
    .map(PlayableItem::Track)
    .collect();
  app.queue_next = Some("Resonance - HOME".to_string());

  app.lyrics = Some(vec![
    (10_000, "Waiting in a car".to_string()),
    (25_000, "Waiting for a ride in the dark".to_string()),
    (45_000, "The night city grows".to_string()),
    (70_000, "Look and see her eyes, they glow".to_string()),
    (95_000, "Waiting in a car".to_string()),
  ]);

  let device = Device {
    id: Some("mock_device".to_string()),
    is_active: true,
    is_private_session: false,
    is_restricted: false,
    name: "Studio Monitors".to_string(),
    _type: DeviceType::Computer,
    volume_percent: Some(68),
  };
  app.current_playback_context = Some(CurrentPlaybackContext {
    device,
    repeat_state: RepeatState::Context,
    shuffle_state: true,
    smart_shuffle: false,
    context: None,
    timestamp: chrono::Utc::now(),
    progress: Some(TimeDelta::milliseconds(84_000)),
    is_playing: true,
    item: Some(PlayableItem::Track(now_track)),
    currently_playing_type: CurrentlyPlayingType::Track,
    actions: Default::default(),
  });
  app.song_progress_ms = 84_000;
  app.playback_known = true;
  app.cover_art = Some(("2222222222222222222201".to_string(), mock_cover_image()));
  app.set_current_route_state(Some(ActiveBlock::TrackTable), Some(ActiveBlock::TrackTable));
  app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn populate_mock_app_succeeds() {
    let mut app = App::default();
    populate_mock_app(&mut app);
    assert!(app.playlists.is_some());
    assert_eq!(app.track_table.tracks.len(), 10);
    assert!(app.cover_art.is_some());
  }
}
