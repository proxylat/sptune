use super::super::app::{
  visible_library_options, ActiveBlock, AlbumTableContext, App, ArtistBlock, SearchResultBlock,
};
use crate::app::{TrackSortColumn, TrackTableContext};
use crate::tui::ColumnId;
use crate::user_config::Theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use rspotify::model::artist::SimplifiedArtist;
use rspotify::model::RepeatState;
use unicode_width::UnicodeWidthStr;

pub const PLAYBAR_HEIGHT: u16 = 4;
pub const SMALL_TERMINAL_HEIGHT: u16 = 45;
// plain volume bar width (label "♪ 100% " + filled bar)
pub const VOLUME_BAR_WIDTH: u16 = 24;
// "♪ " icon preceding the volume bar
pub const VOLUME_LABEL_LEN: u16 = 2;
// filled block characters in the volume bar
pub const VOLUME_BAR_LEN: u16 = 17;
// moderate fixed width of the music progress bar (click precision ~2.57s/cell
// on a 3:00 track; the volume box right of it reserves its own space)
pub const PROGRESS_BAR_LEN: u16 = 70;
// " 0:00 " / " 5:00 " time labels on each side of the progress bar
pub const PLAYBAR_TIME_LEN: u16 = 6;
// clickable settings rows in the '?' menu: black theme, library, playlists,
// volume ramp bar, mouse interactions, theme preset, seek by typing,
// resume last song, restore settings, clear cache, dev view, columns,
// visualizer style
pub const SETTINGS_ROW_COUNT: u16 = 16;

// Prefix for list panel titles; clicking the title row refreshes the list.
pub const REFRESH_GLYPH: &str = "♻ ";

// The volume bar: 1 row, right-aligned inside the playbar, on the same row
// as the music bar. Shared by drawer and mouse hit-test.
pub fn playbar_volume_rect(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + playbar.width.saturating_sub(VOLUME_BAR_WIDTH + 1),
    playbar.y + 2,
    VOLUME_BAR_WIDTH,
    1,
  )
}

// The playbar row holding the song name, full inner width. Shares the first
// inner row (y+1) with the centered transport buttons: the name renders left
// of them.
pub fn playbar_song_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar.y + 1,
    playbar.width.saturating_sub(2),
    1,
  )
}

// The playbar row holding the centered progress bar, sharing the music-bar
// row (y+2): the artist name is drawn on the left, the bar centered.
pub fn playbar_bar_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar.y + 2,
    playbar.width.saturating_sub(2),
    1,
  )
}

// The playbar row holding the artist name, on the music-bar row (y+2), left
// of the centered progress bar.
pub fn playbar_artist_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar.y + 2,
    playbar.width.saturating_sub(2),
    1,
  )
}

// The bar keeps a fixed moderate width (70 cells) regardless of track
// length; the click-to-seek granularity is therefore a constant ~2.57s/cell
// on a 3:00 mock track. Sub-cell eighth blocks show partial seconds.
pub fn playbar_progress_rect(playbar: Rect) -> Option<Rect> {
  let row = playbar_bar_row(playbar);
  let times = PLAYBAR_TIME_LEN * 2;
  // Keep the symmetric margins clear of the volume box when it is visible.
  let vol_span = if playbar.width > 70 {
    VOLUME_BAR_WIDTH
  } else {
    0
  };
  let max_len = row.width.saturating_sub(times + 2 * (vol_span + 1));
  if max_len < 16 {
    return None;
  }
  let bar_len = max_len.min(PROGRESS_BAR_LEN);
  let total = bar_len + times;
  let bar_x = row.x + (row.width - total) / 2 + PLAYBAR_TIME_LEN;
  Some(Rect::new(bar_x, row.y, bar_len, 1))
}

// Geometry of the clickable settings section at the top of the '?' menu,
// shared between the drawer and the mouse hit-test.
pub fn settings_section_rect(area: Rect) -> Rect {
  Layout::default()
    .direction(Direction::Vertical)
    .constraints(
      [
        Constraint::Length(SETTINGS_ROW_COUNT + 2),
        Constraint::Min(1),
      ]
      .as_ref(),
    )
    .margin(2)
    .split(area)[0]
}

