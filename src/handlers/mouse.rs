use super::handle_app;
use crate::app::{
  visible_library_options, ActiveBlock, AlbumTableContext, App, ArtistBlock, RouteId,
  SearchResultBlock, TrackSortColumn, TrackTableContext,
};
use crate::backend::IoEvent;
use crate::event::Key;
use crate::tui::layout;
use crate::tui::layout::{
  build_playbar_controls, build_playbar_title, gear_click_rect, header_height, playbar_artist_row,
  playbar_controls_x, playbar_progress_rect, playbar_song_row, playbar_text_click_range,
  playbar_volume_rect, search_box_rect, settings_section_rect, shortcuts_table_rect,
  song_table_columns, song_table_viewport, sort_column_for, track_table_with_date, PlaybarButton,
  PLAYBAR_HEIGHT, PLAYBAR_TIME_LEN, SETTINGS_ROW_COUNT, SMALL_TERMINAL_HEIGHT, VOLUME_BAR_LEN,
  VOLUME_LABEL_LEN,
};
use crate::tui::ColumnId;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use rspotify::model::PlayableItem;
use rspotify::prelude::Id;
use unicode_width::UnicodeWidthStr;

thread_local! {
  // Active scrollbar thumb drag: where the grab started and the geometry to
  // map the cursor back to a list offset while dragging.
  static SCROLLBAR_DRAG: std::cell::RefCell<Option<ScrollbarDrag>> = const { std::cell::RefCell::new(None) };
  // Active playbar scrub drag: music or volume bar, mapping cursor x back to
  // a value while dragging.
  static PLAYBAR_DRAG: std::cell::RefCell<Option<PlaybarDrag>> = const { std::cell::RefCell::new(None) };
  static SIDEBAR_DRAG: std::cell::RefCell<Option<SidebarDrag>> = const { std::cell::RefCell::new(None) };
  static LIBRARY_DRAG: std::cell::RefCell<Option<SidebarDrag>> = const { std::cell::RefCell::new(None) };
}

// Active scrollbar thumb drag state (thread-local, see SCROLLBAR_DRAG).
#[derive(Clone)]
struct ScrollbarDrag {
  right: Rect,
  block: ActiveBlock,
  count: usize,
  viewport: usize,
  grab_offset: usize,
}

#[derive(Clone)]
struct SidebarDrag {
  start_x: u16,
  start_width: u16,
}

 // Active music/volume bar scrub (thread-local, see PLAYBAR_DRAG). `dragged`
 // becomes true after the first Drag tick, so a plain click commits nothing.
 #[derive(Clone)]
 struct PlaybarDrag {
  music: bool,
  bar_x: u16,
  bar_w: u16,
  duration_ms: u32,
  dragged: bool,
}

pub fn handle_mouse(mouse: MouseEvent, app: &mut App) {
  match mouse.kind {
    MouseEventKind::ScrollUp => {
      handle_wheel(true, mouse, app);
    }
    MouseEventKind::ScrollDown => {
      handle_wheel(false, mouse, app);
    }
    MouseEventKind::Down(MouseButton::Left) => {
      let (x, y) = (mouse.column, mouse.row);
      if x >= app.size.width || y >= app.size.height {
        return;
      }
      if handle_library_drag_down(x, y, app) {
        return;
      }
      if handle_sidebar_drag_down(x, y, app) {
        return;
      }
      // Press on the scrollbar thumb starts a drag
      if handle_scrollbar_down(x, y, app) {
        return;
      }
      // One click = select + activate (Enter). No double-click needed.
      if handle_left_click(x, y, app) {
        handle_app(Key::Enter, app);
      }
    }
    MouseEventKind::Drag(MouseButton::Left) => {
      if handle_library_drag(mouse.row, app) {
        return;
      }
      if handle_sidebar_drag(mouse.column, app) {
        return;
      }
      PLAYBAR_DRAG.with(|d| {
        if let Some(drag) = d.borrow_mut().as_mut() {
          drag.dragged = true;
          let (x, _y) = (mouse.column, mouse.row);
          if drag.music {
            let clamped_x = (x as f32).clamp(
              drag.bar_x as f32,
              (drag.bar_x + drag.bar_w.saturating_sub(1)) as f32,
            );
            // The last cell maps to the full duration (same as click-to-seek),
            // so a drag to the right edge reaches the end of the track.
            let fraction = if clamped_x == (drag.bar_x + drag.bar_w.saturating_sub(1)) as f32 {
              1.0
            } else {
              (clamped_x - drag.bar_x as f32) / drag.bar_w as f32
            };
            app.preview_seek((drag.duration_ms as f32 * fraction) as u32);
          } else {
            let percent =
              (((x as i32 - drag.bar_x as i32) * 100) / drag.bar_w as i32).clamp(0, 100);
            // Last cell = 100% (mirrors the music-bar end snap): dragging to
            // the right edge reaches full volume instead of 98%.
            let percent = if x >= drag.bar_x + drag.bar_w.saturating_sub(1) {
              100
            } else {
              percent
            };
            app.volume_preview = Some(percent as u8);
          }
        }
      });
      handle_scrollbar_drag(mouse.row, app);
    }
    MouseEventKind::Moved => {
      handle_hover(mouse.column, mouse.row, app);
    }
    MouseEventKind::Up(MouseButton::Left) => {
      let mut need_save = false;
      if SIDEBAR_DRAG.with(|d| d.borrow_mut().take().is_some()) {
        need_save = true;
      }
      if LIBRARY_DRAG.with(|d| d.borrow_mut().take().is_some()) {
        need_save = true;
      }
      if need_save {
        app.dispatch(IoEvent::SaveState);
      }
      SCROLLBAR_DRAG.with(|d| *d.borrow_mut() = None);
      // Commit a scrubbed seek/volume once on release (immediate jump on
      // press already dispatched; only a real drag adds the final commit).
      PLAYBAR_DRAG.with(|d| {
        if let Some(drag) = d.borrow_mut().take() {
          if drag.dragged {
            if drag.music {
              if let Some(ms) = app.seek_ms {
                app.seek_to(ms as u32);
              }
            } else {
              if let Some(pct) = app.volume_preview.take() {
                app.dispatch(IoEvent::ChangeVolume(pct));
              }
            }
          } else {
            app.volume_preview = None;
          }
        }
      });
    }
    _ => {}
  }
}

// Returns true when the click landed on a row of a list (so a double-click
// can fire Enter on it). Never true for the playbar, header or empty space.
fn handle_left_click(x: u16, y: u16, app: &mut App) -> bool {
  if app.dialog.is_some() {
    return false;
  }

  // The playbar is drawn on every route (music view included) and its title
  // row carries the fullscreen toggle, so it must win over route dispatch.
  let Some((_, playbar, _)) = main_layout(app) else {
    return false;
  };
  if y >= playbar.y && y < playbar.y + playbar.height {
    handle_playbar_click(x, y, playbar, app);
    return false;
  }

  match app.get_current_route().active_block {
    ActiveBlock::HelpMenu => {
      handle_settings_click(x, y, app);
      return false;
    }
    ActiveBlock::Error | ActiveBlock::SelectDevice => return false,
    ActiveBlock::MusicView => {
      if let Some(PlayableItem::Track(track)) = app
        .current_playback_context
        .as_ref()
        .map(|c| &c.item)
        .unwrap_or(&None)
      {
        let panel = Layout::default()
          .direction(Direction::Horizontal)
          .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
          .split(Rect::new(0, 0, app.size.width, app.size.height))[1];
        if x >= panel.x && x < panel.x + panel.width && y >= panel.y && y < panel.y + panel.height {
          if let Some(artist) = track.artists.first() {
            if let Some(id) = &artist.id {
              app.get_artist(id.id().to_string(), artist.name.clone());
              app.push_navigation_stack(RouteId::Artist, ActiveBlock::ArtistBlock);
            }
          }
        }
      }
      return false;
    }
    _ => {}
  }

  let Some((routes, _, input_box)) = main_layout(app) else {
    return false;
  };

  if let Some(input_box) = input_box {
    if y >= input_box.y && y < input_box.y + input_box.height {
      // Leaving the sidebar for the search box or settings clears the
      // sidebar highlight latch.
      app.sidebar_latched_block = None;
      let search_box = search_box_rect(app, input_box);
      if x >= search_box.x && x < search_box.x + search_box.width {
        // Clicking the clear button wipes the search input.
        if x >= search_box.x + search_box.width.saturating_sub(4) && !app.input.is_empty() {
          app.input = vec![];
          app.input_idx = 0;
          app.input_cursor_position = 0;
        }
        app.set_current_route_state(Some(ActiveBlock::Input), Some(ActiveBlock::Input));
        return false;
      }
      let gear_box = gear_click_rect(app, input_box);
      if x >= gear_box.x && x < gear_box.x + gear_box.width && y == gear_box.y {
        app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
        return false;
      }
      app.set_current_route_state(Some(ActiveBlock::Input), Some(ActiveBlock::Input));
      return false;
    }
  }

  // The sidebar is auto-sized to its content by the drawer; clicks must be
  // routed with that same geometry, or a strip of the song table (between
  // the drawn sidebar and the old fixed 20% mark) selects library/playlists
  // instead of the row under the cursor.
  let (left, right) = layout::sidebar_content_split(app, routes);

  if x < left.x + left.width {
    handle_user_block_click(x, y, left, app)
  } else if app.dev_view && x >= dev_panel_rect(right).x {
    // The dev request-log panel overlays the table; only the title row is
    // clickable and it clears the log.
    let panel = dev_panel_rect(right);
    // The title row is below the 4-row throttle info header inside draw_request_log.
    let title_y = panel.y.saturating_add(4);
    if y == title_y {
      app.request_log.clear();
      app.request_log_index = None;
    }
    false
  } else {
    handle_content_click(x, y, right, app)
  }
}

// Maps a click in the '?' menu's settings section to its toggle row.
fn handle_settings_click(x: u16, y: u16, app: &mut App) {
  let area = Rect::new(0, 0, app.size.width, app.size.height);
  let settings = settings_section_rect(area);
  if x < settings.x
    || x >= settings.x + settings.width
    || y <= settings.y
    || y > settings.y + SETTINGS_ROW_COUNT
  {
    return;
  }
  app.toggle_setting((y - settings.y - 1) as usize);
}

// View-scroll one row per notch, clamped at the ends; the selection stays
// put, only the visible rows and the scrollbar thumb move. Used by the wheel
// over track lists and the gear menu.
fn scroll_view(up: bool, offset: &mut usize, count: usize, viewport: usize) {
  let max_offset = count.saturating_sub(viewport);
  if up {
    *offset = offset.saturating_sub(1);
  } else if *offset < max_offset {
    *offset += 1;
  }
}

fn handle_wheel(up: bool, mouse: MouseEvent, app: &mut App) {
  let (x, y) = (mouse.column, mouse.row);
  if x >= app.size.width || y >= app.size.height || app.dialog.is_some() {
    return;
  }
  match app.get_current_route().active_block {
    ActiveBlock::HelpMenu => {
      // Same as the track list: wheel scrolls the VIEW one row per notch,
      // clamped at the ends; the scrollbar thumb follows the offset.
      let shortcuts = shortcuts_table_rect(Rect::new(0, 0, app.size.width, app.size.height));
      if y < shortcuts.y {
        return;
      }
      let viewport = shortcuts.height.saturating_sub(3) as usize;
      let mut offset = app.help_scroll_offset as usize;
      scroll_view(up, &mut offset, app.help_docs_size as usize, viewport);
      app.help_scroll_offset = offset as u32;
      return;
    }
    ActiveBlock::Error | ActiveBlock::SelectDevice => return,
    _ => {}
  }

  let Some((routes, playbar, input_box)) = main_layout(app) else {
    return;
  };
  if let Some(input_box) = input_box {
    if y >= input_box.y && y < input_box.y + input_box.height {
      return;
    }
  }
  if y >= playbar.y && y < playbar.y + playbar.height {
    return;
  }

  let (left, right) = layout::sidebar_content_split(app, routes);
  // The expanded search block scrolls as a whole; the library sidebar is left
  // to the generic path below.
  if app.get_current_route().id == RouteId::Search && x >= left.x + left.width {
    handle_search_wheel(up, app);
    return;
  }
  let block = if x < left.x + left.width {
    user_block_at(y, left, app)
  } else if app.dev_view && x >= dev_panel_rect(right).x {
    Some(ActiveBlock::RequestLog)
  } else {
    content_block_at(app)
  };

  if block == Some(ActiveBlock::Input) {
    return;
  }

  if let Some(block) = block {
    app.set_current_route_state(Some(block), None);
  }

  // In the track list the wheel scrolls the VIEW (like a website): the
  // highlighted song stays put, only the visible rows and the scrollbar thumb
  // move. Keyboard/click still move the selection.
  if block == Some(ActiveBlock::TrackTable) {
    let viewport = song_table_viewport(app);
    let count = app.track_table.tracks.len() + usize::from(app.track_table_has_more());
    scroll_view(up, &mut app.track_table.scroll_offset, count, viewport);
    return;
  }

  // simple scroll: one row per notch, clamped at the list ends (no wrap-around loop)
  if let Some((index, count)) = scroll_bounds(app) {
    let at_top = index == 0;
    let at_bottom = count == 0 || index + 1 >= count;
    if (up && at_top) || (!up && at_bottom) {
      return;
    }
  }
  handle_app(if up { Key::Up } else { Key::Down }, app);
}

