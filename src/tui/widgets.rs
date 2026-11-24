use super::layout;
use super::layout::get_color;
use crate::app::App;
use ratatui::{
  layout::Rect,
  style::{Modifier, Style},
  text::Span,
  widgets::{Block, BorderType, Borders, List, ListItem, ListState},
  Frame,
};
use rspotify::model::PlayableItem;
use unicode_width::UnicodeWidthStr;

pub enum TableId {
  Album,
  AlbumList,
  Artist,
  Podcast,
  Song,
  RecentlyPlayed,
  PodcastEpisodes,
}

#[derive(PartialEq, Clone, Copy)]
/// Selected + hovered row of a list view. Bundles two `Option<usize>`
/// params so list-drawing helpers stay under clippy's argument limit.
pub struct ListSelection {
  pub selected: Option<usize>,
  pub hovered: Option<usize>,
}

/// Selected row, highlight state and scroll offset of a table view.
pub struct TableSelection {
  pub selected: usize,
  pub highlight: (bool, bool),
  pub offset: Option<usize>,
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum ColumnId {
  #[default]
  None,
  Title,
  Liked,
  Album,
  Artist,
  Length,
  DateAdded,
}

pub struct TableHeader<'a> {
  pub(super) id: TableId,
  pub(super) items: Vec<TableHeaderItem<'a>>,
}

impl TableHeader<'_> {
  pub fn get_index(&self, id: ColumnId) -> Option<usize> {
    self.items.iter().position(|item| item.id == id)
  }
}

#[derive(Default)]
pub struct TableHeaderItem<'a> {
  pub(super) id: ColumnId,
  pub(super) text: &'a str,
  pub(super) width: u16,
}

pub struct TableItem {
  pub(super) id: String,
  pub(super) format: Vec<String>,
}

pub(super) fn load_more_label(base: &str, remaining: Option<usize>) -> String {
  match remaining.filter(|n| *n > 0) {
    Some(n) => format!("{base} ({n} more)"),
    None => base.to_string(),
  }
}

/// Truncate `input` to fit within `max_width` display columns, appending `"..."`
/// when truncated. Returns a borrowed slice when it already fits.
pub(super) fn ellipsize<'a>(input: &'a str, max_width: usize) -> std::borrow::Cow<'a, str> {
  use std::borrow::Cow;
  use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
  if UnicodeWidthStr::width(input) <= max_width {
    return Cow::Borrowed(input);
  }
  if max_width <= 3 {
    return Cow::Borrowed(&"..."[..max_width]);
  }
  let budget = max_width - 3;
  let mut w = 0usize;
  let mut out = String::with_capacity(input.len().min(max_width));
  for c in input.chars() {
    let cw = UnicodeWidthChar::width(c).unwrap_or(0);
    if w + cw > budget {
      break;
    }
    w += cw;
    out.push(c);
  }
  out.push_str("...");
  Cow::Owned(out)
}

// `count` overflows `viewport`. `offset` is the number of scrolled items
// (view offset, or selected_index - viewport for selection lists). Shared by
// the gear menu, sidebar lists and the track table; the mouse drag arm uses
// the same geometry (see handlers/mouse.rs arm_scrollbar).
pub(super) fn draw_scrollbar(
  f: &mut Frame,
  app: &App,
  rect: Rect,
  count: usize,
  viewport: usize,
  offset: usize,
) {
  if count <= viewport {
    return;
  }
  let track_h = rect.height.saturating_sub(2) as usize;
  let (thumb_top, thumb_len) = layout::scrollbar_geometry(track_h, count, viewport, offset);
  let scrollbar_rect = Rect::new(
    rect.x + rect.width.saturating_sub(2),
    rect.y + 1,
    1,
    track_h as u16,
  );
  let thumb_style = Style::default().fg(app.user_config.theme.playbar_progress);
  let track_style = Style::default().fg(app.user_config.theme.inactive);
  for i in 0..track_h {
    let (symbol, style) = if i >= thumb_top && i < thumb_top + thumb_len {
      ("█", thumb_style)
    } else {
      ("│", track_style)
    };
    f.buffer_mut()
      .set_string(scrollbar_rect.x, scrollbar_rect.y + i as u16, symbol, style);
  }
}

