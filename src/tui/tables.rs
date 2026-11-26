use super::layout::{
  create_artist_string, format_playlist_duration, get_color, get_percentage_width,
  millis_to_minutes, song_table_columns, track_table_with_date, SongColumnFlags,
  PLAYLIST_SEARCH_PLACEHOLDER, REFRESH_GLYPH,
};
use super::widgets::{
  draw_selectable_list, draw_table, draw_table_window, load_more_label, ColumnId, ListSelection,
  TableHeader, TableHeaderItem, TableId, TableItem, TableSelection,
};
use crate::app::{
  ActiveBlock, AlbumTableContext, App, EpisodeTableContext, TrackSortColumn, TrackTableContext,
};
use ratatui::{
  layout::Rect,
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, BorderType, Borders, Paragraph},
  Frame,
};
use rspotify::model::show::ResumePoint;
use rspotify::model::PlayHistory;
use rspotify::model::PlayableItem;
use rspotify::prelude::Id;
use unicode_width::UnicodeWidthStr;

pub(super) struct AlbumUi {
  selected_index: usize,
  items: Vec<TableItem>,
  title: String,
}

pub fn draw_artist_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let header = TableHeader {
    id: TableId::Artist,
    items: vec![TableHeaderItem {
      text: "Artist",
      width: get_percentage_width(layout_chunk.width, 1.0),
      ..Default::default()
    }],
  };

  let current_route = app.get_current_route();
  let highlight_state = (
    current_route.active_block == ActiveBlock::Artists,
    current_route.hovered_block == ActiveBlock::Artists,
  );
  let items = app
    .artists
    .iter()
    .map(|item| TableItem {
      id: item.id.to_string(),
      format: vec![item.name.to_owned()],
    })
    .collect::<Vec<TableItem>>();

  draw_table(
    f,
    app,
    layout_chunk,
    ("Artists", &header),
    &items,
    TableSelection {
      selected: app.artists_list_index,
      highlight: highlight_state,
      offset: None,
    },
  )
}

pub fn draw_podcast_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let header = TableHeader {
    id: TableId::Podcast,
    items: vec![TableHeaderItem {
      text: "Name",
      width: get_percentage_width(layout_chunk.width, 4.0 / 5.0),
      ..Default::default()
    }],
  };

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::Podcasts,
    current_route.hovered_block == ActiveBlock::Podcasts,
  );

  let title = format!("{}{}", REFRESH_GLYPH, "Podcasts");
  let Some(saved_shows) = app.library.saved_shows.get_results(None) else {
    let block = Block::default()
      .borders(Borders::ALL)
      .border_type(if app.user_config.terminal.unicode_borders {
        BorderType::Rounded
      } else {
        BorderType::Plain
      })
      .title(title)
      .border_style(get_color(highlight_state, app.user_config.theme));
    f.render_widget(
      Paragraph::new("Loading podcasts…")
        .block(block)
        .style(Style::default().fg(app.user_config.theme.inactive)),
      layout_chunk,
    );
    return;
  };
  if saved_shows.items.is_empty() {
    let block = Block::default()
      .borders(Borders::ALL)
      .border_type(if app.user_config.terminal.unicode_borders {
        BorderType::Rounded
      } else {
        BorderType::Plain
      })
      .title(title)
      .border_style(get_color(highlight_state, app.user_config.theme));
    f.render_widget(
      Paragraph::new("No saved podcasts — follow shows in Spotify or search to add")
        .block(block)
        .style(Style::default().fg(app.user_config.theme.inactive)),
      layout_chunk,
    );
    return;
  }

  let items = saved_shows
    .items
    .iter()
    .map(|show_page| TableItem {
      id: show_page.show.id.to_string(),
      format: vec![show_page.show.name.to_owned()],
    })
    .collect::<Vec<TableItem>>();

  draw_table(
    f,
    app,
    layout_chunk,
    (&title, &header),
    &items,
    TableSelection {
      selected: app.shows_list_index,
      highlight: highlight_state,
      offset: None,
    },
  )
}

