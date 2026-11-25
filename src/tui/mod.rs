pub mod help;
pub mod layout;
pub mod music_view;

mod artist;
mod dialogs;
mod help_menu;
mod playbar;
mod queue;
mod search;
mod sidebar;
mod tables;
mod widgets;

#[cfg(test)]
mod tests;

pub use dialogs::{draw_device_list, draw_error_screen};
pub use help_menu::draw_help_menu;
pub use music_view::draw_music_view;
pub use sidebar::bust_playlist_items_cache;
pub use tables::group_consecutive_plays;
#[cfg(test)]
pub use tables::PlayGroup;
pub use widgets::ColumnId;

#[cfg(test)]
use dialogs::draw_add_to_playlist_dialog;
#[cfg(test)]
use tables::track_index_cell;
#[cfg(test)]
use widgets::{
  draw_table, ellipsize, TableHeader, TableHeaderItem, TableId, TableItem, TableSelection,
};

use crate::app::{ActiveBlock, App, RouteId};
use artist::draw_artist_page;
use dialogs::{draw_dialog, draw_request_log};
use layout::{get_color, PLAYBAR_HEIGHT};
#[cfg(test)]
use music_view::draw_music_lyrics;
use playbar::draw_playbar;
use ratatui::{
  layout::{Alignment, Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span, Text},
  widgets::{Block, BorderType, Borders, Paragraph},
  Frame,
};
use search::draw_search_results;
use sidebar::draw_user_block;
use tables::{
  draw_album_list, draw_album_table, draw_artist_table, draw_made_for_you, draw_podcast_table,
  draw_recently_played_table, draw_show_episodes, draw_song_table,
};
#[cfg(test)]
use tables::{relative_date, song_row_cells};

static LOGO_TEXT: std::sync::LazyLock<Text<'static>> = std::sync::LazyLock::new(|| {
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
  let mut title_lines = Vec::with_capacity(art.len());
  for line in art {
    let mut spans = Vec::with_capacity(line.len());
    for c in line.chars() {
      let t = idx as f64 / total.max(1) as f64;
      idx += 1;
      spans.push(Span::styled(
        c.to_string(),
        Style::default().fg(color_at(t)),
      ));
    }
    idx += 1;
    title_lines.push(Line::from(spans));
  }
  Text::from(title_lines)
});

pub fn draw_input_and_help_box(f: &mut Frame, app: &App, layout_chunk: Rect) {
  // Header: app title on the left, Search box centered, gear zone far right
  let w0 = (layout_chunk.width as u32 * 35 / 100) as u16;
  let w1 = (layout_chunk.width as u32 * 30 / 100) as u16;
  let w2 = layout_chunk.width.saturating_sub(w0 + w1);
  let chunks = [
    Rect::new(layout_chunk.x, layout_chunk.y, w0, layout_chunk.height),
    Rect::new(layout_chunk.x + w0, layout_chunk.y, w1, layout_chunk.height),
    Rect::new(
      layout_chunk.x + w0 + w1,
      layout_chunk.y,
      w2,
      layout_chunk.height,
    ),
  ];

  let current_route = app.get_current_route();

  let highlight_state = (
    current_route.active_block == ActiveBlock::Input,
    current_route.hovered_block == ActiveBlock::Input,
  );

  f.render_widget(
    &*LOGO_TEXT,
    Rect::new(
      chunks[0].x + 1,
      chunks[0].y,
      chunks[0].width.saturating_sub(1),
      5,
    ),
  );

  let input_string: String = app.input.iter().collect();
  // Scroll the input so the cursor stays visible instead of truncating with
  // "...". Reserve 3 cells for the clear ✕ when there is input.
  let inner_w = chunks[1].width.saturating_sub(2) as usize;
  let reserve = if app.input.is_empty() { 0 } else { 3 };
  let viewport_w = inner_w.saturating_sub(reserve);
  let (visible, _) = crate::tui::layout::search_input_visible(
    &input_string,
    app.input_cursor_position as usize,
    viewport_w,
  );
  let lines = Text::from(visible);
  let input = Paragraph::new(lines).block(
    Block::default()
      .borders(Borders::ALL)
      .border_type(if app.user_config.terminal.unicode_borders {
        BorderType::Rounded
      } else {
        BorderType::Plain
      })
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

  // Minimize-to-tray [_] and Settings gear pinned near the right of the
  // header row, inset a few cells so they never clip at the terminal edge.
  let gear_style = Style::default().fg(app.user_config.theme.active);
  f.render_widget(
    Paragraph::new(Line::from(vec![Span::styled(
      "[_]  ⚙\u{FE0F}",
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
  let area = f.area();
  let inner = Rect::new(
    area.x.saturating_add(margin),
    area.y.saturating_add(margin),
    area.width.saturating_sub(margin * 2),
    area.height.saturating_sub(margin * 2),
  );
  let header_h = layout::HEADER_HEIGHT.min(inner.height);
  let playbar_h = PLAYBAR_HEIGHT.min(inner.height.saturating_sub(header_h));
  let body_h = inner.height.saturating_sub(header_h + playbar_h);
  let parent_layout = [
    Rect::new(inner.x, inner.y, inner.width, header_h),
    Rect::new(inner.x, inner.y + header_h, inner.width, body_h),
    Rect::new(inner.x, inner.y + header_h + body_h, inner.width, playbar_h),
  ];

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

  let current_route = app.get_current_route();

  let content = if app.dev_view {
    Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
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
    RouteId::Queue => {
      queue::draw_queue_table(f, app, content);
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
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
        .split(content_rect)[1],
    );
  }
}