pub(super) fn draw_selectable_list<S>(
  f: &mut Frame,
  app: &App,
  layout_chunk: Rect,
  title: &str,
  items: &[S],
  highlight_state: (bool, bool),
  selection: ListSelection,
) where
  S: std::convert::AsRef<str>,
{
  // Keep the selection visible: render a window that ends at the selection.
  let viewport = layout_chunk.height.saturating_sub(2) as usize;
  let offset = match selection.selected {
    Some(selected) => selected.saturating_sub(viewport),
    None => 0,
  };
  let mut state = ListState::default();
  state.select(selection.selected.map(|selected| selected - offset));

  // Only build the visible window of rows (selection-anchored via `offset`),
  // not the whole list every frame — playlist/song lists can be very long and
  // ratatui only ever draws the viewport anyway. Cap to `viewport` rows.
  let lst_items: Vec<ListItem> = items
    .iter()
    .enumerate()
    .skip(offset)
    .take(viewport)
    .map(|(i, item)| {
      let is_load_more =
        i == items.len().saturating_sub(1) && item.as_ref().trim_start().starts_with("Load more");
      // Graceful truncation: keep the head + "..." so long labels never get cut
      // mid-word by the list's hard edge. Inner width = bordered chunk minus 1.
      let max_len = app.user_config.behavior.max_display_length as usize;
      let mut inner_w = layout_chunk.width.saturating_sub(1) as usize;
      if title == "Playlists" {
        inner_w = (inner_w / 2).max(10);
      }
      let fit = ellipsize(
        item.as_ref(),
        if max_len > 0 {
          inner_w.min(max_len)
        } else {
          inner_w
        },
      );
      let is_hovered = app.user_config.behavior.enable_animations
        && selection.hovered == Some(i)
        && selection.selected != Some(i);
      let item = if is_load_more {
        ListItem::new(Span::styled(
          fit.clone(),
          Style::default()
            .fg(app.user_config.theme.load_more)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        ))
      } else if is_hovered {
        let w = UnicodeWidthStr::width(fit.as_ref());
        let pad = inner_w.saturating_sub(w);
        let t = app.user_config.theme;
        ListItem::new(Span::styled(
          format!("{}{}", fit, " ".repeat(pad)),
          Style::default().fg(t.text).bg(t.hovered),
        ))
      } else {
        ListItem::new(Span::raw(fit))
      };
      item
    })
    .collect();

  // Only the focused panel highlights its selected row; an unfocused panel
  // shows its rows in the neutral text color (ratatui otherwise always
  // applies the highlight style to the selected row).
  let focused = highlight_state.0 || highlight_state.1;
  let list = List::new(lst_items)
    .block(
      Block::default()
        .title(Span::styled(
          title,
          get_color(highlight_state, app.user_config.theme),
        ))
        .borders(Borders::ALL)
        .border_type(if app.user_config.terminal.unicode_borders {
          BorderType::Rounded
        } else {
          BorderType::Plain
        })
        .border_style(get_color(highlight_state, app.user_config.theme)),
    )
    .style(Style::default().fg(app.user_config.theme.text))
    .highlight_style(if focused {
      get_color(highlight_state, app.user_config.theme).add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(app.user_config.theme.text)
    });
  f.render_stateful_widget(list, layout_chunk, &mut state);

  // website-style scrollbar just inside the right border, only when the list overflows
  draw_scrollbar(f, app, layout_chunk, items.len(), viewport, offset);
}

