use crate::app::App;
use ratatui::{
  layout::Rect,
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
  Frame,
};
use rspotify::model::PlayableItem;
use rspotify::prelude::Id;
use unicode_width::UnicodeWidthStr;

pub struct MusicViewRects {
  pub left: Rect,
  pub right: Rect,
  pub lyrics: Rect,
  pub visualizer: Rect,
  pub cover: Rect,
  pub details: Rect,
  pub cover_inner: Rect,
}

/// Compute all 4 Music View panel rectangles from `App`'s optional split
/// overrides (shared between rendering, Kitty cover placement, and mouse drag).
pub fn music_view_rects(app: &App, area: Rect) -> MusicViewRects {
  let left_pct = app.music_left_pct.unwrap_or(70).clamp(30, 85);
  let left_w = ((area.width as u32 * left_pct as u32) / 100) as u16;
  let left = Rect::new(area.x, area.y, left_w, area.height);
  let right = Rect::new(
    area.x.saturating_add(left_w),
    area.y,
    area.width.saturating_sub(left_w),
    area.height,
  );

  let lyrics_pct = app.music_lyrics_pct.unwrap_or(70).clamp(20, 85);
  let lyrics_h = ((left.height as u32 * lyrics_pct as u32) / 100) as u16;
  let lyrics = Rect::new(left.x, left.y, left.width, lyrics_h);
  let visualizer = Rect::new(
    left.x,
    left.y.saturating_add(lyrics_h),
    left.width,
    left.height.saturating_sub(lyrics_h),
  );

  let max_cover = right.height.saturating_sub(6).max(6);
  let default_cover = (right.width / 2).clamp(6, right.height.saturating_sub(8).max(6));
  let cover_h = app
    .music_cover_height
    .unwrap_or(default_cover)
    .clamp(6, max_cover)
    .min(right.height);
  let cover = Rect::new(right.x, right.y, right.width, cover_h);
  let details = Rect::new(
    right.x,
    right.y.saturating_add(cover_h),
    right.width,
    right.height.saturating_sub(cover_h),
  );
  let cover_inner = Rect::new(
    cover.x.saturating_add(1),
    cover.y.saturating_add(1),
    cover.width.saturating_sub(2),
    cover.height.saturating_sub(2),
  );

  MusicViewRects {
    left,
    right,
    lyrics,
    visualizer,
    cover,
    details,
    cover_inner,
  }
}

/// Exact inner rectangle of the Cover box in `draw_music_view`,
/// shared with post-draw Kitty graphics placement in `main.rs`.
pub fn cover_inner_rect_for(app: &App, area: Rect) -> Rect {
  music_view_rects(app, area).cover_inner
}

/// Proportional floor-fit of `(img_w, img_h)` pixels into `inner` cells,
/// centered inside `inner` so the terminal never overflows the panel border.
pub fn fit_cover_rect(inner: Rect, img_px: (u32, u32), font_px: (u16, u16)) -> Rect {
  if inner.width == 0 || inner.height == 0 || img_px.0 == 0 || img_px.1 == 0 {
    return Rect::new(inner.x, inner.y, 0, 0);
  }
  let fw = font_px.0.max(1) as f64;
  let fh = font_px.1.max(1) as f64;
  let max_w_px = inner.width as f64 * fw;
  let max_h_px = inner.height as f64 * fh;
  let scale = (max_w_px / img_px.0 as f64).min(max_h_px / img_px.1 as f64);
  let cols = ((img_px.0 as f64 * scale / fw).floor() as u16).clamp(1, inner.width);
  let rows = ((img_px.1 as f64 * scale / fh).floor() as u16).clamp(1, inner.height);
  Rect::new(
    inner.x + inner.width.saturating_sub(cols) / 2,
    inner.y + inner.height.saturating_sub(rows) / 2,
    cols,
    rows,
  )
}

pub fn draw_music_view(f: &mut Frame, app: &App) {
  let area = f.area();
  f.render_widget(Clear, area);
  f.render_widget(
    Block::default().style(Style::default().bg(app.user_config.theme.background)),
    area,
  );
  let rects = music_view_rects(app, area);

  draw_music_lyrics(f, app, rects.lyrics, rects.visualizer);
  draw_music_panel(f, app, &rects);
}

