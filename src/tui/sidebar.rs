use super::layout;
use super::layout::get_color;
use super::widgets::{draw_scrollbar, draw_selectable_list, ellipsize, ListSelection};
use crate::app::{visible_library_options, ActiveBlock, App};
use ratatui::{
  layout::Rect,
  style::{Modifier, Style},
  text::Span,
  widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
  Frame,
};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

pub fn draw_library_block(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let current_route = app.get_current_route();
  let highlight_state = (
    current_route.active_block == ActiveBlock::Library
      || app.sidebar_latched_block == Some(ActiveBlock::Library),
    current_route.hovered_block == ActiveBlock::Library,
  );
  let visible = visible_library_options(&app.hidden_library_sections);
  let (title, items): (String, Vec<String>) = if app.sidebar_minimized {
    // Minimized: single-letter glyphs so each row fits the narrow column.
    let glyphs = visible
      .iter()
      .map(|name| {
        name
          .chars()
          .next()
          .map(|c| c.to_string())
          .unwrap_or_default()
      })
      .collect();
    (crate::tui::layout::SIDEBAR_TOGGLE_GLYPH.to_string(), glyphs)
  } else {
    (
      crate::app::library_block_title(app),
      visible.iter().map(|s| s.to_string()).collect(),
    )
  };
  draw_selectable_list(
    f,
    app,
    layout_chunk,
    &title,
    &items,
    highlight_state,
    ListSelection {
      selected: Some(
        app
          .library
          .selected_index
          .min(items.len().saturating_sub(1)),
      ),
      hovered: app.hovered_library_index,
    },
  );
}

// Rebuilding the playlist name list on every frame is O(n) over the whole
// library (thousands of String allocations per frame). The set only changes
// when a new page is assigned (new pointer), grown (new length), or the
// sidebar collapsed state flips, so key on all three and rebuild only then.
thread_local! {
  static PLAYLIST_ITEMS: std::cell::RefCell<(usize, usize, bool, Vec<String>)> =
    const { std::cell::RefCell::new((0, 0, false, Vec::new())) }; // (page_ptr, len, minimized, items)
}

pub fn bust_playlist_items_cache() {
  PLAYLIST_ITEMS.with(|c| *c.borrow_mut() = (0, 0, false, Vec::new()));
}

pub fn draw_playlist_block(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::MyPlaylists
      || app.sidebar_latched_block == Some(ActiveBlock::MyPlaylists),
    current_route.hovered_block == ActiveBlock::MyPlaylists,
  );

  let title = if app.sidebar_minimized {
    "P".to_string()
  } else {
    crate::app::playlists_block_title()
  };

  PLAYLIST_ITEMS.with(|cell| {
    let mut slot = cell.borrow_mut();
    let key = app
      .playlists
      .as_ref()
      .map(|p| (p as *const _ as usize, p.items.len(), app.sidebar_minimized))
      .unwrap_or((0, 0, app.sidebar_minimized));
    if slot.0 != key.0 || slot.1 != key.1 || slot.2 != key.2 {
      slot.3 = match &app.playlists {
        Some(p) => p
          .items
          .iter()
          .enumerate()
          .map(|(i, item)| {
            if app.sidebar_minimized {
              format!("{}", i + 1)
            } else {
              item.name.clone()
            }
          })
          .collect(),
        None => vec![],
      };
      slot.0 = key.0;
      slot.1 = key.1;
      slot.2 = key.2;
    }
    let items: &Vec<String> = &slot.3;
    draw_selectable_list(
      f,
      app,
      layout_chunk,
      &title,
      items,
      highlight_state,
      ListSelection {
        selected: app.selected_playlist_index,
        hovered: app.hovered_playlist_index,
      },
    );
  });
}