fn user_block_at(y: u16, chunk: Rect, app: &App) -> Option<ActiveBlock> {
  // The search header is global now, so the sidebar only holds library/playlists
  match (app.show_library, app.show_playlists) {
    (true, true) => {
      let (library, playlists) = crate::tui::layout::library_playlists_split(app, chunk);
      if y >= library.y && y < library.y + library.height {
        Some(ActiveBlock::Library)
      } else if y >= playlists.y && y < playlists.y + playlists.height {
        Some(ActiveBlock::MyPlaylists)
      } else {
        None
      }
    }
    (true, false) => Some(ActiveBlock::Library),
    (false, true) => Some(ActiveBlock::MyPlaylists),
    (false, false) => None,
  }
}

fn content_block_at(app: &App) -> Option<ActiveBlock> {
  match app.get_current_route().id {
    RouteId::TrackTable | RouteId::Recommendations => Some(ActiveBlock::TrackTable),
    RouteId::AlbumTracks => Some(ActiveBlock::AlbumTracks),
    RouteId::RecentlyPlayed => Some(ActiveBlock::RecentlyPlayed),
    RouteId::AlbumList => Some(ActiveBlock::AlbumList),
    RouteId::Artists => Some(ActiveBlock::Artists),
    RouteId::Podcasts => Some(ActiveBlock::Podcasts),
    RouteId::PodcastEpisodes => Some(ActiveBlock::EpisodeTable),
    RouteId::MadeForYou => Some(ActiveBlock::MadeForYou),
    RouteId::Search => Some(ActiveBlock::SearchResultBlock),
    _ => None,
  }
}

fn scroll_bounds(app: &App) -> Option<(usize, usize)> {
  let block = app.get_current_route().active_block;
  crate::tui::layout::list_scroll(app, block, Rect::default()).map(|s| (s.index, s.count))
}

fn main_layout(app: &App) -> Option<(Rect, Rect, Option<Rect>)> {
  let margin = if app.size.height > SMALL_TERMINAL_HEIGHT {
    1
  } else {
    0
  };
  // Header row is global now: [Search | title | Settings] on top
  let parent = Layout::default()
    .direction(Direction::Vertical)
    .constraints(
      [
        Constraint::Length(header_height(app)),
        Constraint::Min(1),
        Constraint::Length(PLAYBAR_HEIGHT),
      ]
      .as_ref(),
    )
    .margin(margin)
    .split(app.size);
  Some((parent[1], parent[2], Some(parent[0])))
}

// Pressing on the scrollbar thumb starts a drag; the thumb then follows the
// cursor until the button is released, like a normal website scrollbar.
// One arm for every scrollable list: gear menu, sidebar (library/playlists),
// the dev request-log panel and the right-hand lists. All geometry comes
// from `layout::list_scroll`, the same source the drawer uses.
fn handle_scrollbar_down(x: u16, y: u16, app: &mut App) -> bool {
  if app.dialog.is_some() {
    return false;
  }
  if app.get_current_route().active_block == ActiveBlock::HelpMenu {
    let shortcuts = shortcuts_table_rect(Rect::new(0, 0, app.size.width, app.size.height));
    return arm_scrollbar(x, y, app, ActiveBlock::HelpMenu, shortcuts);
  }
  let Some((routes, playbar, input_box)) = main_layout(app) else {
    return false;
  };
  if let Some(input_box) = input_box {
    if y >= input_box.y && y < input_box.y + input_box.height {
      return false;
    }
  }
  if y >= playbar.y && y < playbar.y + playbar.height {
    return false;
  }
  let (left, right) = layout::sidebar_content_split(app, routes);
  if x < left.x + left.width {
    if let Some(block) = user_block_at(y, left, app) {
      let (library, playlists) = crate::tui::layout::library_playlists_split(app, left);
      let rect = if block == ActiveBlock::Library {
        library
      } else {
        playlists
      };
      return arm_scrollbar(x, y, app, block, rect);
    }
    return false;
  }
  // The dev request-log panel overlays the right column's right quarter.
  if app.dev_view {
    let dev = dev_panel_rect(right);
    if x >= dev.x && x < dev.x + dev.width {
      return arm_scrollbar(x, y, app, ActiveBlock::RequestLog, dev);
    }
  }
  if x != right.x + right.width - 2 {
    return false;
  }
  let Some(block) = content_block_at(app) else {
    return false;
  };
  // The search songs table sits one row below `right` (the tab bar row), so
  // its scrollbar drag needs the list rect, not the content rect.
  if app.get_current_route().id == RouteId::Search {
    let (_, list_rect) = search_layout_parts(app, right);
    return arm_scrollbar(x, y, app, block, list_rect);
  }
  arm_scrollbar(x, y, app, block, right)
}

// Same 75/25 split draw_routes uses for the dev panel overlay.
fn dev_panel_rect(right: Rect) -> Rect {
  Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Percentage(75), Constraint::Percentage(25)].as_ref())
    .split(right)[1]
}

fn arm_scrollbar(x: u16, y: u16, app: &mut App, block: ActiveBlock, rect: Rect) -> bool {
  let Some(scroll) = crate::tui::layout::list_scroll(app, block, rect) else {
    return false;
  };
  if x != rect.x + rect.width - 2 || scroll.count <= scroll.viewport || rect.height <= 2 {
    return false;
  }
  app.set_current_route_state(Some(block), None);
  // The thumb position the drawer renders, so the grabbable thumb is the
  // drawn thumb.
  let thumb_offset = scroll.thumb_offset();
  let (top, thumb_top, thumb_len) =
    thumb_geometry(rect, scroll.count, scroll.viewport, thumb_offset);
  let thumb_top = top + thumb_top;
  // Pressing anywhere on the scrollbar column starts a drag; the thumb jumps
  // to the cursor. Big lists draw a thumb only a couple of rows tall, so a
  // strict thumb-only hit test would make the scrollbar practically grabbable.
  let grab_offset = if y >= thumb_top && y < thumb_top + thumb_len {
    (y - thumb_top) as usize
  } else {
    (y as i32 - thumb_top as i32).clamp(0, thumb_len as i32 - 1) as usize
  };
  SCROLLBAR_DRAG.with(|d| {
    *d.borrow_mut() = Some(ScrollbarDrag {
      right: rect,
      block,
      count: scroll.count,
      viewport: scroll.viewport,
      grab_offset,
    })
  });
  true
}

fn handle_scrollbar_drag(y: u16, app: &mut App) {
  let drag = SCROLLBAR_DRAG.with(|d| d.borrow().clone());
  let Some(drag) = drag else {
    return;
  };
  let track_h = drag.right.height.saturating_sub(2) as usize;
  let max_offset = drag.count - drag.viewport;
  let (_, thumb_len) =
    crate::tui::layout::scrollbar_geometry(track_h, drag.count, drag.viewport, 0);
  let travel = track_h - thumb_len;
  // Where the thumb's top edge should sit, keeping the grab offset
  let y_thumb_top = y as i32 - drag.grab_offset as i32 - drag.right.y as i32 - 1;
  let y_thumb_top = y_thumb_top.clamp(0, (track_h as i32) - 1) as usize;
  let offset = (y_thumb_top * max_offset / travel.max(1)).min(max_offset);
  if drag.block == ActiveBlock::TrackTable {
    app.track_table.scroll_offset = offset;
  } else if drag.block == ActiveBlock::HelpMenu {
    app.help_scroll_offset = offset as u32;
  } else {
    set_selected(
      app,
      drag.block,
      (offset + drag.viewport).min(drag.count - 1),
    );
  }
}

fn handle_sidebar_drag_down(x: u16, y: u16, app: &mut App) -> bool {
  if !app.user_config.behavior.enable_animations {
    return false;
  }
  let Some((routes, _, _)) = main_layout(app) else {
    return false;
  };
  let handle = layout::sidebar_handle_rect(app, routes);
  // 2-col grab zone (handle + first content col) so the 1-col border is not a pixel hunt;
  // we avoid handle-1 which is the scrollbar column (width-2)
  if (x == handle.x || x == handle.x + 1) && y >= handle.y && y < handle.y + handle.height {
    let w = layout::sidebar_width(app, routes);
    SIDEBAR_DRAG.with(|d| {
      *d.borrow_mut() = Some(SidebarDrag {
        start_x: handle.x,
        start_width: w,
      })
    });
    // Dragging from minimized should immediately expand
    if app.sidebar_minimized {
      app.sidebar_minimized = false;
    }
    return true;
  }
  false
}

fn handle_sidebar_drag(x: u16, app: &mut App) -> bool {
  if !app.user_config.behavior.enable_animations {
    return false;
  }
  let drag = SIDEBAR_DRAG.with(|d| d.borrow().clone());
  let Some(drag) = drag else {
    return false;
  };
  let Some((routes, _, _)) = main_layout(app) else {
    return false;
  };
  let delta = x as i16 - drag.start_x as i16;
  let mut new_w = (drag.start_width as i16 + delta).clamp(
    6,
    (routes.width / 2) as i16,
  ) as u16;
  // Snap to minimized when dragged past the minimum interactive width
  if new_w <= 10 {
    if !app.sidebar_minimized {
      app.sidebar_minimized = true;
      // keep the last expanded width for restore on next expand
    }
    return true;
  }
  if app.sidebar_minimized && new_w > 10 {
    app.sidebar_minimized = false;
  }
  new_w = new_w.clamp(layout::SIDEBAR_MIN_WIDTH, routes.width / 2);
  app.sidebar_width_override = Some(new_w);
  true
}

fn handle_library_drag_down(x: u16, y: u16, app: &mut App) -> bool {
  if !app.user_config.behavior.enable_animations {
    return false;
  }
  let Some((routes, _, _)) = main_layout(app) else {
    return false;
  };
  let (left, _) = layout::sidebar_content_split(app, routes);
  if x < left.x || x >= left.x + left.width {
    return false;
  }
  let handle = layout::library_handle_rect(app, left);
  if y == handle.y && x >= handle.x && x < handle.x + handle.width {
    LIBRARY_DRAG.with(|d| {
      *d.borrow_mut() = Some(SidebarDrag {
        start_x: y,
        start_width: handle.y,
      })
    });
    return true;
  }
  false
}

fn handle_library_drag(y: u16, app: &mut App) -> bool {
  if !app.user_config.behavior.enable_animations {
    return false;
  }
  let drag = LIBRARY_DRAG.with(|d| d.borrow().clone());
  let Some(drag) = drag else {
    return false;
  };
  let Some((routes, _, _)) = main_layout(app) else {
    return false;
  };
  let (left, _) = layout::sidebar_content_split(app, routes);
  let delta = y as i16 - drag.start_x as i16;
  let new_h = (drag.start_width as i16 + delta).clamp(4, left.height.saturating_sub(4) as i16) as u16;
  app.library_height_override = Some(new_h);
  true
}

fn handle_hover(x: u16, y: u16, app: &mut App) {
  if !app.user_config.behavior.enable_animations {
    app.hovered_library_index = None;
    app.hovered_playlist_index = None;
    app.hovered_list_index = None;
    return;
  }
  let Some((routes, _, _)) = main_layout(app) else {
    app.hovered_library_index = None;
    app.hovered_playlist_index = None;
    app.hovered_list_index = None;
    return;
  };
  let (left, right) = layout::sidebar_content_split(app, routes);
  // Left sidebar: Library / Playlists per-row hover
  if x >= left.x && x < left.x + left.width {
    app.hovered_list_index = None;
    let (lib_rect, pl_rect) = layout::library_playlists_split(app, left);
    if y == lib_rect.y {
      app.hovered_library_index = None;
      app.hovered_playlist_index = None;
      app.set_current_route_state(None, Some(ActiveBlock::Library));
      return;
    }
    if y == pl_rect.y {
      app.hovered_library_index = None;
      app.hovered_playlist_index = None;
      app.set_current_route_state(None, Some(ActiveBlock::MyPlaylists));
      return;
    }
    if y > lib_rect.y && y < lib_rect.y + lib_rect.height {
      let count = visible_library_options(&app.hidden_library_sections).len();
      if let Some(idx) = list_row_index(y, lib_rect, count, app.library.selected_index) {
        app.hovered_library_index = Some(idx);
        app.hovered_playlist_index = None;
        app.set_current_route_state(None, Some(ActiveBlock::Library));
        return;
      }
    }
    if y > pl_rect.y && y < pl_rect.y + pl_rect.height {
      let count = app.playlists.as_ref().map(|p| p.items.len()).unwrap_or(0);
      let sel = app.selected_playlist_index.unwrap_or(0);
      if let Some(idx) = list_row_index(y, pl_rect, count, sel) {
        app.hovered_playlist_index = Some(idx);
        app.hovered_library_index = None;
        app.set_current_route_state(None, Some(ActiveBlock::MyPlaylists));
        return;
      }
    }
    app.hovered_library_index = None;
    app.hovered_playlist_index = None;
    return;
  }
  // Content area: all song tables and lists get full-row hover bg
  app.hovered_library_index = None;
  app.hovered_playlist_index = None;
  if x < right.x || x >= right.x + right.width || y < right.y || y >= right.y + right.height {
    app.hovered_list_index = None;
    return;
  }
  // Route-specific hover mapping
  let hover = match app.get_current_route().id {
    RouteId::TrackTable | RouteId::Recommendations => {
      let count = app.track_table.tracks.len() + usize::from(app.track_table_has_more());
      let viewport = layout::song_table_viewport(app);
      let offset = app.track_table.scroll_offset.min(count.saturating_sub(viewport));
      if y < right.y + 2 {
        None
      } else {
        let row = (y - (right.y + 2)) as usize;
        let idx = offset + row;
        if idx < count { Some(idx) } else { None }
      }
    }
    RouteId::AlbumTracks => {
      let (count, sel) = match &app.album_table_context {
        AlbumTableContext::Simplified => app.selected_album_simplified.as_ref().map(|a| (a.tracks.items.len() + usize::from(a.tracks.items.len() < a.tracks.total as usize), a.selected_index)).unwrap_or((0,0)),
        AlbumTableContext::Full => (app.selected_album_full.as_ref().map(|a| a.album.tracks.items.len()).unwrap_or(0), app.saved_album_tracks_index),
      };
      table_row_index(y, right, count, sel)
    }
    RouteId::Search => {
      let (_tab_cells, list_rect) = search_layout_parts(app, right);
      if y == list_rect.y {
        None
      } else {
        let block = app.search_results.selected_block.clone();
        let (count, sel) = search_block_state(app, block.clone());
        let total = count + usize::from(app.search_block_has_more(&block));
        if block == SearchResultBlock::SongSearch {
          table_row_index(y, list_rect, total, sel)
        } else {
          list_row_index(y, list_rect, total, sel)
        }
      }
    }
    RouteId::Artist => {
      let Some(artist) = &app.artist else { app.hovered_list_index = None; return; };
      let shown = if artist.artist_selected_block == ArtistBlock::Empty { artist.artist_hovered_block } else { artist.artist_selected_block };
      let (count, sel) = match shown {
        ArtistBlock::TopTracks => (artist.top_tracks.len() + usize::from(artist.top_tracks_has_more), artist.selected_top_track_index),
        ArtistBlock::Albums => (artist.albums.items.len() + usize::from((artist.albums.items.len() as u32) < artist.albums.total), artist.selected_album_index),
        _ => (0,0),
      };
      let list_rect = Rect { x: right.x, y: right.y + 1, width: right.width, height: right.height.saturating_sub(1) };
      if shown == ArtistBlock::TopTracks {
        table_row_index(y, list_rect, count, sel)
      } else {
        list_row_index(y, list_rect, count, sel)
      }
    }
    _ => {
      if let Some(scroll) = layout::list_scroll(app, content_block_at(app).unwrap_or(ActiveBlock::TrackTable), right) {
        // generic fallback via list_scroll geometry
        let y_rel = y.saturating_sub(right.y + 1) as usize;
        let idx = scroll.offset + y_rel;
        if idx < scroll.count { Some(idx) } else { None }
      } else { None }
    }
  };
  app.hovered_list_index = hover;
  if hover.is_some() {
    if let Some(block) = content_block_at(app) {
      app.set_current_route_state(None, Some(block));
    }
  }
}

