use super::super::app::{
  visible_library_options, ActiveBlock, AlbumTableContext, App, ArtistBlock, SearchResultBlock,
};
use crate::app::{TrackSortColumn, TrackTableContext};
use crate::tui::ColumnId;
use crate::user_config::{BehaviorConfig, Theme};
use ratatui::layout::Rect;
use ratatui::style::Style;
use rspotify::model::artist::SimplifiedArtist;
use rspotify::model::RepeatState;
use unicode_width::UnicodeWidthStr;

pub const PLAYBAR_HEIGHT: u16 = 6;
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
pub const SETTINGS_ROW_COUNT: u16 = 43;

// Prefix for list panel titles; clicking the title row refreshes the list.
pub const REFRESH_GLYPH: &str = "♻ ";
// Placeholder in the in-playlist search box. Plain ASCII on every platform:
// the previous Font Awesome PUA glyph (U+F002) renders as tofu on stock
// console fonts, and cfg!(windows) is compile-time so cross-built binaries
// got the wrong branch anyway.
pub const PLAYLIST_SEARCH_PLACEHOLDER: &str = "/";
pub const SIDEBAR_TOGGLE_GLYPH: &str = "≡";
pub const NUMBER_TO_TITLE_GAP: u16 = 2;

pub fn help_full_rect(area: Rect) -> Rect {
  Rect::new(
    area.x + 2,
    area.y + 2,
    area.width.saturating_sub(4),
    area.height.saturating_sub(4),
  )
}

/// First row of the 2-row playbar content block, vertically centered
/// inside the playbar's inner height (y+2 when PLAYBAR_HEIGHT == 6).
pub fn playbar_content_top_y(playbar: Rect) -> u16 {
  let inner_h = playbar.height.saturating_sub(2);
  playbar.y + 1 + inner_h.saturating_sub(2) / 2
}

// The volume bar: 1 row, right-aligned inside the playbar, on the same row
// as the music bar. Shared by drawer and mouse hit-test.
pub fn playbar_volume_rect(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + playbar.width.saturating_sub(VOLUME_BAR_WIDTH + 3),
    playbar_content_top_y(playbar) + 1,
    VOLUME_BAR_WIDTH,
    1,
  )
}

pub const PLAYBAR_HEART_WIDTH: u16 = 2;
pub const PLAYBAR_COVER_COLS: u16 = 10;
pub const PLAYBAR_COVER_GAP: u16 = 1;

pub fn playbar_cover_offset(playbar: Rect, has_cover: bool) -> u16 {
  if has_cover && playbar.width > 60 {
    PLAYBAR_COVER_COLS + PLAYBAR_COVER_GAP
  } else {
    0
  }
}

pub fn playbar_cover_rect(app: &App) -> Option<Rect> {
  let has_track = app
    .current_playback_context
    .as_ref()
    .and_then(|c| c.item.as_ref())
    .is_some();
  if !has_track || app.cover_art.is_none() {
    return None;
  }
  let margin = get_main_layout_margin(app);
  let inner_w = app.size.width.saturating_sub(margin * 2);
  let inner_h = app.size.height.saturating_sub(margin * 2);
  let header_h = HEADER_HEIGHT.min(inner_h);
  let playbar_h = PLAYBAR_HEIGHT.min(inner_h.saturating_sub(header_h));
  if inner_w <= 60 || playbar_h < 4 {
    return None;
  }
  let playbar_y = margin + inner_h.saturating_sub(playbar_h);
  Some(Rect::new(
    margin + 1,
    playbar_y + 1,
    PLAYBAR_COVER_COLS,
    playbar_h.saturating_sub(2).min(4),
  ))
}

// The playbar row holding the song name, full inner width. Shares the first
// inner row (y+1) with the centered transport buttons: the name renders left
// of them.
pub fn playbar_song_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar_content_top_y(playbar),
    playbar.width.saturating_sub(2),
    1,
  )
}