pub(super) fn draw_sidebar_section<S>(
  f: &mut Frame,
  app: &App,
  title: &str,
  items: &[S],
  highlight_state: (bool, bool),
  selection: ListSelection,
  rect: Rect,
) where
  S: AsRef<str>,
{
  if rect.height == 0 || rect.width == 0 {
    return;
  }
  let theme = app.user_config.theme;
  let title_style = get_color(highlight_state, theme).add_modifier(Modifier::BOLD);
  let is_lib_expanded = title.contains("Library") && !app.sidebar_minimized && rect.width >= 4;
  if is_lib_expanded {
    // Single-row header: Library≡ — ≡ immediately after Library (at title width).
    let toggle = crate::tui::layout::SIDEBAR_TOGGLE_GLYPH;
    let title_w = UnicodeWidthStr::width(title) as u16;
    f.render_widget(
      Paragraph::new(Span::styled(title.to_string(), title_style)),
      Rect::new(rect.x, rect.y, title_w.min(rect.width.saturating_sub(1)), 1),
    );
    if title_w < rect.width {
      f.render_widget(
        Paragraph::new(Span::styled(toggle.to_string(), title_style)),
        Rect::new(rect.x + title_w, rect.y, 1, 1),
      );
    }
    if rect.height <= 1 {
      return;
    }
    let list_rect = Rect::new(
      rect.x,
      rect.y + 1,
      rect.width,
      rect.height.saturating_sub(1),
    );
    let viewport = list_rect.height as usize;
    let offset = match selection.selected {
      Some(s) => s.saturating_sub(viewport),
      None => 0,
    };
    let mut state = ListState::default();
    state.select(selection.selected.map(|s| s.saturating_sub(offset)));
    let lst_items: Vec<ListItem> = items
      .iter()
      .enumerate()
      .skip(offset)
      .take(viewport)
      .map(|(i, item)| {
        let is_load_more =
          i == items.len().saturating_sub(1) && item.as_ref().trim_start().starts_with("Load more");
        let mut inner_w = list_rect.width as usize;
        if title == "Playlists" {
          inner_w = (inner_w / 2).max(10);
        }
        let max_len = app.user_config.behavior.max_display_length as usize;
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
        let it = if is_load_more {
          ListItem::new(Span::styled(
            fit.clone(),
            Style::default()
              .fg(theme.load_more)
              .add_modifier(Modifier::BOLD | Modifier::ITALIC),
          ))
        } else if is_hovered {
          let w = UnicodeWidthStr::width(fit.as_ref());
          let pad = inner_w.saturating_sub(w);
          ListItem::new(Span::styled(
            format!("{}{}", fit, " ".repeat(pad)),
            Style::default().fg(theme.text).bg(theme.hovered),
          ))
        } else {
          ListItem::new(Span::raw(fit))
        };
        it
      })
      .collect();
    let focused = highlight_state.0 || highlight_state.1;
    let list = List::new(lst_items)
      .style(Style::default().fg(theme.text))
      .highlight_style(if focused {
        get_color(highlight_state, theme).add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(theme.text)
      });
    f.render_stateful_widget(list, list_rect, &mut state);
    draw_scrollbar(f, app, list_rect, items.len(), viewport, offset);
    return;
  }
  let title_area = Rect::new(rect.x, rect.y, rect.width, 1);
  f.render_widget(
    Paragraph::new(Span::styled(title.to_string(), title_style)),
    title_area,
  );
  if rect.height <= 1 {
    return;
  }
  let list_rect = Rect::new(
    rect.x,
    rect.y + 1,
    rect.width,
    rect.height.saturating_sub(1),
  );
  let viewport = list_rect.height as usize;
  let offset = match selection.selected {
    Some(s) => s.saturating_sub(viewport),
    None => 0,
  };
  let mut state = ListState::default();
  state.select(selection.selected.map(|s| s.saturating_sub(offset)));
  let lst_items: Vec<ListItem> = items
    .iter()
    .enumerate()
    .skip(offset)
    .take(viewport)
    .map(|(i, item)| {
      let is_load_more =
        i == items.len().saturating_sub(1) && item.as_ref().trim_start().starts_with("Load more");
      let mut inner_w = list_rect.width as usize;
      if title == "Playlists" {
        inner_w = (inner_w / 2).max(10);
      }
      let max_len = app.user_config.behavior.max_display_length as usize;
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
      let it = if is_load_more {
        ListItem::new(Span::styled(
          fit.clone(),
          Style::default()
            .fg(theme.load_more)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        ))
      } else if is_hovered {
        let w = UnicodeWidthStr::width(fit.as_ref());
        let pad = inner_w.saturating_sub(w);
        ListItem::new(Span::styled(
          format!("{}{}", fit, " ".repeat(pad)),
          Style::default().fg(theme.text).bg(theme.hovered),
        ))
      } else {
        ListItem::new(Span::raw(fit))
      };
      it
    })
    .collect();
  let focused = highlight_state.0 || highlight_state.1;
  let list = List::new(lst_items)
    .style(Style::default().fg(theme.text))
    .highlight_style(if focused {
      get_color(highlight_state, theme).add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.text)
    });
  f.render_stateful_widget(list, list_rect, &mut state);
  draw_scrollbar(f, app, list_rect, items.len(), viewport, offset);
}

