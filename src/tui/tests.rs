use super::*;
use crate::app::{ArtistBlock, SearchResultBlock, TrackTableContext};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rspotify::model::PlayHistory;
use serde_json::json;

fn mock_history(id: Option<&str>, name: &str) -> PlayHistory {
  serde_json::from_value(json!({
    "track": {
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": id,
      "is_local": false,
      "name": name,
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    },
    "played_at": "2024-01-01T00:00:00Z",
    "context": null,
  }))
  .unwrap()
}

#[test]
fn groups_collapse_consecutive_replays_only() {
  let items = vec![
    mock_history(Some("a"), "A"),
    mock_history(Some("a"), "A"),
    mock_history(Some("a"), "A"),
    mock_history(Some("b"), "B"),
    mock_history(Some("a"), "A"),
    mock_history(None, "Local"),
    mock_history(None, "Local"),
  ];
  let groups = group_consecutive_plays(&items);
  // The second A run stays separate; id-less rows never merge.
  assert_eq!(
    groups,
    vec![
      PlayGroup {
        first_raw: 0,
        count: 3
      },
      PlayGroup {
        first_raw: 3,
        count: 1
      },
      PlayGroup {
        first_raw: 4,
        count: 1
      },
      PlayGroup {
        first_raw: 5,
        count: 1
      },
      PlayGroup {
        first_raw: 6,
        count: 1
      },
    ]
  );
}

fn render_grid(n_items: usize, selected: usize, w: u16, h: u16) -> Vec<Vec<String>> {
  render_grid_offset(n_items, selected, None, w, h)
}

fn render_grid_offset(
  n_items: usize,
  selected: usize,
  view_offset: Option<usize>,
  w: u16,
  h: u16,
) -> Vec<Vec<String>> {
  let backend = TestBackend::new(w, h);
  let mut terminal = Terminal::new(backend).unwrap();
  let header = TableHeader {
    id: TableId::Song,
    items: vec![TableHeaderItem {
      id: ColumnId::Title,
      text: "Title",
      width: w,
    }],
  };
  let items: Vec<TableItem> = (0..n_items)
    .map(|i| TableItem {
      id: format!("id{}", i),
      format: vec![format!("Track {}", i)],
    })
    .collect();
  terminal
    .draw(|f| {
      draw_table(
        f,
        &App::default(),
        Rect::new(0, 0, w, h),
        ("Test", &header),
        &items,
        TableSelection {
          selected,
          highlight: (true, false),
          offset: view_offset,
        },
      );
    })
    .unwrap();
  terminal
    .backend()
    .buffer()
    .content
    .chunks(w as usize)
    .map(|row| row.iter().map(|c| c.symbol().to_string()).collect())
    .collect()
}

#[test]
fn load_more_button_is_last_visible_row_when_selected_at_bottom() {
  // 40 recently-played items + the load-more row, selection on the button
  // (index 40), viewport 36 (chunk.height-3). The draw window must end ON
  // the selection so the button is the last visible data row
  // (y = chunk.y + height - 2), where the mouse maps clicks to index 40.
  let mut app = App::default();
  app.push_navigation_stack(RouteId::RecentlyPlayed, ActiveBlock::RecentlyPlayed);
  let items: Vec<rspotify::model::PlayHistory> = (0..40)
    .map(|i| {
      serde_json::from_value(json!({
        "track": {
          "album": {
            "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
            "external_urls": {},
            "href": null,
            "id": null,
            "images": [],
            "name": "Mock Album",
          },
          "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
          "disc_number": 1,
          "duration_ms": 180_000,
          "explicit": false,
          "external_ids": {},
          "external_urls": {},
          "href": null,
          "id": format!("mocktrack{}", i),
          "is_local": false,
          "name": format!("Mock Song {}", i),
          "preview_url": null,
          "track_number": 1,
          "type": "track",
        },
        "played_at": "2024-01-01T00:00:00Z",
        "context": null,
      }))
      .unwrap()
    })
    .collect();
  app.recently_played.result = Some(rspotify::model::CursorBasedPage {
    href: String::new(),
    items,
    limit: 40,
    next: Some("mock-cursor".to_string()),
    cursors: None,
    total: Some(40),
  });
  app.recently_played.index = 40;

  let backend = TestBackend::new(200, 50);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_recently_played_table(f, &app, Rect::new(41, 6, 158, 39)))
    .unwrap();
  let rows: Vec<String> = terminal
    .backend()
    .buffer()
    .content
    .chunks(200)
    .map(|row| row.iter().map(|c| c.symbol().to_string()).collect())
    .collect();
  assert!(
    rows.iter().any(|r| r.contains("Load more songs...")),
    "button must be visible, got: {:?}",
    rows
  );
}