pub fn draw_album_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let show_in_playlist = b.enable_add_to_playlist;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    SongColumnFlags::for_view(b, false, false, show_in_playlist),
  );
  let header = TableHeader {
    id: TableId::Album,
    items: columns
      .iter()
      .map(|(column, _, width)| TableHeaderItem {
        id: *column,
        text: match column {
          ColumnId::Title => "Title",
          ColumnId::Artist => "Artist",
          ColumnId::Album => "Album",
          ColumnId::Length => "Length",
          ColumnId::DateAdded => "Date Added",
          ColumnId::Liked => "#",
          _ => "",
        },
        width: *width,
      })
      .collect(),
  };

  let current_route = app.get_current_route();
  let highlight_state = (
    current_route.active_block == ActiveBlock::AlbumTracks,
    current_route.hovered_block == ActiveBlock::AlbumTracks,
  );

  let album_ui = match &app.album_table_context {
    AlbumTableContext::Simplified => {
      app
        .selected_album_simplified
        .as_ref()
        .map(|selected_album_simplified| AlbumUi {
          items: {
            let mut items: Vec<TableItem> = selected_album_simplified
              .tracks
              .items
              .iter()
              .enumerate()
              .map(|(index, item)| TableItem {
                id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
                format: {
                  let track_id = item.id.clone().map(|id| id.to_string());
                  let mut cells = song_row_cells(
                    &item.name,
                    &create_artist_string(&item.artists),
                    &selected_album_simplified.album.name,
                    "",
                    item.duration.num_milliseconds() as u128,
                    false,
                    b,
                  );
                  cells[0] = track_index_cell(app, &track_id, index + 1);
                  if show_in_playlist {
                    let in_pl = item
                      .id
                      .as_ref()
                      .map(|id| app.playlist_contains(&id.uri(), None))
                      .unwrap_or(false);
                    if in_pl {
                      cells.push(app.user_config.padded_in_playlist_icon());
                    } else {
                      cells.push(" + ".to_string());
                    }
                  }
                  cells
                },
              })
              .collect::<Vec<TableItem>>();

            if items.len() < selected_album_simplified.tracks.total as usize {
              let remaining = selected_album_simplified.tracks.total as usize - items.len();
              items.push(TableItem {
                id: String::new(),
                format: {
                  let mut load_more_format = vec![
                    "".to_string(),
                    load_more_label("Load more songs...", Some(remaining)),
                  ];
                  if b.show_artist_column {
                    load_more_format.push(String::new());
                  }
                  if b.show_album_column {
                    load_more_format.push(String::new());
                  }
                  if b.show_length_column {
                    load_more_format.push(String::new());
                  }
                  load_more_format
                },
              });
            }
            items
          },
          title: format!(
            "{} by {}",
            selected_album_simplified.album.name,
            create_artist_string(&selected_album_simplified.album.artists)
          ),
          selected_index: selected_album_simplified.selected_index,
        })
    }
    AlbumTableContext::Full => app
      .selected_album_full
      .as_ref()
      .map(|selected_album| AlbumUi {
        items: selected_album
          .album
          .tracks
          .items
          .iter()
          .enumerate()
          .map(|(index, item)| TableItem {
            id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
            format: {
              let track_id = item.id.clone().map(|id| id.to_string());
              let mut cells = song_row_cells(
                &item.name,
                &create_artist_string(&item.artists),
                &selected_album.album.name,
                "",
                item.duration.num_milliseconds() as u128,
                false,
                b,
              );
              cells[0] = track_index_cell(app, &track_id, index + 1);
              if show_in_playlist {
                let in_pl = item
                  .id
                  .as_ref()
                  .map(|id| app.playlist_contains(&id.uri(), None))
                  .unwrap_or(false);
                if in_pl {
                  cells.push(app.user_config.padded_in_playlist_icon());
                } else {
                  cells.push(" + ".to_string());
                }
              }
              cells
            },
          })
          .collect::<Vec<TableItem>>(),
        title: format!(
          "{} by {}",
          selected_album.album.name,
          create_artist_string(&selected_album.album.artists)
        ),
        selected_index: app.saved_album_tracks_index,
      }),
  };

  if let Some(album_ui) = album_ui {
    draw_table(
      f,
      app,
      layout_chunk,
      (&album_ui.title, &header),
      &album_ui.items,
      TableSelection {
        selected: album_ui.selected_index,
        highlight: highlight_state,
        offset: None,
      },
    );
  };
}

