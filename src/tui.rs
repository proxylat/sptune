pub mod help;
pub mod layout;
use std::time::Instant;
use super::app::{
  visible_library_options, ActiveBlock, AlbumTableContext, App, ArtistBlock, DialogContext,
  EpisodeTableContext, RecommendationsContext, RouteId, SearchResultBlock, TrackSortColumn,
  TrackTableContext,
};
use crate::user_config::theme_presets;
use help::get_help_docs;
use layout::{
  build_playbar_controls, build_playbar_title, create_artist_string, format_playlist_duration,
  get_artist_highlight_state, get_color, get_percentage_width, get_search_results_highlight_state,
  millis_to_minutes, repeat_label, song_table_columns, track_table_with_date, PlaybarButton,
  PLAYBAR_HEIGHT, PLAYBAR_TIME_LEN, REFRESH_GLYPH, VOLUME_BAR_LEN,
};
use ratatui::{
  layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span, Text},
  widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, Wrap,
  },
  Frame,
};
use rspotify::model::show::ResumePoint;
use rspotify::model::PlayableItem;
use rspotify::model::RepeatState;
use rspotify::prelude::Id;
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
pub enum ColumnId {
  None,
  Title,
  Liked,
  Album,
  Artist,
  Length,
  DateAdded,
}

impl Default for ColumnId {
  fn default() -> Self {
    ColumnId::None
  }
}

pub struct TableHeader<'a> {
  id: TableId,
  items: Vec<TableHeaderItem<'a>>,
}

impl TableHeader<'_> {
  pub fn get_index(&self, id: ColumnId) -> Option<usize> {
    self.items.iter().position(|item| item.id == id)
  }
}

#[derive(Default)]
pub struct TableHeaderItem<'a> {
  id: ColumnId,
  text: &'a str,
  width: u16,
}

pub struct TableItem {
  id: String,
  format: Vec<String>,
}

fn load_more_label(base: &str, remaining: Option<usize>) -> String {
  match remaining.filter(|n| *n > 0) {
    Some(n) => format!("{base} ({n} more)"),
    None => base.to_string(),
  }
}

/// Truncate a long search input for the header box, keeping the END of the
/// text (a pasted playlist link's id) visible.
fn ellipsize(input: &str, max_width: usize) -> String {
  use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
  if UnicodeWidthStr::width(input) <= max_width {
    return input.to_string();
  }
  if max_width <= 3 {
    return "..."[..max_width.min(3)].to_string();
  }
  let budget = max_width - 3;
  let mut w = 0usize;
  let mut out = String::new();
  for c in input.chars() {
    let cw = UnicodeWidthChar::width(c).unwrap_or(0);
    if w + cw > budget {
      break;
    }
    w += cw;
    out.push(c);
  }
  out.push_str("...");
  out
}

pub fn draw_help_menu(f: &mut Frame, app: &App) {
  let settings_rect = layout::settings_section_rect(f.area());
  let shortcuts_rect = layout::shortcuts_table_rect(f.area());

  let help_menu_style = Style::default().fg(app.user_config.theme.text);

  let settings_rows = settings_rows_text(app, &help_menu_style);
  let settings_list = List::new(settings_rows)
    .block(
      Block::default()
        .borders(Borders::ALL)
        .style(help_menu_style)
        .title(Span::styled(
          "Settings (click a row to toggle, <Esc> to go back)",
          help_menu_style,
        ))
        .border_style(help_menu_style),
    )
    .style(help_menu_style);
  f.render_widget(settings_list, settings_rect);

  // Create a one-column table to avoid flickering due to non-determinism when
  // resolving constraints on widths of table columns.
  let format_row =
    |r: Vec<String>| -> Vec<String> { vec![format!("{:50}{:40}{:20}", r[0], r[1], r[2])] };

  let header = ["Description", "Event", "Context"];
  let header = format_row(header.iter().map(|s| s.to_string()).collect());

  let help_docs = get_help_docs(&app.user_config.keys);
  let help_docs = help_docs
    .into_iter()
    .map(format_row)
    .collect::<Vec<Vec<String>>>();
  let help_docs = &help_docs[app.help_scroll_offset as usize..];

  let rows = help_docs
    .iter()
    .map(|item| Row::new(item.clone()).style(help_menu_style));

  let help_menu = Table::new(rows, [Constraint::Percentage(100)])
    .header(Row::new(header).style(help_menu_style))
    .block(
      Block::default()
        .borders(Borders::ALL)
        .style(help_menu_style)
        .title(Span::styled("Shortcuts", help_menu_style))
        .border_style(help_menu_style),
    )
    .style(help_menu_style)
    .widths(&[Constraint::Percentage(100)]);
  f.render_widget(help_menu, shortcuts_rect);

  // website-style scrollbar just inside the right border, only when
  // `count` overflows `viewport`; geometry shared with the mouse drag arm
  // (src/handlers/mouse.rs arm_scrollbar).
  let viewport = shortcuts_rect.height.saturating_sub(3) as usize;
  draw_scrollbar(
    f,
    app,
    shortcuts_rect,
    app.help_docs_size as usize,
    viewport,
    app.help_scroll_offset as usize,
  );
}

fn settings_rows_text<'a>(app: &App, style: &Style) -> Vec<ListItem<'a>> {
  let black_theme = app.user_config.theme.background == Color::Rgb(0, 0, 0);
  let on_off = |on: bool| if on { "on" } else { "off" };
  let rows = vec![
    format!("Black theme: {}", on_off(black_theme)),
    format!("Library block: {}", on_off(app.show_library)),
    format!("Playlists block: {}", on_off(app.show_playlists)),
    format!(
      "Volume ramp bar: {}",
      on_off(app.user_config.behavior.volume_ramp_bar)
    ),
    format!(
      "Mouse interactions: {} (m)",
      on_off(app.user_config.behavior.enable_mouse)
    ),
    format!(
      "Theme: {} (P)",
      app.theme_preset_index.map_or_else(
        || "Custom".to_string(),
        |i| theme_presets()[i].0.to_string()
      )
    ),
    format!(
      "Timestamp by typing: {}",
      on_off(app.user_config.behavior.seek_by_typing)
    ),
    format!(
      "Resume last song: {}",
      on_off(app.user_config.behavior.resume_track)
    ),
    format!(
      "Restore settings on start: {}",
      on_off(app.user_config.behavior.restore_settings)
    ),
    format!("Dev view: {} (request log)", on_off(app.dev_view)),
    format!(
      "Column Album: {}",
      on_off(app.user_config.behavior.show_album_column)
    ),
    format!(
      "Column Artist: {}",
      on_off(app.user_config.behavior.show_artist_column)
    ),
    format!(
      "Column Length: {}",
      on_off(app.user_config.behavior.show_length_column)
    ),
    format!(
      "Column Date Added: {}",
      on_off(app.user_config.behavior.show_date_added_column)
    ),
    format!(
      "Add to playlist: {} (a)",
      on_off(app.user_config.behavior.enable_add_to_playlist)
    ),
    format!(
      "Liked icon: {}",
      on_off(app.user_config.behavior.show_liked_icon)
    ),
    format!(
      "Remove from playlist: {}",
      on_off(app.user_config.behavior.enable_remove_from_playlist)
    ),
    format!(
      "Max name length: {}",
      if app.user_config.behavior.max_display_length == 0 {
        "off".to_string()
      } else {
        app.user_config.behavior.max_display_length.to_string()
      }
    ),
    format!(
      "Animations: {}",
      on_off(app.user_config.behavior.enable_animations)
    ),
    format!(
      "Spotify auto launch: {}",
      on_off(app.user_config.spotify.auto_launch)
    ),
    format!(
      "Spotify chromium flags: {}",
      on_off(app.user_config.spotify.use_chromium_flags)
    ),
    format!(
      "Spotify suspend children: {}",
      on_off(app.user_config.spotify.suspend_children)
    ),
    format!(
      "Spotify trim WorkingSet: {}",
      on_off(app.user_config.spotify.trim_working_set)
    ),
    format!(
      "Spotify memory limit: {}",
      if app.user_config.spotify.memory_limit_mb == 0 {
        "off".to_string()
      } else {
        format!("{} MB", app.user_config.spotify.memory_limit_mb)
      }
    ),
    // Danger action: styled red and always last in the block.
    match app.user_config.keys.clear_cache {
      Some(key) => format!("Clear cache ({})", key),
      None => "Clear cache".to_string(),
    },
  ];
  let total = rows.len();
  rows
    .into_iter()
    .enumerate()
    .map(|(i, row)| {
      let span = if i + 1 == total {
        Span::styled(row, Style::default().fg(app.user_config.theme.error_text))
      } else {
        Span::styled(row, *style)
      };
      ListItem::new(span)
    })
    .collect()
}