pub(super) fn draw_music_lyrics(f: &mut Frame, app: &App, lyrics_rect: Rect, viz_rect: Rect) {
  let lyrics = app.lyrics.as_deref().unwrap_or(&[]);
  let progress_ms = app.seek_ms.unwrap_or(app.song_progress_ms);
  let current = lyrics
    .iter()
    .rposition(|(ms, _)| *ms <= progress_ms)
    .unwrap_or(0);

  // Fill as much of the Lyrics block vertically as fits.
  let window = (lyrics_rect.height.saturating_sub(2) as usize).max(1);
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
    let inner_h = lyrics_rect.height.saturating_sub(2);
    let pad = inner_h.saturating_sub(lines.len() as u16) / 2;
    if pad > 0 {
      lines.splice(
        0..0,
        std::iter::repeat_with(|| Line::from("")).take(pad as usize),
      );
    }
    // The pad is computed from the full lyrics set, not the visible window,
    // so the text block never shifts as the current line moves.
    let inner_w = lyrics_rect.width.saturating_sub(2) as usize;
    let text_w = lyrics
      .iter()
      .map(|(_, words)| UnicodeWidthStr::width(words.as_str()) + 2)
      .max()
      .unwrap_or(0);
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
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .title("Lyrics by LRCLIB")
    .style(Style::default().fg(app.user_config.theme.inactive));

  let text = Paragraph::new(lines);
  f.render_widget(text.block(block), lyrics_rect);

  draw_music_visualizer(f, app, viz_rect);
}

fn viz_tier_style(row_frac: f32, app: &App) -> Style {
  let color = if row_frac >= 0.85 {
    ratatui::style::Color::Rgb(244, 63, 94)
  } else if row_frac >= 0.68 {
    ratatui::style::Color::Rgb(249, 115, 22)
  } else if row_frac >= 0.48 {
    ratatui::style::Color::Rgb(250, 204, 21)
  } else {
    app.user_config.theme.playbar_progress
  };
  Style::default().fg(color)
}