// Row cells for the song table, in the same order as song_table_columns so a
// hidden column also drops its cells (ratatui zips cells to widths by
// position).
pub(super) fn song_row_cells(
  name: &str,
  artists: &str,
  album: &str,
  date: &str,
  duration_ms: u128,
  with_date: bool,
  behavior: &crate::user_config::BehaviorConfig,
) -> Vec<String> {
  let mut cells = vec![String::new(), name.to_string()];
  if behavior.show_artist_column {
    cells.push(artists.to_string());
  }
  if with_date {
    if behavior.show_album_column {
      cells.push(album.to_string());
    }
    if behavior.show_date_added_column {
      cells.push(date.to_string());
    }
  } else if behavior.show_album_column {
    cells.push(album.to_string());
  }
  if behavior.show_length_column {
    cells.push(millis_to_minutes(duration_ms));
  }
  cells
}

// Leftmost song-table cell: the track number, right-aligned inside a 4-wide
// slot, with the liked heart glued to the number's left when the track is in
// the user's liked set. Right-aligning the whole thing (heart + number) keeps
// every track number aligned on the same column regardless of the heart.
pub(super) fn track_index_cell(app: &App, id: &Option<String>, index: usize) -> String {
  let liked = id
    .as_ref()
    .map(|i| app.liked_song_ids_set.contains(i))
    .unwrap_or(false);
  let inner = if liked {
    format!("{}{}", app.user_config.behavior.liked_icon, index)
  } else {
    index.to_string()
  };
  format!("{inner:>4}")
}

pub(super) fn relative_date_at(
  added_at: chrono::DateTime<chrono::Utc>,
  now: chrono::DateTime<chrono::Utc>,
) -> String {
  let minutes = (now - added_at).num_minutes().max(1);
  if minutes < 60 {
    return format!("{minutes}m ago");
  }
  let hours = (now - added_at).num_hours();
  if hours < 24 {
    return format!("{hours}h ago");
  }
  let days = (now - added_at).num_days();
  if days < 7 {
    return format!("{days}d ago");
  }
  added_at.format("%Y-%m-%d").to_string()
}

// Relative "Xm/Xh/Xd ago" for recent adds (a track added hours ago should read
// "3h ago", not the bare date); absolute date once it's more than a week old.
#[cfg(test)]
pub(super) fn relative_date(added_at: chrono::DateTime<chrono::Utc>) -> String {
  relative_date_at(added_at, chrono::Utc::now())
}

