use super::layout;
use super::layout::{
  create_artist_string, get_color, get_search_results_highlight_state, song_table_columns,
  SongColumnFlags,
};
use super::tables::{song_row_cells, track_index_cell};
use super::widgets::{
  draw_scrollbar, draw_selectable_list, draw_table, load_more_label, ColumnId, ListSelection,
  TableHeader, TableHeaderItem, TableId, TableItem, TableSelection,
};
use crate::app::{App, SearchResultBlock};
use ratatui::{layout::Rect, style::Modifier, Frame};
use rspotify::prelude::Id;

pub fn draw_search_results(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let theme = app.user_config.theme;
  let expanded = app.search_results.selected_block.clone();
  let has_more = app.search_block_has_more(&expanded);
  let (tab_bar, tab_cells, list_rect) =
    layout::search_layout(layout_chunk, expanded.clone(), has_more);

  // Tab bar: the collapsed tabs along the top. The expanded one is selected.
  for (block, rect) in tab_cells {
    let label = match block {
      SearchResultBlock::SongSearch => " Songs ",
      SearchResultBlock::ArtistSearch => " Artists ",
      SearchResultBlock::AlbumSearch => " Albums ",
      SearchResultBlock::PlaylistSearch => " Playlists ",
      SearchResultBlock::ShowSearch => " Podcasts ",
      SearchResultBlock::Empty => "",
    };
    let mut style = get_color(
      get_search_results_highlight_state(app, block.clone()),
      theme,
    );
    if block == expanded {
      style = style.add_modifier(Modifier::BOLD);
    }
    f.buffer_mut().set_string(rect.x, tab_bar.y, label, style);
  }

  match &expanded {
    SearchResultBlock::Empty => {
      let empty: Vec<String> = vec![];
      draw_selectable_list(
        f,
        app,
        list_rect,
        "",
        &empty,
        (false, false),
        ListSelection {
          selected: None,
          hovered: app.hovered_list_index,
        },
      );
    }
    SearchResultBlock::SongSearch => {
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
      let show_in_playlist = b.enable_add_to_playlist;
      let mut items = match &app.search_results.tracks {
        Some(tracks) => tracks
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
              cells
            },
          })
          .collect(),
        None => vec![],
      };
      if has_more {
        let remaining = app.search_results.tracks.as_ref().and_then(|t| {
          (t.total as usize > t.items.len()).then(|| t.total as usize - t.items.len())
        });
        let mut load_more_format = vec!["".to_string(), load_more_label(" Load more ", remaining)];
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
        list_rect,
        ("Songs", &header),
        &items,
        TableSelection {
          selected: app.search_results.selected_tracks_index.unwrap_or(0),
          highlight: get_search_results_highlight_state(app, expanded.clone()),
          offset: None,
        },
      );
      let selected = app.search_results.selected_tracks_index.unwrap_or(0);
      let count = items.len();
      let viewport = list_rect.height.saturating_sub(5) as usize;
      draw_scrollbar(
        f,
        app,
        list_rect,
        count,
        viewport,
        selected
          .checked_sub(viewport)
          .map(|o| o + 1)
          .unwrap_or(0)
          .min(count.saturating_sub(viewport)),
      );
    }
    _ => {
      let (items, title, selected) = search_block_items(app, expanded.clone());
      draw_selectable_list(
        f,
        app,
        list_rect,
        title,
        &items,
        get_search_results_highlight_state(app, expanded.clone()),
        ListSelection {
          selected,
          hovered: app.hovered_list_index,
        },
      );
    }
  }
}

/// The item rows, title and selected index for one search block.
pub(super) fn search_block_items(
  app: &App,
  block: SearchResultBlock,
) -> (Vec<String>, &'static str, Option<usize>) {
  let (mut items, title, selected) = match block {
    // SongSearch renders as a table directly in draw_search_results.
    SearchResultBlock::SongSearch => (vec![], "Songs", app.search_results.selected_tracks_index),
    SearchResultBlock::ArtistSearch => {
      let items = match &app.search_results.artists {
        Some(artists) => artists
          .items
          .iter()
          .map(|item| {
            let mut artist = String::new();
            if app.followed_artist_ids_set.contains(&item.id.to_string()) {
              artist.push_str(&app.user_config.padded_liked_icon());
            }
            artist.push_str(&item.name.to_owned());
            artist
          })
          .collect(),
        None => vec![],
      };
      (items, "Artists", app.search_results.selected_artists_index)
    }
    SearchResultBlock::AlbumSearch => {
      let items = match &app.search_results.albums {
        Some(albums) => albums
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
          .collect(),
        None => vec![],
      };
      (items, "Albums", app.search_results.selected_album_index)
    }
    SearchResultBlock::PlaylistSearch => {
      let items = match &app.search_results.playlists {
        Some(playlists) => playlists
          .items
          .iter()
          .map(|item| item.name.to_owned())
          .collect(),
        None => vec![],
      };
      (
        items,
        "Playlists",
        app.search_results.selected_playlists_index,
      )
    }
    SearchResultBlock::ShowSearch => {
      let items = match &app.search_results.shows {
        Some(podcasts) => podcasts
          .items
          .iter()
          .map(|item| {
            let mut show_name = String::new();
            if app.saved_show_ids_set.contains(&item.id.to_string()) {
              show_name.push_str(&app.user_config.padded_liked_icon());
            }
            show_name.push_str(&item.name);
            show_name
          })
          .collect(),
        None => vec![],
      };
      (items, "Podcasts", app.search_results.selected_shows_index)
    }
    SearchResultBlock::Empty => (vec![], "", None),
  };
  if app.search_block_has_more(&block) {
    let remaining = match block {
      SearchResultBlock::ArtistSearch => app
        .search_results
        .artists
        .as_ref()
        .map(|p| p.total.saturating_sub(p.items.len() as u32)),
      SearchResultBlock::AlbumSearch => app
        .search_results
        .albums
        .as_ref()
        .map(|p| p.total.saturating_sub(p.items.len() as u32)),
      SearchResultBlock::PlaylistSearch => app
        .search_results
        .playlists
        .as_ref()
        .map(|p| p.total.saturating_sub(p.items.len() as u32)),
      SearchResultBlock::ShowSearch => app
        .search_results
        .shows
        .as_ref()
        .map(|p| p.total.saturating_sub(p.items.len() as u32)),
      _ => None,
    };
    items.push(load_more_label(
      " Load more ",
      remaining.map(|r| r as usize),
    ));
  }
  (items, title, selected)
}