// Thumb geometry matching the render side (draw_table): the scrollbar track
// spans rows y+1..y+height-2 (track_h = height-2 rows) and the thumb is
// positioned proportionally to the scrolled offset, shared with the renderer.
// `offset` is already in view semantics (0 ..= count-viewport).
fn thumb_geometry(right: Rect, count: usize, viewport: usize, offset: usize) -> (u16, u16, u16) {
  let track_h = right.height.saturating_sub(2) as usize;
  let (thumb_top, thumb_len) =
    crate::tui::layout::scrollbar_geometry(track_h, count, viewport, offset);
  (right.y + 1, thumb_top as u16, thumb_len as u16)
}

fn set_selected(app: &mut App, block: ActiveBlock, index: usize) {
  app.selection_engaged = true;
  match block {
    ActiveBlock::TrackTable => app.track_table.selected_index = index,
    ActiveBlock::AlbumTracks => match app.album_table_context {
      AlbumTableContext::Simplified => {
        if let Some(album) = &mut app.selected_album_simplified {
          album.selected_index = index;
        }
      }
      AlbumTableContext::Full => app.saved_album_tracks_index = index,
    },
    ActiveBlock::RecentlyPlayed => app.recently_played.index = index,
    ActiveBlock::AlbumList => app.album_list_index = index,
    ActiveBlock::Artists => app.artists_list_index = index,
    ActiveBlock::Podcasts => app.shows_list_index = index,
    ActiveBlock::EpisodeTable => app.episode_list_index = index,
    ActiveBlock::MadeForYou => app.made_for_you_index = index,
    ActiveBlock::MyPlaylists => app.selected_playlist_index = Some(index),
    ActiveBlock::Library => app.library.selected_index = index,
    ActiveBlock::RequestLog => app.request_log_index = Some(index),
    _ => {}
  }
}

fn toggle_music_view(app: &mut App) {
  if app.get_current_route().id == RouteId::MusicView {
    // Pop restores the previous route and its active block, so the
    // dashboard is exactly as it was before the overlay opened.
    app.pop_navigation_stack();
  } else {
    app.get_panel_data();
    app.push_navigation_stack(RouteId::MusicView, ActiveBlock::MusicView);
  }
}

fn handle_playbar_click(x: u16, y: u16, playbar: Rect, app: &mut App) {
  // Title row: the trailing [ ] window toggles the music view fullscreen, any
  // other cell opens the device selector.
  if y == playbar.y {
    if let Some(ctx) = &app.current_playback_context {
      let title = build_playbar_title(
        if ctx.is_playing { "Playing" } else { "Paused" },
        &ctx.device.name,
      );
      let glyph_x = playbar.x + 1 + title.width() as u16;
      if x >= glyph_x.saturating_sub(3) && x < glyph_x {
        toggle_music_view(app);
        return;
      }
    }
    app.dispatch(IoEvent::GetDevices);
    return;
  }
  // Geometry is shared with `draw_playbar`: volume box at the top-right, the
  // progress bar centered on the song row.
  let Some(current_playback_context) = &app.current_playback_context else {
    return;
  };
  let Some(track_item) = &current_playback_context.item else {
    return;
  };
  let duration_ms = match track_item {
    PlayableItem::Track(track) => track.duration.num_milliseconds() as u32,
    PlayableItem::Episode(episode) => episode.duration.num_milliseconds() as u32,
    _ => 0,
  };

  let album_click = match track_item {
    PlayableItem::Track(track) => Some(track.album.clone()),
    _ => None,
  };

  if playbar.width > 70 {
    let volume_rect = playbar_volume_rect(playbar);
    if y == volume_rect.y && x >= volume_rect.x && x < volume_rect.x + volume_rect.width {
      // Skip the "♪ 100% " label; map only over the filled bar.
      let bar_x = volume_rect.x + VOLUME_LABEL_LEN;
      let percent = (((x as i32 - bar_x as i32) * 100) / VOLUME_BAR_LEN as i32).clamp(0, 100);
      app.dispatch(IoEvent::ChangeVolume(percent as u8));
      // Arm a scrub drag over the 17-cell volume bar so subsequent Drag
      // ticks preview locally and Up commits the final value.
      PLAYBAR_DRAG.with(|d| {
        *d.borrow_mut() = Some(PlaybarDrag {
          music: false,
          bar_x,
          bar_w: VOLUME_BAR_LEN,
          duration_ms: 0,
          dragged: false,
        });
      });
      return;
    }
  }

  // Transport buttons (shuffle, prev, play/pause, next, repeat)
  // dead-centered on the first inner row, just above the music bar.
  let controls = build_playbar_controls(
    current_playback_context.is_playing,
    app.smart_shuffle,
  );
  if y == playbar.y + 1 {
    let mut btn_x = playbar_controls_x(playbar, &controls);
    for (kind, text) in &controls {
      let w = text.width() as u16;
      if x >= btn_x && x < btn_x + w {
        match kind {
          PlaybarButton::Shuffle => app.shuffle(),
          PlaybarButton::Prev => app.dispatch(IoEvent::PreviousTrack),
          PlaybarButton::PlayPause => app.toggle_playback(),
          PlaybarButton::Next => app.dispatch(IoEvent::NextTrack),
          PlaybarButton::Repeat => app.repeat(),
        }
        return;
      }
      btn_x += w + 1;
    }
  }

  // Track name opens the album, artist name opens the artist page. Both
  // zones cover only the text as drawn (shared truncation limits), not the
  // whole row.
  if y == playbar_song_row(playbar).y {
    let (_item_id, name) = match track_item {
      PlayableItem::Track(track) => (
        track
          .id
          .as_ref()
          .map(|id| id.to_string())
          .unwrap_or_default(),
        track.name.clone(),
      ),
      PlayableItem::Episode(episode) => (episode.id.to_string(), episode.name.clone()),
      _ => (String::new(), String::new()),
    };
    let track_name = name;
    let (start, end) = playbar_text_click_range(
      playbar_song_row(playbar),
      playbar_controls_x(playbar, &controls),
      &track_name,
    );
    if x >= start && x < end {
      if let Some(album) = &album_click {
        app.track_table.context = Some(TrackTableContext::AlbumSearch);
        app.dispatch(IoEvent::GetAlbumTracks(Box::new(album.clone())));
        return;
      }
    }
  }
  if y == playbar_artist_row(playbar).y {
    if let Some(bar) = playbar_progress_rect(playbar) {
      if let PlayableItem::Track(track) = track_item {
        // Every artist name is its own click zone: a multi-artist track
        // ("Avicii, Nicky Romero") opens whichever name is clicked.
        let row = playbar_artist_row(playbar);
        let limit = (bar
          .x
          .saturating_sub(PLAYBAR_TIME_LEN)
          .saturating_sub(row.x + 1)) as usize;
        let mut offset = 0usize;
        for artist in &track.artists {
          let seg = artist.name.chars().count();
          if offset < limit {
            let start = row.x + offset as u16;
            if x >= start && x < (start + seg as u16).min(row.x + limit as u16) {
              if let Some(artist_id) = artist.id.as_ref().map(|id| id.id().to_string()) {
                app.get_artist(artist_id, artist.name.clone());
                app.push_navigation_stack(RouteId::Artist, ActiveBlock::ArtistBlock);
              }
              return;
            }
          }
          offset += seg + 2; // ", " separator
        }
      }
    }
  }

  // Click-to-seek on the progress bar (the left time label counts as position 0).
  if let Some(bar) = playbar_progress_rect(playbar) {
    if y == bar.y && x >= bar.x - PLAYBAR_TIME_LEN && x < bar.x + bar.width && duration_ms > 0 {
      let clamped_x = x.clamp(bar.x, bar.x + bar.width - 1);
      let fraction = (clamped_x - bar.x) as f32 / bar.width as f32;
      // The last cell jumps to the exact end: raw fractions cap out just
      // short of the final second and "0:03 - 0:03.57" style labels confuse.
      // A click anywhere on the final cell lands on the track end.
      let position_ms = if clamped_x == bar.x + bar.width - 1 {
        duration_ms
      } else {
        (duration_ms as f32 * fraction) as u32
      };
      app.seek_to(position_ms);
      // Arm a scrub drag over the music bar; Drag ticks preview the target
      // time live via seek_ms and Up commits the final position.
      PLAYBAR_DRAG.with(|d| {
        *d.borrow_mut() = Some(PlaybarDrag {
          music: true,
          bar_x: bar.x,
          bar_w: bar.width,
          duration_ms,
          dragged: false,
        });
      });
    }
  }
}

fn handle_user_block_click(_x: u16, y: u16, chunk: Rect, app: &mut App) -> bool {
  // The search header is global now, so the sidebar only holds library/playlists
  match (app.show_library, app.show_playlists) {
    (true, true) => {
      let (library, playlists) = layout::library_playlists_split(app, chunk);

      if y == library.y {
        app.sidebar_minimized = !app.sidebar_minimized;
        return true;
      }
      if y >= library.y && y < library.y + library.height {
        if let Some(index) = list_row_index(
          y,
          library,
          visible_library_options(&app.hidden_library_sections).len(),
          app.library.selected_index,
        ) {
          app.library.selected_index = index;
          app.sidebar_latched_block = Some(ActiveBlock::Library);
          app.set_current_route_state(Some(ActiveBlock::Library), None);
          return true;
        }
      } else if y == playlists.y {
        app.sidebar_minimized = !app.sidebar_minimized;
        return true;
      } else if y >= playlists.y && y < playlists.y + playlists.height {
        let count = app
          .playlists
          .as_ref()
          .map(|playlists| playlists.items.len())
          .unwrap_or(0);
        let selected = app.selected_playlist_index.unwrap_or(0);
        if let Some(index) = list_row_index(y, playlists, count, selected) {
          app.selected_playlist_index = Some(index);
          app.sidebar_latched_block = Some(ActiveBlock::MyPlaylists);
          app.set_current_route_state(Some(ActiveBlock::MyPlaylists), None);
          return true;
        }
      }
    }
    (true, false) => {
      let count = visible_library_options(&app.hidden_library_sections).len();
      if y == chunk.y {
        app.sidebar_minimized = !app.sidebar_minimized;
        return true;
      }
      if y >= chunk.y && y < chunk.y + chunk.height {
        if let Some(index) = list_row_index(y, chunk, count, app.library.selected_index) {
          app.library.selected_index = index;
          app.sidebar_latched_block = Some(ActiveBlock::Library);
          app.set_current_route_state(Some(ActiveBlock::Library), None);
          return true;
        }
      }
    }
    (false, true) => {
      let count = app
        .playlists
        .as_ref()
        .map(|playlists| playlists.items.len())
        .unwrap_or(0);
      let selected = app.selected_playlist_index.unwrap_or(0);
      if y == chunk.y {
        app.sidebar_minimized = !app.sidebar_minimized;
        return true;
      }
      if y >= chunk.y && y < chunk.y + chunk.height {
        if let Some(index) = list_row_index(y, chunk, count, selected) {
          app.selected_playlist_index = Some(index);
          app.sidebar_latched_block = Some(ActiveBlock::MyPlaylists);
          app.set_current_route_state(Some(ActiveBlock::MyPlaylists), None);
          return true;
        }
      }
    }
    (false, false) => {}
  }
  false
}