// Geometry of the shortcuts table below the settings rows, shared between
// the drawer, the mouse hit-test and the scrollbar.
pub fn shortcuts_table_rect(area: Rect) -> Rect {
  Layout::default()
    .direction(Direction::Vertical)
    .constraints(
      [
        Constraint::Length(SETTINGS_ROW_COUNT + 2),
        Constraint::Min(1),
      ]
      .as_ref(),
    )
    .margin(2)
    .split(area)[1]
}

// Header height: tall enough for the 5-line figlet banner in every mode.
pub fn header_height(_app: &App) -> u16 {
  5
}

// Height of the song-table viewport (rows visible between header and playbar,
// minus the table padding the drawer subtracts). Shared by the drawer, the
// wheel handler and the keyboard follow logic so they can never drift.
pub fn song_table_viewport(app: &App) -> usize {
  let margin = if app.size.height > SMALL_TERMINAL_HEIGHT {
    1
  } else {
    0
  };
  app
    .size
    .height
    .saturating_sub(header_height(app))
    .saturating_sub(PLAYBAR_HEIGHT)
    .saturating_sub(2 * margin)
    .saturating_sub(5) as usize
}

// Header-row column zones (mirror of draw_input_and_help_box's split):
// title on the left, centered smaller search box, gear zone on the right.
pub fn header_zones(_app: &App, input_box: Rect) -> Vec<Rect> {
  Layout::default()
    .direction(Direction::Horizontal)
    .constraints(
      [
        Constraint::Percentage(35),
        Constraint::Percentage(30),
        Constraint::Percentage(35),
      ]
      .as_ref(),
    )
    .split(input_box)
    .to_vec()
    .into_iter()
    // The 3-row search box is vertically centered inside the taller banner
    // header; keep click/cursor zones on the visual box only.
    .map(|mut r| {
      if r.height > 3 {
        let inset = (r.height - 3) / 2;
        r.y += inset;
        r.height = 3;
      }
      r
    })
    .collect()
}

// The centered search box of the header row, used to make it clickable.
pub fn search_box_rect(app: &App, input_box: Rect) -> Rect {
  header_zones(app, input_box)[1]
}

// Click zone of a playbar text row (song name / artist name): from the row
// start to the rendered text end, truncated to the same limit the drawer
// uses, so clicks beyond the visible text do nothing. Shared by drawer and
// mouse hit-test.
pub fn playbar_text_click_range(row: Rect, limit_end: u16, text: &str) -> (u16, u16) {
  let limit = (limit_end.saturating_sub(row.x + 1)) as usize;
  let len = text.chars().count().min(limit) as u16;
  (row.x, row.x + len)
}

// The settings gear is drawn right-aligned inside the header's gear zone,
// inset 3 cells from the right edge, vertically centered on the header row
// (mirror of draw_input_and_help_box); the click zone is the glyph cell
// with one cell of padding.
pub fn gear_click_rect(app: &App, input_box: Rect) -> Rect {
  let zone = header_zones(app, input_box)[2];
  let glyph_x = zone.x + zone.width.saturating_sub(3) - 1;
  let start = glyph_x.saturating_sub(1).max(zone.x);
  let row_y = input_box.y + (input_box.height.saturating_sub(1)) / 2;
  Rect::new(start, row_y, (glyph_x + 2).min(zone.x + zone.width) - start, 1)
}

pub fn get_search_results_highlight_state(
  app: &App,
  block_to_match: SearchResultBlock,
) -> (bool, bool) {
  let current_route = app.get_current_route();
  (
    app.search_results.selected_block == block_to_match,
    current_route.hovered_block == ActiveBlock::SearchResultBlock
      && app.search_results.hovered_block == block_to_match,
  )
}

pub fn get_artist_highlight_state(app: &App, block_to_match: ArtistBlock) -> (bool, bool) {
  let current_route = app.get_current_route();
  if let Some(artist) = &app.artist {
    let is_hovered = artist.artist_selected_block == block_to_match;
    let is_selected = current_route.hovered_block == ActiveBlock::ArtistBlock
      && artist.artist_hovered_block == block_to_match;
    (is_hovered, is_selected)
  } else {
    (false, false)
  }
}