pub fn draw_song_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let with_date = track_table_with_date(app.track_table.context.as_ref());
  let b = &app.user_config.behavior;
  let show_remove = b.enable_remove_from_playlist
    && matches!(
      app.track_table.context,
      Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
    );
  let show_in_playlist = !show_remove && b.enable_add_to_playlist;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    SongColumnFlags::for_view(b, with_date, show_remove, show_in_playlist),
  );

  let header_texts: Vec<String> = columns
    .iter()
    .map(|(column, _, _)| {
      let mut text = match column {
        ColumnId::Title => "Title",
        ColumnId::Artist => "Artist",
        ColumnId::Album => "Album",
        ColumnId::Length => "Length",
        ColumnId::DateAdded => "Date Added",
        ColumnId::Liked => "#",
        _ => "",
      }
      .to_string();
      if let Some((sort_column, desc)) = app.track_table_sort {
        let arrow = if desc { " ▼" } else { " ▲" };
        let matches = matches!(
          (column, sort_column),
          (ColumnId::Title, TrackSortColumn::Title)
            | (ColumnId::Artist, TrackSortColumn::Artist)
            | (ColumnId::Album, TrackSortColumn::Album)
            | (ColumnId::Length, TrackSortColumn::Length)
            | (ColumnId::DateAdded, TrackSortColumn::DateAdded)
        );
        if matches {
          text.push_str(arrow);
        }
      }
      text
    })
    .collect();

  let header = TableHeader {
    id: TableId::Song,
    items: columns
      .iter()
      .zip(header_texts.iter())
      .map(|((column, _, width), text)| TableHeaderItem {
        id: *column,
        text,
        width: *width,
      })
      .collect(),
  };

  let current_route = app.get_current_route();
  let highlight_state = (
    current_route.active_block == ActiveBlock::TrackTable,
    current_route.hovered_block == ActiveBlock::TrackTable,
  );

  // In-playlist search filters in a single O(N) pass; only the visible viewport
  // window is formatted into TableItems.
  let filtered_indices: Option<Vec<usize>> = if app.playlist_search_active() {
    let needle = app.playlist_filter.as_deref().unwrap_or("").to_lowercase();
    Some(
      app
        .track_table
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| App::track_matches_needle(t, &needle).then_some(i))
        .collect(),
    )
  } else {
    None
  };
  let matched_len = filtered_indices
    .as_ref()
    .map_or(app.track_table.tracks.len(), Vec::len);
  let has_load_more = app.track_table_has_more() || app.date_added_pending;
  let total_items = matched_len + usize::from(has_load_more);

  let padding = 3u16;
  let viewport = layout_chunk.height.saturating_sub(padding) as usize;
  let offset = app
    .track_table
    .scroll_offset
    .min(total_items.saturating_sub(viewport));
  let end = offset.saturating_add(viewport).min(total_items);
  let now = with_date.then(chrono::Utc::now);

  let mut visible_items = Vec::with_capacity(end.saturating_sub(offset));
  for row_idx in offset..end {
    if row_idx < matched_len {
      let track_idx = filtered_indices.as_ref().map_or(row_idx, |v| v[row_idx]);
      let item = &app.track_table.tracks[track_idx];
      let track_id = item.id.as_ref().map(|id| id.to_string());
      let date = match (
        now,
        app.track_table_added_at.get(track_idx).copied().flatten(),
      ) {
        (Some(now), Some(added_at)) => relative_date_at(added_at, now),
        _ => String::new(),
      };
      let mut cells = song_row_cells(
        &item.name,
        &create_artist_string(&item.artists),
        &item.album.name,
        &date,
        item.duration.num_milliseconds() as u128,
        with_date,
        b,
      );
      cells[0] = track_index_cell(app, &track_id, track_idx + 1);
      if show_remove {
        cells.push(" ✕ ".to_string());
      } else if show_in_playlist {
        let in_pl = item
          .id
          .as_ref()
          .is_some_and(|id| app.playlist_contains(&id.uri(), None));
        if in_pl {
          cells.push(app.user_config.padded_in_playlist_icon());
        } else {
          cells.push(" + ".to_string());
        }
      }
      visible_items.push(TableItem {
        id: track_id.unwrap_or_default(),
        format: cells,
      });
    } else {
      let label = if app.date_added_pending {
        "Loading full playlist...".to_string()
      } else {
        load_more_label("Load more songs...", app.track_table_remaining())
      };
      let mut load_more_format = vec!["".to_string(), label];
      if b.show_artist_column {
        load_more_format.push(String::new());
      }
      if with_date {
        if b.show_album_column {
          load_more_format.push(String::new());
        }
        if b.show_date_added_column {
          load_more_format.push(String::new());
        }
      } else if b.show_album_column {
        load_more_format.push(String::new());
      }
      if b.show_length_column {
        load_more_format.push(String::new());
      }
      if show_remove || show_in_playlist {
        load_more_format.push(String::new());
      }
      visible_items.push(TableItem {
        id: String::new(),
        format: load_more_format,
      });
    }
  }

  let title = match app.track_table.context {
    Some(TrackTableContext::MyPlaylists) => {
      let playlist = app.playlists.as_ref().and_then(|playlists| {
        playlists
          .items
          .get(app.selected_playlist_index.unwrap_or(0))
      });
      playlist
        .map(|playlist| {
          // Count every playlist item (episodes and unavailable tracks
          // included), matching the duration Spotify's header shows.
          let total_ms = app
            .playlist_tracks
            .as_ref()
            .map(|p| {
              p.items
                .iter()
                .filter_map(|item| match item.item.as_ref() {
                  Some(PlayableItem::Track(t)) => Some(t.duration.num_milliseconds()),
                  Some(PlayableItem::Episode(e)) => Some(e.duration.num_milliseconds()),
                  _ => None,
                })
                .sum::<i64>()
            })
            .unwrap_or(0);
          // "~" while pages are still loading: the sum only covers loaded tracks
          let approx = if app.track_table_has_more() { "~" } else { "" };
          format!(
            "{} ({} songs, {}{})",
            playlist.name,
            playlist.items.total,
            approx,
            format_playlist_duration(total_ms)
          )
        })
        .unwrap_or_else(|| "Songs".to_string())
    }
    _ => app
      .playlist_view
      .as_ref()
      .filter(|(name, _)| !name.is_empty())
      .map(|(name, _)| name.clone())
      .unwrap_or_else(|| "Songs".to_string()),
  };
  // In-playlist search box — rendered as a boxed widget on the title row (see
  // draw_playlist_search_box), so it is visibly clickable.
  let title = format!("{}{}", REFRESH_GLYPH, title);
  draw_table_window(
    f,
    app,
    layout_chunk,
    (title.as_str(), &header),
    &visible_items,
    total_items,
    offset,
    TableSelection {
      selected: app.track_table.selected_index,
      highlight: highlight_state,
      offset: Some(app.track_table.scroll_offset),
    },
  );
  draw_playlist_search_box(f, app, layout_chunk, &title);
}

