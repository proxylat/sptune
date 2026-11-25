use super::layout;
use super::layout::{
  build_playbar_controls, build_playbar_title, create_artist_string, get_color, millis_to_minutes,
  repeat_label, PlaybarButton, PLAYBAR_HEART_WIDTH, PLAYBAR_TIME_LEN, VOLUME_BAR_LEN,
};
use crate::app::{ActiveBlock, App};
use ratatui::{
  layout::{Alignment, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, BorderType, Borders, Paragraph},
  Frame,
};
use rspotify::model::PlayableItem;
use rspotify::model::RepeatState;
use std::cell::RefCell;

type ThumbGrid = [[(u8, u8, u8); 8]; 8];

thread_local! {
  static PLAYBAR_THUMB: RefCell<Option<(String, ThumbGrid)>> = const { RefCell::new(None) };
}

fn draw_playbar_cover(f: &mut Frame, app: &App, thumb_rect: Rect) {
  if app.cover_caps.kitty {
    return;
  }
  let Some((id, img)) = &app.cover_art else {
    return;
  };
  let grid = PLAYBAR_THUMB.with(|cell| {
    let mut cache = cell.borrow_mut();
    if let Some((cached_id, grid)) = cache.as_ref() {
      if cached_id == id {
        return *grid;
      }
    }
    let small = img
      .resize_exact(8, 8, image::imageops::FilterType::Lanczos3)
      .to_rgb8();
    let mut grid = [[(0u8, 0u8, 0u8); 8]; 8];
    for (y, row) in grid.iter_mut().enumerate() {
      for (x, px) in row.iter_mut().enumerate() {
        let p = small.get_pixel(x as u32, y as u32).0;
        *px = (p[0], p[1], p[2]);
      }
    }
    *cache = Some((id.clone(), grid));
    grid
  });
  let buf = f.buffer_mut();
  for row in 0..thumb_rect.height.min(4) {
    let py_top = (row * 2) as usize;
    let py_bot = py_top + 1;
    for col in 0..thumb_rect.width.min(8) {
      let x = thumb_rect.x + col;
      let y = thumb_rect.y + row;
      if let Some(cell) = buf.cell_mut((x, y)) {
        let top = grid[py_top][col as usize];
        let bot = grid[py_bot][col as usize];
        cell
          .set_symbol("▀")
          .set_fg(Color::Rgb(top.0, top.1, top.2))
          .set_bg(Color::Rgb(bot.0, bot.1, bot.2));
      }
    }
  }
}