pub fn get_color((is_active, is_hovered): (bool, bool), theme: Theme) -> Style {
  match (is_active, is_hovered) {
    (true, _) => Style::default().fg(theme.selected),
    (false, true) => Style::default().fg(theme.hovered),
    _ => Style::default().fg(theme.inactive),
  }
}

/// Search results layout: a single-row tab bar on top and the expanded
/// block's list below. The load-more row lives inside the list (appended as
/// its last item), so no extra row is reserved here. Shared with the mouse
/// hit-testing so the two never drift.
/// Returns (tab_bar_rect, tab_cells, list_rect).
pub fn search_layout(
  chunk: Rect,
  expanded: SearchResultBlock,
  has_more: bool,
) -> (Rect, Vec<(SearchResultBlock, Rect)>, Rect) {
  let tab_bar = Rect {
    x: chunk.x,
    y: chunk.y,
    width: chunk.width,
    height: 1,
  };
  let cell_w = chunk.width / 5;
  let tabs = [
    (SearchResultBlock::SongSearch, "Songs"),
    (SearchResultBlock::ArtistSearch, "Artists"),
    (SearchResultBlock::AlbumSearch, "Albums"),
    (SearchResultBlock::PlaylistSearch, "Playlists"),
    (SearchResultBlock::ShowSearch, "Podcasts"),
  ];
  let tab_cells = tabs
    .iter()
    .enumerate()
    .map(|(i, (block, _))| {
      (
        block.clone(),
        Rect {
          x: chunk.x + (i as u16) * cell_w,
          y: chunk.y,
          width: cell_w,
          height: 1,
        },
      )
    })
    .collect::<Vec<_>>();

  let below = Rect {
    x: chunk.x,
    y: chunk.y + 1,
    width: chunk.width,
    height: chunk.height.saturating_sub(1),
  };
  if !has_more || expanded == SearchResultBlock::Empty {
    return (tab_bar, tab_cells, below);
  }
  (tab_bar, tab_cells, below)
}

pub fn create_artist_string(artists: &[SimplifiedArtist]) -> String {
  artists
    .iter()
    .map(|artist| artist.name.to_string())
    .collect::<Vec<String>>()
    .join(", ")
}

pub fn millis_to_minutes(millis: u128) -> String {
  let minutes = millis / 60000;
  let seconds = (millis % 60000) / 1000;
  let seconds_display = if seconds < 10 {
    format!("0{}", seconds)
  } else {
    format!("{}", seconds)
  };

  if seconds == 60 {
    format!("{}:00", minutes + 1)
  } else {
    format!("{}:{}", minutes, seconds_display)
  }
}

pub fn display_track_progress(progress: u128, track_duration: u32) -> String {
  let duration = millis_to_minutes(u128::from(track_duration));
  let progress_display = millis_to_minutes(progress);
  let remaining = millis_to_minutes(u128::from(track_duration).saturating_sub(progress));

  format!("{}/{} (-{})", progress_display, duration, remaining,)
}

// `percentage` param needs to be between 0 and 1
pub fn get_percentage_width(width: u16, percentage: f32) -> u16 {
  let padding = 3;
  let width = width - padding;
  (f32::from(width) * percentage) as u16
}

// Make better use of space on small terminals
pub fn get_main_layout_margin(app: &App) -> u16 {
  if app.size.height > SMALL_TERMINAL_HEIGHT {
    1
  } else {
    0
  }
}

// Thumb geometry for the website-style scrollbar, shared by rendering and
// mouse drag so the draggable thumb is exactly the drawn thumb.
// `offset` is the number of scrolled items (0..=count-viewport).
// Returns (thumb_top, thumb_len) within a track of `track_h` rows.
pub fn scrollbar_geometry(
  track_h: usize,
  count: usize,
  viewport: usize,
  offset: usize,
) -> (usize, usize) {
  if track_h < 1 {
    // A track of zero rows (rect height <= 2 in a tiny terminal) can't hold
    // a thumb; the caller skips drawing when both are zero.
    return (0, 0);
  }
  let thumb_len = (track_h * viewport / count.max(1)).clamp(1, track_h);
  let travel = track_h - thumb_len;
  let max_offset = count.saturating_sub(viewport).max(1);
  let thumb_top = (offset.min(max_offset) * travel / max_offset).min(travel);
  (thumb_top, thumb_len)
}