fn handle_content_click(x: u16, y: u16, chunk: Rect, app: &mut App) -> bool {
  match app.get_current_route().id {
    RouteId::Search => return handle_search_click(x, y, chunk, app),
    RouteId::Artist => return handle_artist_click(x, y, chunk, app),
    RouteId::TrackTable | RouteId::Recommendations => {
      if y == chunk.y + 1 {
        handle_table_header_click(x, chunk, app);
        return false;
      }
      if y == chunk.y {
        // Title-row click: split into refresh zone (♻ glyph, leftmost) and
        // search zone (rest of the title row). The refresh glyph sits at
        // block position x+1 (2 chars wide), so x < chunk.x+3 = refresh.
        if x < chunk.x + 3 {
          match app.track_table.context {
            Some(TrackTableContext::SavedTracks) => {
              app.dispatch(IoEvent::RefreshSavedTracks);
            }
            Some(TrackTableContext::MyPlaylists) => {
              if let (Some(playlists), Some(index)) = (
                &app.playlists,
                &app.active_playlist_index.or(app.selected_playlist_index),
              ) {
                if let Some(selected_playlist) = playlists.items.get(*index) {
                  app.dispatch(IoEvent::RefreshPlaylistTracks(
                    selected_playlist.id.to_string(),
                  ));
                }
              }
            }
            _ => {}
          }
          return false;
        }
        // Search zone: focus the search box on playlist pages.
        if matches!(
          app.track_table.context,
          Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
        ) {
          app.playlist_filter = Some(String::new());
          app.set_current_route_state(Some(ActiveBlock::TrackTable), None);
          return false;
        }
        return false;
      }
      let has_more = app.track_table_has_more();
      let count = app.track_table.tracks.len() + usize::from(has_more);
      // The view is wheel-scrolled by scroll_offset (selection stays put), so
      // the clicked row must map through the RENDERED offset, not the
      // selection-derived one.
      let viewport = chunk.height.saturating_sub(3) as usize;
      let offset = app
        .track_table
        .scroll_offset
        .min(count.saturating_sub(viewport));
      let raw_index = offset + (y - (chunk.y + 2)) as usize;

      // When the in-playlist search filter is active, the rendered rows are a
      // subset of app.track_table.tracks. Map the display index back to the
      // original track index so the correct track plays / is removed.
      let (display_count, index) = if app.playlist_search_active() {
        let filtered: Vec<usize> = app
          .track_table
          .tracks
          .iter()
          .enumerate()
          .filter(|(_, t)| app.playlist_filter_matches(t))
          .map(|(i, _)| i)
          .collect();
        let display_count = filtered.len();
        if raw_index < display_count {
          (display_count, filtered[raw_index])
        } else {
          (display_count, raw_index)
        }
      } else {
        (count, raw_index)
      };

      if raw_index < display_count {
        // The remove ✕ column (rightmost 2 cells) only exists on playlist
        // pages when the feature flag is on; clicking it removes the track.
        let show_remove = app.user_config.behavior.enable_remove_from_playlist
          && matches!(
            app.track_table.context,
            Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
          );
        if show_remove
          && index < app.track_table.tracks.len()
          && x >= chunk.x + chunk.width.saturating_sub(2)
        {
          app.track_table.selected_index = index;
          app.set_current_route_state(Some(ActiveBlock::TrackTable), None);
          app.remove_selected_track_from_playlist();
          return false;
        }
        app.track_table.selected_index = index;
        // Clicking a track row while the in-playlist search is active must
        // dismiss the filter so the subsequent Enter can trigger playback.
        // (handle_app captures ALL keys when playlist_search_active.)
        app.playlist_filter = None;
        app.set_current_route_state(Some(ActiveBlock::TrackTable), None);
        if has_more && index == app.track_table.tracks.len() {
          app.load_more_tracks();
          // The load-more click already fetches; swallowing Enter prevents the
          // handler from re-fetching or falling through to playback.
          return false;
        }
        // By default a row click plays the song. Check for Artist/Album
        // columns first: clicking those navigates instead. Everything else
        // (Title, Liked, Length, Date Added, unknown) plays.
        if index < app.track_table.tracks.len() {
          let with_date =
            track_table_with_date(app.track_table.context.as_ref());
          let b = &app.user_config.behavior;
          let show_remove = b.enable_remove_from_playlist
            && matches!(
              app.track_table.context,
              Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
            );
          let columns = song_table_columns(
            chunk.width.saturating_sub(2),
            with_date,
            b.show_album_column,
            b.show_artist_column,
            b.show_length_column,
            b.show_date_added_column,
            show_remove,
            false,
          );
          if let Some(x_in) = x.checked_sub(chunk.x + 1) {
            if let Some((column, _, _)) = columns
              .iter()
              .find(|(_, col_x, col_width)| *col_x <= x_in && x_in < col_x + col_width)
            {
              let track = &app.track_table.tracks[index];
              match column {
                ColumnId::Artist => {
                  if let Some(artist) = track.artists.first() {
                    if let Some(artist_id) = artist.id.as_ref() {
                      app.get_artist(
                        artist_id.to_string(),
                        artist.name.clone(),
                      );
                      app.push_navigation_stack(
                        RouteId::Artist,
                        ActiveBlock::ArtistBlock,
                      );
                    }
                  }
                  return false;
                }
                ColumnId::Album => {
                  app.dispatch(IoEvent::GetAlbumTracks(
                    Box::new(track.album.clone()),
                  ));
                  return false;
                }
                _ => {} // Title, Liked, Length, DateAdded, None → play
              }
            }
          }
        }
        return true;
      }
    }
    RouteId::AlbumTracks => {
      let (count, selected) = match &app.album_table_context {
        AlbumTableContext::Simplified => app
          .selected_album_simplified
          .as_ref()
          .map(|album| {
            let has_more = album.tracks.items.len() < album.tracks.total as usize;
            (
              album.tracks.items.len() + usize::from(has_more),
              album.selected_index,
            )
          })
          .unwrap_or((0, 0)),
        AlbumTableContext::Full => app
          .selected_album_full
          .as_ref()
          .map(|album| (album.album.tracks.items.len(), app.saved_album_tracks_index))
          .unwrap_or((0, 0)),
      };
      if let Some(index) = table_row_index(y, chunk, count, selected) {
        let mut is_load_more = false;
        match app.album_table_context {
          AlbumTableContext::Simplified => {
            is_load_more = app
              .selected_album_simplified
              .as_ref()
              .map(|album| index == album.tracks.items.len())
              .unwrap_or(false);
            if is_load_more {
              app.load_more_album_tracks();
            }
            if let Some(album) = &mut app.selected_album_simplified {
              album.selected_index = index;
            }
          }
          AlbumTableContext::Full => app.saved_album_tracks_index = index,
        }
        app.set_current_route_state(Some(ActiveBlock::AlbumTracks), None);
        return !is_load_more;
      }
    }
    RouteId::RecentlyPlayed => {
      let count = app
        .recently_played
        .result
        .as_ref()
        .map(|recently_played| recently_played.items.len())
        .unwrap_or(0);
      // The load-more row is drawn after the items, so count it when mapping.
      let total_rows = count + usize::from(app.recently_played_has_more());
      if let Some(index) = table_row_index(y, chunk, total_rows, app.recently_played.index) {
        let is_load_more = app.recently_played_has_more() && index == count;
        if is_load_more {
          app.load_more_recently_played();
        }
        app.recently_played.index = index;
        app.set_current_route_state(Some(ActiveBlock::RecentlyPlayed), None);
        // Load-more clicks swallow Enter so the handler cannot fall through to
        // playback with an out-of-range index.
        return !is_load_more;
      }
    }
    RouteId::AlbumList => {
      if y == chunk.y {
        app.dispatch(IoEvent::RefreshSavedAlbums);
        return false;
      }
      let count = app
        .library
        .saved_albums
        .get_results(None)
        .map(|albums| albums.items.len())
        .unwrap_or(0);
      if let Some(index) = table_row_index(y, chunk, count, app.album_list_index) {
        app.album_list_index = index;
        app.set_current_route_state(Some(ActiveBlock::AlbumList), None);
        return true;
      }
    }
    RouteId::Artists => {
      let count = app.artists.len();
      if let Some(index) = table_row_index(y, chunk, count, app.artists_list_index) {
        app.artists_list_index = index;
        app.set_current_route_state(Some(ActiveBlock::Artists), None);
        return true;
      }
    }
    RouteId::Podcasts => {
      if y == chunk.y {
        app.dispatch(IoEvent::RefreshSavedShows);
        return false;
      }
      let count = app
        .library
        .saved_shows
        .get_results(None)
        .map(|shows| shows.items.len())
        .unwrap_or(0);
      if let Some(index) = table_row_index(y, chunk, count, app.shows_list_index) {
        app.shows_list_index = index;
        app.set_current_route_state(Some(ActiveBlock::Podcasts), None);
        return true;
      }
    }
    RouteId::PodcastEpisodes => {
      let count = app
        .library
        .show_episodes
        .get_results(None)
        .map(|episodes| episodes.items.len())
        .unwrap_or(0);
      if let Some(index) = table_row_index(y, chunk, count, app.episode_list_index) {
        app.episode_list_index = index;
        app.set_current_route_state(Some(ActiveBlock::EpisodeTable), None);
        return true;
      }
    }
    RouteId::MadeForYou => {
      // List widget (items start at chunk.y+1); clicking the trailing ✕
      // removes the playlist from For you.
      if let Some(index) =
        list_row_index(y, chunk, app.made_for_you_len(), app.made_for_you_index)
      {
        if x >= chunk.x + chunk.width.saturating_sub(2) {
          app.remove_pasted_playlist_from_for_you(index);
        } else {
          app.made_for_you_index = index;
          app.set_current_route_state(Some(ActiveBlock::MadeForYou), None);
        }
        return true;
      }
    }
    _ => {}
  }
  false
}

/// The search tab cells and the expanded block's list rect, in the same
/// layout the drawer uses (tab bar row on top, list below, load-more row at
/// the bottom of an expandable block).
fn search_layout_parts(app: &App, chunk: Rect) -> (Vec<(SearchResultBlock, Rect)>, Rect) {
  let expanded = app.search_results.selected_block.clone();
  let has_more = app.search_block_has_more(&expanded);
  let (_, tab_cells, list_rect) = layout::search_layout(chunk, expanded, has_more);
  (tab_cells, list_rect)
}

/// Count of items loaded for a search block and its selected index.
fn search_block_state(app: &App, block: SearchResultBlock) -> (usize, usize) {
  match block {
    SearchResultBlock::SongSearch => (
      app
        .search_results
        .tracks
        .as_ref()
        .map(|tracks| tracks.items.len())
        .unwrap_or(0),
      app.search_results.selected_tracks_index.unwrap_or(0),
    ),
    SearchResultBlock::ArtistSearch => (
      app
        .search_results
        .artists
        .as_ref()
        .map(|artists| artists.items.len())
        .unwrap_or(0),
      app.search_results.selected_artists_index.unwrap_or(0),
    ),
    SearchResultBlock::AlbumSearch => (
      app
        .search_results
        .albums
        .as_ref()
        .map(|albums| albums.items.len())
        .unwrap_or(0),
      app.search_results.selected_album_index.unwrap_or(0),
    ),
    SearchResultBlock::PlaylistSearch => (
      app
        .search_results
        .playlists
        .as_ref()
        .map(|playlists| playlists.items.len())
        .unwrap_or(0),
      app.search_results.selected_playlists_index.unwrap_or(0),
    ),
    SearchResultBlock::ShowSearch => (
      app
        .search_results
        .shows
        .as_ref()
        .map(|shows| shows.items.len())
        .unwrap_or(0),
      app.search_results.selected_shows_index.unwrap_or(0),
    ),
    SearchResultBlock::Empty => (0, 0),
  }
}

fn set_search_selected(app: &mut App, block: SearchResultBlock, index: usize) {
  match block {
    SearchResultBlock::SongSearch => app.search_results.selected_tracks_index = Some(index),
    SearchResultBlock::ArtistSearch => app.search_results.selected_artists_index = Some(index),
    SearchResultBlock::AlbumSearch => app.search_results.selected_album_index = Some(index),
    SearchResultBlock::PlaylistSearch => app.search_results.selected_playlists_index = Some(index),
    SearchResultBlock::ShowSearch => app.search_results.selected_shows_index = Some(index),
    SearchResultBlock::Empty => {}
  }
}