/// A bordered search input on the table title row, positioned right after
/// the playlist name text. The box hugs its content so the title border
/// continues after it (`│ Search playlist │────`).
pub(super) fn draw_playlist_search_box(f: &mut Frame, app: &App, layout_chunk: Rect, title: &str) {
  if !matches!(
    app.track_table.context,
    Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
  ) {
    return;
  }
  let highlight_state = (
    app.get_current_route().active_block == ActiveBlock::TrackTable,
    app.get_current_route().hovered_block == ActiveBlock::TrackTable,
  );
  let theme = app.user_config.theme;
  let query = app.playlist_filter.as_deref().unwrap_or("");
  let focused = app.playlist_search_active();
  let title_chars = title.chars().count() as u16;
  let x = layout_chunk
    .x
    .saturating_add(1)
    .saturating_add(title_chars)
    .saturating_add(2);
  let avail = layout_chunk
    .x
    .saturating_add(layout_chunk.width)
    .saturating_sub(x);
  if avail < 4 {
    return;
  }
  // Ideal width hugging content; cap to available so long queries scroll
  // instead of clipping with "..." or disappearing.
  let ideal_content_w: u16 = if focused {
    if query.is_empty() {
      1
    } else {
      query.chars().count() as u16 + 3
    }
  } else if query.is_empty() {
    let placeholder: &str = PLAYLIST_SEARCH_PLACEHOLDER;
    UnicodeWidthStr::width(placeholder) as u16
  } else {
    query.chars().count() as u16
  };
  let ideal_box_w = ideal_content_w.saturating_add(2);
  let box_width = ideal_box_w.min(avail);
  let inner_w = box_width.saturating_sub(2) as usize;
  let reserve = if focused && !query.is_empty() { 3 } else { 0 };
  let viewport_w = inner_w.saturating_sub(reserve);

  let cursor_span = Span::styled("│", Style::default().fg(theme.active));
  let clear_style = Style::default()
    .fg(theme.active)
    .add_modifier(Modifier::BOLD);
  let spans: Vec<Span> = if focused {
    if query.is_empty() {
      vec![cursor_span]
    } else {
      use unicode_width::UnicodeWidthChar;
      let (visible, vis_cursor) = crate::tui::layout::search_input_visible(
        query,
        app.playlist_filter_cursor_position as usize,
        viewport_w,
      );
      // Split visible at cursor width
      let mut left = String::new();
      let mut right = String::new();
      let mut w = 0usize;
      for c in visible.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if w < vis_cursor {
          left.push(c);
          w += cw;
        } else {
          right.push(c);
        }
      }
      // If cursor is past the visible tail (at end), left already holds all
      let clear = Span::styled(" ✕", clear_style);
      if left.is_empty() && right.is_empty() {
        // should not happen when query non-empty, but guard
        vec![cursor_span, clear]
      } else if left.is_empty() {
        vec![cursor_span, Span::raw(right), clear]
      } else if right.is_empty() {
        vec![Span::raw(left), cursor_span, clear]
      } else {
        vec![Span::raw(left), cursor_span, Span::raw(right), clear]
      }
    }
  } else if query.is_empty() {
    let placeholder_glyph: &str = PLAYLIST_SEARCH_PLACEHOLDER;
    let placeholder = Span::styled(placeholder_glyph, Style::default().fg(theme.inactive));
    vec![placeholder]
  } else {
    let (visible, _) = crate::tui::layout::search_input_visible(query, 0, viewport_w);
    vec![Span::raw(visible)]
  };
  let rect = Rect::new(x, layout_chunk.y, box_width, 1);
  let box_widget = Paragraph::new(Line::from(spans))
    .style(Style::default().fg(theme.text))
    .block(
      Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(get_color(highlight_state, theme)),
    );
  f.render_widget(box_widget, rect);
}