#[test]
fn search_songs_load_more_row_visible_with_count_when_selected() {
  // 40 songs, selection on the load-more row (index == 40): the draw window
  // must end ON the selection so the button is the last visible data row
  // (y = chunk.y + height - 2), where mouse clicks map to index 40. The API total (100) must
  // render as "(60 more)".
  let mut app = App::default();
  app.push_navigation_stack(RouteId::Search, ActiveBlock::SearchResultBlock);
  app.search_results.selected_block = SearchResultBlock::SongSearch;
  app.search_results.selected_tracks_index = Some(40);
  app.search_results.tracks = Some(rspotify::model::Page {
    href: String::new(),
    items: (0..40)
      .map(|i| {
        serde_json::from_value(json!({
          "album": {
            "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
            "external_urls": {},
            "href": null,
            "id": null,
            "images": [],
            "name": "Mock Album",
          },
          "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
          "disc_number": 1,
          "duration_ms": 180_000,
          "explicit": false,
          "external_ids": {},
          "external_urls": {},
          "href": null,
          "id": format!("mocktrack{}", i),
          "is_local": false,
          "name": format!("Mock Song {}", i),
          "preview_url": null,
          "track_number": 1,
          "type": "track",
        }))
        .unwrap()
      })
      .collect(),
    limit: 10,
    next: None,
    offset: 0,
    previous: None,
    total: 100,
  });

  let backend = TestBackend::new(200, 50);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_search_results(f, &app, Rect::new(41, 6, 158, 39)))
    .unwrap();
  let rows: Vec<String> = terminal
    .backend()
    .buffer()
    .content
    .chunks(200)
    .map(|row| row.iter().map(|c| c.symbol().to_string()).collect())
    .collect();
  assert!(
    rows.iter().any(|r| r.contains("Load more")),
    "button with remaining count must be visible, got: {:?}",
    rows
  );
}

#[test]
fn song_row_cells_match_hidden_columns() {
  // Cell shape must mirror song_table_columns: a hidden column drops its
  // cells too (ratatui zips cells to widths by position).
  let mut b = crate::user_config::UserConfig::new().behavior;
  b.show_artist_column = true;
  let full = song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b);
  assert_eq!(
    full,
    vec![
      "".to_string(),
      "T".to_string(),
      "A".to_string(),
      "AL".to_string(),
      "2024-01-01".to_string(),
      "3:00".to_string()
    ]
  );
  b.show_artist_column = false;
  assert_eq!(
    song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b),
    vec![
      "".to_string(),
      "T".to_string(),
      "AL".to_string(),
      "2024-01-01".to_string(),
      "3:00".to_string()
    ]
  );
  b.show_date_added_column = false;
  assert_eq!(
    song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b),
    vec![
      "".to_string(),
      "T".to_string(),
      "AL".to_string(),
      "3:00".to_string()
    ]
  );
  b.show_album_column = false;
  b.show_length_column = false;
  assert_eq!(
    song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b),
    vec!["".to_string(), "T".to_string()]
  );
  // Album context (no date column): same shape rules without the date.
  let mut b = crate::user_config::UserConfig::new().behavior;
  b.show_artist_column = false;
  assert_eq!(
    song_row_cells("T", "A", "AL", "", 180_000, false, &b),
    vec![
      "".to_string(),
      "T".to_string(),
      "AL".to_string(),
      "3:00".to_string()
    ]
  );
}

