pub mod help;
pub mod layout;
use super::{
  app::{
    visible_library_options, ActiveBlock, AlbumTableContext, App, ArtistBlock, DialogContext,
    EpisodeTableContext, MADE_FOR_YOU_NAMES, RecommendationsContext, RouteId, SearchResultBlock,
    TrackSortColumn, TrackTableContext,
  },
};
use help::get_help_docs;
use ratatui::{
  layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
  style::{Color, Modifier, Style},
  symbols::Marker,
  text::{Line, Span, Text},
  widgets::{
    canvas::{Canvas, Line as CanvasLine},
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, Wrap,
  },
  Frame,
};
use crate::user_config::{theme_presets, VisualizerStyle};
use rspotify::model::show::ResumePoint;
use rspotify::model::PlayableItem;
use rspotify::model::RepeatState;
use rspotify::prelude::Id;
use layout::{
  build_playbar_controls, build_playbar_title, create_artist_string, format_playlist_duration,
  get_artist_highlight_state, get_color, get_percentage_width, get_search_results_highlight_state,
  millis_to_minutes, repeat_label, song_table_columns,
  track_table_with_date, PlaybarButton, PLAYBAR_HEIGHT, PLAYBAR_TIME_LEN, REFRESH_GLYPH,
  VOLUME_BAR_LEN,
};

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
  draw_scrollbar(f, app, shortcuts_rect, app.help_docs_size as usize, viewport, app.help_scroll_offset as usize);
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
    // Danger action: styled red and always last in the block.
    "Clear cache (c)".to_string(),
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
    (0x1D, 0xB9, 0x54), // Spotify green
    (0x16, 0xA3, 0x8A), // teal
    (0x0E, 0x8C, 0xC2), // blue
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
      spans.push(Span::styled(c.to_string(), Style::default().fg(color_at(t))));
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
  let lines = Text::from((&input_string).as_str());
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

  // Settings hint: a bare gear pinned near the right of the header row,
  // inset a few cells so it never clips at the terminal edge. The right 40%
  // is still the click zone that opens the settings menu.
  let gear_style = Style::default().fg(app.user_config.theme.active);
  f.render_widget(
    Paragraph::new(Line::from(vec![
      Span::styled("⚙\u{FE0F}", gear_style.add_modifier(Modifier::BOLD)),
    ]))
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
  let chunks = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Percentage(20), Constraint::Percentage(80)].as_ref())
    .split(layout_chunk);

  draw_user_block(f, app, chunks[0]);

  let current_route = app.get_current_route();

  let content = if app.dev_view {
    Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(75), Constraint::Percentage(25)].as_ref())
      .split(chunks[1])[0]
  } else {
    chunks[1]
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
        .split(chunks[1])[1],
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
  let title = app
    .user
    .as_ref()
    .and_then(|u| u.display_name.as_deref())
    .map(|name| format!("{}({}) Library", REFRESH_GLYPH, name))
    .unwrap_or_else(|| format!("{}{}", REFRESH_GLYPH, "Library"));
  draw_selectable_list(
    f,
    app,
    layout_chunk,
    &title,
    &visible,
    highlight_state,
    Some(
      app
        .library
        .selected_index
        .min(visible.len().saturating_sub(1)),
    ),
  );
}

pub fn draw_playlist_block(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let playlist_items = match &app.playlists {
    Some(p) => p.items.iter().map(|item| item.name.to_owned()).collect(),
    None => vec![],
  };

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::MyPlaylists
      || app.sidebar_latched_block == Some(ActiveBlock::MyPlaylists),
    current_route.hovered_block == ActiveBlock::MyPlaylists,
  );

  draw_selectable_list(
    f,
    app,
    layout_chunk,
    &format!("{}{}", REFRESH_GLYPH, "Playlists"),
    &playlist_items,
    highlight_state,
    app.selected_playlist_index,
  );
}

pub fn draw_user_block(f: &mut Frame, app: &App, layout_chunk: Rect) {
  // The search header is global now (draw_main_layout), so the sidebar just
  // holds the library and playlists regardless of width
  match (app.show_library, app.show_playlists) {
    (true, true) => {
      let (library, playlists) = layout::library_playlists_split(app, layout_chunk);
      draw_library_block(f, app, library);
      draw_playlist_block(f, app, playlists);
    }
    (true, false) => draw_library_block(f, app, layout_chunk),
    (false, true) => draw_playlist_block(f, app, layout_chunk),
    (false, false) => {}
  }
}