// Sidebar split: the Library box grows to fit its (small) entry list so it
// never needs a scrollbar; playlists take the rest. The cap keeps playlists
// visible when the library itself overflows.
pub fn library_playlists_split(app: &App, chunk: Rect) -> (Rect, Rect) {
  let library_len = visible_library_options(&app.hidden_library_sections).len() + 2;
  let cap = ((chunk.height * 2 / 5) as usize).max(4);
  let lib_h = library_len.min(cap) as u16;
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(lib_h), Constraint::Min(1)].as_ref())
    .split(chunk);
  (chunks[0], chunks[1])
}

/// Facts about a scrollable list: how many rows it scrolls through, how many
/// fit the viewport and where the thumb sits. The drawer, the wheel and the
/// scrollbar drag all read from this one place, so count/viewport/offset can
/// never drift apart again (a `+1` load-more row slipping into one but not
/// the other is what made the songs scrollbar stop short of the end).
#[derive(Clone, Copy, Debug)]
pub struct ListScroll {
  pub count: usize,
  pub viewport: usize,
  /// true: the list holds an explicit view offset; false: the selection
  /// index is the truth and the thumb follows `index - viewport`.
  pub view_mode: bool,
  pub index: usize,
  pub offset: usize,
}

impl ListScroll {
  /// Thumb position on the track, in view rows, as the drawer renders it.
  pub fn thumb_offset(&self) -> usize {
    if self.view_mode {
      self.offset
    } else {
      self.index.saturating_sub(self.viewport)
    }
  }
}

/// Single source of truth for (count, viewport, index/offset) of every
/// scrollable list. `rect` is where the list is drawn; the viewport derives
/// from it (tables reserve a header row + selection rows, selectable lists
/// only the title).
pub fn list_scroll(app: &App, block: ActiveBlock, rect: Rect) -> Option<ListScroll> {
  let selection = |index: usize, count: usize| ListScroll {
    count,
    viewport: rect.height.saturating_sub(5) as usize,
    view_mode: false,
    index,
    offset: 0,
  };
  Some(match block {
    ActiveBlock::HelpMenu => ListScroll {
      count: app.help_docs_size as usize,
      viewport: rect.height.saturating_sub(3) as usize,
      view_mode: true,
      index: 0,
      offset: app.help_scroll_offset as usize,
    },
    ActiveBlock::RequestLog => ListScroll {
      count: app.request_log.len(),
      viewport: rect.height.saturating_sub(2) as usize,
      view_mode: false,
      index: app.request_log_index.unwrap_or(0),
      offset: 0,
    },
    ActiveBlock::Library => ListScroll {
      count: visible_library_options(&app.hidden_library_sections).len(),
      viewport: rect.height.saturating_sub(2) as usize,
      view_mode: false,
      index: app.library.selected_index,
      offset: 0,
    },
    ActiveBlock::MyPlaylists => ListScroll {
      count: app
        .playlists
        .as_ref()
        .map(|playlists| playlists.items.len())
        .unwrap_or(0),
      viewport: rect.height.saturating_sub(2) as usize,
      view_mode: false,
      index: app.selected_playlist_index.unwrap_or(0),
      offset: 0,
    },
    ActiveBlock::TrackTable => ListScroll {
      // The load-more row is part of the drawn table, so it scrolls too.
      count: app.track_table.tracks.len() + usize::from(app.track_table_has_more()),
      viewport: rect.height.saturating_sub(5) as usize,
      view_mode: true,
      index: 0,
      offset: app.track_table.scroll_offset,
    },
    ActiveBlock::AlbumTracks => match &app.album_table_context {
      AlbumTableContext::Simplified => {
        let album = app.selected_album_simplified.as_ref()?;
        selection(album.selected_index, album.tracks.items.len())
      }
      AlbumTableContext::Full => selection(
        app.saved_album_tracks_index,
        app
          .selected_album_full
          .as_ref()
          .map(|album| album.album.tracks.items.len())
          .unwrap_or(0),
      ),
    },
    ActiveBlock::RecentlyPlayed => {
      let recently_played = app.recently_played.result.as_ref()?;
      selection(app.recently_played.index, recently_played.items.len())
    }
    ActiveBlock::AlbumList => {
      let albums = app.library.saved_albums.get_results(None)?;
      selection(app.album_list_index, albums.items.len())
    }
    ActiveBlock::Artists => selection(app.artists_list_index, app.artists.len()),
    ActiveBlock::Podcasts => {
      let shows = app.library.saved_shows.get_results(None)?;
      selection(app.shows_list_index, shows.items.len())
    }
    ActiveBlock::EpisodeTable => {
      let episodes = app.library.show_episodes.get_results(None)?;
      selection(app.episode_list_index, episodes.items.len())
    }
    ActiveBlock::MadeForYou => selection(app.made_for_you_index, 5),
    _ => return None,
  })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybarButton {
  Shuffle,
  Prev,
  PlayPause,
  Next,
  Repeat,
}

// The playbar transport row: shuffle, previous, play/pause, next, repeat,
// centered on the first inner row, just above the music bar. The song name
// shares this row, drawn left of the buttons (truncated so it never
// overlaps).
pub fn playbar_controls_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar.y + 1,
    playbar.width.saturating_sub(2),
    1,
  )
}