// The playbar row holding the centered progress bar, at the bottom inner row.
pub fn playbar_bar_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar.y + playbar.height.saturating_sub(2),
    playbar.width.saturating_sub(2),
    1,
  )
}

// The playbar row holding the artist name, vertically centered below the song name.
pub fn playbar_artist_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar_content_top_y(playbar) + 1,
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

// Visible slice of the header search input that keeps the cursor in view.
// Returns (visible_text, cursor_offset_in_visible).
pub fn search_input_visible(input: &str, cursor: usize, viewport: usize) -> (String, usize) {
  use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
  if viewport == 0 {
    return (String::new(), 0);
  }
  let total = UnicodeWidthStr::width(input);
  if total <= viewport {
    return (input.to_string(), cursor.min(total));
  }
  let max_start = total.saturating_sub(viewport);
  let desired = cursor.saturating_sub(viewport.saturating_sub(1));
  let start = desired.min(max_start);
  let mut out = String::new();
  let mut w = 0usize;
  let mut visible_w = 0usize;
  for c in input.chars() {
    let cw = UnicodeWidthChar::width(c).unwrap_or(0);
    let char_start = w;
    let char_end = w + cw;
    w = char_end;
    if char_start >= start && char_start < start + viewport {
      // char starts inside window; if it would overflow window, still include
      // as long as its start is inside (wide chars may spill 1 cell, ok)
      if visible_w + cw <= viewport + 1 {
        out.push(c);
        visible_w += cw;
      }
    } else if char_start < start && char_end > start {
      // start cuts inside a wide char (shouldn't happen for cursor-aligned
      // starts but guard): include the char
      out.push(c);
      visible_w += cw;
    }
  }
  let cursor_in_view = cursor.saturating_sub(start).min(viewport);
  (out, cursor_in_view)
}

// Header height: tall enough for the 5-line figlet banner in every mode.
pub const HEADER_HEIGHT: u16 = 5;

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
    .saturating_sub(HEADER_HEIGHT)
    .saturating_sub(PLAYBAR_HEIGHT)
    .saturating_sub(2 * margin)
    .saturating_sub(3) as usize
}

// Header-row column zones (exact mirror of draw_input_and_help_box's integer split):
// title on the left, centered smaller search box, gear zone on the right.
pub fn header_zones(_app: &App, input_box: Rect) -> Vec<Rect> {
  let w0 = (input_box.width as u32 * 35 / 100) as u16;
  let w1 = (input_box.width as u32 * 30 / 100) as u16;
  let w2 = input_box.width.saturating_sub(w0 + w1);
  [
    Rect::new(input_box.x, input_box.y, w0, input_box.height),
    Rect::new(input_box.x + w0, input_box.y, w1, input_box.height),
    Rect::new(input_box.x + w0 + w1, input_box.y, w2, input_box.height),
  ]
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
  Rect::new(
    start,
    row_y,
    (glyph_x + 2).min(zone.x + zone.width) - start,
    1,
  )
}