fn draw_request_log(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let items: Vec<String> = app
    .request_log
    .iter()
    .map(|e| {
      if e.count > 1 {
        format!("{} ({})", e.text, e.count)
      } else {
        e.text.clone()
      }
    })
    .collect();
  draw_selectable_list(
    f,
    app,
    layout_chunk,
    "Requests (Dev) - Clear",
    &items,
    (false, false),
    app
      .request_log_index
      .map(|index| index.min(items.len().saturating_sub(1))),
  );
}

pub fn draw_search_results(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let theme = app.user_config.theme;
  let expanded = app.search_results.selected_block.clone();
  let has_more = app.search_block_has_more(&expanded);
  let (tab_bar, tab_cells, list_rect) = layout::search_layout(layout_chunk, expanded.clone(), has_more);

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
    let mut style = get_color(get_search_results_highlight_state(app, block.clone()), theme);
    if block == expanded {
      style = style.add_modifier(Modifier::BOLD);
    }
    f.buffer_mut().set_string(rect.x, tab_bar.y, label, style);
  }

  match &expanded {
    SearchResultBlock::Empty => {
      let empty: Vec<String> = vec![];
      draw_selectable_list(f, app, list_rect, "", &empty, (false, false), None);
    }
    SearchResultBlock::SongSearch => {
      let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width,
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
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
              _ => "",
            },
            width: *width,
          })
          .collect(),
      };
      let mut items = match &app.search_results.tracks {
        Some(tracks) => tracks
          .items
          .iter()
          .map(|item| TableItem {
            id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
            format: {
              let mut cells = song_row_cells(
                &item.name,
                &create_artist_string(&item.artists),
                &item.album.name,
                "",
                item.duration.num_milliseconds() as u128,
                false,
                b,
              );
              cells[0] = if item
                .id
                .as_ref()
                .map(|id| app.playlist_contains(&id.uri(), None))
                .unwrap_or(false)
              {
                app.user_config.padded_in_playlist_icon()
              } else {
                "  ".to_string()
              };
              cells
            },
          })
          .collect(),
        None => vec![],
      };
      if has_more {
        let mut load_more_format = vec!["".to_string(), " Load more ".to_string()];
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
        selected.checked_sub(viewport).unwrap_or(0),
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
      (items, "Playlists", app.search_results.selected_playlists_index)
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
    items.push(" Load more ".to_string());
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
  let columns = song_table_columns(
    layout_chunk.width,
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
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
              .map(|item| TableItem {
                id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
                format: {
                  let mut cells = song_row_cells(
                    &item.name,
                    &create_artist_string(&item.artists),
                    &selected_album_simplified.album.name,
                    "",
                    item.duration.num_milliseconds() as u128,
                    true,
                    b,
                  );
                  cells[0] = item
                    .id
                    .as_ref()
                    .map(|id| {
                      if app.user_config.behavior.show_liked_icon
                        && app.liked_song_ids_set.contains(&id.to_string())
                      {
                        format!("{} ", app.user_config.behavior.liked_icon)
                      } else if app.playlist_contains(&id.uri(), None) {
                        app.user_config.padded_in_playlist_icon()
                      } else {
                        String::new()
                      }
                    })
                    .unwrap_or_default();
                  cells
                },
              })
              .collect::<Vec<TableItem>>();

            if items.len() < selected_album_simplified.tracks.total as usize {
              items.push(TableItem {
                id: String::new(),
                format: {
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
          .map(|item| TableItem {
            id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
            format: {
              let mut cells = song_row_cells(
                &item.name,
                &create_artist_string(&item.artists),
                &selected_album.album.name,
                "",
                item.duration.num_milliseconds() as u128,
                false,
                b,
              );
              cells[0] = item
                .id
                .as_ref()
                .map(|id| {
                  if app.user_config.behavior.show_liked_icon
                    && app.liked_song_ids_set.contains(&id.to_string())
                  {
                    format!("{} ", app.user_config.behavior.liked_icon)
                  } else if app.playlist_contains(&id.uri(), None) {
                    app.user_config.padded_in_playlist_icon()
                  } else {
                    String::new()
                  }
                })
                .unwrap_or_default();
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
    layout_chunk.width,
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
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
    .map(|item| TableItem {
      id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
      format: song_row_cells(
        &item.name,
        &create_artist_string(&item.artists),
        &item.album.name,
        "",
        item.duration.num_milliseconds() as u128,
        false,
        b,
      ),
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

// The playlist currently being viewed, so its own rows are not all marked
// "already in playlist" (every row is in it by definition).
fn current_playlist_id(app: &App) -> Option<String> {
  match app.track_table.context {
    Some(TrackTableContext::MyPlaylists) => app.playlists.as_ref().and_then(|playlists| {
      playlists
        .items
        .get(app.active_playlist_index.or(app.selected_playlist_index).unwrap_or(0))
        .map(|playlist| playlist.id.to_string())
    }),
    Some(TrackTableContext::MadeForYou) => app
      .made_for_you_ids
      .get(app.made_for_you_index)
      .and_then(|id| id.clone()),
    _ => None,
  }
}

pub fn draw_song_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let with_date = track_table_with_date(app.track_table.context.as_ref());
  let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width,
    with_date,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
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

  let items = app
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
            .and_then(|added_at| added_at.map(|date| date.format("%Y-%m-%d").to_string()))
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
        let in_playlist = app.track_table.context != Some(TrackTableContext::MyPlaylists)
          && item
            .id
            .as_ref()
            .map(|id| app.playlist_contains(&id.uri(), current_playlist_id(app).as_deref()))
            .unwrap_or(false);
        let liked = app.user_config.behavior.show_liked_icon
          && item
            .id
            .as_ref()
            .map(|id| app.liked_song_ids_set.contains(&id.to_string()))
            .unwrap_or(false);
        cells[0] = if liked {
          format!("{} ", app.user_config.behavior.liked_icon)
        } else if in_playlist {
          app.user_config.padded_in_playlist_icon()
        } else {
          "  ".to_string()
        };
        cells
      },
    })
    .collect::<Vec<TableItem>>();

  let mut items = items;
  if app.track_table_has_more() || app.date_added_pending {
    let label = if app.date_added_pending {
      "Loading full playlist...".to_string()
    } else {
      "Load more songs...".to_string()
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
    _ => "Songs".to_string(),
  };
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
  )
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

  let lines: Vec<Line> = lyrics[start..end]
    .iter()
    .enumerate()
    .map(|(i, (_ms, words))| {
      if start + i == current {
        Line::from(vec![Span::styled(
          format!("▶ {}", words),
          Style::default().fg(app.user_config.theme.selected),
        )])
      } else {
        Line::from(vec![Span::styled(
          format!("  {}", words),
          Style::default().fg(app.user_config.theme.active),
        )])
      }
    })
    .collect();

  let text = if lyrics.is_empty() {
    Paragraph::new(Line::from(Span::styled(
      "No lyrics available for this track",
      Style::default().fg(app.user_config.theme.inactive),
    )))
  } else {
    Paragraph::new(lines)
  };
  let block = Block::default()
    .borders(Borders::ALL)
    .title("Lyrics")
    .style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(text.block(block), vertical[0]);

  draw_music_visualizer(f, app, vertical[1]);
}

/// Normalized 0..1 loudness per column for the MusicView visualizer.
/// Tier 1 is the real loudness envelope — used only when it belongs to the
/// currently playing track, so a stale envelope from a previous song never
/// wins. Tier 2 is a beat-synced wave from the audio features; tier 3 is a
/// simulated pattern so the panel is never empty.
fn visualizer_columns(app: &App, width: usize) -> Vec<f32> {
  if let Some((env_uri, env)) = &app.audio_envelope {
    let current = match &app.current_playback_context {
      Some(ctx) => match &ctx.item {
        Some(PlayableItem::Track(t)) => t.id.as_ref().map(|id| id.to_string()),
        _ => None,
      },
      None => None,
    };
    if current.as_deref() == Some(env_uri.as_str()) && !env.is_empty() {
      let fraction = match &app.current_playback_context {
        Some(ctx) => match &ctx.item {
          Some(PlayableItem::Track(t)) => t.duration.num_milliseconds() as f32,
          Some(PlayableItem::Episode(e)) => e.duration.num_milliseconds() as f32,
          _ => 0.0,
        },
        None => 0.0,
      };
      let elapsed = app.seek_ms.unwrap_or(app.song_progress_ms) as f32;
      // Clamp the window so it never runs past the envelope's end; with a
      // wider panel than envelope (width > len) the window pins at the start.
      let window = if fraction > 0.0 {
        let start = ((elapsed / fraction) * env.len() as f32) as usize;
        start
          .min(env.len().saturating_sub(width.min(env.len())))
          .min(env.len() - 1)
      } else {
        0
      };
      return (0..width)
        .map(|col| env.get(window + col).copied().unwrap_or(0.0))
        .collect();
    }
  }
  if let Some((_, features)) = &app.audio_features {
    // Beat-synced wave: tempo ~BPM -> radians/s, seeded by playback time,
    // amplitude scaled by the track's energy.
    let elapsed = app.seek_ms.unwrap_or(app.song_progress_ms) as f32 / 1000.0;
    let tempo = features.tempo.max(1.0);
    let phase = elapsed * (tempo / 60.0) * std::f32::consts::TAU;
    return (0..width)
      .map(|col| {
        let col_phase = phase + (col as f32 / width.max(1) as f32) * std::f32::consts::TAU * 2.0;
        (col_phase.sin() * 0.5 + 0.5) * features.energy
      })
      .collect();
  }
  // Simulated pattern, seeded by sub-second progress so it shifts every
  // redraw tick rather than ~4x/s (progress >> 8), which reads as laggy.
  let seed = app.seek_ms.unwrap_or(app.song_progress_ms) as u64 >> 2;
  (0..width)
    .map(|col| {
      let x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        ^ (col as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
      let v = ((x >> 33) & 0xffff) as usize;
      (v as f32 + 1.0) / 0x10000 as f32
    })
    .collect()
}

fn draw_music_visualizer(f: &mut Frame, app: &App, area: Rect) {
  let columns = visualizer_columns(app, area.width.saturating_sub(2) as usize);
  let block = Block::default()
    .borders(Borders::ALL)
    .title("Visualizer")
    .style(Style::default().fg(app.user_config.theme.inactive));
  match app.user_config.behavior.visualizer_style {
    VisualizerStyle::Bars => {
      draw_visualizer_bars(f, app, block, &columns, area);
    }
    VisualizerStyle::Oscilloscope => {
      draw_visualizer_scope(f, app, block, &columns, area);
    }
  }
}

fn draw_visualizer_bars(f: &mut Frame, app: &App, block: Block, columns: &[f32], area: Rect) {
  let bar_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
  let height = bar_chars.len();
  let mut rows: Vec<Line> = Vec::new();
  for row in (0..height).rev() {
    let mut spans: Vec<Span> = Vec::new();
    for v in columns {
      let h = ((v.clamp(0.0, 1.0) * height as f32).ceil() as usize).min(height);
      let ch = if row < h {
        bar_chars[height - 1 - row]
      } else {
        ' '
      };
      spans.push(Span::styled(
        ch.to_string(),
        Style::default().fg(app.user_config.theme.playbar_progress),
      ));
    }
    rows.push(Line::from(spans));
  }
  f.render_widget(Paragraph::new(rows).block(block), area);
}

fn draw_visualizer_scope(f: &mut Frame, app: &App, block: Block, columns: &[f32], area: Rect) {
  let inner_h = (area.height.saturating_sub(2) as usize).max(1) as f64;
  let width = columns.len().max(1) as f64;
  let color = app.user_config.theme.playbar_progress;
  let canvas = Canvas::default()
    .block(block)
    .marker(Marker::Braille)
    .x_bounds([0.0, width])
    .y_bounds([0.0, inner_h])
    .paint(move |ctx| {
      for col in 0..columns.len().saturating_sub(1) {
        let y1 = (1.0 - columns[col].clamp(0.0, 1.0)) as f64 * inner_h;
        let y2 = (1.0 - columns[col + 1].clamp(0.0, 1.0)) as f64 * inner_h;
        ctx.draw(&CanvasLine::new(col as f64, y1, col as f64 + 1.0, y2, color));
      }
    });
  f.render_widget(canvas, area);
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
        lines.push(Line::from("Now Playing"));
        lines.push(Line::from(Span::styled(
          track.name.as_str(),
          Style::default().fg(app.user_config.theme.selected),
        )));
        lines.push(Line::from(Span::styled(
          artists,
          Style::default().fg(app.user_config.theme.active),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(format!("Album: {}", album)));
        lines.push(Line::from(format!(
          "Duration: {}:{:02}",
          duration / 60,
          duration % 60
        )));
        lines.push(Line::from(format!(
          "Progress: {}:{:02} / {}:{:02}",
          progress_ms / 60000,
          (progress_ms / 1000) % 60,
          duration / 60,
          duration % 60
        )));
        lines.push(Line::from(format!(
          "Volume: {}%",
          context
            .device
            .volume_percent
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".to_string())
        )));
        lines.push(Line::from(format!(
          "Device: {}",
          context.device.name.as_str()
        )));
        lines.push(Line::from(""));
        if let Some(n) = app.monthly_listeners {
          lines.push(Line::from(format!(
            "Monthly listeners: {}",
            format_count(n)
          )));
        }
        if let Some(credits) = &app.track_credits {
          for credit in credits {
            lines.push(Line::from(credit.as_str()));
          }
        }
        if let Some(q) = &app.queue_next {
          lines.push(Line::from(format!("Up next: {}", q)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
          "Artist profile →",
          Style::default().fg(app.user_config.theme.selected),
        )));
      }
      Some(PlayableItem::Episode(episode)) => {
        lines.push(Line::from("Now Playing"));
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
      let controls = build_playbar_controls(current_playback_context.is_playing);
      let repeat_text = repeat_label(current_playback_context.repeat_state);
      let controls_row = layout::playbar_controls_row(layout_chunk);
      let controls_start =
        layout::playbar_controls_x(layout_chunk, &controls);
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

      // Song name on the row above the music bar, left of the centered
      // transport buttons (truncated so the two never overlap).
      let name_style = Style::default()
        .fg(theme.selected)
        .add_modifier(Modifier::BOLD);
      f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(name_text, name_style)])),
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
          Span::styled("█".repeat(full.min(bar_len.saturating_sub(has_partial as usize))), fill_style),
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
      ArtistBlock::Empty | ArtistBlock::RelatedArtists => "",
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
  let columns = song_table_columns(
    layout_chunk.width,
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
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
              _ => "",
            },
            width: *width,
          })
          .collect(),
      };
      let mut items = artist
        .top_tracks
        .iter()
        .map(|item| TableItem {
          id: item.id.clone().map(|id| id.to_string()).unwrap_or_default(),
          format: song_row_cells(
            &item.name,
            &create_artist_string(&item.artists),
            &item.album.name,
            "",
            item.duration.num_milliseconds() as u128,
            false,
            b,
          ),
        })
        .collect::<Vec<TableItem>>();
      if artist.top_tracks_has_more {
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
            albums.push("Load more albums...".to_string());
          }
          (
            albums,
            "Albums".to_string(),
            Some(artist.selected_album_index),
          )
        }
        ArtistBlock::Empty
        | ArtistBlock::RelatedArtists
        | ArtistBlock::TopTracks => (vec![], String::new(), None),
      };

      draw_selectable_list(
        f,
        app,
        list_rect,
        &title,
        &items,
        get_artist_highlight_state(app, shown),
        selected,
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
  let names: Vec<String> = MADE_FOR_YOU_NAMES
    .iter()
    .map(|name| name.to_string())
    .collect();

  let current_route = app.get_current_route();
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
  );
}

pub fn draw_recently_played_table(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let b = &app.user_config.behavior;
  let columns = song_table_columns(
    layout_chunk.width,
    false,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
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
      .map(|item| TableItem {
        id: item
          .track
          .id
          .clone()
          .map(|id| id.to_string())
          .unwrap_or_default(),
        format: song_row_cells(
          &item.track.name,
          &create_artist_string(&item.track.artists),
          &item.track.album.name,
          "",
          item.track.duration.num_milliseconds() as u128,
          false,
          b,
        ),
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
fn draw_scrollbar(f: &mut Frame, app: &App, rect: Rect, count: usize, viewport: usize, offset: usize) {
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

  let lst_items: Vec<ListItem> = items
    .iter()
    .skip(offset)
    .map(|i| ListItem::new(Span::raw(i.as_ref())))
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
  f.render_widget(text, rect.inner(Margin { vertical: 2, horizontal: 2 }));

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
  let padding = 5;
  let viewport = layout_chunk.height.saturating_sub(padding) as usize;
  // TrackTable passes its wheel-scrolled view offset; it is rendered verbatim
  // (capped at the list end) so the wheel can always scroll back up. Keeping
  // the selection visible is the job of the keyboard handlers, which nudge
  // scroll_offset when the selection crosses the viewport edge.
  let offset = match view_offset {
    Some(scrolled) => scrolled.min(items.len().saturating_sub(viewport)),
    None => selected_index.checked_sub(viewport).unwrap_or(0),
  };

  let rows = items.iter().skip(offset).enumerate().map(|(i, item)| {
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

    // Next check if the item is under selection.
    if app.selection_engaged && Some(i) == selected_index.checked_sub(offset) {
      style = selected_style;
    }

    // Return row styled data
    Row::new(formatted_row).style(style)
  });

  let widths = header
    .items
    .iter()
    .map(|h| Constraint::Length(h.width))
    .collect::<Vec<ratatui::layout::Constraint>>();

  let table = Table::new(rows, widths)
    .header(
      Row::new(header.items.iter().map(|h| h.text)).style(
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
  fn song_row_cells_match_hidden_columns() {
    // Cell shape must mirror song_table_columns: a hidden column drops its
    // cells too (ratatui zips cells to widths by position).
    let mut b = crate::user_config::UserConfig::new().behavior;
    let full = song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b);
    assert_eq!(full, ["", "T", "A", "AL", "2024-01-01", "3:00"]);
    b.show_artist_column = false;
    assert_eq!(
      song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b),
      ["", "T", "AL", "2024-01-01", "3:00"]
    );
    b.show_date_added_column = false;
    assert_eq!(
      song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b),
      ["", "T", "AL", "3:00"]
    );
    b.show_album_column = false;
    b.show_length_column = false;
    assert_eq!(
      song_row_cells("T", "A", "AL", "2024-01-01", 180_000, true, &b),
      ["", "T"]
    );
    // Album context (no date column): same shape rules without the date.
    let mut b = crate::user_config::UserConfig::new().behavior;
    b.show_artist_column = false;
    assert_eq!(
      song_row_cells("T", "A", "AL", "", 180_000, false, &b),
      ["", "T", "AL", "3:00"]
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
    // h=10, padding=5 -> viewport=5. 6 items overflow, 5 items fit.
    let overflow = render_grid(6, 0, 60, 10);
    let fits = render_grid(5, 0, 60, 10);
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
    // 30 items, h=10, viewport=5. Thumb position should move down with selection.
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
    let header_x = lines[1].find("Date Added").expect("Date Added header missing");
    let first_row = &lines[2];
    assert!(
      first_row[header_x..].starts_with("2024-01-15"),
      "date not under the Date Added header (x={header_x}):\nrow: {first_row}"
    );
  }

  #[test]
  fn stale_envelope_does_not_shadow_current_track() {
    // An envelope for a previous track must not be drawn for the current one;
    // the columns fall through to the features/simulated tiers instead.
    // The id must be a valid base62 spotify id, else TrackId deserializes
    // to None and the envelope can never match.
    let mut app = playback_app_with_track("4iV5W9uYEdYUVa79Axb7Rh");
    app.audio_envelope = Some(("old-track".to_string(), vec![1.0; 512]));
    let columns = visualizer_columns(&app, 40);
    assert!(
      columns.iter().any(|&v| v < 1.0),
      "stale envelope must not win for a different track"
    );

    // Matching uri: the envelope drives the bars (Display of a TrackId is
    // its full URI, "spotify:track:...").
    app.audio_envelope = Some((
      "spotify:track:4iV5W9uYEdYUVa79Axb7Rh".to_string(),
      vec![0.5; 512],
    ));
    let columns = visualizer_columns(&app, 40);
    assert!(
      columns.iter().all(|&v| (v - 0.5).abs() < 1e-6),
      "envelope wins only for its own track"
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
    // data rows from y=3. list_rect height 7 → draw_table viewport = 7-5 = 2,
    // offset = 12-2 = 10 → rows y=3..4 hold Mock Song 10..11, matching
    // table_row_index (index 0 at y=3 = rect.y+2). y=5 is the bottom border.
    assert!(
      rows[2].contains("Title"),
      "row 2 should be the table header, got: {}",
      rows[2]
    );
    for (row, expected) in [(3, 10), (4, 11)] {
      assert!(
        rows[row].contains(&format!("Mock Song {expected}")),
        "row {row} should hold Mock Song {expected}, got: {}",
        rows[row]
      );
    }
  }
}