pub fn draw_input_and_help_box(f: &mut Frame, app: &App, layout_chunk: Rect) {
  // Header: app title on the left, Search box centered, gear zone far right
  let chunks = Layout::default()
    .direction(Direction::Horizontal)
    .constraints(
      [
        Constraint::Percentage(35),
        Constraint::Percentage(30),
        Constraint::Percentage(35),
      ]
      .as_ref(),
    )
    .split(layout_chunk);

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::Input,
    current_route.hovered_block == ActiveBlock::Input,
  );

  // App title banner on the left of the header: figlet "sptune" artwork with
  // a gradient flowing over the whole logo (Spotify green -> teal)
  let art = [
    " ___ _ __ | |_ _   _ _ __   ___",
    "/ __| '_ \\| __| | | | '_ \\ / _ \\",
    "\\__ \\ |_) | |_| |_| | | | |  __/",
    "|___/ .__/ \\__|\\__,_|_| |_|\\___|",
    "    |_|",
  ];
  let stops = [
    (0x0A, 0x3A, 0x4A), // dark teal
    (0x40, 0xC0, 0xE0), // ice blue
    (0xE0, 0xF8, 0xFF), // white
  ];
  let color_at = |t: f64| {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let i = t as usize;
    let f = t - i as f64;
    let (r0, g0, b0) = stops[i.min(stops.len() - 1)];
    let (r1, g1, b1) = stops[(i + 1).min(stops.len() - 1)];
    Color::Rgb(
      (r0 as f64 + (r1 as f64 - r0 as f64) * f) as u8,
      (g0 as f64 + (g1 as f64 - g0 as f64) * f) as u8,
      (b0 as f64 + (b1 as f64 - b0 as f64) * f) as u8,
    )
  };
  let total = art.iter().map(|l| l.chars().count() + 1).sum::<usize>();
  let mut idx = 0usize;
  let mut title_lines = Vec::new();
  for line in art {
    let mut spans = Vec::new();
    for c in line.chars() {
      let t = idx as f64 / total.max(1) as f64;
      idx += 1;
      spans.push(Span::styled(
        c.to_string(),
        Style::default().fg(color_at(t)),
      ));
    }
    idx += 1; // line break consumes gradient position
    title_lines.push(Line::from(spans));
  }
  let title = Text::from(title_lines);
  f.render_widget(
    Paragraph::new(title).style(Style::default().fg(app.user_config.theme.banner)),
    Rect::new(
      chunks[0].x + 1,
      chunks[0].y,
      chunks[0].width.saturating_sub(1),
      5,
    ),
  );

  let input_string: String = app.input.iter().collect();
  // Keep the tail of a long input (e.g. a pasted playlist link) visible;
  // reserve 3 cells for the clear button when there is input.
  let input_string = ellipsize(
    &input_string,
    (chunks[1].width as usize / 2).saturating_sub(if app.input.is_empty() { 1 } else { 3 }),
  );
  let lines = Text::from(input_string);
  let input = Paragraph::new(lines).block(
    Block::default()
      .borders(Borders::ALL)
      .title(Span::styled(
        "Search",
        get_color(highlight_state, app.user_config.theme),
      ))
      .border_style(get_color(highlight_state, app.user_config.theme)),
  );
  f.render_widget(
    input,
    Rect::new(
      chunks[1].x,
      chunks[1].y + (chunks[1].height.saturating_sub(3)) / 2,
      chunks[1].width,
      3,
    ),
  );
  if !app.input.is_empty() {
    // Clear button: a bare ✕ pinned near the right edge of the search box.
    let clear_style = Style::default().fg(app.user_config.theme.active);
    f.render_widget(
      Paragraph::new(Line::from(vec![Span::styled(
        "✕",
        clear_style.add_modifier(Modifier::BOLD),
      )]))
      .alignment(Alignment::Right),
      Rect::new(
        chunks[1].x + chunks[1].width.saturating_sub(4),
        chunks[1].y + (chunks[1].height.saturating_sub(1)) / 2,
        3,
        1,
      ),
    );
  }

  // Settings hint: a bare gear pinned near the right of the header row,
  // inset a few cells so it never clips at the terminal edge. The right 40%
  // is still the click zone that opens the settings menu.
  let gear_style = Style::default().fg(app.user_config.theme.active);
  f.render_widget(
    Paragraph::new(Line::from(vec![Span::styled(
      "⚙\u{FE0F}",
      gear_style.add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Right),
    Rect::new(
      chunks[2].x,
      chunks[2].y + (chunks[2].height.saturating_sub(1)) / 2,
      chunks[2].width.saturating_sub(3),
      1,
    ),
  );
}

pub fn draw_main_layout(f: &mut Frame, app: &App) {
  let margin = layout::get_main_layout_margin(app);
  let parent_layout = Layout::default()
    .direction(Direction::Vertical)
    .constraints(
      [
        Constraint::Length(layout::header_height(app)),
        Constraint::Min(1),
        Constraint::Length(PLAYBAR_HEIGHT),
      ]
      .as_ref(),
    )
    .margin(margin)
    .split(f.area());

  // Header: Search, centered title, Settings
  draw_input_and_help_box(f, app, parent_layout[0]);

  // Nested main block with potential routes
  draw_routes(f, app, parent_layout[1]);

  // Currently playing
  draw_playbar(f, app, parent_layout[2]);

  // Possibly draw confirm dialog
  draw_dialog(f, app);
}

pub fn draw_routes(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let (sidebar, content_rect) = layout::sidebar_content_split(app, layout_chunk);

  draw_user_block(f, app, sidebar);
  if !app.sidebar_minimized && app.user_config.behavior.enable_animations {
    let style = Style::default().fg(app.user_config.theme.inactive);
    if app.show_library && app.show_playlists {
      // Single outer block: vertical handle inside the outer border, horizontal
      // handle is the separator line itself.
      let handle = layout::sidebar_handle_rect(app, layout_chunk);
      for y in sidebar.y + 1..sidebar.y + sidebar.height.saturating_sub(1) {
        if let Some(cell) = f.buffer_mut().cell_mut((handle.x, y)) {
          cell.set_symbol("│");
          cell.set_style(style);
        }
      }
      let sep = layout::sidebar_combined_separator_rect(app, sidebar);
      for x in sep.x..sep.x + sep.width {
        if let Some(cell) = f.buffer_mut().cell_mut((x, sep.y)) {
          cell.set_symbol("─");
          cell.set_style(style);
        }
      }
    } else {
      let handle = layout::sidebar_handle_rect(app, layout_chunk);
      let (lib_rect, pl_rect) = layout::library_playlists_split(app, sidebar);
      for r in [lib_rect, pl_rect] {
        for y in r.y + 1..r.y + r.height.saturating_sub(1) {
          if let Some(cell) = f.buffer_mut().cell_mut((handle.x, y)) {
            cell.set_symbol("│");
            cell.set_style(style);
          }
        }
      }
      let lib_handle = layout::library_handle_rect(app, sidebar);
      for x in lib_handle.x + 1..lib_handle.x + lib_handle.width.saturating_sub(1) {
        if let Some(cell) = f.buffer_mut().cell_mut((x, lib_handle.y)) {
          cell.set_symbol("─");
          cell.set_style(style);
        }
      }
    }
  }

  let current_route = app.get_current_route();

  let content = if app.dev_view {
    Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(75), Constraint::Percentage(25)].as_ref())
      .split(content_rect)[0]
  } else {
    content_rect
  };

  match current_route.id {
    RouteId::Search => {
      draw_search_results(f, app, content);
    }
    RouteId::TrackTable => {
      draw_song_table(f, app, content);
    }
    RouteId::AlbumTracks => {
      draw_album_table(f, app, content);
    }
    RouteId::RecentlyPlayed => {
      draw_recently_played_table(f, app, content);
    }
    RouteId::Artist => {
      draw_artist_page(f, app, content);
    }
    RouteId::AlbumList => {
      draw_album_list(f, app, content);
    }
    RouteId::PodcastEpisodes => {
      draw_show_episodes(f, app, content);
    }
    RouteId::MadeForYou => {
      draw_made_for_you(f, app, content);
    }
    RouteId::Artists => {
      draw_artist_table(f, app, content);
    }
    RouteId::Podcasts => {
      draw_podcast_table(f, app, content);
    }
    RouteId::Recommendations => {
      draw_recommendations_table(f, app, content);
    }
    RouteId::Error => {} // This is handled as a "full screen" route in main.rs
    RouteId::SelectedDevice => {} // This is handled as a "full screen" route in main.rs
    RouteId::MusicView => {} // This is handled as a "full screen" route in main.rs
    RouteId::Dialog => {} // This is handled in the draw_dialog function in mod.rs
  };

  if app.dev_view {
    draw_request_log(
      f,
      app,
      Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)].as_ref())
        .split(content_rect)[1],
    );
  }
}

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
      .map(|name| name.chars().next().map(|c| c.to_string()).unwrap_or_default())
      .collect();
    ("L".to_string(), glyphs)
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
    Some(
      app
        .library
        .selected_index
        .min(items.len().saturating_sub(1)),
    ),
    app.hovered_library_index,
  );
}

// Rebuilding the playlist name list on every frame is O(n) over the whole
// library (thousands of String allocations per frame). The set only changes
// when a new page is assigned (new pointer), grown (new length), or the
// sidebar collapsed state flips, so key on all three and rebuild only then.
thread_local! {
  static PLAYLIST_ITEMS: std::cell::RefCell<(usize, usize, bool, Vec<String>)> =
    std::cell::RefCell::new((0, 0, false, Vec::new())); // (page_ptr, len, minimized, items)
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
      app.selected_playlist_index,
      app.hovered_playlist_index,
    );
  });
}

