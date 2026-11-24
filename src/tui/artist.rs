use super::layout;
use super::layout::{
  create_artist_string, get_artist_highlight_state, get_color, song_table_columns, SongColumnFlags,
};
use super::tables::{song_row_cells, track_index_cell};
use super::widgets::{
  draw_selectable_list, draw_table, ellipsize, load_more_label, ColumnId, ListSelection,
  TableHeader, TableHeaderItem, TableId, TableItem, TableSelection,
};
use crate::app::{App, ArtistBlock};
use ratatui::{
  layout::Rect,
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, BorderType, Borders, Paragraph},
  Frame,
};
use rspotify::prelude::Id;

fn draw_artist_header(f: &mut Frame, app: &App, artist: &crate::app::Artist, header: Rect) {
  let theme = app.user_config.theme;
  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .style(Style::default().fg(theme.inactive));
  f.render_widget(block, header);

  let cover = layout::artist_cover_rect(header);
  let follow_rect = layout::artist_follow_rect(header);
  let text_x = cover.x + cover.width + 2;
  let text_w = follow_rect.x.saturating_sub(text_x + 1);
  if text_w > 0 {
    let name_area = Rect::new(text_x, header.y + 1, text_w, 1);
    f.render_widget(
      Paragraph::new(Span::styled(
        ellipsize(&artist.artist_name, text_w as usize).into_owned(),
        Style::default()
          .fg(theme.selected)
          .add_modifier(Modifier::BOLD),
      )),
      name_area,
    );

    let mut stats_parts: Vec<String> = Vec::new();
    if let Some(fcount) = artist.followers.filter(|&n| n > 0) {
      stats_parts.push(format!(
        "{} followers",
        crate::metadata::format_compact_count(fcount as u64)
      ));
    }
    if let Some(meta) = &artist.meta {
      if let Some(kind) = &meta.kind {
        stats_parts.push(kind.clone());
      }
      if let Some(lcount) = meta.listeners {
        stats_parts.push(format!(
          "{} listeners",
          crate::metadata::format_compact_count(lcount)
        ));
      }
      if let Some(country) = &meta.country {
        stats_parts.push(country.clone());
      }
      if !meta.tags.is_empty() {
        stats_parts.push(meta.tags.join(" · "));
      } else if !artist.genres.is_empty() {
        stats_parts.push(
          artist
            .genres
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · "),
        );
      }
    } else if !artist.genres.is_empty() {
      stats_parts.push(
        artist
          .genres
          .iter()
          .take(3)
          .cloned()
          .collect::<Vec<_>>()
          .join(" · "),
      );
    }

    let info_w = header.width.saturating_sub(cover.width + 4);
    if info_w > 0 {
      let summary = if stats_parts.is_empty() {
        "Spotify Artist".to_string()
      } else {
        stats_parts.join("  |  ")
      };
      let stats_line = ellipsize(&summary, info_w as usize).into_owned();
      f.render_widget(
        Paragraph::new(Span::styled(
          stats_line,
          Style::default().fg(theme.inactive),
        )),
        Rect::new(text_x, header.y + 2, info_w, 1),
      );
    }

    if let Some(bio) = artist.meta.as_ref().and_then(|m| m.bio.as_deref()) {
      if info_w > 0 && header.height >= 5 {
        let bio_line = ellipsize(bio, info_w as usize).into_owned();
        f.render_widget(
          Paragraph::new(Line::from(Span::styled(
            bio_line,
            Style::default().fg(theme.text),
          ))),
          Rect::new(text_x, header.y + 3, info_w, 1),
        );
      }
    }
  }

  let (follow_label, follow_style) = match artist.is_following {
    Some(true) => (
      "[Following ✓]",
      Style::default()
        .fg(theme.selected)
        .add_modifier(Modifier::BOLD),
    ),
    _ => ("[+ Follow]", Style::default().fg(theme.active)),
  };
  if follow_rect.width > 0 {
    f.buffer_mut()
      .set_string(follow_rect.x, follow_rect.y, follow_label, follow_style);
  }
}