// x where the transport buttons text starts — the button group
// dead-centered on the playbar row, so the buttons always sit above the
// middle of the music bar regardless of repeat mode. Shared by drawer and
// mouse hit-test.
pub fn playbar_controls_x(playbar: Rect, controls: &[(PlaybarButton, String)]) -> u16 {
  let row = playbar_controls_row(playbar);
  let group: u16 = controls.iter().map(|(_, t)| t.width() as u16 + 1).sum();
  row.x + row.width.saturating_sub(group) / 2
}

// The five transport buttons with their glyphs, left to right: shuffle,
// prev, play/pause, next, repeat. Active state is carried by the drawer
// (color) and by the play/pause glyph itself; the repeat mode word is a
// separate span rendered after the group (see repeat_label) so the button
// group keeps a constant width. All strings are fixed-width so the group
// never re-centers between states; the centering math uses unicode widths.
pub fn build_playbar_controls(is_playing: bool) -> Vec<(PlaybarButton, String)> {
  vec![
    (PlaybarButton::Shuffle, "⇄".into()),
    (PlaybarButton::Prev, "⏮".into()),
    (
      PlaybarButton::PlayPause,
      if is_playing {
        " ‖".into()
      } else {
        " ▶".into()
      },
    ),
    (PlaybarButton::Next, "⏭".into()),
    (PlaybarButton::Repeat, " ↻".into()),
  ]
}

// The repeat-mode word, rendered after the button group (in the margin) so
// the group itself never changes width when the mode changes.
pub fn repeat_label(repeat: RepeatState) -> Option<String> {
  match repeat {
    RepeatState::Off => None,
    RepeatState::Context => Some("All".into()),
    RepeatState::Track => Some("One".into()),
  }
}

// Builds the playbar title string.
// Playbar title: the trailing cells of the title text are the fullscreen
// toggle (see handle_playbar_click). Box-drawing characters so every terminal
// font renders it, three cells wide so it reads as a window.
pub fn build_playbar_title(play_title: &str, device_name: &str) -> String {
  format!("{:-7} ({}) - ┌─┐", play_title, device_name)
}

// Which track-table contexts have a Date Added column (they carry added_at).
pub fn track_table_with_date(context: Option<&TrackTableContext>) -> bool {
  matches!(
    context,
    Some(TrackTableContext::MyPlaylists)
      | Some(TrackTableContext::SavedTracks)
      | Some(TrackTableContext::MadeForYou)
  )
}