pub(super) fn draw_table(
  f: &mut Frame,
  app: &App,
  layout_chunk: Rect,
  table_layout: (&str, &TableHeader), // (title, header colums)
  items: &[TableItem], // The nested vector must have the same length as the `header_columns`
  selection: TableSelection,
) {
  let padding = 3;
  let viewport = layout_chunk.height.saturating_sub(padding) as usize;
  let offset = match selection.offset {
    Some(scrolled) => scrolled.min(items.len().saturating_sub(viewport)),
    None => selection
      .selected
      .checked_sub(viewport)
      .map(|o| o + 1)
      .unwrap_or(0)
      .min(items.len().saturating_sub(viewport)),
  };
  let end = offset.saturating_add(viewport).min(items.len());
  let visible = items.get(offset..end).unwrap_or(&[]);
  draw_table_window(
    f,
    app,
    layout_chunk,
    table_layout,
    visible,
    items.len(),
    offset,
    selection,
  );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_table_window(
  f: &mut Frame,
  app: &App,
  layout_chunk: Rect,
  table_layout: (&str, &TableHeader),
  visible_items: &[TableItem],
  total_items: usize,
  offset: usize,
  selection: TableSelection,
) {
  let selected_style =
    get_color(selection.highlight, app.user_config.theme).add_modifier(Modifier::BOLD);

  let playing_id: Option<String> = app
    .current_playback_context
    .as_ref()
    .and_then(|ctx| ctx.item.as_ref())
    .and_then(|item| match item {
      PlayableItem::Track(track) => track.id.as_ref().map(|id| id.to_string()),
      PlayableItem::Episode(episode) => Some(episode.id.to_string()),
      _ => None,
    });

  let track_playing_offset_index = playing_id
    .as_deref()
    .and_then(|pid| visible_items.iter().position(|item| item.id == pid));

  let (title, header) = table_layout;
  let padding = 3;
  let viewport = layout_chunk.height.saturating_sub(padding) as usize;
  let title_gap = " ".repeat(crate::tui::layout::NUMBER_TO_TITLE_GAP as usize);
  let dim = Style::default().fg(app.user_config.theme.inactive);

  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .style(Style::default().fg(app.user_config.theme.text))
    .title(Span::styled(
      title,
      get_color(selection.highlight, app.user_config.theme),
    ))
    .border_style(get_color(selection.highlight, app.user_config.theme));
  let inner = block.inner(layout_chunk);
  f.render_widget(block, layout_chunk);

  if inner.width == 0 || inner.height == 0 {
    draw_scrollbar(f, app, layout_chunk, total_items, viewport, offset);
    return;
  }

  let buf = f.buffer_mut();
  let right_edge = inner.x + inner.width;
  let header_style = Style::default()
    .fg(app.user_config.theme.header)
    .add_modifier(Modifier::BOLD);
  let text_style = Style::default().fg(app.user_config.theme.text);
  let hover_bg = Style::default().bg(app.user_config.theme.hovered);
  let max_len = app.user_config.behavior.max_display_length as usize;

  // Header row (inner.y)
  let mut col_x = inner.x;
  for h in &header.items {
    if col_x >= right_edge {
      break;
    }
    let avail = (right_edge - col_x).min(h.width) as usize;
    if matches!(h.id, ColumnId::Liked | ColumnId::None) {
      let w = UnicodeWidthStr::width(h.text);
      let pad = (h.width as usize).saturating_sub(w);
      let start_x = col_x.saturating_add(pad as u16);
      if start_x < right_edge {
        buf.set_stringn(
          start_x,
          inner.y,
          h.text,
          (right_edge - start_x) as usize,
          header_style,
        );
      }
    } else if matches!(h.id, ColumnId::Title) {
      let gap = title_gap.len().min(avail);
      if gap > 0 {
        buf.set_stringn(col_x, inner.y, &title_gap, gap, header_style);
      }
      if avail > gap {
        buf.set_stringn(
          col_x + gap as u16,
          inner.y,
          h.text,
          avail - gap,
          header_style,
        );
      }
    } else {
      buf.set_stringn(col_x, inner.y, h.text, avail, header_style);
    }
    col_x = col_x.saturating_add(h.width);
  }

  // Visible body rows (inner.y + 1 ..)
  let body_height = inner.height.saturating_sub(1) as usize;
  for (i, item) in visible_items.iter().take(body_height).enumerate() {
    let row_y = inner.y + 1 + i as u16;
    let mut style = text_style;
    let mut playing_title_override: Option<String> = None;

    match header.id {
      TableId::Song | TableId::RecentlyPlayed | TableId::Album | TableId::PodcastEpisodes => {
        if let Some(title_idx) = header.get_index(ColumnId::Title) {
          if track_playing_offset_index == Some(i) {
            playing_title_override = Some(format!("▶ {}", item.format[title_idx]));
            style = Style::default()
              .fg(app.user_config.theme.active)
              .add_modifier(Modifier::BOLD);
          }
        }
      }
      _ => {}
    }

    if offset + i == total_items.saturating_sub(1) && item.id.is_empty() {
      style = Style::default()
        .fg(app.user_config.theme.load_more)
        .add_modifier(Modifier::BOLD | Modifier::ITALIC);
    }

    let is_hovered = app.user_config.behavior.enable_animations
      && app.hovered_list_index == Some(offset + i)
      && !(app.selection_engaged && selection.selected == offset + i);

    if app.selection_engaged && Some(i) == selection.selected.checked_sub(offset) {
      style = selected_style;
    }

    if style != text_style {
      buf.set_style(Rect::new(inner.x, row_y, inner.width, 1), style);
    }

    let title_col_idx = header.get_index(ColumnId::Title);
    let mut col_x = inner.x;
    for (idx, raw_text) in item.format.iter().enumerate() {
      if col_x >= right_edge {
        break;
      }
      let Some(h) = header.items.get(idx) else {
        break;
      };
      let col_w = h.width as usize;
      let avail = (right_edge - col_x).min(h.width) as usize;
      let col_id = h.id;
      let text = if Some(idx) == title_col_idx {
        playing_title_override.as_deref().unwrap_or(raw_text)
      } else {
        raw_text.as_str()
      };
      let col_width = col_w.saturating_sub(1);
      let effective_width =
        if max_len > 0 && matches!(col_id, ColumnId::Title | ColumnId::Artist | ColumnId::Album) {
          col_width.min(max_len)
        } else {
          col_width
        };
      let padded: std::borrow::Cow<'_, str> = if matches!(col_id, ColumnId::Title) {
        std::borrow::Cow::Owned(format!("{}{}", title_gap, text))
      } else {
        std::borrow::Cow::Borrowed(text)
      };
      let fit = if matches!(col_id, ColumnId::Liked | ColumnId::None) {
        padded
      } else {
        match ellipsize(&padded, effective_width) {
          std::borrow::Cow::Borrowed(_) => padded,
          std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s),
        }
      };
      let is_secondary = matches!(
        col_id,
        ColumnId::Artist | ColumnId::Album | ColumnId::DateAdded | ColumnId::Length
      );
      let base_style = if is_secondary {
        style.patch(dim)
      } else {
        style
      };
      if is_hovered && !matches!(col_id, ColumnId::None) {
        if matches!(col_id, ColumnId::Liked) {
          let trimmed = fit.trim_start();
          let leading = fit.len().saturating_sub(trimmed.len());
          let h_rect = Rect::new(
            col_x.saturating_add(leading as u16),
            row_y,
            (avail.saturating_sub(leading)) as u16,
            1,
          );
          buf.set_style(h_rect, base_style.patch(hover_bg));
          buf.set_stringn(col_x, row_y, fit.as_ref(), avail, base_style);
        } else {
          buf.set_style(
            Rect::new(col_x, row_y, avail as u16, 1),
            base_style.patch(hover_bg),
          );
          buf.set_stringn(
            col_x,
            row_y,
            fit.as_ref(),
            avail,
            base_style.patch(hover_bg),
          );
        }
      } else {
        buf.set_stringn(col_x, row_y, fit.as_ref(), avail, base_style);
      }
      col_x = col_x.saturating_add(h.width);
    }
  }

  // website-style scrollbar just inside the right border, only when the list overflows
  draw_scrollbar(f, app, layout_chunk, total_items, viewport, offset);
}