pub fn draw_album_list(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let header = TableHeader {
    id: TableId::AlbumList,
    items: vec![
      TableHeaderItem {
        text: "Name",
        width: get_percentage_width(layout_chunk.width, 2.0 / 5.0),
        ..Default::default()
      },
      TableHeaderItem {
        text: "Artists",
        width: get_percentage_width(layout_chunk.width, 2.0 / 5.0),
        ..Default::default()
      },
      TableHeaderItem {
        text: "Release Date",
        width: get_percentage_width(layout_chunk.width, 1.0 / 5.0),
        ..Default::default()
      },
    ],
  };

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::AlbumList,
    current_route.hovered_block == ActiveBlock::AlbumList,
  );

  let selected_song_index = app.album_list_index;

  if let Some(saved_albums) = app.library.saved_albums.get_results(None) {
    let items = saved_albums
      .items
      .iter()
      .map(|album_page| TableItem {
        id: album_page.album.id.to_string(),
        format: vec![
          format!(
            "{}{}",
            app.user_config.padded_liked_icon(),
            &album_page.album.name
          ),
          create_artist_string(&album_page.album.artists),
          album_page.album.release_date.to_owned(),
        ],
      })
      .collect::<Vec<TableItem>>();

    draw_table(
      f,
      app,
      layout_chunk,
      (&format!("{}{}", REFRESH_GLYPH, "Saved Albums"), &header),
      &items,
      TableSelection {
        selected: selected_song_index,
        highlight: highlight_state,
        offset: None,
      },
    )
  };
}