fn sample_interpolated(app: &App, pos: f32, cap: bool) -> f32 {
  let max_i = crate::visualizer::VIZ_BINS - 1;
  let f = pos.clamp(0.0, 1.0) * max_i as f32;
  let i1 = (f.floor() as usize).min(max_i);
  let i0 = i1.saturating_sub(1);
  let i2 = (i1 + 1).min(max_i);
  let i3 = (i1 + 2).min(max_i);
  let t = f - i1 as f32;
  let get = |idx: usize| {
    if cap {
      app.visualizer.cap(idx)
    } else {
      app.visualizer.smooth(idx)
    }
  };
  let (p0, p1, p2, p3) = (get(i0), get(i1), get(i2), get(i3));
  let t2 = t * t;
  let t3 = t2 * t;
  let v = 0.5
    * ((2.0 * p1)
      + (-p0 + p2) * t
      + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
      + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
  v.clamp(0.0, 1.0)
}

pub(super) fn draw_music_visualizer(f: &mut Frame, app: &App, area: Rect) {
  const SUB_BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
  let title = format!("Visualizer [{}] (v: cycle)", app.visualizer.mode.name());
  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .title(title)
    .style(Style::default().fg(app.user_config.theme.inactive));
  let inner = block.inner(area);
  f.render_widget(block, area);
  if inner.width == 0 || inner.height == 0 || app.visualizer.mode == crate::visualizer::VizMode::Off
  {
    return;
  }
  let cap_style = Style::default()
    .fg(ratatui::style::Color::Rgb(255, 255, 255))
    .add_modifier(Modifier::BOLD);
  let buf = f.buffer_mut();
  let h = inner.height as f32;

  match app.visualizer.mode {
    crate::visualizer::VizMode::Off => {}
    crate::visualizer::VizMode::Wave => {
      // High-resolution 2x4 Braille spectrum curve across the full inner width.
      let px_w = (inner.width as usize) * 2;
      let px_h = (inner.height as usize) * 4;
      let mut grid = vec![0u8; (inner.width as usize) * (inner.height as usize)];
      const BRAILLE_BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
      for px in 0..px_w {
        let pos = if px_w > 1 {
          px as f32 / (px_w - 1) as f32
        } else {
          0.0
        };
        let v = sample_interpolated(app, pos, false);
        let top_py = ((1.0 - v) * (px_h.saturating_sub(1)) as f32).round() as usize;
        let cx = px / 2;
        let sx = px % 2;
        for py in top_py..px_h {
          let cy = py / 4;
          let sy = py % 4;
          grid[cy * inner.width as usize + cx] |= BRAILLE_BITS[sy][sx];
        }
      }
      for cy in 0..inner.height as usize {
        let row_frac = 1.0 - (cy as f32 / h.max(1.0));
        let st = viz_tier_style(row_frac, app);
        for cx in 0..inner.width as usize {
          let bits = grid[cy * inner.width as usize + cx];
          if bits != 0 {
            let ch = char::from_u32(0x2800 + bits as u32).unwrap_or('⣿');
            let mut s = [0u8; 4];
            if let Some(cell) = buf.cell_mut((inner.x + cx as u16, inner.y + cy as u16)) {
              cell.set_symbol(ch.encode_utf8(&mut s)).set_style(st);
            }
          }
        }
      }
    }
    crate::visualizer::VizMode::Mirror => {
      let step = if inner.width >= 64 { 2usize } else { 1usize };
      let bars = (inner.width as usize / step).max(1);
      let total_w = bars * step;
      let start_x = inner.x + inner.width.saturating_sub(total_w as u16) / 2;
      let mid_y = inner.y + inner.height / 2;
      let half_h = (inner.height / 2).max(1) as f32;
      for i in 0..bars {
        let pos = if bars > 1 {
          i as f32 / (bars - 1) as f32
        } else {
          0.0
        };
        let v = sample_interpolated(app, pos, false);
        let span = (v * half_h).round() as u16;
        let bx = start_x + (i * step) as u16;
        for r in 0..=span {
          let frac = (r as f32 / half_h).clamp(0.0, 1.0);
          let st = viz_tier_style(frac, app);
          if let Some(y_up) = mid_y.checked_sub(r) {
            if y_up >= inner.y {
              if let Some(cell) = buf.cell_mut((bx, y_up)) {
                cell.set_symbol("█").set_style(st);
              }
            }
          }
          let y_dn = mid_y + r;
          if y_dn < inner.y + inner.height {
            if let Some(cell) = buf.cell_mut((bx, y_dn)) {
              cell.set_symbol("█").set_style(st);
            }
          }
        }
      }
    }
    crate::visualizer::VizMode::Mosaic => {
      for r in 0..inner.height as usize {
        for dx in 0..inner.width as usize {
          let v = app.visualizer.mosaic_value(r, dx).clamp(0.0, 1.0);
          if v > 0.05 {
            let sym = if v > 0.65 {
              "█"
            } else if v > 0.35 {
              "▓"
            } else {
              "░"
            };
            let st = viz_tier_style(v, app);
            if let Some(cell) = buf.cell_mut((inner.x + dx as u16, inner.y + r as u16)) {
              cell.set_symbol(sym).set_style(st);
            }
          }
        }
      }
    }
    crate::visualizer::VizMode::Led => {
      let bins = crate::visualizer::VIZ_BINS;
      let bar_w = (inner.width as usize / bins).max(1);
      let total_w = (bar_w * bins).min(inner.width as usize);
      let start_x = inner.x + (inner.width.saturating_sub(total_w as u16)) / 2;
      for i in 0..bins {
        let bx = start_x + (i * bar_w) as u16;
        if bx >= inner.x + inner.width {
          break;
        }
        let (body, peak) = app.visualizer.led(i);
        let filled_rows = (body.clamp(0.0, 1.0) * h).round() as u16;
        for r in 0..filled_rows.min(inner.height) {
          let y = inner.y + inner.height - 1 - r;
          let st = viz_tier_style((r as f32 + 1.0) / h.max(1.0), app);
          for dx in 0..bar_w.saturating_sub(1).max(1) {
            if let Some(cell) = buf.cell_mut((bx + dx as u16, y)) {
              cell.set_symbol("▬").set_style(st);
            }
          }
        }
        let cap_row = ((peak.clamp(0.0, 1.0)) * h).floor() as u16;
        if cap_row > 0 && cap_row <= inner.height {
          let y = inner.y + inner.height - cap_row;
          for dx in 0..bar_w.saturating_sub(1).max(1) {
            if let Some(cell) = buf.cell_mut((bx + dx as u16, y)) {
              cell.set_symbol("▀").set_style(cap_style);
            }
          }
        }
      }
    }
    crate::visualizer::VizMode::Bars
    | crate::visualizer::VizMode::Peak
    | crate::visualizer::VizMode::Bricks
    | crate::visualizer::VizMode::Columns
    | crate::visualizer::VizMode::Ascii => {
      // Adaptive bar density: 2-col bar + 1-col gap on wide terminals (up to 48 bars).
      let (bar_w, gap) = if inner.width >= 96 {
        (2usize, 1usize)
      } else if inner.width >= 48 {
        (1usize, 1usize)
      } else {
        (1usize, 0usize)
      };
      let stride = bar_w + gap;
      let bars = ((inner.width as usize + gap) / stride).clamp(1, 48);
      let total_w = bars * stride - gap;
      let start_x = inner.x + inner.width.saturating_sub(total_w as u16) / 2;

      for i in 0..bars {
        let pos = if bars > 1 {
          i as f32 / (bars - 1) as f32
        } else {
          0.0
        };
        let bx = start_x + (i * stride) as u16;
        if bx >= inner.x + inner.width {
          break;
        }
        let val = sample_interpolated(app, pos, false);
        let exact_rows = val * h;
        let full_rows = (exact_rows.floor() as u16).min(inner.height);
        let rem_eighth = ((exact_rows - full_rows as f32) * 8.0).round() as usize;
        let body_sym = match app.visualizer.mode {
          crate::visualizer::VizMode::Bricks => "▄",
          crate::visualizer::VizMode::Ascii => "#",
          _ => "█",
        };
        for r in 0..full_rows {
          let y = inner.y + inner.height - 1 - r;
          let st = viz_tier_style((r as f32 + 1.0) / h.max(1.0), app);
          for dx in 0..bar_w {
            if let Some(cell) = buf.cell_mut((bx + dx as u16, y)) {
              cell.set_symbol(body_sym).set_style(st);
            }
          }
        }
        if full_rows < inner.height
          && rem_eighth > 0
          && matches!(
            app.visualizer.mode,
            crate::visualizer::VizMode::Bars
              | crate::visualizer::VizMode::Peak
              | crate::visualizer::VizMode::Columns
          )
        {
          let y = inner.y + inner.height - 1 - full_rows;
          let st = viz_tier_style((full_rows as f32 + 1.0) / h.max(1.0), app);
          let sym = SUB_BLOCKS[rem_eighth.min(8)];
          for dx in 0..bar_w {
            if let Some(cell) = buf.cell_mut((bx + dx as u16, y)) {
              cell.set_symbol(sym).set_style(st);
            }
          }
        }
        if app.visualizer.mode == crate::visualizer::VizMode::Peak {
          let cap_val = sample_interpolated(app, pos, true);
          let cap_row = (cap_val * h).floor() as u16;
          if cap_row > 0 && cap_row <= inner.height {
            let y = inner.y + inner.height - cap_row;
            for dx in 0..bar_w {
              if let Some(cell) = buf.cell_mut((bx + dx as u16, y)) {
                cell.set_symbol("▔").set_style(cap_style);
              }
            }
          }
        }
      }
    }
  }
}

pub(super) fn draw_music_panel(f: &mut Frame, app: &App, rects: &MusicViewRects) {
  let cover_inner = rects.cover_inner;
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
        let title_str = if track.explicit {
          format!("🅴 {}", track.name)
        } else {
          track.name.clone()
        };
        lines.push(Line::from(Span::styled(
          title_str,
          Style::default().fg(app.user_config.theme.selected),
        )));
        lines.push(Line::from(Span::styled(
          artists,
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(Span::styled(
          format!("Album: {}", album),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        if let Some(album_url) = track
          .album
          .external_urls
          .get("spotify")
          .cloned()
          .or_else(|| {
            track
              .album
              .id
              .as_ref()
              .map(|id| format!("https://open.spotify.com/album/{}", id.id()))
          })
        {
          lines.push(Line::from(Span::styled(
            album_url,
            Style::default().fg(app.user_config.theme.inactive),
          )));
        }
        if let Some(rel) = track.album.release_date.as_deref() {
          lines.push(Line::from(Span::styled(
            format!("Released: {}  Track #{}", rel, track.track_number),
            Style::default().fg(app.user_config.theme.inactive),
          )));
        }
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
          format!("Device: {}", context.device.name.as_str()),
          Style::default().fg(app.user_config.theme.inactive),
        )));
        lines.push(Line::from(""));
        if let Some(credits) = &app.track_credits {
          for credit in credits {
            lines.push(Line::from(Span::styled(
              credit.as_str(),
              Style::default().fg(app.user_config.theme.inactive),
            )));
          }
        }
        if let Some(tm) = &app.track_meta {
          let mut parts = Vec::new();
          if let Some(l) = tm.listeners {
            parts.push(format!(
              "Listeners: {}",
              crate::metadata::format_compact_count(l)
            ));
          }
          if let Some(p) = tm.playcount {
            parts.push(format!(
              "Plays: {}",
              crate::metadata::format_compact_count(p)
            ));
          }
          if !parts.is_empty() {
            lines.push(Line::from(Span::styled(
              parts.join("  |  "),
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
            lines.push(Line::from(Span::styled(
              text,
              Style::default().fg(app.user_config.theme.inactive),
            )));
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
        let ep_url = episode
          .external_urls
          .get("spotify")
          .cloned()
          .unwrap_or_else(|| format!("https://open.spotify.com/episode/{}", episode.id.id()));
        lines.push(Line::from(Span::styled(
          ep_url,
          Style::default().fg(app.user_config.theme.inactive),
        )));
        if let Some(tm) = &app.track_meta {
          let mut parts = Vec::new();
          if let Some(l) = tm.listeners {
            parts.push(format!(
              "Listeners: {}",
              crate::metadata::format_compact_count(l)
            ));
          }
          if let Some(p) = tm.playcount {
            parts.push(format!(
              "Plays: {}",
              crate::metadata::format_compact_count(p)
            ));
          }
          if !parts.is_empty() {
            lines.push(Line::from(Span::styled(
              parts.join("  |  "),
              Style::default().fg(app.user_config.theme.inactive),
            )));
          }
        }
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
  let cover_rect = rects.cover;
  let details_rect = rects.details;

  let cover_block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .title("Cover")
    .style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(cover_block, cover_rect);

  let current_id = crate::cover_art::now_playing(
    app
      .current_playback_context
      .as_ref()
      .and_then(|c| c.item.as_ref()),
  )
  .map(|(id, _)| id);

  if !app.cover_caps.kitty {
    if let (Some(want_id), Some((have_id, img))) = (current_id, &app.cover_art) {
      if &want_id == have_id && cover_inner.width > 0 && cover_inner.height > 0 {
        let img_area = fit_cover_rect(
          cover_inner,
          (img.width(), img.height()),
          app.cover_caps.font_size,
        );
        crate::cover_art::draw_halfblock(f, img_area, img);
      }
    }
  }

  let details_title = if app
    .music_copied_until
    .is_some_and(|t| t > std::time::Instant::now())
  {
    "Track Details — Copied link!"
  } else {
    "Track Details (click to copy link)"
  };
  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .title(details_title)
    .style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(
    Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
    details_rect,
  );
}