fn draw_sidebar_section<S>(
  f: &mut Frame,
  app: &App,
  title: &str,
  items: &[S],
  highlight_state: (bool, bool),
  selected_index: Option<usize>,
  hovered_index: Option<usize>,
  rect: Rect,
) where
  S: AsRef<str>,
{
  if rect.height == 0 || rect.width == 0 {
    return;
  }
  let theme = app.user_config.theme;
  // Title row
  let title_style = get_color(highlight_state, theme).add_modifier(Modifier::BOLD);
  let title_area = Rect::new(rect.x, rect.y, rect.width, 1);
  f.render_widget(
    Paragraph::new(Span::styled(title.to_string(), title_style)),
    title_area,
  );
  if rect.height <= 1 {
    return;
  }
  let list_rect = Rect::new(rect.x, rect.y + 1, rect.width, rect.height.saturating_sub(1));
  let viewport = list_rect.height as usize;
  let offset = match selected_index {
    Some(s) => s.checked_sub(viewport).unwrap_or(0),
    None => 0,
  };
  let mut state = ListState::default();
  state.select(selected_index.map(|s| s.saturating_sub(offset)));
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
      let mut it = if is_load_more {
        ListItem::new(Span::styled(
          fit,
          Style::default()
            .fg(theme.load_more)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        ))
      } else {
        ListItem::new(Span::raw(fit))
      };
      if app.user_config.behavior.enable_animations
        && hovered_index == Some(i)
        && selected_index != Some(i)
      {
        it = it.style(Style::default().bg(theme.hovered));
      }
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
      let outer_active = lib_active || pl_active;
      let outer_hovered = lib_hovered || pl_hovered;
      let outer_style = get_color((outer_active, outer_hovered), app.user_config.theme);
      let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(outer_style);
      f.render_widget(outer_block, layout_chunk);
      let (lib_sec, sep, pl_sec) = layout::sidebar_combined_split(app, layout_chunk);
      // Horizontal line separator
      if sep.height > 0 && sep.width > 0 {
        let line = "─".repeat(sep.width as usize);
        f.buffer_mut()
          .set_string(sep.x, sep.y, &line, Style::default().fg(app.user_config.theme.inactive));
      }
      // Library section
      let visible = visible_library_options(&app.hidden_library_sections);
      let (lib_title, lib_items): (String, Vec<String>) = if app.sidebar_minimized {
        let glyphs = visible
          .iter()
          .map(|n| n.chars().next().map(|c| c.to_string()).unwrap_or_default())
          .collect();
        ("L".to_string(), glyphs)
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
        Some(lib_items.len().saturating_sub(1).min(app.library.selected_index))
      } else {
        None
      };
      draw_sidebar_section(
        f,
        app,
        &lib_title,
        &lib_items,
        lib_hl,
        lib_selected,
        app.hovered_library_index,
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
          app.selected_playlist_index,
          app.hovered_playlist_index,
          pl_sec,
        );
      });
    }
    (true, false) => draw_library_block(f, app, layout_chunk),
    (false, true) => draw_playlist_block(f, app, layout_chunk),
    (false, false) => {}
  }
}

fn draw_request_log(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let theme = app.user_config.theme;
  let inactive = Style::default().fg(theme.inactive);

  // Throttle info header (top 4 rows).
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(4), Constraint::Min(0)].as_ref())
    .split(layout_chunk);

  let backoff_line = match app.api_backoff_until {
    Some(until) => {
      let remaining = until.saturating_duration_since(Instant::now());
      format!("Backoff: {}s", remaining.as_secs())
    }
    None => "Backoff: none".to_string(),
  };
  let load_more_line = match app.last_load_more {
    Some(t) => {
      let elapsed = t.elapsed().as_secs();
      if elapsed < 2 {
        format!("Load-more: {}s left", 2 - elapsed)
      } else {
        "Load-more: ready".to_string()
      }
    }
    None => "Load-more: idle".to_string(),
  };
  let throttle_text = format!(
    "Tokens: {:.1} / {}\n{}\n{}",
    app.api_tokens,
    crate::backend::API_BURST as u32,
    backoff_line,
    load_more_line
  );
  f.render_widget(
    Paragraph::new(throttle_text).style(inactive),
    chunks[0],
  );

  // Request log list.
  let items: Vec<String> = app
    .request_log
    .iter()
    .map(|e| {
      if e.count > 1 {
        format!("{}({})", e.text, e.count)
      } else {
        e.text.clone()
      }
    })
    .collect();
  draw_selectable_list(
    f,
    app,
    chunks[1],
    "Requests (Dev) - Clear",
    &items,
    (false, false),
    app
      .request_log_index
      .map(|index| index.min(items.len().saturating_sub(1))),
    app.hovered_list_index,
  );
}

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
      draw_selectable_list(f, app, list_rect, "", &empty, (false, false), None, app.hovered_list_index);
    }
    SearchResultBlock::SongSearch => {
      let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
    false,
    true,
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
      let show_in_playlist = true;
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
              if show_in_playlist
                && item
                  .id
                  .as_ref()
                  .map(|id| app.playlist_contains(&id.uri(), None))
                  .unwrap_or(false)
              {
                cells.push(app.user_config.padded_in_playlist_icon());
              }
              cells
            },
          })
          .collect(),
        None => vec![],
      };
      if has_more {
        let remaining = app
          .search_results
          .tracks
          .as_ref()
          .map(|t| (t.total as usize > t.items.len()).then(|| t.total as usize - t.items.len()))
          .flatten();
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
        app.search_results.selected_tracks_index.unwrap_or(0),
        get_search_results_highlight_state(app, expanded.clone()),
        None,
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
        selected,
        app.hovered_list_index,
      );
    }
  }
}

/// The item rows, title and selected index for one search block.
fn search_block_items(
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

struct AlbumUi {
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
    app.artists_list_index,
    highlight_state,
    None,
  )
}

pub fn draw_podcast_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let header = TableHeader {
    id: TableId::Podcast,
    items: vec![
      TableHeaderItem {
        text: "Name",
        width: get_percentage_width(layout_chunk.width, 2.0 / 5.0),
        ..Default::default()
      },
      TableHeaderItem {
        text: "Name",
        width: get_percentage_width(layout_chunk.width, 2.0 / 5.0),
        ..Default::default()
      },
    ],
  };

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::Podcasts,
    current_route.hovered_block == ActiveBlock::Podcasts,
  );

  if let Some(saved_shows) = app.library.saved_shows.get_results(None) {
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
      (&format!("{}{}", REFRESH_GLYPH, "Podcasts"), &header),
      &items,
      app.shows_list_index,
      highlight_state,
      None,
    )
  };
}

pub fn draw_album_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let show_in_playlist = true;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
    false,
    true,
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
                  if show_in_playlist
                    && item
                      .id
                      .as_ref()
                      .map(|id| app.playlist_contains(&id.uri(), None))
                      .unwrap_or(false)
                  {
                    cells.push(app.user_config.padded_in_playlist_icon());
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
    AlbumTableContext::Full => match app.selected_album_full.clone() {
      Some(selected_album) => Some(AlbumUi {
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
              if show_in_playlist
                && item
                  .id
                  .as_ref()
                  .map(|id| app.playlist_contains(&id.uri(), None))
                  .unwrap_or(false)
              {
                cells.push(app.user_config.padded_in_playlist_icon());
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
      None => None,
    },
  };

  if let Some(album_ui) = album_ui {
    draw_table(
      f,
      app,
      layout_chunk,
      (&album_ui.title, &header),
      &album_ui.items,
      album_ui.selected_index,
      highlight_state,
      None,
    );
  };
}

pub fn draw_recommendations_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
    false,
    false,
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

  let current_route = app.get_current_route();
  let highlight_state = (
    current_route.active_block == ActiveBlock::TrackTable,
    current_route.hovered_block == ActiveBlock::TrackTable,
  );

  let items = app
    .track_table
    .tracks
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
      TableItem {
        id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
        format: cells,
      }
    })
    .collect::<Vec<TableItem>>();
  // match RecommendedContext
  let recommendations_ui = match &app.recommendations_context {
    Some(RecommendationsContext::Song) => format!(
      "Recommendations based on Song \'{}\'",
      &app.recommendations_seed
    ),
    Some(RecommendationsContext::Artist) => format!(
      "Recommendations based on Artist \'{}\'",
      &app.recommendations_seed
    ),
    None => "Recommendations".to_string(),
  };
  draw_table(
    f,
    app,
    layout_chunk,
    (&recommendations_ui[..], &header),
    &items,
    app.track_table.selected_index,
    highlight_state,
    Some(app.track_table.scroll_offset),
  )
}