#[test]
fn wheel_scrolled_view_is_not_snapped_to_selection() {
  // 30 items, viewport 5: the view wheel-scrolled to offset 20 with the
  // selection pinned at the bottom must KEEP showing the scrolled rows,
  // not snap back down to the selection (the old snap locked the wheel).
  let grid = render_grid_offset(30, 29, Some(20), 60, 10);
  assert!(
    grid[2].concat().contains("Track 20"),
    "view must render the wheel-scrolled offset verbatim"
  );
}

#[test]
fn scrollbar_only_shows_on_overflow() {
  // h=10, padding=3 -> viewport=7. 8 items overflow, 7 items fit.
  let overflow = render_grid(8, 0, 60, 10);
  let fits = render_grid(7, 0, 60, 10);
  // scrollbar column is x+width-2 (index 58)
  assert_eq!(overflow[2][58], "█", "thumb must show when list overflows");
  assert_eq!(fits[2][58], " ", "no scrollbar when list fits");
  // right border at x+width-1 (index 59) is always preserved
  assert_eq!(overflow[2][59], "│", "right border must survive scrollbar");
  assert_eq!(
    fits[2][59], "│",
    "right border must survive empty scrollbar"
  );
}

#[test]
fn scrollbar_thumb_tracks_selection() {
  // 30 items, h=10, viewport=7. Thumb position should move down with selection.
  let top = render_grid(30, 0, 60, 10);
  let bottom = render_grid(30, 29, 60, 10);
  let thumb_at =
    |grid: &[Vec<String>]| -> usize { grid.iter().position(|row| row[58] == "█").unwrap() };
  assert!(
    thumb_at(&bottom) > thumb_at(&top),
    "thumb moves down as selection moves"
  );
}

#[test]
fn scrollbar_thumb_inside_border() {
  let grid = render_grid(30, 15, 60, 10);
  // Border columns must keep the double-line from Borders::ALL on interior rows
  for row in &grid[1..grid.len() - 1] {
    assert_eq!(row[59], "│", "right border column untouched");
    assert_eq!(row[0], "│", "left border column untouched");
  }
}

fn playback_app_with_track(id: &str) -> App {
  let mut app = App::default();
  let playback: rspotify::model::CurrentPlaybackContext = serde_json::from_value(json!({
    "device": {
      "id": "mock-device",
      "is_active": true,
      "is_private_session": false,
      "is_restricted": false,
      "name": "Mock Device",
      "type": "Computer",
      "volume_percent": 50,
    },
    "repeat_state": "off",
    "shuffle_state": false,
    "context": null,
    "timestamp": 0,
    "progress_ms": 0,
    "is_playing": true,
    "item": {
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": id,
      "is_local": false,
      "name": "Mock Song",
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    },
    "currently_playing_type": "track",
    "actions": { "disallows": {} },
  }))
  .unwrap();
  app.current_playback_context = Some(playback);
  app
}