fn handle_search_click(x: u16, y: u16, chunk: Rect, app: &mut App) -> bool {
  let (tab_cells, list_rect) = search_layout_parts(app, chunk);

  // Tab click: expand (and lazily load) that block. Returns false so the
  // click-to-Enter chain is skipped: switching tabs must not play the first
  // song of the block (like the artist page tabs).
  if let Some((block, _)) = tab_cells
    .into_iter()
    .find(|(_, rect)| x >= rect.x && x < rect.x + rect.width && y == rect.y)
  {
    app.load_search_block(&block);
    app.search_results.selected_block = block;
    app.set_current_route_state(Some(ActiveBlock::SearchResultBlock), None);
    return false;
  }

  let block = app.search_results.selected_block.clone();
  if block == SearchResultBlock::Empty {
    return false;
  }

  let has_more = app.search_block_has_more(&block);
  let (count, selected) = search_block_state(app, block.clone());
  if count + usize::from(has_more) > 0 {
    // The tracks tab renders as a table (header row above the data rows),
    // the other tabs as plain lists.
    let index = if block == SearchResultBlock::SongSearch {
      table_row_index(y, list_rect, count + usize::from(has_more), selected)
    } else {
      list_row_index(y, list_rect, count + usize::from(has_more), selected)
    };
    if let Some(index) = index {
      if has_more && index == count {
        // The " Load more " row: fetch the next page and swallow Enter so the
        // handler cannot fall through to playback with an out-of-range index.
        // Anchor the selection on the row so the draw window keeps the button
        // visible for the next click.
        set_search_selected(app, block.clone(), count);
        app.load_more_search_block(&block);
        app.search_results.selected_block = block;
        app.set_current_route_state(Some(ActiveBlock::SearchResultBlock), None);
        return false;
      }
      set_search_selected(app, block.clone(), index);
      // Clicking an artist result opens the profile, like Enter does.
      if block == SearchResultBlock::ArtistSearch {
        if let Some(artist) = app
          .search_results
          .artists
          .as_ref()
          .and_then(|r| r.items.get(index))
        {
          app.get_artist(artist.id.to_string(), artist.name.clone());
          app.push_navigation_stack(RouteId::Artist, ActiveBlock::ArtistBlock);
          app.search_results.selected_block = block;
          app.set_current_route_state(Some(ActiveBlock::SearchResultBlock), None);
          return true;
        }
      }
      app.search_results.selected_block = block;
      app.set_current_route_state(Some(ActiveBlock::SearchResultBlock), None);
      return true;
    }
  }
  false
}

/// Wheel over the expanded search block moves its selection one row per
/// notch. No auto-load: paging is a deliberate button click.
fn handle_search_wheel(up: bool, app: &mut App) {
  let block = app.search_results.selected_block.clone();
  if block == SearchResultBlock::Empty {
    return;
  }
  let (count, selected) = search_block_state(app, block.clone());
  if count == 0 {
    return;
  }
  if up {
    if selected == 0 {
      return;
    }
    set_search_selected(app, block.clone(), selected - 1);
  } else {
    // The load-more row lives one past the last result (index == count), so
    // allow the selection to land on it when another page could arrive.
    let has_more = app.search_block_has_more(&block);
    if selected + 1 >= count + usize::from(has_more) {
      return;
    }
    set_search_selected(app, block.clone(), selected + 1);
  }
  app.search_results.selected_block = block;
  app.set_current_route_state(Some(ActiveBlock::SearchResultBlock), None);
}

fn handle_artist_click(x: u16, y: u16, chunk: Rect, app: &mut App) -> bool {
  if y == chunk.y {
    let cell_w = chunk.width / 2;
    let tab = if x < chunk.x + cell_w {
      ArtistBlock::TopTracks
    } else {
      ArtistBlock::Albums
    };
    app.artist_select_tab(tab);
    app.set_current_route_state(Some(ActiveBlock::ArtistBlock), None);
    // Return false: a tab click only switches tabs. The caller's
    // click-to-Enter chain must not fire here, or the freshly selected
    // top-tracks tab would start playing its first song.
    return false;
  }

  let list_rect = Rect {
    x: chunk.x,
    y: chunk.y + 1,
    width: chunk.width,
    height: chunk.height.saturating_sub(1),
  };

  let (shown, top_has_more, albums_has_more, count, selected) = {
    let Some(artist) = &app.artist else {
      return false;
    };
    let shown = if artist.artist_selected_block == ArtistBlock::Empty {
      artist.artist_hovered_block
    } else {
      artist.artist_selected_block
    };
    let top_has_more = artist.top_tracks_has_more;
    let albums_has_more = artist.albums.items.len() < artist.albums.total as usize;
    let (count, selected) = match shown {
      ArtistBlock::TopTracks => (
        artist.top_tracks.len() + usize::from(top_has_more),
        artist.selected_top_track_index,
      ),
      ArtistBlock::Albums => (
        artist.albums.items.len() + usize::from(albums_has_more),
        artist.selected_album_index,
      ),
      ArtistBlock::Empty => (0, 0),
    };
    (shown, top_has_more, albums_has_more, count, selected)
  };
  if shown == ArtistBlock::Empty {
    return false;
  }

  // The top-tracks tab renders a real table (header row at list_rect.y+1,
  // rows from list_rect.y+2); the albums tab keeps the plain list.
  let index = if shown == ArtistBlock::TopTracks {
    table_row_index(y, list_rect, count, selected)
  } else {
    list_row_index(y, list_rect, count, selected)
  };
  let Some(index) = index else {
    return false;
  };

  // Top-tracks rows are a table: clicking a name of an artist other than
  // the page artist, inside the Artist column, opens that artist.
  if shown == ArtistBlock::TopTracks {
    let nav = app.artist.as_ref().and_then(|artist| {
      let track = artist.top_tracks.get(index)?;
      let b = &app.user_config.behavior;
      let columns = song_table_columns(
        list_rect.width.saturating_sub(2),
        true,
        b.show_album_column,
        b.show_artist_column,
        b.show_length_column,
        b.show_date_added_column,
        false,
        true,
      );
      let (col_x, col_w) = columns
        .iter()
        .find(|(column, _, _)| *column == ColumnId::Artist)
        .map(|(_, x, w)| (list_rect.x + x, *w))?;
      if x < col_x || x >= col_x + col_w {
        return None;
      }
      let mut offset = 0usize;
      for a in &track.artists {
        let seg = a.name.chars().count() as u16;
        let start = col_x + offset as u16;
        if x >= start && x < start + seg.min(col_w) {
          if let (Some(aid), Some(page_id)) = (
            a.id.as_ref().map(|id| id.id().to_string()),
            Some(artist.artist_id.clone()),
          ) {
            if aid != page_id {
              return Some((aid, a.name.clone()));
            }
          }
          return None;
        }
        offset += seg as usize + 2; // ", " separator
      }
      None
    });
    if let Some((artist_id, artist_name)) = nav {
      app.get_artist(artist_id, artist_name);
      app.push_navigation_stack(RouteId::Artist, ActiveBlock::ArtistBlock);
      return true;
    }
  }

  if shown == ArtistBlock::TopTracks && top_has_more && index == count - 1 {
    app.load_more_artist_top_tracks();
    if let Some(artist) = &mut app.artist {
      artist.selected_top_track_index = index;
    }
    app.set_current_route_state(Some(ActiveBlock::ArtistBlock), None);
    return false;
  }

  if shown == ArtistBlock::Albums && albums_has_more && index == count - 1 {
    app.load_more_albums();
    if let Some(artist) = &mut app.artist {
      artist.selected_album_index = index;
    }
    app.set_current_route_state(Some(ActiveBlock::ArtistBlock), None);
    return false;
  }

  if let Some(artist) = &mut app.artist {
    match shown {
      ArtistBlock::TopTracks => artist.selected_top_track_index = index,
      ArtistBlock::Albums => artist.selected_album_index = index,
      ArtistBlock::Empty => return false,
    }
    app.set_current_route_state(Some(ActiveBlock::ArtistBlock), None);
    return true;
  }
  false
}

fn list_row_index(y: u16, chunk: Rect, count: usize, selected: usize) -> Option<usize> {
  if count == 0 || y < chunk.y + 1 || y >= chunk.y + chunk.height {
    return None;
  }
  let viewport = chunk.height.saturating_sub(2) as usize;
  // The drawn window is items[selected - viewport + 1 ..= selected] (ratatui
  // keeps the selection visible at the bottom row), so clicks must map back
  // through the same +1 offset.
  let offset = if selected >= viewport {
    selected - viewport + 1
  } else {
    0
  };
  let index = offset + (y - (chunk.y + 1)) as usize;
  if index < count {
    Some(index)
  } else {
    None
  }
}

fn table_row_index(y: u16, chunk: Rect, count: usize, selected: usize) -> Option<usize> {
  if count == 0 || y < chunk.y + 2 || y >= chunk.y + chunk.height {
    return None;
  }
  let visible = chunk.height.saturating_sub(3) as usize;
  // The drawn window is items[selected - visible + 1 ..= selected] (see
  // draw_table), so clicks must map back through the same +1 offset. With the
  // selection on the load-more row, the last visible row maps to index == count.
  let offset = if selected >= visible {
    selected - visible + 1
  } else {
    0
  };
  let index = offset + (y - (chunk.y + 2)) as usize;
  if index < count {
    Some(index)
  } else {
    None
  }
}