pub(super) fn draw_artist_page(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let Some(artist) = &app.artist else {
    return;
  };
  if let Some(header) = layout::artist_header_rect(layout_chunk) {
    draw_artist_header(f, app, artist, header);
  }
  let shown = if artist.artist_selected_block == ArtistBlock::Empty {
    artist.artist_hovered_block
  } else {
    artist.artist_selected_block
  };
  let theme = app.user_config.theme;
  let (tab_bar, tab_cells, list_rect) = layout::artist_layout(layout_chunk, shown);

  for (block, rect) in tab_cells {
    let label = match block {
      ArtistBlock::TopTracks => " Artist ",
      ArtistBlock::Albums => " Albums ",
      ArtistBlock::Singles => " Singles ",
      ArtistBlock::EPs => " EPs ",
      ArtistBlock::AppearsOn => " Featuring ",
      ArtistBlock::Empty => "",
    };
    let mut style = get_color(get_artist_highlight_state(app, block), theme);
    if block == shown {
      style = style.add_modifier(Modifier::BOLD);
    }
    f.buffer_mut().set_string(rect.x, tab_bar.y, label, style);
  }

  match shown {
    ArtistBlock::TopTracks => {
      let b = &app.user_config.behavior;
      let show_in_playlist = b.enable_add_to_playlist;
      let columns = song_table_columns(
        layout_chunk.width.saturating_sub(2),
        SongColumnFlags::for_view(b, false, false, show_in_playlist),
      );
      let header = TableHeader {
        id: TableId::Song,
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
      let mut items = artist
        .top_tracks
        .iter()
        .enumerate()
        .map(|(index, item)| {
          let track_id = item.id.clone().map(|id| id.to_string());
          let mut cells = song_row_cells(
            &item.name,
            &create_artist_string(&item.artists),
            &item.album.name,
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
          TableItem {
            id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
            format: cells,
          }
        })
        .collect::<Vec<TableItem>>();
      if artist.top_tracks_has_more {
        let remaining = artist
          .top_tracks_total
          .saturating_sub(artist.top_tracks.len());
        let mut load_more_format = vec![
          "".to_string(),
          load_more_label("Load more songs...", (remaining > 0).then_some(remaining)),
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
        items.push(TableItem {
          id: String::new(),
          format: load_more_format,
        });
      }
      let title = format!("{} - Artist", artist.artist_name);
      draw_table(
        f,
        app,
        list_rect,
        (title.as_str(), &header),
        &items,
        TableSelection {
          selected: artist.selected_top_track_index,
          highlight: get_artist_highlight_state(app, shown),
          offset: None,
        },
      );
    }
    _ => {
      let (items, title, selected) = match shown {
        ArtistBlock::Albums => {
          let mut albums = artist
            .albums
            .items
            .iter()
            .map(|item| {
              let mut album_artist = String::new();
              if let Some(album_id) = &item.id {
                if app.saved_album_ids_set.contains(&album_id.to_string()) {
                  album_artist.push_str(&app.user_config.padded_liked_icon());
                }
              }
              album_artist.push_str(&format!(
                "{} - {} ({})",
                item.name.to_owned(),
                create_artist_string(&item.artists),
                item.album_type.as_deref().unwrap_or("unknown")
              ));
              album_artist
            })
            .collect::<Vec<String>>();
          if artist.albums.items.len() < artist.albums.total as usize {
            let remaining = artist.albums.total as usize - artist.albums.items.len();
            albums.push(load_more_label("Load more albums...", Some(remaining)));
          }
          (
            albums,
            "Albums".to_string(),
            Some(artist.selected_album_index),
          )
        }
        ArtistBlock::Singles => {
          let mut singles = artist
            .singles
            .items
            .iter()
            .map(|item| {
              let mut s = String::new();
              if let Some(album_id) = &item.id {
                if app.saved_album_ids_set.contains(&album_id.to_string()) {
                  s.push_str(&app.user_config.padded_liked_icon());
                }
              }
              s.push_str(&format!(
                "{} - {} ({})",
                item.name,
                create_artist_string(&item.artists),
                item.album_type.as_deref().unwrap_or("single")
              ));
              s
            })
            .collect::<Vec<String>>();
          if artist.singles.items.len() < artist.singles.total as usize {
            let remaining = artist.singles.total as usize - artist.singles.items.len();
            singles.push(load_more_label("Load more singles...", Some(remaining)));
          }
          (
            singles,
            "Singles".to_string(),
            Some(artist.selected_singles_index),
          )
        }
        ArtistBlock::EPs => {
          let mut eps = artist
            .eps
            .items
            .iter()
            .map(|item| {
              let mut s = String::new();
              if let Some(album_id) = &item.id {
                if app.saved_album_ids_set.contains(&album_id.to_string()) {
                  s.push_str(&app.user_config.padded_liked_icon());
                }
              }
              s.push_str(&format!(
                "{} - {} (EP)",
                item.name,
                create_artist_string(&item.artists)
              ));
              s
            })
            .collect::<Vec<String>>();
          if artist.eps.items.len() < artist.eps.total as usize {
            let remaining = artist.eps.total as usize - artist.eps.items.len();
            eps.push(load_more_label("Load more EPs...", Some(remaining)));
          }
          (eps, "EPs".to_string(), Some(artist.selected_eps_index))
        }
        ArtistBlock::AppearsOn => {
          let mut items = artist
            .appears_on
            .items
            .iter()
            .map(|item| {
              let mut s = String::new();
              if let Some(album_id) = &item.id {
                if app.saved_album_ids_set.contains(&album_id.to_string()) {
                  s.push_str(&app.user_config.padded_liked_icon());
                }
              }
              s.push_str(&format!(
                "{} - {}",
                item.name,
                create_artist_string(&item.artists)
              ));
              s
            })
            .collect::<Vec<String>>();
          if artist.appears_on.items.len() < artist.appears_on.total as usize {
            let remaining = artist.appears_on.total as usize - artist.appears_on.items.len();
            items.push(load_more_label("Load more featuring...", Some(remaining)));
          }
          (
            items,
            "Featuring".to_string(),
            Some(artist.selected_appears_on_index),
          )
        }
        ArtistBlock::Empty | ArtistBlock::TopTracks => (vec![], String::new(), None),
      };

      draw_selectable_list(
        f,
        app,
        list_rect,
        &title,
        &items,
        get_artist_highlight_state(app, shown),
        ListSelection {
          selected,
          hovered: app.hovered_list_index,
        },
      );
    }
  }
}