pub fn draw_show_episodes(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let header = TableHeader {
    id: TableId::PodcastEpisodes,
    items: vec![
      TableHeaderItem {
        // Column to mark an episode as fully played
        text: "",
        width: 2,
        ..Default::default()
      },
      TableHeaderItem {
        text: "Date",
        width: get_percentage_width(layout_chunk.width, 0.5 / 5.0) - 2,
        ..Default::default()
      },
      TableHeaderItem {
        text: "Name",
        width: get_percentage_width(layout_chunk.width, 3.5 / 5.0),
        id: ColumnId::Title,
      },
      TableHeaderItem {
        text: "Duration",
        width: get_percentage_width(layout_chunk.width, 1.0 / 5.0),
        ..Default::default()
      },
    ],
  };

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::EpisodeTable,
    current_route.hovered_block == ActiveBlock::EpisodeTable,
  );

  if let Some(episodes) = app.library.show_episodes.get_results(None) {
    let items = episodes
      .items
      .iter()
      .map(|episode| {
        let (played_str, time_str) = match episode.resume_point {
          Some(ResumePoint {
            fully_played,
            resume_position,
          }) => (
            if fully_played {
              " ✔".to_owned()
            } else {
              "".to_owned()
            },
            format!(
              "{} / {}",
              millis_to_minutes(resume_position.num_milliseconds() as u128),
              millis_to_minutes(episode.duration.num_milliseconds() as u128)
            ),
          ),
          None => (
            "".to_owned(),
            millis_to_minutes(episode.duration.num_milliseconds() as u128),
          ),
        };
        TableItem {
          id: episode.id.to_string(),
          format: vec![
            played_str,
            episode.release_date.to_owned(),
            episode.name.to_owned(),
            time_str,
          ],
        }
      })
      .collect::<Vec<TableItem>>();

    let title = match &app.episode_table_context {
      EpisodeTableContext::Simplified => match &app.selected_show_simplified {
        Some(selected_show) => selected_show.show.name.to_owned().to_string(),
        None => "Episodes".to_owned(),
      },
      EpisodeTableContext::Full => match &app.selected_show_full {
        Some(selected_show) => selected_show.show.name.to_owned().to_string(),
        None => "Episodes".to_owned(),
      },
    };

    draw_table(
      f,
      app,
      layout_chunk,
      (&title, &header),
      &items,
      TableSelection {
        selected: app.episode_list_index,
        highlight: highlight_state,
        offset: None,
      },
    );
  };
}

pub fn draw_made_for_you(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let current_route = app.get_current_route();
  let paste_h = if layout_chunk.height >= 6 { 3 } else { 0 };
  let (paste_rect, list_rect) = (
    Rect::new(layout_chunk.x, layout_chunk.y, layout_chunk.width, paste_h),
    Rect::new(
      layout_chunk.x,
      layout_chunk.y + paste_h,
      layout_chunk.width,
      layout_chunk.height.saturating_sub(paste_h),
    ),
  );
  if paste_h > 0 {
    let paste_block = Block::default()
      .borders(Borders::ALL)
      .border_type(if app.user_config.terminal.unicode_borders {
        BorderType::Rounded
      } else {
        BorderType::Plain
      })
      .title("Add playlist to For you (Ctrl+V or click to paste link)")
      .border_style(Style::default().fg(app.user_config.theme.inactive));
    let preview = app
      .for_you_paste_input
      .as_deref()
      .filter(|s| !s.is_empty())
      .unwrap_or("https://open.spotify.com/playlist/... or spotify:playlist:...");
    let style = if app
      .for_you_paste_input
      .as_ref()
      .is_some_and(|s| !s.is_empty())
    {
      Style::default().fg(app.user_config.theme.text)
    } else {
      Style::default().fg(app.user_config.theme.inactive)
    };
    f.render_widget(
      Paragraph::new(Span::styled(preview, style)).block(paste_block),
      paste_rect,
    );
  }
  // The List content area is width-2 (borders eat 2 cols); name + " ✕" must
  // fit, so the name field gets width-4.
  let max_name = list_rect.width.saturating_sub(4) as usize;
  let names: Vec<String> = (0..app.made_for_you_len())
    .filter_map(|index| app.made_for_you_name(index))
    .map(|name| {
      let name: String = name.chars().take(max_name).collect();
      format!("{:<width$} ✕", name, width = max_name)
    })
    .collect();

  let highlight_state = (
    current_route.active_block == ActiveBlock::MadeForYou,
    current_route.hovered_block == ActiveBlock::MadeForYou,
  );

  draw_selectable_list(
    f,
    app,
    list_rect,
    "For you",
    &names,
    highlight_state,
    ListSelection {
      selected: if app.selection_engaged {
        Some(app.made_for_you_index)
      } else {
        None
      },
      hovered: app.hovered_list_index,
    },
  );
}