fn handle_table_header_click(x: u16, chunk: Rect, app: &mut App) {
  let with_date = track_table_with_date(app.track_table.context.as_ref());
  let b = &app.user_config.behavior;
  let show_remove = b.enable_remove_from_playlist
    && matches!(
      app.track_table.context,
      Some(TrackTableContext::MyPlaylists | TrackTableContext::PlaylistSearch)
    );
  let columns = song_table_columns(
    chunk.width.saturating_sub(2),
    with_date,
    b.show_album_column,
    b.show_artist_column,
    b.show_length_column,
    b.show_date_added_column,
    show_remove,
    false,
  );
  let Some(x_in) = x.checked_sub(chunk.x + 1) else {
    return;
  };
  let Some((column, _, _)) = columns
    .iter()
    .find(|(_, col_x, col_width)| *col_x <= x_in && x_in < col_x + col_width)
  else {
    return;
  };
  let Some(sort_column) = sort_column_for(*column) else {
    return;
  };
  let desc = match app.track_table_sort {
    Some((column, desc)) if column == sort_column => !desc,
    _ => sort_column == TrackSortColumn::DateAdded,
  };
  app.track_table_sort = Some((sort_column, desc));

  if sort_column == TrackSortColumn::DateAdded {
    // Date Added is a pure reordering of the raw playlist order; the full
    // playlist must be loaded first so the sort spans the real list.
    let playlist_id = app.track_table_playlist_uri();
    let fully_loaded = app
      .playlist_tracks
      .as_ref()
      .map(|p| p.items.len() as u32 >= p.total)
      .unwrap_or(true);
    if fully_loaded {
      app.materialize_date_added();
    } else {
      app.date_added_pending = true;
      if let Some(playlist_id) = playlist_id {
        app.dispatch(IoEvent::LoadAllPlaylistItems(playlist_id));
      }
    }
    return;
  }

  app.sort_tracks();
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::TrackTableContext;
  use crossterm::event::KeyModifiers;
  use ratatui::layout::Rect;
  use serde_json::json;

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

  fn track_table_app(n: usize) -> App {
    // Clear shared mouse state: tests run in parallel and share the thread-local statics.
    SCROLLBAR_DRAG.with(|d| *d.borrow_mut() = None);
    PLAYBAR_DRAG.with(|d| *d.borrow_mut() = None);
    let mut app = App::default();
    app.size = Rect::new(0, 0, 200, 50);
    app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
    app.track_table.tracks = (0..n).map(mock_track).collect();
    app
  }

  fn wheel_event(up: bool, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
      kind: if up {
        MouseEventKind::ScrollUp
      } else {
        MouseEventKind::ScrollDown
      },
      column,
      row,
      modifiers: KeyModifiers::NONE,
    }
  }

  fn wheel_over_track_table(up: bool, app: &mut App) {
    handle_mouse(wheel_event(up, 150, 10), app);
  }

  fn search_app(n_items: usize, total: u32) -> App {
    SCROLLBAR_DRAG.with(|d| *d.borrow_mut() = None);
    PLAYBAR_DRAG.with(|d| *d.borrow_mut() = None);
    let mut app = App::default();
    app.size = Rect::new(0, 0, 200, 50);
    app.push_navigation_stack(RouteId::Search, ActiveBlock::SearchResultBlock);
    app.search_results.selected_block = SearchResultBlock::SongSearch;
    app.search_results.tracks = Some(rspotify::model::Page {
      href: String::new(),
      items: (0..n_items).map(mock_track).collect(),
      limit: 10,
      next: None,
      offset: 0,
      previous: None,
      total,
    });
    app
  }

  #[test]
  fn search_wheel_down_stops_on_load_more_row() {
    // Full page (10 songs): the " Load more " row (index == count) must be
    // reachable by scrolling down so the button can be clicked repeatedly,
    // then clamp on it.
    let mut app = search_app(10, 20);
    app.search_results.selected_tracks_index = Some(9);
    handle_search_wheel(false, &mut app);
    assert_eq!(app.search_results.selected_tracks_index, Some(10));
    handle_search_wheel(false, &mut app);
    assert_eq!(app.search_results.selected_tracks_index, Some(10));
  }

  #[test]
  fn search_wheel_down_clamps_at_last_result_without_more() {
    // Short page (9 < limit): no load-more row, the selection stops at the end.
    let mut app = search_app(9, 20);
    app.search_results.selected_tracks_index = Some(8);
    handle_search_wheel(false, &mut app);
    assert_eq!(app.search_results.selected_tracks_index, Some(8));
  }

  #[test]
  fn search_keyboard_down_moves_onto_load_more_row() {
    let mut app = search_app(10, 20);
    app.search_results.selected_tracks_index = Some(9);
    handle_app(Key::Down, &mut app);
    assert_eq!(app.search_results.selected_tracks_index, Some(10));
  }

  #[test]
  fn search_keyboard_down_wraps_to_top_without_more() {
    let mut app = search_app(9, 20);
    app.search_results.selected_tracks_index = Some(8);
    handle_app(Key::Down, &mut app);
    assert_eq!(app.search_results.selected_tracks_index, Some(0));
  }

  #[test]
  fn wheel_scrolls_one_row() {
    let mut app = track_table_app(40);
    // Wheel scrolls the VIEW, not the selection: scroll_offset advances,
    // selected_index stays put.
    wheel_over_track_table(false, &mut app);
    assert_eq!(app.track_table.scroll_offset, 1);
    assert_eq!(app.track_table.selected_index, 0);
  }

  #[test]
  fn wheel_clamps_at_top() {
    let mut app = track_table_app(40);
    wheel_over_track_table(true, &mut app);
    assert_eq!(app.track_table.scroll_offset, 0);
  }

  #[test]
  fn wheel_clamps_at_bottom() {
    let mut app = track_table_app(40);
    // viewport = 34 → max offset = 40 - 34 = 6
    app.track_table.scroll_offset = 6;
    wheel_over_track_table(false, &mut app);
    assert_eq!(app.track_table.scroll_offset, 6);
    assert_eq!(app.track_table.selected_index, 0);
  }

  #[test]
  fn wheel_up_scrolls_back_with_selection_at_bottom() {
    // The reported bug: after clicking "Load more songs..." the selection sits
    // on the bottom rows, and wheel-up used to be swallowed by the drawer snap.
    let mut app = track_table_app(40);
    app.track_table.selected_index = 39;
    app.track_table.scroll_offset = 6;
    wheel_over_track_table(true, &mut app);
    assert_eq!(app.track_table.scroll_offset, 5);
    assert_eq!(
      app.track_table.selected_index, 39,
      "wheel keeps selection put"
    );
  }

  #[test]
  fn keyboard_down_keeps_selection_visible() {
    // viewport = 36 (200x50, 3-row header): selection below the window must
    // push the view so the selection stays on the bottom row.
    let mut app = track_table_app(40);
    app.track_table.selected_index = 35;
    app.track_table.scroll_offset = 0;
    handle_app(Key::Down, &mut app);
    assert_eq!(app.track_table.selected_index, 36);
    assert_eq!(app.track_table.scroll_offset, 1);
  }

  #[test]
  fn keyboard_up_keeps_selection_visible() {
    let mut app = track_table_app(40);
    app.track_table.selected_index = 4;
    app.track_table.scroll_offset = 30;
    handle_app(Key::Up, &mut app);
    assert_eq!(app.track_table.selected_index, 3);
    assert_eq!(app.track_table.scroll_offset, 3);
  }

  #[test]
  fn wheel_over_playbar_does_not_scroll() {
    let mut app = track_table_app(20);
    handle_mouse(wheel_event(false, 150, 47), &mut app);
    assert_eq!(app.track_table.selected_index, 0);
  }

  #[test]
  fn wheel_over_left_panel_scrolls_library() {
    let mut app = track_table_app(20);
    app.size = Rect::new(0, 0, 200, 50);
    app.set_current_route_state(Some(ActiveBlock::Library), None);
    app.library.selected_index = 0;
    handle_mouse(wheel_event(false, 5, 10), &mut app);
    assert_eq!(app.library.selected_index, 1);
  }

  fn click_event(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column,
      row,
      modifiers: KeyModifiers::NONE,
    }
  }

  #[test]
  fn click_dev_view_row_toggles_request_log() {
    // settings rect = (2,2,196,13) with margin 2; Dev view row
    // sits at y=12.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(!app.dev_view);
    handle_mouse(click_event(10, 12), &mut app);
    assert!(app.dev_view);
    handle_mouse(click_event(10, 12), &mut app);
    assert!(!app.dev_view);
  }

  #[test]
  fn click_on_playbar_does_not_start_playback() {
    let mut app = track_table_app(20);
    // Row 47 is the playbar. A click there must not fire Enter
    // (which would dispatch StartPlayback / restart the song).
    handle_mouse(click_event(100, 47), &mut app);
    assert!(!app.is_loading);
  }

  #[test]
  fn click_on_track_row_plays_track() {
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    // Row 8 is the first data row of the content area (routes start at y=6,
    // right column x=150). A single click selects AND fires Enter.
    handle_mouse(click_event(150, 8), &mut app);
    assert!(app.is_loading);
  }

  #[test]
  fn click_on_empty_space_does_not_start_playback() {
    let mut app = track_table_app(20);
    // Row 48 is outside the list but still in the content area — no Enter.
    handle_mouse(click_event(150, 48), &mut app);
    assert!(!app.is_loading);
  }

  #[test]
  fn scrollbar_drag_moves_selection() {
    let mut app = track_table_app(40);
    // size 200x50: right=(41,6,158,39), track_h=37, viewport=36, count=40,
    // max offset 4. TrackTable drags move the VIEW offset (website-style):
    // the thumb reaches the end of the track, selection stays put.
    // Press on the thumb (top = right.y+1 = 7), then drag down to the bottom.
    handle_mouse(click_event(197, 7), &mut app);
    handle_mouse(drag_event(197, 43), &mut app);
    assert_eq!(app.track_table.scroll_offset, 4);
    assert_eq!(app.track_table.selected_index, 0);
    // Drag back to the top of the track.
    handle_mouse(drag_event(197, 2), &mut app);
    assert_eq!(app.track_table.scroll_offset, 0);
    // Releasing ends the drag; later clicks are not affected.
    handle_mouse(up_event(197, 2), &mut app);
  }

  #[test]
  fn scrollbar_drag_ignored_without_overflow() {
    let mut app = track_table_app(5);
    // 5 tracks <= viewport 34 → no scrollbar drawn, press must not drag.
    handle_mouse(click_event(197, 41), &mut app);
    handle_mouse(drag_event(197, 2), &mut app);
    assert_eq!(app.track_table.selected_index, 0);
  }

  #[test]
  fn scrollbar_track_press_jumps_thumb_and_drags() {
    let mut app = track_table_app(40);
    // Press on the scrollbar TRACK well below the thumb (thumb is 1-2 rows on
    // big lists): the drag arms anyway and the thumb jumps under the cursor.
    handle_mouse(click_event(197, 43), &mut app);
    handle_mouse(drag_event(197, 43), &mut app);
    assert_eq!(app.track_table.scroll_offset, 4);
    handle_mouse(up_event(197, 43), &mut app);
  }

  #[test]
  fn sidebar_playlist_click_uses_autosize_geometry() {
    let mut app = track_table_app(20);
    app.playlists = Some(mock_playlist_page(40));
    // Autosize split at 200x50: library box = 6 entries + 2 borders = 8
    // rows → playlists start at y=14 (the old fixed 30/70 handler placed
    // them at y=19, off by 5 rows). A click on a drawn playlist row must
    // map to that row: index = y - (chunk.y + 1) = y - 15.
    handle_mouse(click_event(5, 20), &mut app);
    assert_eq!(app.selected_playlist_index, Some(5));
    assert_eq!(
      app.get_current_route().active_block,
      ActiveBlock::MyPlaylists
    );
    // Row straddling the old split boundary is a normal playlist row too.
    handle_mouse(click_event(5, 19), &mut app);
    assert_eq!(app.selected_playlist_index, Some(4));
    // A click outside the playlists box maps to nothing: no selection change.
    handle_mouse(click_event(5, 46), &mut app);
    assert_eq!(app.selected_playlist_index, Some(4));
  }

  fn mock_playlist_page(n: usize) -> rspotify::model::Page<rspotify::model::SimplifiedPlaylist> {
    let items = (0..n)
      .map(|i| {
        serde_json::from_value(json!({
          "collaborative": false,
          "external_urls": {},
          "href": format!("spotify:playlist:mockplaylist{}", i),
          "id": format!("mockplaylist{}", i),
          "images": [],
          "name": format!("Mock Playlist {}", i),
          "owner": {
            "display_name": "Mock User",
            "external_urls": {},
            "followers": null,
            "href": "spotify:user:mockuser",
            "id": "mockuser",
            "images": [],
          },
          "public": null,
          "snapshot_id": format!("snap{}", i),
          "items": { "href": "", "total": 150 },
        }))
        .unwrap()
      })
      .collect();
    rspotify::model::Page {
      href: String::new(),
      items,
      limit: n as u32,
      next: None,
      offset: 0,
      previous: None,
      total: n as u32,
    }
  }

  #[test]
  fn sidebar_scrollbar_drag_scrolls_playlist_selection() {
    let mut app = track_table_app(20);
    app.playlists = Some(mock_playlist_page(40));
    // Derive the sidebar rect the same way the drawer and handler do (auto
    // width: block titles, library options and the longest "Mock Playlist N"
    // name all fit). The Library box grows to fit its 6 entries (+2 borders =
    // 8 rows), so playlists start at left.y+8 with the rest of the column →
    // viewport 30 < 40 items → scrollbar drawn at x = left.x + left.width - 2.
    let (routes, _, _) = main_layout(&app).unwrap();
    let (left, _) = layout::sidebar_content_split(&app, routes);
    let scrollbar_x = left.x + left.width - 2;
    // Press on the track and drag to the bottom: the selection scrolls to the
    // last playlist.
    handle_mouse(click_event(scrollbar_x, 40), &mut app);
    handle_mouse(drag_event(scrollbar_x, 40), &mut app);
    assert_eq!(app.selected_playlist_index, Some(35));
    assert_eq!(
      app.get_current_route().active_block,
      ActiveBlock::MyPlaylists
    );
    handle_mouse(up_event(scrollbar_x, 40), &mut app);
  }

  #[test]
  fn sidebar_library_box_fits_items_no_scrollbar() {
    let mut app = track_table_app(20);
    // Library box height = entries + 2 border rows, so the fit list never
    // overflows and the scrollbar arm refuses to start a drag there.
    let (routes, _, _) = main_layout(&app).unwrap();
    let (left, _) = layout::sidebar_content_split(&app, routes);
    let (library, _) = crate::tui::layout::library_playlists_split(&app, left);
    let visible = visible_library_options(&app.hidden_library_sections).len();
    assert_eq!(library.height as usize, visible + 2);
    handle_mouse(click_event(left.x + left.width - 2, 8), &mut app);
    handle_mouse(drag_event(left.x + left.width - 2, 45), &mut app);
    assert_eq!(app.selected_playlist_index, None);
  }

  #[test]
  fn track_scrollbar_drag_reaches_end_with_load_more() {
    // The load-more row is part of the drawn table, so the scrollbar range
    // includes it (count = tracks + 1). The drag handler must agree with the
    // drawer and the wheel — this was the "scrollbar doesn't go to the end
    // when dragging" bug (drag max offset was one row short).
    let mut app = track_table_app(58);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.playlist_tracks = Some(rspotify::model::Page {
      href: String::new(),
      items: vec![],
      limit: 0,
      next: None,
      offset: 0,
      previous: None,
      total: 505,
    });
    // right=(41,6,158,39), track rows 7..44, viewport 36, count 59 → max
    // offset 23. Press the thumb at the top, drag to the bottom.
    handle_mouse(click_event(197, 7), &mut app);
    handle_mouse(drag_event(197, 44), &mut app);
    assert_eq!(app.track_table.scroll_offset, 23);
    // And back to the top.
    handle_mouse(drag_event(197, 7), &mut app);
    assert_eq!(app.track_table.scroll_offset, 0);
    handle_mouse(up_event(197, 7), &mut app);
  }

  #[test]
  fn request_log_scrollbar_drag_and_wheel() {
    let mut app = track_table_app(20);
    app.dev_view = true;
    app.request_log.clear();
    for i in 0..80 {
      app.request_log.push_back(crate::app::RequestLogEntry {
        text: format!("request {}", i),
        count: 1,
      });
    }
    // Dev panel = right column's right quarter; derive the rects the exact
    // same way the handlers do (auto-sized sidebar, 75/25 dev split), so the
    // click column lands exactly on the drawn scrollbar.
    let (routes, _, _) = main_layout(&app).unwrap();
    let (_, right) = layout::sidebar_content_split(&app, routes);
    let dev = dev_panel_rect(right);
    let scrollbar_x = dev.x + dev.width - 2;
    // count 80, viewport = 39−2 (panel height 40) → max offset 42: dragging
    // to the bottom lands the selection on the last row (79).
    handle_mouse(click_event(scrollbar_x, 7), &mut app);
    handle_mouse(drag_event(scrollbar_x, 44), &mut app);
    assert_eq!(app.request_log_index, Some(79));
    assert_eq!(
      app.get_current_route().active_block,
      ActiveBlock::RequestLog
    );
    // Wheel over the dev panel: at the bottom the generic list path refuses
    // to scroll past the end, wheel up moves one row.
    handle_mouse(wheel_event(false, dev.x + 2, 10), &mut app);
    assert_eq!(app.request_log_index, Some(79));
    handle_mouse(wheel_event(true, dev.x + 2, 10), &mut app);
    assert_eq!(app.request_log_index, Some(78));
    handle_mouse(up_event(scrollbar_x, 44), &mut app);
  }

  #[test]
  fn request_log_title_click_clears_log() {
    let mut app = track_table_app(20);
    app.dev_view = true;
    app.request_log_index = Some(3);
    for i in 0..5 {
      app.request_log.push_back(crate::app::RequestLogEntry {
        text: format!("request {}", i),
        count: 1,
      });
    }
    let (routes, _, _) = main_layout(&app).unwrap();
    let (_, right) = layout::sidebar_content_split(&app, routes);
    let dev = dev_panel_rect(right);
    // Title row is 4 rows below dev rect (below throttle info header).
    handle_mouse(click_event(dev.x + 2, dev.y + 4), &mut app);
    assert!(app.request_log.is_empty());
    assert_eq!(app.request_log_index, None);
    // A click on a log row must not clear or select anything.
    handle_mouse(click_event(dev.x + 2, dev.y + 3), &mut app);
    assert!(app.request_log.is_empty());
  }

  fn mock_playlist_item(i: usize) -> rspotify::model::PlaylistItem {
    serde_json::from_value(json!({
      "added_at": null,
      "added_by": null,
      "is_local": false,
      "track": json!(mock_track(i)),
    }))
    .unwrap()
  }

  #[test]
  fn date_added_header_click_dispatches_load_all_when_partial() {
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.playlists = Some(mock_playlist_page(3));
    app.active_playlist_index = Some(0);
    app.track_table.tracks = (0..57).map(mock_track).collect();
    app.track_table_added_at = (0..57).map(|_| None).collect();
    app.playlist_tracks = Some(rspotify::model::Page {
      href: String::new(),
      items: (0..57).map(mock_playlist_item).collect(),
      limit: 50,
      next: None,
      offset: 57,
      previous: None,
      total: 505,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    // right=(41,6,158,39): header row y=7, Date Added column spans the
    // absolute x range 160..184.
    handle_mouse(click_event(170, 7), &mut app);
    assert_eq!(
      app.track_table_sort,
      Some((TrackSortColumn::DateAdded, true))
    );
    // One backend-driven page loop is dispatched, not one event per page.
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::LoadAllPlaylistItems(
        "spotify:playlist:mockplaylist0".to_string()
      )]
    );
    // Fully loaded playlists sort in place without paging.
    app.playlist_tracks.as_mut().unwrap().total = 57;
    handle_mouse(click_event(170, 7), &mut app);
    assert!(rx.try_iter().next().is_none());
  }

  #[test]
  fn sorted_table_enter_plays_raw_playlist_index() {
    // Simulate a Date Added sort: the displayed table is newest-first
    // (reversed) while the raw cumulative playlist stays in original order.
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.playlists = Some(mock_playlist_page(3));
    app.active_playlist_index = Some(0);
    let raw: Vec<rspotify::model::FullTrack> = (0..10).map(mock_track).collect();
    app.track_table.tracks = raw.iter().rev().cloned().collect();
    app.track_table_added_at = (0..10).map(|_| None).collect();
    app.track_table_raw_index = (0..10).rev().collect();
    app.track_table_sort = Some((TrackSortColumn::DateAdded, true));
    app.playlist_tracks = Some(rspotify::model::Page {
      href: String::new(),
      items: (0..10).map(mock_playlist_item).collect(),
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total: 10,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    // Row 0 of the sorted table = mocktrack9 (newest); StartPlaybackAt must
    // carry the URI of the displayed track, not a raw offset.
    app.track_table.selected_index = 0;
    crate::handlers::handle_app(Key::Enter, &mut app);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::StartPlaybackAt(
        Some("spotify:playlist:mockplaylist0".to_string()),
        Some("spotify:track:mocktrack9".to_string())
      )]
    );
    // Unsorted tables display the raw order, so row 0 is mocktrack0.
    app.track_table_sort = None;
    app.track_table.tracks = raw.clone();
    app.track_table_raw_index = (0..10).collect();
    let (tx2, rx2) = std::sync::mpsc::channel();
    app.io_tx = Some(tx2);
    crate::handlers::handle_app(Key::Enter, &mut app);
    let dispatched: Vec<IoEvent> = rx2.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::StartPlaybackAt(
        Some("spotify:playlist:mockplaylist0".to_string()),
        Some("spotify:track:mocktrack0".to_string())
      )]
    );
  }

  fn drag_event(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
      kind: MouseEventKind::Drag(MouseButton::Left),
      column,
      row,
      modifiers: KeyModifiers::NONE,
    }
  }

  fn up_event(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
      kind: MouseEventKind::Up(MouseButton::Left),
      column,
      row,
      modifiers: KeyModifiers::NONE,
    }
  }

  #[test]
  fn search_opened_playlist_enter_uses_displayed_track() {
    // A playlist opened from search results (PlaylistSearch context) must
    // resolve the context uri from the search results, not the sidebar, and
    // start playback at the URI of the displayed track after a sort.
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::PlaylistSearch);
    app.search_results.selected_playlists_index = Some(1);
    app.search_results.playlists = Some(mock_playlist_page(3));
    let raw: Vec<rspotify::model::FullTrack> = (0..10).map(mock_track).collect();
    app.track_table.tracks = raw.iter().rev().cloned().collect();
    app.track_table_added_at = (0..10).map(|_| None).collect();
    app.track_table_raw_index = (0..10).rev().collect();
    app.track_table_sort = Some((TrackSortColumn::DateAdded, true));
    app.playlist_tracks = Some(rspotify::model::Page {
      href: String::new(),
      items: (0..10).map(mock_playlist_item).collect(),
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total: 10,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    app.track_table.selected_index = 0;
    crate::handlers::handle_app(Key::Enter, &mut app);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::StartPlaybackAt(
        Some("spotify:playlist:mockplaylist1".to_string()),
        Some("spotify:track:mocktrack9".to_string())
      )]
    );
  }

  #[test]
  fn date_added_header_click_then_row_click_plays_displayed_song() {
    // Full user flow: a fully-loaded playlist opened from the sidebar, click
    // the Date Added header (materializes the reversal), then click a song
    // row and press Enter. The played offset must be the raw playlist
    // position of the song DISPLAYED under the cursor.
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.playlists = Some(mock_playlist_page(3));
    app.active_playlist_index = Some(0);
    let n = 517;
    let raw: Vec<rspotify::model::FullTrack> = (0..n).map(mock_track).collect();
    app.track_table.tracks = raw.clone();
    app.track_table_added_at = (0..n).map(|_| None).collect();
    app.track_table_raw_index = (0..n).collect();
    app.playlist_tracks = Some(rspotify::model::Page {
      href: String::new(),
      items: (0..n).map(mock_playlist_item).collect(),
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total: n as u32,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    // 1. Click the Date Added header (chunk.y+1 = 7; column x 160..184).
    handle_mouse(click_event(170, 7), &mut app);
    assert_eq!(
      app.track_table_sort,
      Some((TrackSortColumn::DateAdded, true))
    );
    assert_eq!(app.track_table.tracks[0].name, "Mock Song 516");
    assert_eq!(app.track_table_raw_index[0], 516);
    assert_eq!(app.track_table_raw_index.len(), n);

    // 2. Click song row 8 in the Title column area (x=50 is safely inside
    //    the name column of a 158-wide table) so it triggers playback.
    handle_mouse(click_event(50, 15), &mut app);
    assert_eq!(app.track_table.selected_index, 7);

    // 3. The click must have dispatched StartPlaybackAt for the song displayed
    //    at row 8 = raw index 509 (the 8th newest), resolved by URI.
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::StartPlaybackAt(
        Some("spotify:playlist:mockplaylist0".to_string()),
        Some("spotify:track:mocktrack509".to_string())
      )]
    );
  }

  #[test]
  fn click_selects_row_under_cursor_with_scrolled_view() {
    let mut app = track_table_app(40);
    // Wheel-scroll the view all the way down (viewport 36 → max offset 4).
    for _ in 0..10 {
      wheel_over_track_table(false, &mut app);
    }
    assert_eq!(app.track_table.scroll_offset, 4);
    // size 200x50: right=(41,6,158,39); first data row is chunk.y+2 = 8.
    // Clicking row 10 with offset 4 → index 4 + (10-8) = 6.
    handle_mouse(click_event(150, 10), &mut app);
    assert_eq!(app.track_table.selected_index, 6);
    // Click the row right below the header (row 8) → index == offset.
    handle_mouse(click_event(150, 8), &mut app);
    assert_eq!(app.track_table.selected_index, 4);
  }

  #[test]
  fn click_selects_first_row_when_not_scrolled() {
    let mut app = track_table_app(40);
    handle_mouse(click_event(150, 8), &mut app);
    assert_eq!(app.track_table.selected_index, 0);
    handle_mouse(click_event(150, 13), &mut app);
    assert_eq!(app.track_table.selected_index, 5);
  }

  #[test]
  fn playlist_title_row_click_activates_search_and_captures_typing() {
    // Clicking the title row of a playlist page must activate the in-playlist
    // search (set the filter) instead of selecting a song, and every key
    // pressed afterwards must feed the query rather than move/activate.
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.playlists = Some(mock_playlist_page(3));
    app.active_playlist_index = Some(0);
    assert!(app.playlist_filter.is_none());
    // chunk.y=6 is the title row for size 200x50.
    handle_mouse(click_event(150, 6), &mut app);
    assert_eq!(app.playlist_filter, Some(String::new()));
    // Typing must land in the query, not move the selection.
    crate::handlers::handle_app(Key::Char('h'), &mut app);
    crate::handlers::handle_app(Key::Char('i'), &mut app);
    assert_eq!(app.playlist_filter, Some("hi".to_string()));
    assert_eq!(app.track_table.selected_index, 0);
    // The search_in_playlist toggle key closes the filter.
    let toggle = app.user_config.keys.search_in_playlist.unwrap();
    crate::handlers::handle_app(toggle, &mut app);
    assert!(app.playlist_filter.is_none());
  }

  #[test]
  fn playlist_search_loses_focus_when_navigating_away() {
    // The in-playlist search must drop focus when the user switches to another
    // playlist, clicks another panel, or opens another view.
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.playlists = Some(mock_playlist_page(3));
    app.active_playlist_index = Some(0);
    app.playlist_filter = Some("hello".to_string());

    // Clicking another playlist in the sidebar switches the active block and
    // must drop the search focus. Playlist rows start at y=15 with this mock.
    handle_mouse(click_event(5, 16), &mut app);
    assert_eq!(app.get_current_route().active_block, ActiveBlock::MyPlaylists);
    assert!(app.playlist_filter.is_none());

    app.playlist_filter = Some("hello".to_string());
    // Opening another view pushes a route.
    app.push_navigation_stack(RouteId::Search, ActiveBlock::Empty);
    assert!(app.playlist_filter.is_none());
  }

  #[test]
  fn made_for_you_row_click_dispatches_expand() {
    // MadeForYou renders as a List widget (items start at chunk.y+1); chunk.y=6
    // here → the first playlist is y=7.
    SCROLLBAR_DRAG.with(|d| *d.borrow_mut() = None);
    PLAYBAR_DRAG.with(|d| *d.borrow_mut() = None);
    let mut app = App::default();
    app.made_for_you_custom.push(("My Mix".to_string(), "mix1".to_string()));
    app.size = Rect::new(0, 0, 200, 50);
    app.push_navigation_stack(RouteId::MadeForYou, ActiveBlock::MadeForYou);
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);

    handle_mouse(click_event(150, 7), &mut app);
    assert_eq!(app.made_for_you_index, 0);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::GetMadeForYouPlaylistItems("mix1".into(), 0)]
    );
  }

  fn recently_played_app(n: usize, has_more: bool) -> (App, std::sync::mpsc::Receiver<IoEvent>) {
    // Clear shared mouse state: tests run in parallel and share the thread-local statics.
    SCROLLBAR_DRAG.with(|d| *d.borrow_mut() = None);
    PLAYBAR_DRAG.with(|d| *d.borrow_mut() = None);
    let mut app = App::default();
    app.size = Rect::new(0, 0, 200, 50);
    app.push_navigation_stack(RouteId::RecentlyPlayed, ActiveBlock::RecentlyPlayed);
    let items: Vec<rspotify::model::PlayHistory> = (0..n)
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
      limit: if has_more { n as u32 } else { 0 },
      next: if has_more {
        Some("mock-cursor".to_string())
      } else {
        None
      },
      cursors: None,
      total: Some(n as u32),
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    (app, rx)
  }

  #[test]
  fn recently_played_load_more_click_loads_more_and_does_not_play() {
    // 40 items fill a full page (limit 40 → has_more), selection sits on the
    // load-more row (index 40). The button is the last visible row: clicking
    // it must fetch the next page, NOT fire Enter and play the song behind it.
    let (mut app, rx) = recently_played_app(40, true);
    app.recently_played.index = 40;
    handle_mouse(click_event(150, 43), &mut app);
    assert_eq!(app.recently_played.index, 40);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::GetMoreRecentlyPlayed(None)]);
  }

  #[test]
  fn recently_played_row_click_plays_that_song() {
    let (mut app, rx) = recently_played_app(3, false);
    // Row 8 is the first data row of the content area (routes start at y=6,
    // right column x=150). A single click selects AND fires Enter.
    handle_mouse(click_event(150, 8), &mut app);
    assert_eq!(app.recently_played.index, 0);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(
      dispatched,
      vec![IoEvent::StartPlayback(
        None,
        Some(vec![
          "spotify:track:mocktrack0".to_string(),
          "spotify:track:mocktrack1".to_string(),
          "spotify:track:mocktrack2".to_string(),
        ]),
        Some(0)
      )]
    );
  }

  #[test]
  fn recently_played_enter_on_load_more_row_never_plays_when_list_exhausted() {
    // After the last page loads, has_more flips false while the selection can
    // still sit on the (now gone) button row. Enter must not dispatch playback.
    let (mut app, rx) = recently_played_app(3, false);
    app.recently_played.index = 3;
    handle_app(Key::Enter, &mut app);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert!(dispatched.is_empty(), "dispatched: {:?}", dispatched);
  }

  #[test]
  fn track_table_enter_on_missing_row_never_plays() {
    // Selection on a row past the end of the list (the load-more slot after
    // the last page loaded): Enter must not dispatch playback.
    let mut app = track_table_app(20);
    app.track_table.context = Some(TrackTableContext::SavedTracks);
    let page = rspotify::model::Page {
      href: String::new(),
      items: (0..20)
        .map(|i| {
          serde_json::from_value(json!({
            "added_at": "2024-01-01T00:00:00Z",
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
          }))
          .unwrap()
        })
        .collect(),
      limit: 50,
      next: None,
      offset: 0,
      previous: None,
      total: 20,
    };
    app.library.saved_tracks.add_pages(page);
    app.track_table.selected_index = 20;
    let (tx, rx) = std::sync::mpsc::channel();
    app.io_tx = Some(tx);
    handle_app(Key::Enter, &mut app);
    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert!(dispatched.is_empty(), "dispatched: {:?}", dispatched);
  }

  fn playback_app() -> App {
    let mut app = track_table_app(20);
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
      "item": mock_track(0),
      "currently_playing_type": "track",
      "actions": { "disallows": {} },
    }))
    .unwrap();
    app.current_playback_context = Some(playback);
    app
  }

  #[test]
  fn click_on_progress_bar_seeks() {
    // size 200x50, margin 1: playbar=(1,45,198,4), bar row y=47;
    // bar_len=70 cells (1 cell ≈ 2.57s of a 180s track), centered:
    // bar_x = 2+(196-82)/2+6 = 65, so the bar spans x 65..134 and the
    // left "0:00" label sits in x 59..64.
    // Clicking the middle (x=100) must seek to ~50% of 180_000 ms.
    let mut app = playback_app();
    handle_mouse(click_event(100, 47), &mut app);
    assert!(app.seek_ms.is_some());
    let seek = app.seek_ms.unwrap();
    assert!((85_000..95_000).contains(&seek), "seek = {}", seek);
    // Clicking the left time label (x=61, right before the bar) seeks to start.
    handle_mouse(click_event(61, 47), &mut app);
    assert_eq!(app.seek_ms, Some(0));
    // The first bar cell (x=65) must seek into the first ~2 seconds,
    // so the 1-second mark is reachable by dragging.
    handle_mouse(click_event(65, 47), &mut app);
    assert!((0..3_000).contains(&app.seek_ms.unwrap()));
  }

  #[test]
  fn click_volume_ramp_bar_row_toggles_setting() {
    // settings rect = (2,2,196,6) with margin 2; row 3 (Volume ramp bar)
    // is the last settings row at y=6.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(!app.user_config.behavior.volume_ramp_bar);
    handle_mouse(click_event(10, 6), &mut app);
    assert!(app.user_config.behavior.volume_ramp_bar);
    handle_mouse(click_event(10, 6), &mut app);
    assert!(!app.user_config.behavior.volume_ramp_bar);
  }

  #[test]
  fn click_playlists_row_toggles_show_playlists() {
    // settings rect = (2,2,196,6) with margin 2; row 2 (Playlists block)
    // sits at y=5.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(app.show_playlists);
    handle_mouse(click_event(10, 5), &mut app);
    assert!(!app.show_playlists);
    handle_mouse(click_event(10, 5), &mut app);
    assert!(app.show_playlists);
  }

  #[test]
  fn click_mouse_settings_row_toggles_enable_mouse() {
    // settings rect = (2,2,196,8) with margin 2; row 4 (Mouse interactions)
    // sits at y=7.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(app.user_config.behavior.enable_mouse);
    handle_mouse(click_event(10, 7), &mut app);
    assert!(!app.user_config.behavior.enable_mouse);
    handle_mouse(click_event(10, 7), &mut app);
    assert!(app.user_config.behavior.enable_mouse);
  }

  #[test]
  fn click_theme_row_cycles_presets_then_back_to_custom() {
    // Row 5 (Theme) sits at y=8 in the settings rect (2,2,196,8).
    use crate::user_config::theme_presets;
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    let presets = theme_presets();
    assert_eq!(app.theme_preset_index, None);
    handle_mouse(click_event(10, 8), &mut app);
    assert_eq!(app.theme_preset_index, Some(0));
    assert_eq!(app.user_config.theme.background, presets[0].1.background);
    handle_mouse(click_event(10, 8), &mut app);
    assert_eq!(app.theme_preset_index, Some(1));
    assert_eq!(app.user_config.theme.background, presets[1].1.background);
    handle_mouse(click_event(10, 8), &mut app);
    assert_eq!(app.theme_preset_index, None);
    assert_eq!(
      app.user_config.theme.background,
      app.config_theme.background
    );
  }

  #[test]
  fn click_seek_by_typing_row_toggles_setting() {
    // settings rect = (2,2,196,9) with margin 2; row 6 (Seek by typing)
    // sits at y=9.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(!app.user_config.behavior.seek_by_typing);
    handle_mouse(click_event(10, 9), &mut app);
    assert!(app.user_config.behavior.seek_by_typing);
    handle_mouse(click_event(10, 9), &mut app);
    assert!(!app.user_config.behavior.seek_by_typing);
  }

  #[test]
  fn click_resume_track_row_toggles_setting() {
    // settings rect = (2,2,196,10) with margin 2; row 7 (Resume last song)
    // sits at y=10.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(!app.user_config.behavior.resume_track);
    handle_mouse(click_event(10, 10), &mut app);
    assert!(app.user_config.behavior.resume_track);
    handle_mouse(click_event(10, 10), &mut app);
    assert!(!app.user_config.behavior.resume_track);
  }

  #[test]
  fn click_restore_settings_row_toggles_setting() {
    // settings rect = (2,2,196,11) with margin 2; row 8 (Restore settings
    // on start) sits at y=11.
    let mut app = playback_app();
    app.set_current_route_state(Some(ActiveBlock::HelpMenu), None);
    assert!(!app.user_config.behavior.restore_settings);
    handle_mouse(click_event(10, 11), &mut app);
    assert!(app.user_config.behavior.restore_settings);
    handle_mouse(click_event(10, 11), &mut app);
    assert!(!app.user_config.behavior.restore_settings);
  }

  #[test]
  fn click_volume_box_changes_volume() {
    let mut app = playback_app();
    // volume rect = (174, 47, 24, 1) on the music bar row; bar_x = 174+2 = 176
    // ("♪ " prefix 2 wide), bar spans 17 chars (176..194); clicking 190 → ~82%.
    handle_mouse(click_event(190, 47), &mut app);
    assert!(app.is_loading);
  }

  #[test]
  fn drag_on_progress_bar_previews_then_seeks() {
    let mut app = playback_app();
    // Press = immediate jump (click at x=100 → ~50% of 180s).
    handle_mouse(click_event(100, 47), &mut app);
    assert!((85_000..95_000).contains(&app.seek_ms.unwrap()));
    // Drag right (x=130 → ~93%): live preview only, nothing dispatched yet.
    handle_mouse(drag_event(130, 47), &mut app);
    let preview = app.seek_ms.unwrap();
    assert!(
      (165_000..172_000).contains(&preview),
      "preview = {}",
      preview
    );
    // Drag left (x=72 → ~10%): preview follows the cursor.
    handle_mouse(drag_event(72, 47), &mut app);
    let preview = app.seek_ms.unwrap();
    assert!((12_000..24_000).contains(&preview), "preview = {}", preview);
    // Release commits the scrubbed position once (is_loading = dispatched).
    app.is_loading = false;
    handle_mouse(up_event(72, 47), &mut app);
    assert!(app.is_loading);
    assert!((12_000..24_000).contains(&app.seek_ms.unwrap()));
  }

  #[test]
  fn drag_on_volume_previews_then_commits() {
    let mut app = playback_app();
    // Press on the volume bar still applies instantly.
    handle_mouse(click_event(190, 47), &mut app);
    assert!(app.is_loading);
    app.is_loading = false;
    app.volume_preview = None;
    // Drag over the 17-cell bar (bar_x = 176): preview only, no dispatch.
    handle_mouse(drag_event(180, 47), &mut app);
    assert_eq!(app.volume_preview, Some(23));
    handle_mouse(drag_event(187, 47), &mut app);
    assert_eq!(app.volume_preview, Some(64));
    assert!(!app.is_loading);
    // Release dispatches the final volume once and clears the preview.
    handle_mouse(up_event(187, 47), &mut app);
    assert!(app.is_loading);
    assert_eq!(app.volume_preview, None);
  }

  #[test]
  fn click_without_drag_commits_nothing_extra() {
    let mut app = playback_app();
    handle_mouse(click_event(100, 47), &mut app);
    let seek = app.seek_ms.unwrap();
    app.is_loading = false;
    // Plain release after a press: no second dispatch, no stray preview.
    handle_mouse(up_event(100, 47), &mut app);
    assert_eq!(app.seek_ms, Some(seek));
    assert!(!app.is_loading);
    assert_eq!(app.volume_preview, None);
  }

  #[test]
  fn click_play_pause_button_dispatches() {
    let mut app = playback_app();
    // Buttons dead-centered on the playbar's first inner row (y = playbar.y
    // + 1); window x's derive from the same math the drawer uses, so they
    // stay correct no matter the glyph widths. Click the play/pause glyph's
    // midpoint: it dispatches (is_loading turns on).
    let ctx = app.current_playback_context.as_ref().unwrap();
    let controls = build_playbar_controls(ctx.is_playing, false);
    let (_, playbar, _) = main_layout(&app).unwrap();
    let mut btn_x = playbar_controls_x(playbar, &controls);
    let mut play_x = None;
    for (kind, text) in controls {
      let w = text.width() as u16;
      if matches!(kind, PlaybarButton::PlayPause) {
        play_x = Some(btn_x + w / 2);
      }
      btn_x += w + 1;
    }
    let play_x = play_x.unwrap();
    handle_mouse(click_event(play_x, playbar.y + 1), &mut app);
    assert!(app.is_loading);
    // The title row is a button: any cell except the [ ] window opens the
    // device selector, the window toggles the music view fullscreen.
    app.is_loading = false;
    handle_mouse(click_event(play_x + 2, playbar.y), &mut app);
    assert!(app.is_loading);
    // The device selector route is pushed by the backend asynchronously.
    app.pop_navigation_stack();
    let ctx = app.current_playback_context.as_ref().unwrap();
    let title = build_playbar_title(
      if ctx.is_playing { "Playing" } else { "Paused" },
      &ctx.device.name,
    );
    // The [ ] button is three cells: any of them toggles the music view.
    let glyph_right = playbar.x + 1 + title.width() as u16;
    for cell in [glyph_right - 1, glyph_right - 2, glyph_right - 3] {
      handle_mouse(click_event(cell, playbar.y), &mut app);
      assert_eq!(app.get_current_route().id, crate::app::RouteId::MusicView);
      handle_mouse(click_event(cell, playbar.y), &mut app);
      assert_ne!(app.get_current_route().id, crate::app::RouteId::MusicView);
    }
    // One cell before the button opens the device selector instead.
    app.is_loading = false;
    handle_mouse(click_event(glyph_right - 4, playbar.y), &mut app);
    assert!(app.is_loading);
  }

  #[test]
  fn playlist_filter_click_maps_to_correct_track() {
    // When the in-playlist search filter is active, clicking a displayed row
    // must set selected_index to the ORIGINAL track index, not the display row.
    let mut app = track_table_app(6);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    // Rename tracks so the filter picks a non-contiguous subset.
    // Only tracks 2 and 4 contain "x" → filter "x" shows 2 rows.
    app.track_table.tracks[0].name = "one".to_string();
    app.track_table.tracks[1].name = "two".to_string();
    app.track_table.tracks[2].name = "xenon".to_string();
    app.track_table.tracks[3].name = "three".to_string();
    app.track_table.tracks[4].name = "x-ray".to_string();
    app.track_table.tracks[5].name = "five".to_string();

    // Activate the filter: only tracks 2,4 match "x".
    app.playlist_filter = Some("x".to_string());
    assert!(app.playlist_search_active());
    // Filtered display: row 0 = track 2 (xenon), row 1 = track 4 (x-ray).
    // chunk.y=6, first data row is chunk.y+2=8; display row 0 is y=8.
    handle_mouse(click_event(150, 8), &mut app);
    assert_eq!(app.track_table.selected_index, 2);
    // After click the filter is cleared (to let Enter play the song), so
    // re-activate it for the second click.
    app.playlist_filter = Some("x".to_string());
    // Click display row 1 → should select track 4 (x-ray).
    handle_mouse(click_event(150, 9), &mut app);
    assert_eq!(app.track_table.selected_index, 4);
  }

  #[test]
  fn escape_from_music_view_restores_the_previous_route() {
    let mut app = track_table_app(5);
    let before = app.get_current_route().active_block;
    app.push_navigation_stack(
      crate::app::RouteId::MusicView,
      crate::app::ActiveBlock::MusicView,
    );
    assert_eq!(app.get_current_route().id, crate::app::RouteId::MusicView);
    handle_app(Key::Esc, &mut app);
    // ESC pops the overlay, restoring the exact route (and active block)
    // that was on top before the tab view opened — not a black Empty state.
    assert_eq!(app.get_current_route().id, crate::app::RouteId::TrackTable);
    assert_eq!(app.get_current_route().active_block, before);
    assert_ne!(
      app.get_current_route().active_block,
      crate::app::ActiveBlock::Empty
    );
  }
}