// Row cells for the song table, in the same order as song_table_columns so a
// hidden column also drops its cells (ratatui zips cells to widths by
// position).
fn song_row_cells(
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
fn track_index_cell(app: &App, id: &Option<String>, index: usize) -> String {
  let liked = app.user_config.behavior.show_liked_icon
    && id
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

// Relative "Xm/Xh/Xd ago" for recent adds (a track added hours ago should read
// "3h ago", not the bare date); absolute date once it's more than a week old.
fn relative_date(added_at: chrono::DateTime<chrono::Utc>) -> String {
  let now = chrono::Utc::now();
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

pub fn draw_song_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let with_date = track_table_with_date(app.track_table.context.as_ref());
  let b = &app.user_config.behavior;
  let show_remove = b.enable_remove_from_playlist
    && matches!(
      app.track_table.context,
      Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
    );
  let show_in_playlist = false;
      let columns = song_table_columns(
        layout_chunk.width.saturating_sub(2),
        with_date,
        b.show_album_column,
        b.show_artist_column,
        b.show_length_column,
        b.show_date_added_column,
        show_remove,
        show_in_playlist,
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
        let matches = match (column, sort_column) {
          (ColumnId::Title, TrackSortColumn::Title)
          | (ColumnId::Artist, TrackSortColumn::Artist)
          | (ColumnId::Album, TrackSortColumn::Album)
          | (ColumnId::Length, TrackSortColumn::Length)
          | (ColumnId::DateAdded, TrackSortColumn::DateAdded) => true,
          _ => false,
        };
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

  let mut items = app
    .track_table
    .tracks
    .iter()
    .enumerate()
    .map(|(index, item)| TableItem {
      id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
      format: {
        let date = if with_date {
          app
            .track_table_added_at
            .get(index)
            .and_then(|added_at| added_at.map(relative_date))
            .unwrap_or_default()
        } else {
          String::new()
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
        let track_id = item.id.clone().map(|id| id.to_string());
        cells[0] = track_index_cell(app, &track_id, index + 1);
        if show_remove {
          cells.push(" ✕".to_string());
        }
        cells
      },
    })
    .collect::<Vec<TableItem>>();

  // In-playlist search filters the visible rows in place; the search box is
  // shown in the title row (see below), so the table body stays unshifted.
  if app.playlist_search_active() {
    items.retain(|item| {
      app.track_table.tracks.iter().any(|t| {
        t.id.as_ref().map(|id| id.to_string()).unwrap_or_default() == item.id
          && app.playlist_filter_matches(t)
      })
    });
  }
  if app.track_table_has_more() || app.date_added_pending {
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
    if show_remove {
      load_more_format.push(String::new());
    }
    items.push(TableItem {
      id: String::new(),
      format: load_more_format,
    });
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
   draw_table(
    f,
    app,
    layout_chunk,
    (title.as_str(), &header),
    &items,
    app.track_table.selected_index,
    highlight_state,
    Some(app.track_table.scroll_offset),
  );
  draw_playlist_search_box(f, app, layout_chunk, &title);
}

/// A bordered search input on the table title row, positioned right after
/// the playlist name text. The box hugs its content so the title border
/// continues after it (`│ Search playlist │────`).
fn draw_playlist_search_box(f: &mut Frame, app: &App, layout_chunk: Rect, title: &str) {
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
  let cursor = Span::styled("│", Style::default().fg(theme.active));
  let clear_style = Style::default().fg(theme.active).add_modifier(Modifier::BOLD);
  let (spans, content_w): (Vec<Span>, u16) = if focused {
    if query.is_empty() {
      (vec![cursor], 1)
    } else {
      let clear = Span::styled(" ✕", clear_style);
      (vec![Span::raw(query), cursor, clear], query.chars().count() as u16 + 3)
    }
  } else if query.is_empty() {
    let placeholder = Span::styled("", Style::default().fg(theme.inactive));
    let w = placeholder.content.width() as u16;
    (vec![placeholder], w)
  } else {
    (vec![Span::raw(query)], query.chars().count() as u16)
  };
  let box_width = content_w.saturating_add(2);
  let title_chars = title.chars().count() as u16;
  let x = layout_chunk
    .x
    .saturating_add(1)
    .saturating_add(title_chars)
    .saturating_add(2);
  if x + box_width > layout_chunk.x + layout_chunk.width {
    return;
  }
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


pub fn draw_music_view(f: &mut Frame, app: &App) {
  let layout = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
    .split(f.area());

  draw_music_lyrics(f, app, layout[0]);
  draw_music_panel(f, app, layout[1]);
}

fn draw_music_lyrics(f: &mut Frame, app: &App, area: Rect) {
  let vertical = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
    .split(area);

  let lyrics = app.lyrics.as_deref().unwrap_or(&[]);
  let progress_ms = app.seek_ms.unwrap_or(app.song_progress_ms);
  let current = lyrics
    .iter()
    .rposition(|(ms, _)| *ms <= progress_ms)
    .unwrap_or(0);

  // Center a fixed-height window around the current line.
  let window = 11usize;
  let (start, end) = if lyrics.is_empty() {
    (0, 0)
  } else {
    let raw_start = current.saturating_sub(window / 2);
    let start = raw_start.min(lyrics.len().saturating_sub(window));
    (start, (start + window).min(lyrics.len()))
  };

  let mut lines: Vec<Line> = if lyrics.is_empty() {
    vec![Line::from(Span::styled(
      "No lyrics available for this track",
      Style::default().fg(app.user_config.theme.inactive),
    ))]
  } else {
    lyrics[start..end]
      .iter()
      .enumerate()
      .map(|(i, (_ms, words))| {
        if start + i == current {
          Line::from(vec![Span::styled(
            format!("▶ {}", words),
            Style::default()
              .fg(app.user_config.theme.selected)
              .add_modifier(Modifier::BOLD),
          )])
        } else {
          Line::from(vec![Span::styled(
            format!("  {}", words),
            Style::default()
              .fg(app.user_config.theme.active)
              .add_modifier(Modifier::BOLD),
          )])
        }
      })
      .collect()
  };
  if !lyrics.is_empty() {
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
      // Two-space indent to align with the lyric words; DIM renders the
      // attribution smaller/lighter than the bold lyric lines.
      "  Lyrics by: LRCLIB",
      Style::default()
        .fg(app.user_config.theme.inactive)
        .add_modifier(Modifier::DIM),
    )));
  }

  // The empty state stays at the top of the block, as before. Real lyrics
  // are centered as a unit, vertically and horizontally: blank rows pad the
  // top, and a uniform left pad moves the whole text block to the middle.
  // Each line keeps its own structure — no per-line Alignment::Center.
  if !lyrics.is_empty() {
    let inner_h = vertical[0].height.saturating_sub(2);
    let pad = inner_h.saturating_sub(lines.len() as u16) / 2;
    if pad > 0 {
      lines.splice(
        0..0,
        std::iter::repeat_with(|| Line::from("")).take(pad as usize),
      );
    }
    // The pad is computed from the full lyrics set, not the visible window,
    // so the text block never shifts as the current line moves.
    let inner_w = vertical[0].width.saturating_sub(2) as usize;
    let text_w = lyrics
      .iter()
      .map(|(_, words)| UnicodeWidthStr::width(words.as_str()) + 2)
      .max()
      .unwrap_or(0)
      .max(UnicodeWidthStr::width("Lyrics by: LRCLIB") + 2);
    let pad_l = inner_w.saturating_sub(text_w) / 2;
    if pad_l > 0 {
      let pad_str = " ".repeat(pad_l);
      for line in &mut lines {
        line.spans.insert(0, Span::raw(pad_str.clone()));
      }
    }
  }

  // The block spans the full panel width (its original space); the lyrics
  // text stays vertically centered and left-aligned inside it.
  let block = Block::default()
    .borders(Borders::ALL)
    .title("Lyrics")
    .style(Style::default().fg(app.user_config.theme.inactive));

  let text = Paragraph::new(lines);
  f.render_widget(text.block(block), vertical[0]);

  draw_music_visualizer(f, app, vertical[1]);
}

fn draw_music_visualizer(f: &mut Frame, app: &App, area: Rect) {
  let block = Block::default()
    .borders(Borders::ALL)
    .title("Visualizer")
    .style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(block, area);
}

fn format_count(n: u64) -> String {
  if n >= 1_000_000_000 {
    format!("{:.1}B", n as f64 / 1_000_000_000.0)
  } else if n >= 1_000_000 {
    format!("{:.1}M", n as f64 / 1_000_000.0)
  } else if n >= 1_000 {
    format!("{:.0}K", n as f64 / 1_000.0)
  } else {
    n.to_string()
  }
}

fn draw_music_panel(f: &mut Frame, app: &App, area: Rect) {
  let mut lines: Vec<Line> = Vec::new();
  if let Some(context) = &app.current_playback_context {
    match &context.item {
      Some(PlayableItem::Track(track)) => {
        let artists = track
          .artists
          .iter()
          .map(|a| a.name.as_str())
          .collect::<Vec<_>>()
          .join(", ");
        let album = track.album.name.as_str();
        let duration = track.duration.num_seconds() as u128;
        let progress_ms = app.seek_ms.unwrap_or(app.song_progress_ms);
        lines.push(Line::from(Span::styled(
          track.name.as_str(),
          Style::default().fg(app.user_config.theme.selected),
        )));
        lines.push(Line::from(Span::styled(
          ellipsize(&artists, area.width.saturating_sub(2) as usize),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(Span::styled(
          format!("Album: {}", ellipsize(album, area.width.saturating_sub(2) as usize)),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(Span::styled(
          format!("Duration: {}:{:02}", duration / 60, duration % 60),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(Span::styled(
          format!(
            "Progress: {}:{:02} / {}:{:02}",
            progress_ms / 60000,
            (progress_ms / 1000) % 60,
            duration / 60,
            duration % 60
          ),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(Span::styled(
          format!(
            "Volume: {}%",
            context
              .device
              .volume_percent
              .map(|v| v.to_string())
              .unwrap_or_else(|| "?".to_string())
          ),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(Span::styled(
          format!(
            "Device: {}",
            ellipsize(context.device.name.as_str(), area.width.saturating_sub(2) as usize)
          ),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(""));
        if let Some(n) = app.monthly_listeners {
          lines.push(Line::from(Span::styled(
            format!("Monthly listeners: {}", format_count(n)),
            Style::default().fg(app.user_config.theme.inactive),
          )));
        }
        if let Some(credits) = &app.track_credits {
          for credit in credits {
            lines.push(Line::from(Span::styled(
              ellipsize(credit, area.width.saturating_sub(2) as usize),
              Style::default().fg(app.user_config.theme.inactive),
            )));
          }
        }
        if let Some(q) = &app.queue_next {
          lines.push(Line::from(format!("Up next: {}", q)));
        }
        // Playlist membership
        if let Some(uri) = app.playing_track_uri() {
          let names = app.playlists_containing(&uri);
          if names.is_empty() {
            lines.push(Line::from(Span::styled(
              "Not in any playlist",
              Style::default().fg(app.user_config.theme.inactive),
            )));
          } else {
            let text = format!("In playlists: {}", names.join(", "));
            lines.push(Line::from(Span::styled(text, Style::default().fg(app.user_config.theme.inactive))));
          }
        }
      }
      Some(PlayableItem::Episode(episode)) => {
        lines.push(Line::from(Span::styled(
          episode.name.as_str(),
          Style::default().fg(app.user_config.theme.selected),
        )));
        lines.push(Line::from(Span::styled(
          episode.show.name.as_str(),
          Style::default().fg(app.user_config.theme.active),
        )));
        if let Some(q) = &app.queue_next {
          lines.push(Line::from(""));
          lines.push(Line::from(format!("Up next: {}", q)));
        }
      }
      _ => {}
    }
  }
  if lines.is_empty() {
    lines.push(Line::from("Nothing playing"));
  }
  let block = Block::default()
    .borders(Borders::ALL)
    .title("Track Details")
    .style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(Paragraph::new(lines).block(block), area);
}

pub fn draw_playbar(f: &mut Frame, app: &App, layout_chunk: Rect) {
  // If no track is playing, render paragraph showing which device is selected, if no selected
  // give hint to choose a device
  if let Some(current_playback_context) = &app.current_playback_context {
    if let Some(track_item) = &current_playback_context.item {
      let play_title = if current_playback_context.is_playing {
        "Playing"
      } else {
        "Paused"
      };

      let current_route = app.get_current_route();
      let highlight_state = (
        current_route.active_block == ActiveBlock::PlayBar,
        current_route.hovered_block == ActiveBlock::PlayBar,
      );
      let theme = app.user_config.theme;

      let title = build_playbar_title(play_title, &current_playback_context.device.name);
      let border_style = get_color(highlight_state, theme);
      let title_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

      f.render_widget(title_block, layout_chunk);

      // Song metadata used for the name row and the progress bar.
      let (_item_id, name, duration_ms) = match track_item {
        PlayableItem::Track(track) => (
          track
            .id
            .as_ref()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "".to_string()),
          track.name.to_owned(),
          track.duration.num_milliseconds() as u32,
        ),
        PlayableItem::Episode(episode) => (
          episode.id.to_string(),
          episode.name.to_owned(),
          episode.duration.num_milliseconds() as u32,
        ),
        _ => ("".to_string(), "".to_string(), 0),
      };

      let track_name = name;

      // Transport buttons (shuffle, prev, play/pause, next, repeat)
      // dead-centered on the first inner row, just above the music bar. The
      // song name shares this row, left of the buttons (truncated so it
      // never overlaps).
      let controls = build_playbar_controls(current_playback_context.is_playing, app.smart_shuffle);
      let repeat_text = repeat_label(current_playback_context.repeat_state);
      let controls_row = layout::playbar_controls_row(layout_chunk);
      let controls_start = layout::playbar_controls_x(layout_chunk, &controls);
      let name_limit = (controls_start.saturating_sub(controls_row.x + 1)) as usize;
      let name_text: String = if track_name.chars().count() > name_limit {
        track_name
          .chars()
          .take(name_limit.saturating_sub(1))
          .chain(std::iter::once('…'))
          .collect()
      } else {
        track_name
      };
      let dim_style = Style::default().fg(theme.playbar_background);
      let accent_style = Style::default().fg(theme.playbar_progress);
      let repeat_style = match current_playback_context.repeat_state {
        RepeatState::Off => dim_style,
        RepeatState::Context => accent_style,
        RepeatState::Track => accent_style.add_modifier(Modifier::BOLD),
      };
      let mut spans: Vec<Span> = vec![Span::raw(
        " ".repeat((controls_start - controls_row.x) as usize),
      )];
      for (kind, text) in controls {
        let style = match kind {
          PlaybarButton::Shuffle => {
            if current_playback_context.shuffle_state {
              accent_style.add_modifier(Modifier::BOLD)
            } else {
              dim_style.add_modifier(Modifier::BOLD)
            }
          }
          PlaybarButton::Repeat => repeat_style,
          PlaybarButton::PlayPause => accent_style.add_modifier(Modifier::BOLD),
          PlaybarButton::Prev | PlaybarButton::Next => accent_style.add_modifier(Modifier::BOLD),
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
      }
      // Repeat-mode word right of the group (it doesn't affect centering,
      // so the buttons never jump when the mode changes).
      if let Some(label) = &repeat_text {
        spans.push(Span::styled(label.clone(), repeat_style));
        spans.push(Span::raw(" "));
      }
      f.render_widget(Paragraph::new(Line::from(spans)), controls_row);

      let volume_percent = app
        .volume_preview
        .or(
          current_playback_context
            .device
            .volume_percent
            .map(|v| v as u8),
        )
        .unwrap_or(0);
      if layout_chunk.width > 70 {
        let bar_len = VOLUME_BAR_LEN as usize;
        let filled = bar_len * volume_percent as usize / 100;
        let fill_style = Style::default().fg(theme.playbar_progress);
        let mut vol_spans: Vec<Span> = vec![Span::styled(
          "♪ ",
          Style::default().fg(theme.playbar_progress_text),
        )];
        if app.user_config.behavior.volume_ramp_bar {
          // Ramp mode: one rising mountain of the eight half-height blocks
          // across the whole bar (never repeats): ▁▁▂▂▃▃▄▄▅▅▆▆▇▇███
          let ramp = "▁▂▃▄▅▆▇█";
          let fill_text: String = (0..filled.min(bar_len))
            .map(|i| ramp.chars().nth(i * 7 / bar_len.saturating_sub(1)).unwrap())
            .collect();
          vol_spans.push(Span::styled(fill_text, fill_style));
          vol_spans.push(Span::styled(
            "░".repeat(bar_len.saturating_sub(filled.min(bar_len))),
            Style::default().fg(theme.playbar_progress_text),
          ));
        } else {
          // Solid fill like the progress bar: full █ cells with an eighth
          // partial cell at the fill edge, then ░ empty cells.
          let eighths = bar_len * 8 * volume_percent as usize / 100;
          let full = (eighths / 8).min(bar_len);
          let part = eighths % 8;
          let partial_glyph = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"][part];
          let has_partial = part > 0;
          let rest = bar_len.saturating_sub(full + has_partial as usize);
          vol_spans.push(Span::styled("█".repeat(full), fill_style));
          vol_spans.push(Span::styled(partial_glyph, fill_style));
          vol_spans.push(Span::styled(
            "░".repeat(rest),
            Style::default().fg(theme.playbar_progress_text),
          ));
        }
        vol_spans.push(Span::styled(
          format!(" {:>2}%", volume_percent),
          Style::default().fg(theme.playbar_progress_text),
        ));
        let volume_rect = layout::playbar_volume_rect(layout_chunk);
        let volume_content = Paragraph::new(Line::from(vol_spans));
        f.render_widget(volume_content, volume_rect);
      }

      let artists = match track_item {
        PlayableItem::Track(track) => create_artist_string(&track.artists),
        PlayableItem::Episode(episode) => format!("{} - {}", episode.name, episode.show.name),
        _ => String::new(),
      };

      let song_row = layout::playbar_song_row(layout_chunk);
      let artist_row = layout::playbar_artist_row(layout_chunk);
      let bar_rect = layout::playbar_progress_rect(layout_chunk);

      // Song name with playlist membership prefix: ✓ if in any playlist else + (clickable to add)
      let name_style = Style::default()
        .fg(theme.selected)
        .add_modifier(Modifier::BOLD);
      let uri = app.playing_track_uri();
      let in_playlist = uri.as_deref().map(|u| app.is_in_any_playlist(u)).unwrap_or(false);
      let prefix = if in_playlist {
        app.user_config.behavior.in_playlist_icon.clone() + " "
      } else {
        "+ ".to_string()
      };
      // prefix is 2 cols, keep truncation inside song_row
      let prefix_style = if in_playlist {
        Style::default().fg(theme.active).add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(theme.active).add_modifier(Modifier::BOLD)
      };
      f.render_widget(
        Paragraph::new(Line::from(vec![
          Span::styled(prefix, prefix_style),
          Span::styled(name_text, name_style),
        ])),
        song_row,
      );

      // Artist name (smaller: italic) and the centered progress bar share
      // the music-bar row; the artist ends before the bar's left label.
      let artist_style = Style::default()
        .fg(theme.inactive)
        .add_modifier(Modifier::ITALIC);
      let mut line_spans: Vec<Span> = Vec::new();
      if let Some(bar) = bar_rect {
        let bar_start: u16 = bar.x.saturating_sub(PLAYBAR_TIME_LEN);
        let artist_limit = bar_start.saturating_sub(artist_row.x + 1) as usize;
        let artist_text: String = if artists.chars().count() > artist_limit {
          artists
            .chars()
            .take(artist_limit.saturating_sub(1))
            .chain(std::iter::once('…'))
            .collect()
        } else {
          artists
        };
        line_spans.push(Span::styled(artist_text.clone(), artist_style));
        line_spans.push(Span::raw(" ".repeat(
          bar_start.saturating_sub(artist_row.x + artist_text.chars().count() as u16) as usize,
        )));

        let progress_ms = match app.seek_ms {
          Some(seek_ms) => seek_ms,
          None => app.song_progress_ms,
        };

        let fill_style = Style::default().fg(theme.playbar_progress);
        let bar_len = bar.width as usize;
        // Sub-cell eighth blocks (smaller █): 61 cells / 180s ≈ 2.7 eighths
        // per second, so every second shows its own step without extending
        // the bar; floor keeps the fill edges on clean cell boundaries.
        let eighths = ((progress_ms.min(u128::from(duration_ms)) * bar_len as u128 * 8)
          / u128::from(duration_ms)) as usize;
        let full = (eighths / 8).min(bar_len);
        let part = eighths % 8;
        let partial_glyph = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"][part];
        let has_partial = part > 0;
        let rest = bar_len.saturating_sub(full + has_partial as usize);
        let bar_spans = vec![
          Span::styled(
            format!("{:>5} ", millis_to_minutes(progress_ms)),
            Style::default().fg(theme.playbar_progress_text),
          ),
          Span::styled(
            "█".repeat(full.min(bar_len.saturating_sub(has_partial as usize))),
            fill_style,
          ),
          Span::styled(partial_glyph, fill_style),
          Span::styled(
            "░".repeat(rest),
            Style::default().fg(theme.playbar_progress_text),
          ),
          Span::styled(
            format!(" {:>4} ", millis_to_minutes(duration_ms as u128)),
            Style::default().fg(theme.playbar_progress_text),
          ),
        ];
        line_spans.extend(bar_spans);
      } else {
        line_spans.push(Span::styled(artists, artist_style));
      }
      f.render_widget(Paragraph::new(Line::from(line_spans)), artist_row);
    }
  }
}

pub fn draw_error_screen(f: &mut Frame, app: &App) {
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(100)].as_ref())
    .margin(5)
    .split(f.area());

  let playing_text = vec![
    Line::from(vec![
      Span::raw("Api response: "),
      Span::styled(
        &app.api_error,
        Style::default().fg(app.user_config.theme.error_text),
      ),
    ]),
    Line::from(Span::styled(
      "If you are trying to play a track, please check that",
      Style::default().fg(app.user_config.theme.text),
    )),
    Line::from(Span::styled(
      " 1. You have a Spotify Premium Account",
      Style::default().fg(app.user_config.theme.text),
    )),
    Line::from(Span::styled(
      " 2. Your playback device is active and selected - press `d` to go to device selection menu",
      Style::default().fg(app.user_config.theme.text),
    )),
    Line::from(Span::styled(
      " 3. If you're using spotifyd as a playback device, your device name must not contain spaces",
      Style::default().fg(app.user_config.theme.text),
    )),
    Line::from(Span::styled("Hint: a playback device must be either an official spotify client or a light weight alternative such as spotifyd",
        Style::default().fg(app.user_config.theme.hint)
        ),
    ),
    Line::from(
      Span::styled(
        format!(
          "Press {} to copy this error to clipboard",
          app.user_config.keys.copy_error
        ),
        Style::default().fg(app.user_config.theme.inactive),
      ),
    ),
    Line::from(
      Span::styled(
          "\nPress <Esc> to return",
          Style::default().fg(app.user_config.theme.inactive),
      ),
    )
  ];

  let playing_paragraph = Paragraph::new(playing_text)
    .wrap(Wrap { trim: true })
    .style(Style::default().fg(app.user_config.theme.text))
    .block(
      Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
          "Error",
          Style::default().fg(app.user_config.theme.error_border),
        ))
        .border_style(Style::default().fg(app.user_config.theme.error_border)),
    );
  f.render_widget(playing_paragraph, chunks[0]);
}

fn draw_artist_page(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let Some(artist) = &app.artist else {
    return;
  };
  let shown = if artist.artist_selected_block == ArtistBlock::Empty {
    artist.artist_hovered_block
  } else {
    artist.artist_selected_block
  };
  let theme = app.user_config.theme;
  let (tab_bar, tab_cells, list_rect) = layout::artist_layout(layout_chunk, shown);

  for (block, rect) in tab_cells {
    let label = match block {
      ArtistBlock::TopTracks => " Top tracks ",
      ArtistBlock::Albums => " Albums ",
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
      let show_in_playlist = true;
      let columns = song_table_columns(
        layout_chunk.width.saturating_sub(2),
        false,
        b.show_album_column,
        b.show_artist_column,
        b.show_length_column,
        b.show_date_added_column,
        false,
        true,
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
          if show_in_playlist
            && item
              .id
              .as_ref()
              .map(|id| app.playlist_contains(&id.uri(), None))
              .unwrap_or(false)
          {
            cells.push(app.user_config.padded_in_playlist_icon());
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
      let title = format!("{} - Top Tracks", artist.artist_name);
      draw_table(
        f,
        app,
        list_rect,
        (title.as_str(), &header),
        &items,
        artist.selected_top_track_index,
        get_artist_highlight_state(app, shown),
        None,
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
        ArtistBlock::Empty | ArtistBlock::TopTracks => {
          (vec![], String::new(), None)
        }
      };

      draw_selectable_list(
        f,
        app,
        list_rect,
        &title,
        &items,
        get_artist_highlight_state(app, shown),
        selected,
        app.hovered_list_index,
      );
    }
  }
}

pub fn draw_device_list(f: &mut Frame, app: &App) {
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(20), Constraint::Percentage(80)].as_ref())
    .margin(5)
    .split(f.area());

  let device_instructions: Vec<Line> = vec![
        "To play tracks, please select a device. ",
        "Use `j/k` or up/down arrow keys to move up and down and <Enter> to select. ",
        "Your choice here will be cached so you can jump straight back in when you next open `sptune`. ",
        "You can change the playback device at any time by pressing `d`.",
    ].into_iter().map(|instruction| Line::from(Span::raw(instruction))).collect();

  let instructions = Paragraph::new(device_instructions)
    .style(Style::default().fg(app.user_config.theme.text))
    .wrap(Wrap { trim: true })
    .block(
      Block::default().borders(Borders::NONE).title(Span::styled(
        "Welcome to sptune!",
        Style::default()
          .fg(app.user_config.theme.active)
          .add_modifier(Modifier::BOLD),
      )),
    );
  f.render_widget(instructions, chunks[0]);

  let no_device_message = Span::raw("No devices found: Make sure a device is active");

  let items = match &app.devices {
    Some(items) => {
      if items.is_empty() {
        vec![ListItem::new(no_device_message)]
      } else {
        items
          .iter()
          .map(|device| ListItem::new(Span::raw(&device.name)))
          .collect()
      }
    }
    None => vec![ListItem::new(no_device_message)],
  };

  let mut state = ListState::default();
  state.select(app.selected_device_index);
  let list = List::new(items)
    .block(
      Block::default()
        .title(Span::styled(
          "Devices",
          Style::default().fg(app.user_config.theme.active),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.user_config.theme.inactive)),
    )
    .style(Style::default().fg(app.user_config.theme.text))
    .highlight_style(
      Style::default()
        .fg(app.user_config.theme.active)
        .add_modifier(Modifier::BOLD),
    );
  f.render_stateful_widget(list, chunks[1], &mut state);
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
      selected_song_index,
      highlight_state,
      None,
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
        Some(selected_show) => {
          format!("{}", selected_show.show.name.to_owned())
        }
        None => "Episodes".to_owned(),
      },
      EpisodeTableContext::Full => match &app.selected_show_full {
        Some(selected_show) => {
          format!("{}", selected_show.show.name.to_owned())
        }
        None => "Episodes".to_owned(),
      },
    };

    draw_table(
      f,
      app,
      layout_chunk,
      (&title, &header),
      &items,
      app.episode_list_index,
      highlight_state,
      None,
    );
  };
}

pub fn draw_made_for_you(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let current_route = app.get_current_route();
  // The List content area is width-2 (borders eat 2 cols); name + " ✕" must
  // fit, so the name field gets width-4.
  let max_name = layout_chunk.width.saturating_sub(4) as usize;
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
    layout_chunk,
    "For you",
    &names,
    highlight_state,
    if app.selection_engaged {
      Some(app.made_for_you_index)
    } else {
      None
    },
    app.hovered_list_index,
  );
}

pub fn draw_recently_played_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width.saturating_sub(2),
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
    false,
    false,
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

    let mut items = recently_played
      .items
      .iter()
      .enumerate()
      .map(|(index, item)| {
        let track_id = item.track.id.clone().map(|id| id.to_string());
        let mut cells = song_row_cells(
          &item.track.name,
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
      selected_song_index,
      highlight_state,
      None,
    )
  };
}

// website-style scrollbar just inside the right border of `rect`, only when
// `count` overflows `viewport`. `offset` is the number of scrolled items
// (view offset, or selected_index - viewport for selection lists). Shared by
// the gear menu, sidebar lists and the track table; the mouse drag arm uses
// the same geometry (see handlers/mouse.rs arm_scrollbar).
fn draw_scrollbar(
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

fn draw_selectable_list<S>(
  f: &mut Frame,
  app: &App,
  layout_chunk: Rect,
  title: &str,
  items: &[S],
  highlight_state: (bool, bool),
  selected_index: Option<usize>,
  hovered_index: Option<usize>,
) where
  S: std::convert::AsRef<str>,
{
  // Keep the selection visible: render a window that ends at the selection.
  let viewport = layout_chunk.height.saturating_sub(2) as usize;
  let offset = match selected_index {
    Some(selected) => selected.checked_sub(viewport).unwrap_or(0),
    None => 0,
  };
  let mut state = ListState::default();
  state.select(selected_index.map(|selected| selected - offset));

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
      let fit = ellipsize(item.as_ref(), if max_len > 0 { inner_w.min(max_len) } else { inner_w });
      let mut item = if is_load_more {
        ListItem::new(Span::styled(
          fit,
          Style::default()
            .fg(app.user_config.theme.load_more)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        ))
      } else {
        ListItem::new(Span::raw(fit))
      };
      if app.user_config.behavior.enable_animations && hovered_index == Some(i) && selected_index != Some(i) {
        item = item.style(Style::default().bg(app.user_config.theme.hovered));
      }
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

fn draw_dialog(f: &mut Frame, app: &App) {
  if let ActiveBlock::Dialog(context) = app.get_current_route().active_block {
    match context {
      DialogContext::SeekTime => draw_seek_dialog(f, app),
      DialogContext::AddToPlaylist => draw_add_to_playlist_dialog(f, app),
      _ => {
        if let Some(playlist) = app.dialog.as_ref() {
          let bounds = f.area();
          // maybe do this better
          let width = std::cmp::min(bounds.width - 2, 45);
          let height = 8;
          let left = (bounds.width - width) / 2;
          let top = bounds.height / 4;

          let rect = Rect::new(left, top, width, height);

          f.render_widget(Clear, rect);

          let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.user_config.theme.inactive));

          f.render_widget(block, rect);

          let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([Constraint::Min(3), Constraint::Length(3)].as_ref())
            .split(rect);

          // suggestion: possibly put this as part of
          // app.dialog, but would have to introduce lifetime
          let text = vec![
            Line::from(Span::raw("Are you sure you want to delete the playlist: ")),
            Line::from(Span::styled(
              playlist.as_str(),
              Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::raw("?")),
          ];

          let text = Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);

          f.render_widget(text, vchunks[0]);

          let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .horizontal_margin(3)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
            .split(vchunks[1]);

          let ok_text = Span::raw("Ok");
          let ok = Paragraph::new(ok_text)
            .style(Style::default().fg(if app.confirm {
              app.user_config.theme.hovered
            } else {
              app.user_config.theme.inactive
            }))
            .alignment(Alignment::Center);

          f.render_widget(ok, hchunks[0]);

          let cancel_text = Span::raw("Cancel");
          let cancel = Paragraph::new(cancel_text)
            .style(Style::default().fg(if app.confirm {
              app.user_config.theme.inactive
            } else {
              app.user_config.theme.hovered
            }))
            .alignment(Alignment::Center);

          f.render_widget(cancel, hchunks[1]);
        }
      }
    }
  }
}

// Playlist picker: the user's own playlists, arrows to move, Enter to add
// the captured track, q/Esc to cancel.
fn draw_add_to_playlist_dialog(f: &mut Frame, app: &App) {
  let bounds = f.area();
  let width = std::cmp::min(bounds.width - 2, 45);
  let count = app.playlists.as_ref().map_or(0, |p| p.items.len());
  let height = std::cmp::min(count.saturating_add(6), bounds.height as usize - 2) as u16;
  let left = (bounds.width - width) / 2;
  let top = bounds.height / 4;
  let rect = Rect::new(left, top, width, height);

  f.render_widget(Clear, rect);

  let block = Block::default()
    .borders(Borders::ALL)
    .title(Span::styled(
      " Add to playlist ",
      Style::default().fg(app.user_config.theme.active),
    ))
    .border_style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(block, rect);

  // Keep the selection visible in the panel viewport.
  let viewport = height.saturating_sub(4) as usize;
  let offset = app.playlist_picker_index.saturating_sub(viewport / 2);

  let lines: Vec<Line> = app
    .playlists
    .as_ref()
    .map(|playlists| {
      playlists
        .items
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .map(|(i, playlist)| {
          let selected = i == app.playlist_picker_index;
          Line::from(Span::styled(
            playlist.name.as_str(),
            Style::default().fg(if selected {
              app.user_config.theme.selected
            } else {
              app.user_config.theme.text
            }),
          ))
        })
        .collect()
    })
    .unwrap_or_default();

  let text = Paragraph::new(lines)
    .wrap(Wrap { trim: true })
    .alignment(Alignment::Left);
  f.render_widget(
    text,
    rect.inner(Margin {
      vertical: 2,
      horizontal: 2,
    }),
  );

  let hint = Paragraph::new(Line::from(Span::styled(
    "↑/↓ pick   Enter add   q cancel",
    Style::default().fg(app.user_config.theme.inactive),
  )))
  .alignment(Alignment::Center);
  f.render_widget(
    hint,
    Rect::new(rect.x + 2, rect.y + rect.height - 2, rect.width - 4, 1),
  );
}

fn draw_seek_dialog(f: &mut Frame, app: &App) {
  let bounds = f.area();
  let width = std::cmp::min(bounds.width - 2, 45);
  let height = 8;
  let left = (bounds.width - width) / 2;
  let top = bounds.height / 4;

  let rect = Rect::new(left, top, width, height);

  f.render_widget(Clear, rect);

  let block = Block::default()
    .borders(Borders::ALL)
    .title(Span::styled(
      " Seek to (m:ss) ",
      Style::default().fg(app.user_config.theme.active),
    ))
    .border_style(Style::default().fg(app.user_config.theme.inactive));

  f.render_widget(block, rect);

  let digits = app.dialog.as_deref().unwrap_or_default();
  let displayed = format!("{:0>3}", digits);

  let text = Paragraph::new(vec![
    Line::from(Span::raw(" ")),
    Line::from(Span::styled(
      displayed.as_str(),
      Style::default()
        .fg(app.user_config.theme.text)
        .add_modifier(Modifier::BOLD),
    )),
    Line::from(Span::styled(
      "Type digits (last 2 = seconds).  ↑/→ +10s  ↓/← -10s",
      Style::default().fg(app.user_config.theme.inactive),
    )),
  ])
  .wrap(Wrap { trim: true })
  .alignment(Alignment::Center);

  f.render_widget(text, rect);
}

fn draw_table(
  f: &mut Frame,
  app: &App,
  layout_chunk: Rect,
  table_layout: (&str, &TableHeader), // (title, header colums)
  items: &[TableItem], // The nested vector must have the same length as the `header_columns`
  selected_index: usize,
  highlight_state: (bool, bool),
  view_offset: Option<usize>,
) {
  let selected_style =
    get_color(highlight_state, app.user_config.theme).add_modifier(Modifier::BOLD);

  let track_playing_index = app.current_playback_context.to_owned().and_then(|ctx| {
    ctx.item.and_then(|item| match item {
      PlayableItem::Track(track) => items.iter().position(|item| {
        track
          .id
          .clone()
          .map(|id| id.to_string() == item.id)
          .unwrap_or(false)
      }),
      PlayableItem::Episode(episode) => items
        .iter()
        .position(|item| episode.id.to_string() == item.id),
      _ => None,
    })
  });

  let (title, header) = table_layout;

  // Make sure that the selected item is visible on the page. Need to add some rows of padding
  // to chunk height for header and header space to get a true table height
  let padding = 3;
  let viewport = layout_chunk.height.saturating_sub(padding) as usize;
  // TrackTable passes its wheel-scrolled view offset; it is rendered verbatim
  // (capped at the list end) so the wheel can always scroll back up. Keeping
  // the selection visible is the job of the keyboard handlers, which nudge
  // scroll_offset when the selection crosses the viewport edge.
  let offset = match view_offset {
    Some(scrolled) => scrolled.min(items.len().saturating_sub(viewport)),
    // Keep the selection (and a load-more row pushed as the last item) visible
    // at the bottom row; the window is items[selected - viewport + 1 ..= selected].
    // table_row_index in mouse.rs maps clicks back through the same +1.
    None => selected_index
      .checked_sub(viewport)
      .map(|o| o + 1)
      .unwrap_or(0)
      .min(items.len().saturating_sub(viewport)),
  };

  let rows = items.iter().skip(offset).take(viewport).enumerate().map(|(i, item)| {
    let mut formatted_row = item.format.clone();
    let mut style = Style::default().fg(app.user_config.theme.text); // default styling

    // if table displays songs
    match header.id {
      TableId::Song | TableId::RecentlyPlayed | TableId::Album => {
        // First check if the song should be highlighted because it is currently playing
        if let Some(title_idx) = header.get_index(ColumnId::Title) {
          if let Some(track_playing_offset_index) =
            track_playing_index.and_then(|idx| idx.checked_sub(offset))
          {
            if i == track_playing_offset_index {
              formatted_row[title_idx] = format!("▶ {}", &formatted_row[title_idx]);
              style = Style::default()
                .fg(app.user_config.theme.active)
                .add_modifier(Modifier::BOLD);
            }
          }
        }
      }
      TableId::PodcastEpisodes => {
        if let Some(name_idx) = header.get_index(ColumnId::Title) {
          if let Some(track_playing_offset_index) =
            track_playing_index.and_then(|idx| idx.checked_sub(offset))
          {
            if i == track_playing_offset_index {
              formatted_row[name_idx] = format!("▶ {}", &formatted_row[name_idx]);
              style = Style::default()
                .fg(app.user_config.theme.active)
                .add_modifier(Modifier::BOLD);
            }
          }
        }
      }
      _ => {}
    }

    // The load-more row (pushed with an empty id as the last item) gets a
    // distinct accent so it reads as a button rather than a song.
    if offset + i == items.len().saturating_sub(1) && item.id.is_empty() {
      style = Style::default()
        .fg(app.user_config.theme.load_more)
        .add_modifier(Modifier::BOLD | Modifier::ITALIC);
    }

    // Hover bg for every panel (herdr-like full-row highlight) — before selection overlay
    if app.user_config.behavior.enable_animations
      && app.hovered_list_index == Some(offset + i)
      && !(app.selection_engaged && selected_index == offset + i)
    {
      style = style.bg(app.user_config.theme.hovered);
    }

    // Next check if the item is under selection.
    if app.selection_engaged && Some(i) == selected_index.checked_sub(offset) {
      style = selected_style;
    }

    // Return row styled data. Secondary columns are dimmed and every cell is
    // ellipsized to its column width (keeps head + "...") so long text never
    // gets cut mid-word by the table's hard edge.
    let dim = Style::default().fg(app.user_config.theme.inactive);
    let cells = formatted_row
      .iter()
      .enumerate()
      .map(|(idx, text)| {
        let col_width = header
          .items
          .get(idx)
          .map(|h| h.width.saturating_sub(1) as usize)
          .unwrap_or(usize::MAX);
        let col_id = header.items.get(idx).map(|h| h.id);
        let max_len = app.user_config.behavior.max_display_length as usize;
        let effective_width = if max_len > 0 && matches!(col_id, Some(ColumnId::Title | ColumnId::Artist | ColumnId::Album)) {
          col_width.min(max_len)
        } else {
          col_width
        };
        let padded = if matches!(col_id, Some(ColumnId::Title)) {
          format!("{}{}", " ".repeat(crate::tui::layout::NUMBER_TO_TITLE_GAP as usize), text)
        } else {
          text.clone()
        };
        let fit = if matches!(col_id, Some(ColumnId::Liked | ColumnId::None)) {
          padded
        } else {
          ellipsize(&padded, effective_width)
        };
        let is_secondary = matches!(
          col_id,
          Some(ColumnId::Artist | ColumnId::Album | ColumnId::DateAdded | ColumnId::Length)
        );
        if is_secondary {
          ratatui::widgets::Cell::from(fit).style(dim)
        } else {
          ratatui::widgets::Cell::from(fit)
        }
      })
      .collect::<Vec<_>>();
    Row::new(cells).style(style)
  });

  let widths = header
    .items
    .iter()
    .map(|h| Constraint::Length(h.width))
    .collect::<Vec<ratatui::layout::Constraint>>();

  let table = Table::new(rows, widths)
    .header(
      Row::new(header.items.iter().map(|h| {
        if matches!(h.id, ColumnId::Liked | ColumnId::None) {
          format!("{:>width$}", h.text, width = h.width as usize)
        } else if matches!(h.id, ColumnId::Title) {
          format!(
            "{}{}",
            " ".repeat(crate::tui::layout::NUMBER_TO_TITLE_GAP as usize),
            h.text
          )
        } else {
          h.text.to_string()
        }
      }))
      .style(
        Style::default()
          .fg(app.user_config.theme.header)
          .add_modifier(Modifier::BOLD),
      ),
    )
    .block(
      Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(app.user_config.theme.text))
        .title(Span::styled(
          title,
          get_color(highlight_state, app.user_config.theme),
        ))
        .border_style(get_color(highlight_state, app.user_config.theme)),
    )
    .style(Style::default().fg(app.user_config.theme.text));
  f.render_widget(table, layout_chunk);

  // website-style scrollbar just inside the right border, only when the list overflows
  draw_scrollbar(f, app, layout_chunk, items.len(), viewport, offset);
}

#[cfg(test)]
mod tests {
  use super::*;
  use ratatui::backend::TestBackend;
  use ratatui::Terminal;
  use serde_json::json;

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
          selected,
          (true, false),
          view_offset,
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
    assert_eq!(relative_date(now - chrono::Duration::days(60)), (now - chrono::Duration::days(60)).format("%Y-%m-%d").to_string());
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
    let mut app = App::default();
    app.artist = Some(crate::app::Artist {
      artist_id: "mockartist1".to_string(),
      artist_name: "Mock Artist".to_string(),
      albums: rspotify::model::Page {
        href: String::new(),
        items: vec![],
        limit: 0,
        next: None,
        offset: 0,
        previous: None,
        total: 0,
      },
      related_artists: vec![],
      top_tracks: (0..10).map(mock_track).collect(),
      top_tracks_total: 26,
      top_tracks_has_more: true,
      selected_album_index: 0,
      selected_related_artist_index: 0,
      selected_top_track_index: 0,
      artist_hovered_block: ArtistBlock::TopTracks,
      artist_selected_block: ArtistBlock::Empty,
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
    let mut app = App::default();
    app.artist = Some(crate::app::Artist {
      artist_id: "mockartist1".to_string(),
      artist_name: "Mock Artist".to_string(),
      albums: rspotify::model::Page {
        href: String::new(),
        items: vec![],
        limit: 0,
        next: None,
        offset: 0,
        previous: None,
        total: 0,
      },
      related_artists: vec![],
      top_tracks: (0..15).map(mock_track).collect(),
      top_tracks_total: 15,
      top_tracks_has_more: false,
      selected_album_index: 0,
      selected_related_artist_index: 0,
      selected_top_track_index: 12,
      artist_hovered_block: ArtistBlock::TopTracks,
      artist_selected_block: ArtistBlock::TopTracks,
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
      .draw(|f| draw_music_lyrics(f, &app, Rect::new(0, 0, 30, 20)))
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
    // inner height 12, 4 lyric lines → 4 blank pad rows below the border, so
    // the text is vertically centered. The block spans the full width while
    // each text line is centered horizontally inside it.
    assert!(
      !rows[1].contains("line"),
      "row 1 should be pad, got: {}",
      rows[1]
    );
    assert!(
      !rows[4].contains("line"),
      "row 4 should be pad, got: {}",
      rows[4]
    );
    assert!(
      rows[5].starts_with('│'),
      "block should hug x=0, got: {}",
      rows[5]
    );
    assert!(
      rows[5].contains("▶ line one") && !rows[5].starts_with("│▶"),
      "row 5 should hold the centered current line, got: {}",
      rows[5]
    );
    assert!(
      rows[6].contains("line two"),
      "row 6 should hold line two, got: {}",
      rows[6]
    );
    assert!(
      rows[8].contains("Lyrics by: LRCLIB"),
      "row 8 should hold the attribution, got: {}",
      rows[8]
    );
  }

  #[test]
  fn lyrics_empty_state_stays_at_the_top() {
    let mut app = App::default();
    app.lyrics = None;

    let backend = TestBackend::new(40, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
      .draw(|f| draw_music_lyrics(f, &app, Rect::new(0, 0, 40, 20)))
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
      row0.starts_with('┌'),
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
    assert_eq!(super::track_index_cell(&app, &Some("liked".to_string()), 1), "  ♥1");
    assert_eq!(super::track_index_cell(&app, &Some("liked".to_string()), 10), " ♥10");
    assert_eq!(super::track_index_cell(&app, &Some("liked".to_string()), 100), "♥100");
    assert_eq!(super::track_index_cell(&app, &Some("liked".to_string()), 1000), "♥1000");
    // An unliked id never shows the heart.
    assert_eq!(super::track_index_cell(&app, &Some("other".to_string()), 7), "   7");
    // show_liked_icon off suppresses the heart too.
    app.user_config.behavior.show_liked_icon = false;
    assert_eq!(super::track_index_cell(&app, &Some("liked".to_string()), 1), "   1");
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
    let spacing = 1;
    let header: String = (0..60)
      .map(|x| buffer.cell((x, 1)).map(|c| c.symbol()).unwrap_or(" "))
      .collect();
    let hash_byte = header.find('#').unwrap();
    let title_byte = header.find("Title").unwrap();
    let hash_idx = header[..hash_byte].chars().count();
    let title_idx = header[..title_byte].chars().count();
    assert_eq!(title_idx - hash_idx, 1 + spacing + gap, "header gap:\n{header}");
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
}