/// One row of the grouped Recently Played view: a run of consecutive plays
/// of the same track. `first_raw` indexes the raw history items so actions
/// (play, save, queue) resolve to the real entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayGroup {
  pub first_raw: usize,
  pub count: usize,
}

/// Collapse consecutive replays of the same track into one row, request-log
/// style. The group key is the track id; id-less items (episodes, local
/// files) never merge — each keeps its own row.
pub fn group_consecutive_plays(items: &[PlayHistory]) -> Vec<PlayGroup> {
  let mut groups: Vec<PlayGroup> = Vec::with_capacity(items.len());
  for (i, item) in items.iter().enumerate() {
    let continues = i > 0 && item.track.id.is_some() && item.track.id == items[i - 1].track.id;
    if continues {
      if let Some(last) = groups.last_mut() {
        last.count += 1;
      }
    } else {
      groups.push(PlayGroup {
        first_raw: i,
        count: 1,
      });
    }
  }
  groups
}

pub fn draw_recently_played_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    SongColumnFlags::for_view(b, false, false, false),
  );
  let header = TableHeader {
    id: TableId::RecentlyPlayed,
    items: columns
      .iter()
      .map(|(column, _, width)| TableHeaderItem {
        id: *column,
        text: match column {
          ColumnId::Title => "Title",
          ColumnId::Artist => "Artist",
          ColumnId::Album => "Album",
          ColumnId::Length => "Length",
          ColumnId::DateAdded => "Date Added",
          ColumnId::Liked => "#",
          _ => "",
        },
        width: *width,
      })
      .collect(),
  };

  if let Some(recently_played) = &app.recently_played.result {
    let current_route = app.get_current_route();

    let highlight_state = (
      current_route.active_block == ActiveBlock::RecentlyPlayed,
      current_route.hovered_block == ActiveBlock::RecentlyPlayed,
    );

    let selected_song_index = app.recently_played.index;

    let groups = group_consecutive_plays(&recently_played.items);
    let mut items = groups
      .iter()
      .enumerate()
      .map(|(index, group)| {
        let item = &recently_played.items[group.first_raw];
        let track_id = item.track.id.clone().map(|id| id.to_string());
        let title = if group.count > 1 {
          format!("{} (×{})", item.track.name, group.count)
        } else {
          item.track.name.clone()
        };
        let mut cells = song_row_cells(
          &title,
          &create_artist_string(&item.track.artists),
          &item.track.album.name,
          "",
          item.track.duration.num_milliseconds() as u128,
          false,
          b,
        );
        cells[0] = track_index_cell(app, &track_id, index + 1);
        TableItem {
          id: item
            .track
            .id
            .clone()
            .map(|id| id.to_string())
            .unwrap_or_default(),
          format: cells,
        }
      })
      .collect::<Vec<TableItem>>();

    if app.recently_played_has_more() {
      let mut load_more_format = vec!["".to_string(), "Load more songs...".to_string()];
      if b.show_artist_column {
        load_more_format.push(String::new());
      }
      if b.show_album_column {
        load_more_format.push(String::new());
      }
      if b.show_length_column {
        load_more_format.push(String::new());
      }
      items.push(TableItem {
        id: String::new(),
        format: load_more_format,
      });
    }

    draw_table(
      f,
      app,
      layout_chunk,
      ("Recently Played Tracks", &header),
      &items,
      TableSelection {
        selected: selected_song_index,
        highlight: highlight_state,
        offset: None,
      },
    )
  };
}

// website-style scrollbar just inside the right border of `rect`, only when