pub fn minimize_click_rect(app: &App, input_box: Rect) -> Rect {
  let zone = header_zones(app, input_box)[2];
  let glyph_x = zone.x + zone.width.saturating_sub(3) - 1;
  let start = glyph_x.saturating_sub(6).max(zone.x);
  let row_y = input_box.y + (input_box.height.saturating_sub(1)) / 2;
  Rect::new(
    start,
    row_y,
    3.min((zone.x + zone.width).saturating_sub(start)),
    1,
  )
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
    let is_active = current_route.active_block == ActiveBlock::ArtistBlock
      && artist.artist_selected_block == block_to_match;
    let is_hovered = artist.artist_hovered_block == block_to_match;
    (is_active, is_hovered)
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

/// Artist page layout, mirroring the search page: a single-row tab bar
/// (Artist / Albums / Singles / EPs / Featuring) and the active tab's list below.
/// Shared with the mouse hit-testing so the two never drift.
/// Returns (tab_bar_rect, tab_cells, list_rect).
pub fn main_body_content_rect(app: &App) -> Option<Rect> {
  let margin = get_main_layout_margin(app);
  let inner = Rect::new(
    margin,
    margin,
    app.size.width.saturating_sub(margin * 2),
    app.size.height.saturating_sub(margin * 2),
  );
  let header_h = HEADER_HEIGHT.min(inner.height);
  let playbar_h = PLAYBAR_HEIGHT.min(inner.height.saturating_sub(header_h));
  let body_h = inner.height.saturating_sub(header_h + playbar_h);
  if body_h == 0 || inner.width == 0 {
    return None;
  }
  let routes = Rect::new(inner.x, inner.y + header_h, inner.width, body_h);
  let (_, content) = sidebar_content_split(app, routes);
  Some(content)
}

pub const ARTIST_HEADER_HEIGHT: u16 = 6;

pub fn artist_header_rect(chunk: Rect) -> Option<Rect> {
  (chunk.height >= 16 && chunk.width >= 20).then_some(Rect {
    x: chunk.x,
    y: chunk.y,
    width: chunk.width,
    height: ARTIST_HEADER_HEIGHT,
  })
}

pub fn artist_cover_rect(header: Rect) -> Rect {
  Rect {
    x: header.x + 1,
    y: header.y + 1,
    width: 10.min(header.width.saturating_sub(2)),
    height: header.height.saturating_sub(2),
  }
}

pub fn artist_follow_rect(header: Rect) -> Rect {
  let w = 13u16.min(header.width.saturating_sub(14));
  Rect {
    x: header.x + header.width.saturating_sub(w + 2),
    y: header.y + 1,
    width: w,
    height: 1,
  }
}

pub fn artist_layout(chunk: Rect, expanded: ArtistBlock) -> (Rect, Vec<(ArtistBlock, Rect)>, Rect) {
  let header_h = if artist_header_rect(chunk).is_some() {
    ARTIST_HEADER_HEIGHT
  } else {
    0
  };
  let tab_y = chunk.y + header_h;
  let tab_bar = Rect {
    x: chunk.x,
    y: tab_y,
    width: chunk.width,
    height: 1,
  };
  let tabs = [
    (ArtistBlock::TopTracks, "Artist"),
    (ArtistBlock::Albums, "Albums"),
    (ArtistBlock::Singles, "Singles"),
    (ArtistBlock::EPs, "EPs"),
    (ArtistBlock::AppearsOn, "Featuring"),
  ];
  let n = tabs.len() as u16;
  let cell_w = chunk.width.checked_div(n).unwrap_or(chunk.width);
  let tab_cells = tabs
    .iter()
    .enumerate()
    .map(|(i, (block, _))| {
      (
        *block,
        Rect {
          x: chunk.x + (i as u16) * cell_w,
          y: tab_y,
          width: cell_w,
          height: 1,
        },
      )
    })
    .collect::<Vec<_>>();
  let below = Rect {
    x: chunk.x,
    y: tab_y + 1,
    width: chunk.width,
    height: chunk.height.saturating_sub(header_h + 1),
  };
  if expanded == ArtistBlock::Empty {
    return (tab_bar, tab_cells, below);
  }
  (tab_bar, tab_cells, below)
}

pub fn create_artist_string(artists: &[SimplifiedArtist]) -> String {
  match artists {
    [] => String::new(),
    [one] => one.name.clone(),
    many => {
      let cap: usize = many.iter().map(|a| a.name.len() + 2).sum();
      let mut out = String::with_capacity(cap);
      for (i, artist) in many.iter().enumerate() {
        if i > 0 {
          out.push_str(", ");
        }
        out.push_str(&artist.name);
      }
      out
    }
  }
}

pub fn millis_to_minutes(millis: u128) -> String {
  let minutes = millis / 60_000;
  let seconds = (millis % 60_000) / 1_000;
  format!("{minutes}:{seconds:02}")
}

pub fn format_playlist_duration(total_ms: i64) -> String {
  // Sum in ms, round once to the nearest minute (Spotify's header rounds)
  let total_minutes = (total_ms + 30_000) / 60_000;
  let hours = total_minutes / 60;
  let mins = total_minutes % 60;
  if hours > 0 && mins > 0 {
    format!("{}h {}m", hours, mins)
  } else if hours > 0 {
    format!("{}h", hours)
  } else {
    format!("{}m", mins)
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

// Sidebar content split: auto-sizes the sidebar to fit its content, with the
// content area taking the remaining space.
pub fn sidebar_content_split(app: &App, chunk: Rect) -> (Rect, Rect) {
  let w = sidebar_width(app, chunk).min(chunk.width);
  (
    Rect::new(chunk.x, chunk.y, w, chunk.height),
    Rect::new(chunk.x + w, chunk.y, chunk.width - w, chunk.height),
  )
}

pub const SIDEBAR_MIN_WIDTH: u16 = 12;

/// Sidebar width: in expanded mode, sized to the longest rendered line (block
/// titles, library options, playlist names) + padding; in minimized mode,
/// sized to the widest playlist number. When `sidebar_width_override` is set
/// (drag resize, persisted to state.json) it wins over the auto size, clamped
/// to [SIDEBAR_MIN_WIDTH, chunk.width/2].
pub fn sidebar_width(app: &App, chunk: Rect) -> u16 {
  if !app.sidebar_minimized {
    if let Some(w) = app.sidebar_width_override {
      return w.clamp(SIDEBAR_MIN_WIDTH, chunk.width / 2);
    }
  }
  let playlist_len = app.playlists.as_ref().map(|p| p.items.len()).unwrap_or(0);
  let playlist_digits = (playlist_len.max(1).ilog10() as usize) + 1;
  let content_len = if app.sidebar_minimized {
    playlist_digits
  } else {
    crate::app::sidebar_max_content_len(app).max(1)
  };
  // +2 for the two block borders. A minimized sidebar has no scrollbar
  // column, so it needs no extra cell; the expanded one reserves one for it.
  let extra = if app.sidebar_minimized { 0 } else { 1 };
  (content_len as u16 + 2 + extra).min(chunk.width / 2)
}

pub fn sidebar_handle_rect(app: &App, chunk: Rect) -> Rect {
  let w = sidebar_width(app, chunk);
  // 1-col handle at the sidebar's right edge, inside the sidebar column
  Rect::new(chunk.x + w.saturating_sub(1), chunk.y, 1, chunk.height)
}

// Sidebar split: the Library box grows to fit its (small) entry list so it
// never needs a scrollbar; playlists take the rest. The cap keeps playlists
// visible when the library itself overflows.
pub fn library_playlists_split(app: &App, chunk: Rect) -> (Rect, Rect) {
  let override_h = app
    .user_config
    .behavior
    .enable_animations
    .then_some(app.library_height_override)
    .flatten();
  let lib_h = match override_h {
    Some(h) => h.clamp(4, chunk.height.saturating_sub(4)),
    None => {
      let library_len = visible_library_options(&app.hidden_library_sections).len() + 2;
      let cap = ((chunk.height * 2 / 5) as usize).max(4);
      library_len.min(cap) as u16
    }
  }
  .min(chunk.height);
  (
    Rect::new(chunk.x, chunk.y, chunk.width, lib_h),
    Rect::new(chunk.x, chunk.y + lib_h, chunk.width, chunk.height - lib_h),
  )
}

pub fn library_handle_rect(app: &App, chunk: Rect) -> Rect {
  let (lib, _) = library_playlists_split(app, chunk);
  Rect::new(
    chunk.x,
    lib.y + lib.height.saturating_sub(1),
    chunk.width,
    1,
  )
}

// Combined sidebar: one outer block with a horizontal separator inside.
// Returns (library_section, separator, playlist_section) all inside the
// outer border (inset by 1). Library height follows the same cap/override
// as library_playlists_split but on the inner height.
pub fn sidebar_combined_split(app: &App, chunk: Rect) -> (Rect, Rect, Rect) {
  let inner = Rect::new(
    chunk.x + 1,
    chunk.y + 1,
    chunk.width.saturating_sub(2),
    chunk.height.saturating_sub(2),
  );
  if inner.height == 0 || inner.width == 0 {
    return (inner, Rect::default(), Rect::default());
  }
  // Reserve 1 row for the separator.
  let for_lib = Rect::new(
    inner.x,
    inner.y,
    inner.width,
    inner.height.saturating_sub(1),
  );
  let (tmp_lib, _) = library_playlists_split(app, for_lib);
  let lib_h = tmp_lib.height.min(inner.height.saturating_sub(1));
  let library = Rect::new(inner.x, inner.y, inner.width, lib_h);
  let separator = Rect::new(inner.x, library.y + library.height, inner.width, 1);
  let playlist_h = inner.height.saturating_sub(lib_h + 1);
  let playlist = Rect::new(inner.x, separator.y + 1, inner.width, playlist_h);
  (library, separator, playlist)
}

pub fn sidebar_combined_separator_rect(app: &App, chunk: Rect) -> Rect {
  let (_, sep, _) = sidebar_combined_split(app, chunk);
  sep
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

/// Count/viewport/offset triple shared by every scrollable list. The
/// viewport is the visible height minus the list's own chrome (borders,
/// tabs, filter rows); `index`/`view_mode` still vary per list, so callers
/// override them with struct-update syntax.
fn scroll_parts(count: usize, height: u16, pad: u16, offset: usize) -> ListScroll {
  ListScroll {
    count,
    viewport: height.saturating_sub(pad) as usize,
    view_mode: false,
    index: 0,
    offset,
  }
}

/// Single source of truth for (count, viewport, index/offset) of every
/// scrollable list. `rect` is where the list is drawn; the viewport derives
/// from it via [`scroll_parts`] (tables reserve a header row + selection
/// rows, selectable lists only the title).
pub fn list_scroll(app: &App, block: ActiveBlock, rect: Rect) -> Option<ListScroll> {
  let selection = |index: usize, count: usize| ListScroll {
    index,
    ..scroll_parts(count, rect.height, 5, 0)
  };
  Some(match block {
    ActiveBlock::HelpMenu => ListScroll {
      view_mode: true,
      ..scroll_parts(
        app.help_docs_size as usize,
        rect.height,
        3,
        app.help_scroll_offset as usize,
      )
    },
    ActiveBlock::RequestLog => ListScroll {
      index: app.request_log_index.unwrap_or(0),
      ..scroll_parts(app.request_log.len(), rect.height, 2, 0)
    },
    ActiveBlock::Library => ListScroll {
      index: app.library.selected_index,
      ..scroll_parts(
        visible_library_options(&app.hidden_library_sections).len(),
        rect.height,
        2,
        0,
      )
    },
    ActiveBlock::MyPlaylists => ListScroll {
      index: app.selected_playlist_index.unwrap_or(0),
      ..scroll_parts(
        app
          .playlists
          .as_ref()
          .map(|playlists| playlists.items.len())
          .unwrap_or(0),
        rect.height,
        2,
        0,
      )
    },
    ActiveBlock::TrackTable => ListScroll {
      // The load-more row is part of the drawn table, so it scrolls too.
      view_mode: true,
      ..scroll_parts(
        app.track_table.tracks.len() + usize::from(app.track_table_has_more()),
        rect.height,
        3,
        app.track_table.scroll_offset,
      )
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
      selection(
        app.recently_played.index,
        super::group_consecutive_plays(&recently_played.items).len(),
      )
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
    ActiveBlock::MadeForYou => selection(app.made_for_you_index, app.made_for_you_len()),
    ActiveBlock::SearchResultBlock => {
      let tracks = app.search_results.tracks.as_ref()?;
      selection(
        app.search_results.selected_tracks_index.unwrap_or(0),
        tracks.items.len() + usize::from(app.search_block_has_more(&SearchResultBlock::SongSearch)),
      )
    }
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
// Resume Last Listening: the single centered button shown in the playbar
// when nothing is playing. Shared by the drawer and the mouse hit-test.
pub fn playbar_resume_label() -> &'static str {
  "\u{25b6} Resume"
}

pub fn playbar_resume_rect(playbar: Rect) -> Rect {
  let label = playbar_resume_label();
  let w = label.width() as u16;
  Rect::new(
    playbar.x + playbar.width.saturating_sub(w) / 2,
    playbar.y + playbar.height / 2,
    w,
    1,
  )
}

// The manual suspend/resume button, drawn in the playbar title row just
// right of the [ ] window toggle (separated by ──). Shared by the drawer and
// the mouse hit-test so the hit zone follows the drawn label exactly.
pub fn playbar_processes_label(suspended: bool) -> &'static str {
  if suspended {
    "Resume Processes"
  } else {
    "Suspend Processes"
  }
}

// X range [start, end) of the processes button on the title row.
pub fn playbar_processes_range(playbar: Rect, device_name: &str, suspended: bool) -> (u16, u16) {
  let title = build_playbar_title(
    "",
    device_name,
    playbar_title_max_width(playbar.width, suspended),
  );
  let start = playbar.x + 1 + title.width() as u16 + 2; // ── separator after [ ]
  (
    start,
    start + playbar_processes_label(suspended).width() as u16,
  )
}

pub fn playbar_controls_row(playbar: Rect) -> Rect {
  Rect::new(
    playbar.x + 1,
    playbar_content_top_y(playbar),
    playbar.width.saturating_sub(2),
    1,
  )
}

// x where the transport buttons text starts — the button group
// dead-centered on the progress bar midpoint, so the 5 symmetric transport
// buttons always sit dead-center above the music bar regardless of Smart
// Shuffle or repeat mode. Shared by drawer and mouse hit-test.
pub fn playbar_controls_x(playbar: Rect, controls: &[(PlaybarButton, String)]) -> u16 {
  let row = playbar_controls_row(playbar);
  let group: u16 = controls.iter().map(|(_, t)| t.width() as u16 + 1).sum();
  let center = playbar_progress_rect(playbar)
    .map(|b| b.x + b.width / 2)
    .unwrap_or_else(|| row.x + row.width / 2);
  center.saturating_sub(group / 2).max(row.x)
}

// The five transport buttons with their glyphs, left to right: shuffle,
// prev, play/pause, next, repeat. Every button is 2 cols (" <glyph>") + 1
// trailing space = 15 cols total, keeping PlayPause dead-center. The
// repeat mode word renders in the right margin so the group never shifts.
pub fn build_playbar_controls(is_playing: bool) -> Vec<(PlaybarButton, String)> {
  vec![
    (PlaybarButton::Shuffle, " ⇄".into()),
    (PlaybarButton::Prev, " «".into()),
    (
      PlaybarButton::PlayPause,
      if is_playing {
        " ‖".into()
      } else {
        " ▶".into()
      },
    ),
    (PlaybarButton::Next, " »".into()),
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
// toggle (see handle_playbar_click). ASCII so every terminal renders it.
// The device name is truncated to `max_width` so the trailing [ ] is never
// cut off by block-title truncation (drawn and hit-tested from this same
// string, so both always agree).
pub fn build_playbar_title(_play_title: &str, device_name: &str, max_width: usize) -> String {
  let fixed = UnicodeWidthStr::width("Device-()──[ ]");
  let device = super::widgets::ellipsize(device_name, max_width.saturating_sub(fixed));
  format!("{:-7}({})──[ ]", "Device", device)
}

// Title-row budget for build_playbar_title: block borders, the ── separator
// and the trailing processes label all eat into the available width.
pub fn playbar_title_max_width(playbar_width: u16, suspended: bool) -> usize {
  (playbar_width.saturating_sub(2) as usize)
    .saturating_sub(2)
    .saturating_sub(UnicodeWidthStr::width(playbar_processes_label(suspended)))
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
/// Visibility flags for song-table columns. Bundles seven `bool` params
/// so `song_table_columns` stays under clippy's argument limit.
pub struct SongColumnFlags {
  pub show_album: bool,
  pub show_artist: bool,
  pub show_length: bool,
  pub show_date_added: bool,
  pub show_remove: bool,
  pub show_in_playlist: bool,
}

impl SongColumnFlags {
  /// Flags for a song table. `with_date` is context (only playlist tables
  /// carry dates); the date column shows only when the context has dates
  /// *and* the gear menu enables it, so both fold into `show_date_added`.
  pub fn for_view(
    behavior: &BehaviorConfig,
    with_date: bool,
    show_remove: bool,
    show_in_playlist: bool,
  ) -> Self {
    Self {
      show_album: behavior.show_album_column,
      show_artist: behavior.show_artist_column,
      show_length: behavior.show_length_column,
      show_date_added: with_date && behavior.show_date_added_column,
      show_remove,
      show_in_playlist,
    }
  }
}

pub fn song_table_columns(width: u16, flags: SongColumnFlags) -> Vec<(ColumnId, u16, u16)> {
  // Artist keeps its 30% share; DateAdded and Length are pinned to the
  // right. Title and Album split whatever is left over, with ~40% of the
  // flex going to Album so hiding a column widens the song names instead of
  // leaving a gap.
  let artist = if flags.show_artist {
    get_percentage_width(width, 0.3)
  } else {
    0
  };
  let date = if flags.show_date_added {
    get_percentage_width(width, 0.15)
  } else {
    0
  };
  let length = if flags.show_length {
    get_percentage_width(width, 0.1)
  } else {
    0
  };
  let right = if flags.show_remove || flags.show_in_playlist {
    6
  } else {
    0
  };
  // Number column stays 4 wide (anchored). Title is GAP narrower than the
  // flex share so draw_table can pad its cell with leading spaces, pushing the
  // song names right without moving the track numbers.
  let flex = width
    .saturating_sub(4 + right + artist + date + length)
    .max(1);
  let album = if flags.show_album {
    (flex * 2 / 5).min(flex.saturating_sub(4)).max(1)
  } else {
    0
  };
  let title = flex.saturating_sub(album).max(1);

  let mut columns = vec![(ColumnId::Liked, 0, 4)];
  let mut x = 4;
  columns.push((ColumnId::Title, x, title));
  x += title;
  if flags.show_artist {
    columns.push((ColumnId::Artist, x, artist));
    x += artist;
  }
  if flags.show_album {
    columns.push((ColumnId::Album, x, album));
    x += album;
  }
  if flags.show_date_added {
    columns.push((ColumnId::DateAdded, x, date));
    x += date;
  }
  if flags.show_length {
    columns.push((ColumnId::Length, x, length));
    x += length;
  }
  if flags.show_remove || flags.show_in_playlist {
    columns.push((ColumnId::None, x, 6));
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

  fn behavior() -> BehaviorConfig {
    BehaviorConfig {
      seek_milliseconds: 5_000,
      volume_increment: 5,
      tick_rate_milliseconds: 250,
      enable_text_emphasis: false,
      seek_by_typing: false,
      enforce_wide_search_bar: false,
      liked_icon: String::new(),
      shuffle_icon: String::new(),
      repeat_track_icon: String::new(),
      repeat_context_icon: String::new(),
      playing_icon: String::new(),
      paused_icon: String::new(),
      set_window_title: false,
      enable_mouse: false,
      show_album_column: true,
      show_artist_column: true,
      show_length_column: true,
      show_date_added_column: false,
      enable_add_to_playlist: false,
      in_playlist_icon: String::new(),
      enable_remove_from_playlist: false,
      max_display_length: 0,
      enable_animations: false,
      playback_poll_secs: 5,
      lastfm_api_key: None,
    }
  }

  #[test]
  fn scrollbar_geometry_degenerate_rect() {
    // A scrollbar track with zero rows (rect height <= 2 in a tiny
    // terminal) must not panic on the thumb clamp.
    assert_eq!(scrollbar_geometry(0, 5, 3, 0), (0, 0));
    assert_eq!(scrollbar_geometry(0, 0, 0, 0), (0, 0));
  }

  #[test]
  fn sidebar_split_shrinks_in_minimized_mode() {
    let mut app = App::default();
    let chunk = Rect::new(0, 0, 200, 50);
    // Expanded: sidebar fits the longest label (15) + 2 borders + scrollbar.
    let (left, right) = sidebar_content_split(&app, chunk);
    assert_eq!(left.width, 18);
    assert_eq!(right.width, 182);
    // Minimized: only the playlist digit + 2 borders, no scrollbar column.
    app.sidebar_minimized = true;
    let (left, right) = sidebar_content_split(&app, chunk);
    assert_eq!(left.width, 3);
    assert_eq!(right.width, 197);
  }

  #[test]
  fn song_table_columns_number_column_is_four_wide() {
    // The leftmost column is the 4-wide number slot; the right-most None
    // column (✓ / ✕) appears only when remove or in-playlist is requested.
    let no_right = song_table_columns(
      120,
      SongColumnFlags::for_view(&behavior(), false, false, false),
    );
    let (id, x, w) = no_right[0];
    assert!(matches!(id, ColumnId::Liked));
    assert_eq!(x, 0);
    assert_eq!(w, 4);
    assert!(!no_right
      .iter()
      .any(|(id, _, _)| matches!(id, ColumnId::None)));

    let with_playlist = song_table_columns(
      120,
      SongColumnFlags::for_view(&behavior(), false, false, true),
    );
    let (id, x, w) = with_playlist[0];
    assert!(matches!(id, ColumnId::Liked));
    assert_eq!(x, 0);
    assert_eq!(w, 4);
    assert!(with_playlist
      .iter()
      .any(|(id, _, _)| matches!(id, ColumnId::None)));

    let with_remove = song_table_columns(
      120,
      SongColumnFlags::for_view(&behavior(), false, true, false),
    );
    assert!(with_remove
      .iter()
      .any(|(id, _, _)| matches!(id, ColumnId::None)));
  }

  #[test]
  fn date_column_needs_context_and_gear() {
    let mut gear = behavior();
    gear.show_date_added_column = true;
    assert!(SongColumnFlags::for_view(&gear, true, false, false).show_date_added);
    assert!(!SongColumnFlags::for_view(&gear, false, false, false).show_date_added);
    assert!(!SongColumnFlags::for_view(&behavior(), true, false, false).show_date_added);
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
  fn format_playlist_duration_test() {
    assert_eq!(format_playlist_duration(0), "0m");
    assert_eq!(format_playlist_duration(29_999), "0m");
    assert_eq!(format_playlist_duration(30_000), "1m");
    assert_eq!(format_playlist_duration(60_000), "1m");
    assert_eq!(format_playlist_duration(3_599_000), "1h");
    assert_eq!(format_playlist_duration(7_200_000), "2h");
    assert_eq!(format_playlist_duration(8_100_000), "2h 15m");
    assert_eq!(format_playlist_duration(21_570_000), "6h");
    assert_eq!(format_playlist_duration(21_569_999), "5h 59m");
    assert_eq!(format_playlist_duration(21_570_001), "6h");
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
    let app = App::default();
    let rect = gear_click_rect(&app, Rect::new(0, 0, 200, 5));
    assert_eq!((rect.x, rect.y, rect.width), (195, 2, 3));
    // A shifted header keeps the same relative geometry.
    let rect = gear_click_rect(&app, Rect::new(4, 0, 200, 3));
    assert_eq!((rect.x, rect.y), (199, 1));
  }

  #[test]
  fn playbar_content_rows_are_vertically_centered() {
    let playbar = Rect::new(0, 20, 120, PLAYBAR_HEIGHT);
    assert_eq!(playbar_song_row(playbar).y, 22);
    assert_eq!(playbar_controls_row(playbar).y, 22);
    assert_eq!(playbar_artist_row(playbar).y, 23);
    assert_eq!(playbar_volume_rect(playbar).y, 23);
    assert_eq!(playbar_bar_row(playbar).y, 24);
  }
}
