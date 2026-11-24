use super::layout::{create_artist_string, song_table_columns, SongColumnFlags};
use super::tables::{song_row_cells, track_index_cell};
use super::widgets::{
  draw_table, ColumnId, TableHeader, TableHeaderItem, TableId, TableItem, TableSelection,
};
use crate::app::{ActiveBlock, App};
use ratatui::{layout::Rect, Frame};
use rspotify::model::PlayableItem;

pub(super) fn draw_queue_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    SongColumnFlags::for_view(b, false, false, false),
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
          ColumnId::Liked => "#",
          _ => "",
        },
        width: *width,
      })
      .collect(),
  };

  let current_route = app.get_current_route();
  let highlight_state = (
    current_route.active_block == ActiveBlock::Queue,
    current_route.hovered_block == ActiveBlock::Queue,
  );

  let items: Vec<TableItem> = app
    .queue_items
    .iter()
    .enumerate()
    .map(|(idx, item)| match item {
      PlayableItem::Track(t) => {
        let track_id = t.id.as_ref().map(|id| id.to_string());
        let title = if t.explicit {
          format!("🅴 {}", t.name)
        } else {
          t.name.clone()
        };
        let mut cells = song_row_cells(
          &title,
          &create_artist_string(&t.artists),
          &t.album.name,
          "",
          t.duration.num_milliseconds() as u128,
          false,
          b,
        );
        cells[0] = track_index_cell(app, &track_id, idx + 1);
        TableItem {
          id: track_id.unwrap_or_default(),
          format: cells,
        }
      }
      PlayableItem::Episode(e) => {
        let mut cells = song_row_cells(
          &e.name,
          &e.show.name,
          "",
          "",
          e.duration.num_milliseconds() as u128,
          false,
          b,
        );
        cells[0] = format!("{:>2}  ", idx + 1);
        TableItem {
          id: e.id.to_string(),
          format: cells,
        }
      }
      PlayableItem::Unknown(_) => TableItem {
        id: String::new(),
        format: vec![format!("{:>2}  ", idx + 1), "Unknown".to_string()],
      },
    })
    .collect();

  draw_table(
    f,
    app,
    layout_chunk,
    ("Queue (Up Next)", &header),
    &items,
    TableSelection {
      selected: app.queue_selected_index,
      highlight: highlight_state,
      offset: None,
    },
  );
}