// Columns of the song table: (ColumnId, x offset from content start, width).
// Hidden columns (gear menu) are skipped; the remaining widths are unchanged.
pub fn song_table_columns(
  width: u16,
  with_date: bool,
  show_album: bool,
  show_artist: bool,
  show_length: bool,
  show_date_added: bool,
) -> Vec<(ColumnId, u16, u16)> {
  // The non-title columns keep their fixed percentage widths; the title
  // column absorbs whatever the visible columns leave over, so hiding a
  // column widens the song names instead of leaving a gap.
  let mut tail: Vec<(ColumnId, u16)> = vec![];
  if show_artist {
    tail.push((ColumnId::Artist, get_percentage_width(width, 0.3)));
  }
  if with_date {
    if show_album {
      tail.push((ColumnId::Album, get_percentage_width(width, 0.15)));
    }
    if show_date_added {
      tail.push((ColumnId::DateAdded, get_percentage_width(width, 0.15)));
    }
  } else if show_album {
    tail.push((ColumnId::Album, get_percentage_width(width, 0.3)));
  }
  if show_length {
    tail.push((ColumnId::Length, get_percentage_width(width, 0.1)));
  }
  let fixed: u16 = tail.iter().map(|(_, w)| w).sum();
  let title = width.saturating_sub(2 + fixed).max(1);

  let mut columns = vec![(ColumnId::Liked, 0, 2)];
  let mut x = 2;
  columns.push((ColumnId::Title, x, title));
  x += title;
  for (column, w) in tail {
    columns.push((column, x, w));
    x += w;
  }
  columns
}

pub fn sort_column_for(column: ColumnId) -> Option<TrackSortColumn> {
  match column {
    ColumnId::Title => Some(TrackSortColumn::Title),
    ColumnId::Artist => Some(TrackSortColumn::Artist),
    ColumnId::Album => Some(TrackSortColumn::Album),
    ColumnId::Length => Some(TrackSortColumn::Length),
    ColumnId::DateAdded => Some(TrackSortColumn::DateAdded),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn scrollbar_geometry_degenerate_rect() {
    // A scrollbar track with zero rows (rect height <= 2 in a tiny
    // terminal) must not panic on the thumb clamp.
    assert_eq!(scrollbar_geometry(0, 5, 3, 0), (0, 0));
    assert_eq!(scrollbar_geometry(0, 0, 0, 0), (0, 0));
  }

  #[test]
  fn library_playlists_split_fits_library_items() {
    let app = App::default();
    let chunk = Rect::new(1, 6, 40, 40);
    let (lib, playlists) = library_playlists_split(&app, chunk);
    let items = visible_library_options(&app.hidden_library_sections).len();
    assert_eq!(lib.height, (items + 2) as u16);
    assert_eq!(playlists.y, lib.y + lib.height);
    assert_eq!(playlists.height, chunk.height - lib.height);
  }

  #[test]
  fn millis_to_minutes_test() {
    assert_eq!(millis_to_minutes(0), "0:00");
    assert_eq!(millis_to_minutes(1000), "0:01");
    assert_eq!(millis_to_minutes(1500), "0:01");
    assert_eq!(millis_to_minutes(1900), "0:01");
    assert_eq!(millis_to_minutes(60 * 1000), "1:00");
    assert_eq!(millis_to_minutes(60 * 1500), "1:30");
  }

  #[test]
  fn display_track_progress_test() {
    assert_eq!(
      display_track_progress(0, 2 * 60 * 1000),
      "0:00/2:00 (-2:00)"
    );

    assert_eq!(
      display_track_progress(60 * 1000, 2 * 60 * 1000),
      "1:00/2:00 (-1:00)"
    );
  }

  #[test]
  fn gear_click_rect_hits_the_drawn_glyph() {
    // Header is 5 rows (banner): the gear is drawn vertically centered on
    // the FULL input_box row (h-1)/2 = 2 — not the inset zone row (1).
    // 35% of 200 = 70 → gear zone x 130..200, glyph at 196.
    let mut app = App::default();
    app.mock = true;
    let rect = gear_click_rect(&app, Rect::new(0, 0, 200, 5));
    assert_eq!((rect.x, rect.y, rect.width), (195, 2, 3));
    // Real mode keeps the same 5-row banner geometry.
    app.mock = false;
    let rect = gear_click_rect(&app, Rect::new(0, 0, 200, 5));
    assert_eq!((rect.x, rect.y, rect.width), (195, 2, 3));
    // A shifted header keeps the same relative geometry.
    let rect = gear_click_rect(&app, Rect::new(4, 0, 200, 3));
    assert_eq!((rect.x, rect.y), (199, 1));
  }
}