#[test]
fn playing_icon_only_on_the_playing_row() {
  fn mock_track(i: usize) -> rspotify::model::FullTrack {
    serde_json::from_value(json!({
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": format!("mocktrack{}", i),
      "is_local": false,
      "name": format!("Mock Song {}", i),
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    }))
    .unwrap()
  }
  // Playback is the SECOND track; the icon must land on it, not row 0.
  let mut app = playback_app_with_track("mocktrack1");
  app.track_table.tracks = (0..3).map(mock_track).collect();
  app.push_navigation_stack(crate::app::RouteId::TrackTable, ActiveBlock::TrackTable);

  let backend = TestBackend::new(180, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_song_table(f, &app, Rect::new(0, 0, 180, 40)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let mut lines = vec![];
  for y in 0..40 {
    let line: String = (0..180)
      .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
      .collect();
    lines.push(line);
  }
  let joined = lines.join("\n");
  assert!(
    joined.contains("▶ Mock Song 1"),
    "playing icon missing on the playing row:\n{joined}"
  );
  assert!(
    !joined.contains("▶ Mock Song 0"),
    "playing icon wrongly on the first row:\n{joined}"
  );
}

#[test]
fn date_added_column_renders_and_aligns_for_playlist_context() {
  fn mock_track(i: usize) -> rspotify::model::FullTrack {
    serde_json::from_value(json!({
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": format!("mocktrack{}", i),
      "is_local": false,
      "name": format!("Mock Song {}", i),
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    }))
    .unwrap()
  }
  // A playlist context carries added_at: the Date Added header must exist
  // and the date value must sit under it (not under Length) — regression
  // guard for the recurring columns-vs-cells with_date mismatch.
  let mut app = App::default();
  app.track_table.context = Some(TrackTableContext::MyPlaylists);
  app.track_table.tracks = (0..2).map(mock_track).collect();
  let added: chrono::DateTime<chrono::Utc> =
    chrono::DateTime::parse_from_rfc3339("2024-01-15T00:00:00Z")
      .unwrap()
      .with_timezone(&chrono::Utc);
  app.track_table_added_at = vec![Some(added), None];
  app.push_navigation_stack(crate::app::RouteId::TrackTable, ActiveBlock::TrackTable);

  let backend = TestBackend::new(180, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_song_table(f, &app, Rect::new(0, 0, 180, 40)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let lines: Vec<String> = (0..40)
    .map(|y| {
      (0..180)
        .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
        .collect()
    })
    .collect();
  let header_x = lines[1]
    .match_indices("Date Added")
    .next()
    .map(|(i, _)| lines[1][..i].chars().count())
    .expect("Date Added header missing");
  let first_row = &lines[2];
  let row_tail: String = first_row.chars().skip(header_x).collect();
  assert!(
    row_tail.starts_with("2024-01-15"),
    "date not under the Date Added header (x={header_x}):\nrow: {first_row}"
  );
}

#[test]
fn relative_date_formats_hours_days_and_old_dates() {
  let now = chrono::Utc::now();
  // 2 hours ago → "2h ago"
  assert_eq!(relative_date(now - chrono::Duration::hours(2)), "2h ago");
  // 3 days ago → "3d ago"
  assert_eq!(relative_date(now - chrono::Duration::days(3)), "3d ago");
  // 2 months ago → absolute date
  assert_eq!(
    relative_date(now - chrono::Duration::days(60)),
    (now - chrono::Duration::days(60))
      .format("%Y-%m-%d")
      .to_string()
  );
}

#[test]
fn artist_top_tracks_renders_load_more_row() {
  fn mock_track(i: usize) -> rspotify::model::FullTrack {
    serde_json::from_value(json!({
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": format!("mocktrack{}", i),
      "is_local": false,
      "name": format!("Mock Song {}", i),
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    }))
    .unwrap()
  }
  let empty_page = rspotify::model::Page {
    href: String::new(),
    items: vec![],
    limit: 0,
    next: None,
    offset: 0,
    previous: None,
    total: 0,
  };
  let mut app = App::default();
  app.artist = Some(crate::app::Artist {
    artist_id: "mockartist1".to_string(),
    artist_name: "Mock Artist".to_string(),
    albums: empty_page.clone(),
    singles: empty_page.clone(),
    eps: empty_page.clone(),
    appears_on: empty_page.clone(),
    related_artists: vec![],
    top_tracks: (0..10).map(mock_track).collect(),
    top_tracks_total: 26,
    top_tracks_has_more: true,
    selected_album_index: 0,
    selected_singles_index: 0,
    selected_eps_index: 0,
    selected_appears_on_index: 0,
    selected_related_artist_index: 0,
    selected_top_track_index: 0,
    artist_hovered_block: ArtistBlock::TopTracks,
    artist_selected_block: ArtistBlock::Empty,
    image_url: None,
    is_following: None,
    followers: None,
    genres: vec![],
    meta: None,
  });

  let backend = TestBackend::new(180, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_artist_page(f, &app, Rect::new(0, 0, 180, 40)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let mut lines = vec![];
  for y in 0..40 {
    let line: String = (0..180)
      .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
      .collect();
    lines.push(line);
  }
  let joined = lines.join("\n");
  assert!(
    joined.contains("Load more songs..."),
    "load-more row missing from rendered artist top tracks:\n{joined}"
  );
  assert!(
    joined.contains("Mock Song 9"),
    "top tracks themselves missing:\n{joined}"
  );
}

#[test]
fn scrolled_table_window_matches_click_mapping() {
  fn mock_track(i: usize) -> rspotify::model::FullTrack {
    serde_json::from_value(json!({
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {},
        "href": null,
        "id": null,
        "images": [],
        "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1,
      "duration_ms": 180_000,
      "explicit": false,
      "external_ids": {},
      "external_urls": {},
      "href": null,
      "id": format!("mocktrack{}", i),
      "is_local": false,
      "name": format!("Mock Song {}", i),
      "preview_url": null,
      "track_number": 1,
      "type": "track",
    }))
    .unwrap()
  }
  let empty_page2 = rspotify::model::Page {
    href: String::new(),
    items: vec![],
    limit: 0,
    next: None,
    offset: 0,
    previous: None,
    total: 0,
  };
  let mut app = App::default();
  app.artist = Some(crate::app::Artist {
    artist_id: "mockartist1".to_string(),
    artist_name: "Mock Artist".to_string(),
    albums: empty_page2.clone(),
    singles: empty_page2.clone(),
    eps: empty_page2.clone(),
    appears_on: empty_page2.clone(),
    related_artists: vec![],
    top_tracks: (0..15).map(mock_track).collect(),
    top_tracks_total: 15,
    top_tracks_has_more: false,
    selected_album_index: 0,
    selected_singles_index: 0,
    selected_eps_index: 0,
    selected_appears_on_index: 0,
    selected_related_artist_index: 0,
    selected_top_track_index: 12,
    artist_hovered_block: ArtistBlock::TopTracks,
    artist_selected_block: ArtistBlock::TopTracks,
    image_url: None,
    is_following: None,
    followers: None,
    genres: vec![],
    meta: None,
  });

  let backend = TestBackend::new(80, 8);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_artist_page(f, &app, Rect::new(0, 0, 80, 8)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let mut rows = vec![];
  for y in 0..8 {
    let line: String = (0..40)
      .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
      .collect();
    rows.push(line);
  }
  // The top-tracks tab is a real table now: border row y=1, header y=2,
  // data rows from y=3. list_rect height 7 → draw_table viewport = 7-3 = 4,
  // offset = 12-4+1 = 9 → rows y=3..6 hold Mock Song 9..12, the window
  // ends ON the selection, matching table_row_index (index 0 at y=3).
  // y=7 is the bottom border.
  assert!(
    rows[2].contains("Title"),
    "row 2 should be the table header, got: {}",
    rows[2]
  );
  for (row, expected) in [(3, 9), (6, 12)] {
    assert!(
      rows[row].contains(&format!("Mock Song {expected}")),
      "row {row} should hold Mock Song {expected}, got: {}",
      rows[row]
    );
  }
}

#[test]
fn lyrics_block_is_vertically_centered_and_text_centered() {
  let mut app = App::default();
  app.lyrics = Some(vec![
    (0, "line one".to_string()),
    (5000, "line two".to_string()),
  ]);

  let backend = TestBackend::new(30, 20);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_music_lyrics(f, &app, Rect::new(0, 0, 30, 14), Rect::new(0, 14, 30, 6)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let rows: Vec<String> = (0..20)
    .map(|y| {
      (0..30)
        .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
        .collect()
    })
    .collect();
  // The lyrics block is 70% of the rect (height 14): border rows 0 and 13,
  // inner height 12, 2 lyric lines → 5 blank pad rows below the border, so
  // the text is vertically centered. The attribution sits in the top border.
  assert!(
    rows[0].contains("Lyrics by LRCLIB"),
    "row 0 should hold the attribution title, got: {}",
    rows[0]
  );
  assert!(
    !rows[1].contains("line"),
    "row 1 should be pad, got: {}",
    rows[1]
  );
  assert!(
    !rows[5].contains("line"),
    "row 5 should be pad, got: {}",
    rows[5]
  );
  assert!(
    rows[6].starts_with('│'),
    "block should hug x=0, got: {}",
    rows[6]
  );
  assert!(
    rows[6].contains("▶ line one") && !rows[6].starts_with("│▶"),
    "row 6 should hold the centered current line, got: {}",
    rows[6]
  );
  assert!(
    rows[7].contains("line two"),
    "row 7 should hold line two, got: {}",
    rows[7]
  );
}

#[test]
fn lyrics_empty_state_stays_at_the_top() {
  let mut app = App::default();
  app.lyrics = None;

  let backend = TestBackend::new(40, 20);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_music_lyrics(f, &app, Rect::new(0, 0, 40, 14), Rect::new(0, 14, 40, 6)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let row1: String = (0..40)
    .map(|x| buffer.cell((x, 1)).map(|c| c.symbol()).unwrap_or(" "))
    .collect();
  // No vertical padding: the message sits right below the top border, and
  // the block keeps its full width.
  assert!(
    row1.contains("No lyrics available for this track"),
    "row 1 should hold the message at the top, got: {}",
    row1
  );
  let row0: String = (0..40)
    .map(|x| buffer.cell((x, 0)).map(|c| c.symbol()).unwrap_or(" "))
    .collect();
  assert!(
    row0.starts_with('┌') || row0.starts_with('╭'),
    "block should span full width, got: {}",
    row0
  );
}

#[test]
fn ellipsize_keeps_the_head_of_a_long_input() {
  let url = "https://open.spotify.com/playlist/37i9dQZEVXcO7jjUV7WpTq?si=6f9dfe1178b6482e";
  // Short input is unchanged.
  assert_eq!(ellipsize("short", 10), "short");
  // Long input keeps the START, the ellipsis goes at the END.
  let out = ellipsize(url, 30);
  assert_eq!(out.chars().count(), 30);
  assert!(out.starts_with("https://open.spotify.com/"));
  assert!(out.ends_with("..."));
  // A tiny box degrades to the ellipsis only.
  assert_eq!(ellipsize(url, 3), "...");
}

#[test]
fn track_index_cell_numbers_and_heart() {
  // Number only (right-aligned in a 4-wide slot), heart glued to the left
  // of the number when liked, so every number stays column-aligned.
  let mut app = App::default();
  app.user_config.behavior.liked_icon = "♥".to_string();
  assert_eq!(super::track_index_cell(&app, &None, 1), "   1");
  assert_eq!(super::track_index_cell(&app, &None, 10), "  10");
  assert_eq!(super::track_index_cell(&app, &None, 100), " 100");
  assert_eq!(super::track_index_cell(&app, &None, 1000), "1000");

  app.liked_song_ids_set.insert("liked".to_string());
  assert_eq!(
    super::track_index_cell(&app, &Some("liked".to_string()), 1),
    "  ♥1"
  );
  assert_eq!(
    super::track_index_cell(&app, &Some("liked".to_string()), 10),
    " ♥10"
  );
  assert_eq!(
    super::track_index_cell(&app, &Some("liked".to_string()), 100),
    "♥100"
  );
  assert_eq!(
    super::track_index_cell(&app, &Some("liked".to_string()), 1000),
    "♥1000"
  );
  // An unliked id never shows the heart.
  assert_eq!(
    super::track_index_cell(&app, &Some("other".to_string()), 7),
    "   7"
  );
  assert_eq!(
    super::track_index_cell(&app, &Some("liked".to_string()), 1),
    "  ♥1"
  );
}

#[test]
fn number_column_renders_in_song_table() {
  fn mock_track(i: usize) -> rspotify::model::FullTrack {
    serde_json::from_value(json!({
      "album": {
        "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {}, "href": null, "id": null, "images": [], "name": "Mock Album",
      },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1, "duration_ms": 180_000, "explicit": false, "external_ids": {},
      "external_urls": {}, "href": null, "id": format!("mocktrack{}", i), "is_local": false,
      "name": format!("Mock Song {}", i), "preview_url": null, "track_number": 1, "type": "track",
    }))
    .unwrap()
  }
  let mut app = playback_app_with_track("mocktrack0");
  app.track_table.tracks = (0..3).map(mock_track).collect();
  app.push_navigation_stack(crate::app::RouteId::TrackTable, ActiveBlock::TrackTable);

  let backend = TestBackend::new(180, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_song_table(f, &app, Rect::new(0, 0, 180, 40)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  let mut lines = vec![];
  for y in 0..40 {
    let line: String = (0..180)
      .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
      .collect();
    lines.push(line);
  }
  let joined = lines.join("\n");
  // First track is index 1; the 4-wide slot right-aligns it to "   1".
  assert!(joined.contains("   1"), "number column missing:\n{joined}");
}

#[test]
fn title_starts_gap_after_number_column() {
  fn mock_track(i: usize) -> rspotify::model::FullTrack {
    serde_json::from_value(json!({
      "album": { "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
        "external_urls": {}, "href": null, "id": null, "images": [], "name": "Mock Album" },
      "artists": [{ "external_urls": {}, "href": null, "id": null, "name": "Mock Artist" }],
      "disc_number": 1, "duration_ms": 180_000, "explicit": false, "external_ids": {},
      "external_urls": {}, "href": null, "id": format!("mocktrack{}", i), "is_local": false,
      "name": format!("Mock Song {}", i), "preview_url": null, "track_number": 1, "type": "track",
    })).unwrap()
  }
  let mut app = App::default();
  app.track_table.tracks = (0..10).map(mock_track).collect();
  app.track_table.context = Some(crate::app::TrackTableContext::MyPlaylists);
  app.track_table_added_at = (0..10).map(|_| None).collect();
  app.push_navigation_stack(crate::app::RouteId::TrackTable, ActiveBlock::TrackTable);
  let backend = TestBackend::new(200, 18);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_song_table(f, &app, Rect::new(0, 0, 200, 18)))
    .unwrap();
  let buffer = terminal.backend().buffer();
  // The Title text starts GAP columns to the right of the # header. ratatui
  // Table inserts a default 1-space column_spacing between columns, so the
  // visible gap = 1 (# itself) + spacing(1) + gap. Compare char indices
  // (the leading border glyph is multibyte, so byte offsets would skew).
  let gap = crate::tui::layout::NUMBER_TO_TITLE_GAP as usize;
  let spacing = 0;
  let header: String = (0..60)
    .map(|x| buffer.cell((x, 1)).map(|c| c.symbol()).unwrap_or(" "))
    .collect();
  let hash_byte = header.find('#').unwrap();
  let title_byte = header.find("Title").unwrap();
  let hash_idx = header[..hash_byte].chars().count();
  let title_idx = header[..title_byte].chars().count();
  assert_eq!(
    title_idx - hash_idx,
    1 + spacing + gap,
    "header gap:\n{header}"
  );
  // First data row: the track name "Mock Song 0" starts GAP cols after the
  // "   1" number cell (cell width 4 + spacing 1 + gap).
  let row: String = (0..60)
    .map(|x| buffer.cell((x, 2)).map(|c| c.symbol()).unwrap_or(" "))
    .collect();
  let num_byte = row.find("   1").unwrap();
  let name_byte = row.find("Mock Song 0").unwrap();
  let num_idx = row[..num_byte].chars().count();
  let name_idx = row[..name_byte].chars().count();
  assert_eq!(name_idx - num_idx, 4 + spacing + gap, "row gap:\n{row}");
}

#[test]
fn playlist_picker_selected_row_is_bold() {
  // The picker marks its row with color + BOLD (color alone is invisible
  // on terminals where selected ≈ text).
  let mut app = App::default();
  app.playlists = Some(rspotify::model::Page {
    href: String::new(),
    items: (0..3)
      .map(|i| {
        serde_json::from_value(json!({
          "collaborative": false,
          "external_urls": {},
          "href": format!("spotify:playlist:mockplaylist{}", i),
          "id": format!("mockplaylist{}", i),
          "images": [],
          "name": format!("Mock Playlist {}", i),
          "owner": {
            "display_name": "Mock User",
            "external_urls": {},
            "followers": null,
            "href": "spotify:user:mockuser",
            "id": "mockuser",
            "images": [],
          },
          "public": null,
          "snapshot_id": format!("snap{}", i),
          "items": { "href": "", "total": 150 },
        }))
        .unwrap()
      })
      .collect(),
    limit: 3,
    next: None,
    offset: 0,
    previous: None,
    total: 3,
  });
  app.playlist_picker_index = 1;
  let backend = TestBackend::new(200, 50);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal
    .draw(|f| draw_add_to_playlist_dialog(f, &app))
    .unwrap();
  // Dialog geometry for 200x50: width 45 at x=77, top y=12, inner list at
  // x=79 starting y=14 — index 1 sits on y=15.
  let buffer = terminal.backend().buffer();
  let bold_at = |x: u16, y: u16| {
    buffer
      .cell((x, y))
      .map(|c| c.modifier.contains(Modifier::BOLD))
      .unwrap_or(false)
  };
  assert!(bold_at(79, 15), "selected playlist row must be bold");
  assert!(!bold_at(79, 14), "unselected row must not be bold");
  assert!(!bold_at(79, 16), "unselected row must not be bold");
}

#[test]
fn playbar_heart_reflects_liked_state() {
  // Song row starts with [+ / ✓] then the heart: filled when the
  // now-playing track is liked, outline otherwise, name right after.
  let song_row_text = |liked: bool| {
    let mut app = playback_app_with_track("mocktrack1");
    if liked {
      app
        .liked_song_ids_set
        .insert("spotify:track:mocktrack1".to_string());
    }
    let backend = TestBackend::new(200, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
      .draw(|f| draw_playbar(f, &app, Rect::new(0, 0, 200, 10)))
      .unwrap();
    let song_y = crate::tui::layout::playbar_song_row(Rect::new(0, 0, 200, 10)).y;
    (0..60)
      .map(|x| {
        terminal
          .backend()
          .buffer()
          .cell((x, song_y))
          .map(|c| c.symbol())
          .unwrap_or(" ")
      })
      .collect::<String>()
  };
  let liked_row = song_row_text(true);
  assert!(
    liked_row.contains("+ ♥ Mock Song"),
    "liked row:\n{liked_row}"
  );
  let unliked_row = song_row_text(false);
  assert!(
    unliked_row.contains("+ ♡ Mock Song"),
    "unliked row:\n{unliked_row}"
  );
}

#[test]
fn sidebar_divider_and_right_border_share_chrome_color() {
  for hovered in [Some(ActiveBlock::MyPlaylists), None] {
    let mut app = App::default();
    app.set_current_route_state(None, hovered);
    let area = Rect::new(0, 0, 60, 30);
    let backend = TestBackend::new(area.width, area.height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw_routes(f, &app, area)).unwrap();

    let buffer = terminal.backend().buffer();
    let sidebar = layout::sidebar_content_split(&app, area).0;
    let (_, sep, _) = layout::sidebar_combined_split(&app, sidebar);
    let divider = buffer.cell((sep.x + 1, sep.y)).unwrap();
    let border = buffer
      .cell((sidebar.x + sidebar.width - 1, sidebar.y + 5))
      .unwrap();
    let theme = &app.user_config.theme;
    let expected = if hovered.is_some() {
      theme.hovered
    } else {
      theme.inactive
    };
    assert_eq!(
      divider.fg, expected,
      "divider should follow the box accent, got {:?}",
      divider.fg
    );
    assert_eq!(
      border.fg, expected,
      "right border should follow the box accent, got {:?}",
      border.fg
    );
    assert_eq!(border.symbol(), "│");
  }
}

#[test]
fn fit_cover_rect_stays_strictly_inside_inner_panel() {
  for (w, h) in [(120u16, 40u16), (80, 24), (200, 50), (40, 16)] {
    let inner =
      crate::tui::music_view::cover_inner_rect_for(&App::default(), Rect::new(0, 0, w, h));
    for img_px in [(640, 640), (640, 360), (360, 640)] {
      let fitted = crate::tui::music_view::fit_cover_rect(inner, img_px, (9, 18));
      assert!(fitted.x >= inner.x);
      assert!(fitted.y >= inner.y);
      assert!(fitted.x + fitted.width <= inner.x + inner.width);
      assert!(fitted.y + fitted.height <= inner.y + inner.height);
    }
  }
}