pub fn draw_playbar(f: &mut Frame, app: &App, layout_chunk: Rect) {
  // Empty state (nothing playing): the block carries exactly one button —
  // "▶ Resume" to restart the previous session's last track — or a device
  // hint when there is nothing saved to resume.
  let has_track = app
    .current_playback_context
    .as_ref()
    .and_then(|c| c.item.as_ref())
    .is_some();
  if !has_track {
    let block = Block::default()
      .borders(Borders::ALL)
      .border_type(if app.user_config.terminal.unicode_borders {
        BorderType::Rounded
      } else {
        BorderType::Plain
      })
      .border_style(Style::default().fg(app.user_config.theme.playbar_background));
    f.render_widget(block, layout_chunk);
    // Live state not known yet (startup): never offer Resume — the user
    // may have music playing elsewhere and the first poll hasn't landed.
    if !app.playback_known {
      let hint = Paragraph::new(Span::styled(
        "Connecting…",
        Style::default().fg(app.user_config.theme.inactive),
      ))
      .alignment(Alignment::Center);
      f.render_widget(hint, layout_chunk);
      return;
    }
    if app.saved_last_track.is_some() {
      let rect = layout::playbar_resume_rect(layout_chunk);
      let button = Paragraph::new(Span::styled(
        layout::playbar_resume_label(),
        Style::default()
          .fg(app.user_config.theme.playbar_progress)
          .add_modifier(Modifier::BOLD),
      ));
      f.render_widget(button, rect);
    } else {
      let hint = Paragraph::new(Span::styled(
        "Nothing playing — press d to select a device",
        Style::default().fg(app.user_config.theme.inactive),
      ))
      .alignment(Alignment::Center);
      f.render_widget(hint, layout_chunk);
    }
    return;
  }
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

      let max_title_w = if app.show_process_suspend {
        layout::playbar_title_max_width(layout_chunk.width, app.processes_suspended)
      } else {
        layout_chunk.width.saturating_sub(2) as usize
      };
      let title = build_playbar_title(
        play_title,
        &current_playback_context.device.name,
        max_title_w,
      );
      let title = if app.show_process_suspend {
        format!(
          "{}──{}",
          title,
          layout::playbar_processes_label(app.processes_suspended)
        )
      } else {
        title
      };
      let border_style = get_color(highlight_state, theme);
      let title_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if app.user_config.terminal.unicode_borders {
          BorderType::Rounded
        } else {
          BorderType::Plain
        })
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

      let cover_off = layout::playbar_cover_offset(layout_chunk, app.cover_art.is_some());
      if cover_off > 0 && layout_chunk.height >= 4 {
        draw_playbar_cover(
          f,
          app,
          Rect::new(
            layout_chunk.x + 1,
            layout_chunk.y + 1,
            layout::PLAYBAR_COVER_COLS,
            layout_chunk.height.saturating_sub(2).min(4),
          ),
        );
      }

      // Transport buttons (shuffle, prev, play/pause, next, repeat)
      // dead-centered on the first inner row, just above the music bar. The
      // song name shares this row, left of the buttons (truncated so it
      // never overlaps).
      let controls = build_playbar_controls(current_playback_context.is_playing);
      let repeat_text = repeat_label(current_playback_context.repeat_state);
      let controls_row = layout::playbar_controls_row(layout_chunk);
      let controls_start = layout::playbar_controls_x(layout_chunk, &controls);
      let mut song_row = layout::playbar_song_row(layout_chunk);
      song_row.x = song_row.x.saturating_add(cover_off);
      song_row.width = song_row.width.saturating_sub(cover_off);
      let mut artist_row = layout::playbar_artist_row(layout_chunk);
      artist_row.x = artist_row.x.saturating_add(cover_off);
      artist_row.width = artist_row.width.saturating_sub(cover_off);
      // Room for the [+ / ✓] prefix plus the liked heart; both fixed-width.
      let name_right = if song_row.y == controls_row.y {
        controls_start
      } else {
        song_row.x + song_row.width
      };
      let name_limit = name_right.saturating_sub(song_row.x + 1 + 2 + PLAYBAR_HEART_WIDTH) as usize;
      let name_text: String = if track_name.chars().count() > name_limit {
        track_name
          .chars()
          .take(name_limit.saturating_sub(1))
          .chain(std::iter::once('…'))
          .collect()
      } else {
        track_name
      };
      // Off-state icons use the dim gray, never the playbar background
      // color: background-as-foreground is invisible on dark terminals
      // (shuffle/repeat looked "hidden" / "disappearing").
      let dim_style = Style::default().fg(theme.inactive);
      let accent_style = Style::default().fg(theme.playbar_progress);
      let repeat_style = match current_playback_context.repeat_state {
        RepeatState::Off => dim_style,
        RepeatState::Context => accent_style,
        RepeatState::Track => accent_style.add_modifier(Modifier::BOLD),
      };
      let mut spans: Vec<Span> = vec![Span::raw(
        " ".repeat(controls_start.saturating_sub(controls_row.x) as usize),
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
        let fill_style = Style::default().fg(theme.playbar_progress);
        let mut vol_spans: Vec<Span> = vec![Span::styled(
          "♪ ",
          Style::default().fg(theme.playbar_progress_text),
        )];
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

      let bar_rect = layout::playbar_progress_rect(layout_chunk);

      // Song name with playlist membership prefix: ✓ if in any playlist else + (clickable to add)
      let name_style = Style::default()
        .fg(theme.selected)
        .add_modifier(Modifier::BOLD);
      let uri = app.playing_track_uri();
      let in_playlist = uri
        .as_deref()
        .map(|u| app.is_in_any_playlist(u))
        .unwrap_or(false);
      let prefix = if in_playlist {
        app.user_config.behavior.in_playlist_icon.clone() + " "
      } else {
        "+ ".to_string()
      };
      // prefix is 2 cols, heart is PLAYBAR_HEART_WIDTH cols, keep truncation inside song_row
      let prefix_style = Style::default()
        .fg(theme.active)
        .add_modifier(Modifier::BOLD);
      // Liked heart next to the prefix (blank for episodes): filled when the
      // now-playing track is liked, outline otherwise. Fixed width always.
      let (heart_text, heart_style) = match track_item {
        PlayableItem::Track(track) => {
          let liked = track
            .id
            .as_ref()
            .map(|id| app.liked_song_ids_set.contains(&id.to_string()))
            .unwrap_or(false);
          if liked {
            (
              format!("{} ", app.user_config.behavior.liked_icon),
              Style::default()
                .fg(theme.active)
                .add_modifier(Modifier::BOLD),
            )
          } else {
            ("♡ ".to_string(), Style::default().fg(theme.inactive))
          }
        }
        _ => ("  ".to_string(), Style::default()),
      };
      f.render_widget(
        Paragraph::new(Line::from(vec![
          Span::styled(prefix, prefix_style),
          Span::styled(heart_text, heart_style),
          Span::styled(name_text, name_style),
        ])),
        song_row,
      );

      // Artist name (smaller: italic) on artist_row left of controls,
      // and the centered progress bar on the bottom row (bar_row).
      let artist_style = Style::default()
        .fg(theme.inactive)
        .add_modifier(Modifier::ITALIC);
      let artist_right = if artist_row.y == controls_row.y {
        controls_start
      } else if let Some(bar) = bar_rect {
        bar.x.saturating_sub(PLAYBAR_TIME_LEN)
      } else {
        artist_row.x + artist_row.width
      };
      let artist_limit = artist_right.saturating_sub(artist_row.x + 1) as usize;
      let artist_text: String = if artists.chars().count() > artist_limit {
        artists
          .chars()
          .take(artist_limit.saturating_sub(1))
          .chain(std::iter::once('…'))
          .collect()
      } else {
        artists
      };
      let artist_draw_rect = Rect::new(
        artist_row.x,
        artist_row.y,
        artist_right.saturating_sub(artist_row.x),
        1,
      );
      f.render_widget(
        Paragraph::new(Line::from(Span::styled(artist_text, artist_style))),
        artist_draw_rect,
      );

      if let Some(bar) = bar_rect {
        let bar_start: u16 = bar.x.saturating_sub(PLAYBAR_TIME_LEN);
        let progress_ms = match app.seek_ms {
          Some(seek_ms) => seek_ms,
          None => app.song_progress_ms,
        };

        let fill_style = Style::default().fg(theme.playbar_progress);
        let bar_len = bar.width as usize;
        // Sub-cell eighth blocks (smaller █): 61 cells / 180s ≈ 2.7 eighths
        // per second, so every second shows its own step without extending
        // the bar; floor keeps the fill edges on clean cell boundaries.
        let denom = u128::from(duration_ms).max(1);
        let eighths = ((progress_ms.min(denom) * bar_len as u128 * 8) / denom) as usize;
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
        let bar_draw_rect = Rect::new(
          bar_start,
          bar.y,
          bar.width.saturating_add(PLAYBAR_TIME_LEN * 2),
          1,
        );
        f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_draw_rect);
      }
    }
  }
}