/// (artist, album) groups with their track rows and newest-track timestamp.
pub(super) type TimestampGroups = Vec<(String, Vec<(String, u32)>, Instant)>;

pub fn draw_user_block(f: &mut Frame, app: &App, layout_chunk: Rect) {
  match (app.show_library, app.show_playlists) {
    (true, true) => {
      // One outer block with a horizontal separator between library and playlists.
      let cur = app.get_current_route();
      let lib_active = cur.active_block == ActiveBlock::Library
        || app.sidebar_latched_block == Some(ActiveBlock::Library);
      let pl_active = cur.active_block == ActiveBlock::MyPlaylists
        || app.sidebar_latched_block == Some(ActiveBlock::MyPlaylists);
      let lib_hovered = cur.hovered_block == ActiveBlock::Library;
      let pl_hovered = cur.hovered_block == ActiveBlock::MyPlaylists;
      let outer_style = get_color(
        (lib_active || pl_active, lib_hovered || pl_hovered),
        app.user_config.theme,
      );
      let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if app.user_config.terminal.unicode_borders {
          BorderType::Rounded
        } else {
          BorderType::Plain
        })
        .border_style(outer_style);
      f.render_widget(outer_block, layout_chunk);
      let (lib_sec, sep, pl_sec) = layout::sidebar_combined_split(app, layout_chunk);
      // Horizontal line separator
      if sep.height > 0 && sep.width > 0 {
        let line = "─".repeat(sep.width as usize);
        f.buffer_mut().set_string(sep.x, sep.y, &line, outer_style);
      }
      // Library section
      let visible = visible_library_options(&app.hidden_library_sections);
      let (lib_title, lib_items): (String, Vec<String>) = if app.sidebar_minimized {
        let glyphs = visible
          .iter()
          .map(|n| n.chars().next().map(|c| c.to_string()).unwrap_or_default())
          .collect();
        (crate::tui::layout::SIDEBAR_TOGGLE_GLYPH.to_string(), glyphs)
      } else {
        (
          crate::app::library_block_title(app),
          visible.iter().map(|s| s.to_string()).collect(),
        )
      };
      let lib_hl = (lib_active, lib_hovered);
      let lib_selected = if app.selection_engaged
        || app.sidebar_latched_block == Some(ActiveBlock::Library)
        || app.get_current_route().active_block == ActiveBlock::Library
      {
        Some(
          lib_items
            .len()
            .saturating_sub(1)
            .min(app.library.selected_index),
        )
      } else {
        None
      };
      draw_sidebar_section(
        f,
        app,
        &lib_title,
        &lib_items,
        lib_hl,
        ListSelection {
          selected: lib_selected,
          hovered: app.hovered_library_index,
        },
        lib_sec,
      );
      // Playlist section
      let pl_title = if app.sidebar_minimized {
        "P".to_string()
      } else {
        crate::app::playlists_block_title()
      };
      let pl_hl = (pl_active, pl_hovered);
      // Reuse cached playlist items
      PLAYLIST_ITEMS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let key = app
          .playlists
          .as_ref()
          .map(|p| (p as *const _ as usize, p.items.len(), app.sidebar_minimized))
          .unwrap_or((0, 0, app.sidebar_minimized));
        if slot.0 != key.0 || slot.1 != key.1 || slot.2 != key.2 {
          slot.3 = match &app.playlists {
            Some(p) => p
              .items
              .iter()
              .enumerate()
              .map(|(i, it)| {
                if app.sidebar_minimized {
                  format!("{}", i + 1)
                } else {
                  it.name.clone()
                }
              })
              .collect(),
            None => vec![],
          };
          slot.0 = key.0;
          slot.1 = key.1;
          slot.2 = key.2;
        }
        let items: &Vec<String> = &slot.3;
        draw_sidebar_section(
          f,
          app,
          &pl_title,
          items,
          pl_hl,
          ListSelection {
            selected: app.selected_playlist_index,
            hovered: app.hovered_playlist_index,
          },
          pl_sec,
        );
      });
    }
    (true, false) => draw_library_block(f, app, layout_chunk),
    (false, true) => draw_playlist_block(f, app, layout_chunk),
    (false, false) => {}
  }
}
