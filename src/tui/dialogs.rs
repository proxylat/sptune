use super::layout::get_color;
use super::sidebar::TimestampGroups;
use super::widgets::draw_scrollbar;
use crate::app::{ActiveBlock, App, DialogContext};
use ratatui::{
  layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
  style::{Modifier, Style},
  text::{Line, Span, Text},
  widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
  Frame,
};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

pub(super) fn draw_request_log(f: &mut Frame, app: &App, layout_chunk: Rect) {
  let theme = app.user_config.theme;
  let inactive = Style::default().fg(theme.inactive);

  // Throttle info header (top 4 rows).
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(4), Constraint::Min(0)])
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
  let live_tokens = match app.api_tokens_updated {
    Some(t) => (app.api_tokens + t.elapsed().as_secs_f64() * crate::backend::API_RATE_PER_SEC)
      .clamp(0.0, crate::backend::API_BURST),
    None => app.api_tokens,
  };
  let display_used =
    (crate::backend::API_BURST - live_tokens).clamp(0.0, crate::backend::API_BURST);
  let first_draw_line = match app.first_draw_ms {
    Some(ms) => format!("First draw: {}ms", ms),
    None => "First draw: pending".to_string(),
  };
  let throttle_text = format!(
    "{:.1} \\ {:.1}\n{}\n{}\n{}",
    display_used,
    crate::backend::API_BURST,
    backoff_line,
    load_more_line,
    first_draw_line
  );
  f.render_widget(Paragraph::new(throttle_text).style(inactive), chunks[0]);

  // Request log list — grouped by HH:MM, deduped globally. Memoized per
  // (request_log_gen, len) so redraws don't rebuild HashMap/BTreeMap every frame.
  thread_local! {
    static REQ_GROUPS_CACHE: std::cell::RefCell<((u64, usize), TimestampGroups)> =
      const { std::cell::RefCell::new(((u64::MAX, 0), Vec::new())) };
  }
  let cache_key = (app.request_log_gen, app.request_log.len());
  let timestamp_style = Style::default()
    .fg(theme.inactive)
    .add_modifier(Modifier::DIM);
  let name_style = Style::default().fg(theme.text).add_modifier(Modifier::BOLD);
  let available = chunks[1].height.saturating_sub(2) as usize;
  let (visible, group_count): (Vec<ListItem>, usize) = REQ_GROUPS_CACHE.with(|cell| {
    let mut cached = cell.borrow_mut();
    if cached.0 != cache_key {
      let mut by_text: HashMap<&str, (u32, Instant)> = HashMap::new();
      for e in &app.request_log {
        let entry = by_text.entry(e.text.as_str()).or_insert((0, e.instant));
        entry.0 = entry.0.saturating_add(e.count);
        if e.instant > entry.1 {
          entry.1 = e.instant;
        }
      }
      let now_local = chrono::Local::now();
      let mut by_hhmm: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
      let mut hhmm_max: HashMap<String, Instant> = HashMap::new();
      for (text, (count, latest)) in by_text {
        let hhmm = (now_local
          - chrono::Duration::from_std(latest.elapsed()).unwrap_or(chrono::Duration::zero()))
        .format("%H:%M")
        .to_string();
        by_hhmm
          .entry(hhmm.clone())
          .or_default()
          .push((text.to_string(), count));
        let m = hhmm_max.entry(hhmm).or_insert(latest);
        if latest > *m {
          *m = latest;
        }
      }
      let mut groups: TimestampGroups = by_hhmm
        .into_iter()
        .map(|(hhmm, mut v)| {
          v.sort_by(|a, b| a.0.cmp(&b.0));
          let max_i = hhmm_max.remove(&hhmm).unwrap_or(Instant::now());
          (hhmm, v, max_i)
        })
        .collect();
      groups.sort_by_key(|song| std::cmp::Reverse(song.2));
      *cached = (cache_key, groups);
    }

    let groups = &cached.1;
    let mut visible: Vec<ListItem> = Vec::new();
    let mut used = 0usize;
    for (hhmm, names, _) in groups {
      let h = 1 + names.len();
      if available > 0 && used + h > available && !visible.is_empty() {
        break;
      }
      let mut lines = Vec::with_capacity(h);
      lines.push(Line::from(Span::styled(hhmm.clone(), timestamp_style)));
      for (text, count) in names {
        let name = if *count > 1 {
          format!("{}({})", text, count)
        } else {
          text.clone()
        };
        lines.push(Line::from(Span::styled(name, name_style)));
      }
      visible.push(ListItem::new(Text::from(lines)));
      used += h;
      if available > 0 && used >= available {
        break;
      }
    }
    if visible.is_empty() && !groups.is_empty() && available == 0 {
      let (hhmm, names, _) = &groups[0];
      let mut lines = vec![Line::from(Span::styled(hhmm.clone(), timestamp_style))];
      for (text, count) in names.iter().take(1) {
        let name = if *count > 1 {
          format!("{}({})", text, count)
        } else {
          text.clone()
        };
        lines.push(Line::from(Span::styled(name, name_style)));
      }
      visible.push(ListItem::new(Text::from(lines)));
    }
    (visible, groups.len())
  });
  let viewport = visible.len().max(1);
  let highlight_state = (false, false);
  let focused = highlight_state.0 || highlight_state.1;
  let mut state = ListState::default();
  state.select(None);
  let list = List::new(visible)
    .block(
      Block::default()
        .title(Span::styled(
          "Requests - Clear",
          get_color(highlight_state, theme),
        ))
        .borders(Borders::ALL)
        .border_type(if app.user_config.terminal.unicode_borders {
          BorderType::Rounded
        } else {
          BorderType::Plain
        })
        .border_style(get_color(highlight_state, theme)),
    )
    .style(Style::default().fg(theme.text))
    .highlight_style(if focused {
      get_color(highlight_state, theme).add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.text)
    });
  f.render_stateful_widget(list, chunks[1], &mut state);
  draw_scrollbar(f, app, chunks[1], group_count, viewport, 0);
}

pub fn draw_error_screen(f: &mut Frame, app: &App) {
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(100)])
    .margin(5)
    .split(f.area());

  let errors_text = app.api_errors_text();
  let error_style = Style::default().fg(app.user_config.theme.error_text);
  // One Line per text line: embedded newlines inside a single Line are
  // ignored by the renderer, which collapsed the whole history into one run.
  let mut playing_text: Vec<Line> = errors_text
    .lines()
    .map(|l| Line::from(Span::styled(l.to_string(), error_style)))
    .collect();
  if let Some(first) = playing_text.first_mut() {
    first.spans.insert(0, Span::raw("Api response: "));
  }
  playing_text.extend([
    Line::from(Span::styled(
      "If you are trying to play a track, please make sure:",
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
    ),
  ]);

  let playing_paragraph = Paragraph::new(playing_text)
    .wrap(Wrap { trim: true })
    .style(Style::default().fg(app.user_config.theme.text))
    .block(
      Block::default()
        .borders(Borders::ALL)
        .border_type(if app.user_config.terminal.unicode_borders {
          BorderType::Rounded
        } else {
          BorderType::Plain
        })
        .title(Span::styled(
          "Error",
          Style::default().fg(app.user_config.theme.error_border),
        ))
        .border_style(Style::default().fg(app.user_config.theme.error_border)),
    );
  f.render_widget(playing_paragraph, chunks[0]);
}

pub fn draw_device_list(f: &mut Frame, app: &App) {
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
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
        .border_type(if app.user_config.terminal.unicode_borders {
          BorderType::Rounded
        } else {
          BorderType::Plain
        })
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

pub(super) fn draw_dialog(f: &mut Frame, app: &App) {
  if let ActiveBlock::Dialog(context) = app.get_current_route().active_block {
    match context {
      DialogContext::SeekTime => draw_seek_dialog(f, app),
      DialogContext::AddToPlaylist => draw_add_to_playlist_dialog(f, app),
      DialogContext::ConfirmRemoveFromPlaylist => draw_confirm_remove_dialog(f, app),
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
            .border_type(if app.user_config.terminal.unicode_borders {
              BorderType::Rounded
            } else {
              BorderType::Plain
            })
            .border_style(Style::default().fg(app.user_config.theme.inactive));

          f.render_widget(block, rect);

          let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
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
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
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

pub(super) fn draw_confirm_remove_dialog(f: &mut Frame, app: &App) {
  let bounds = f.area();
  let width = std::cmp::min(bounds.width - 2, 65);
  let height = 8;
  let left = (bounds.width - width) / 2;
  let top = bounds.height / 4;
  let rect = Rect::new(left, top, width, height);
  f.render_widget(Clear, rect);
  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .border_style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(block, rect);
  let vchunks = Layout::default()
    .direction(Direction::Vertical)
    .margin(2)
    .constraints([Constraint::Min(3), Constraint::Length(3)])
    .split(rect);
  let track = app
    .pending_remove_track_name
    .as_deref()
    .unwrap_or("this song");
  let playlist = app
    .pending_remove_playlist_name
    .as_deref()
    .unwrap_or("this playlist");
  let text = vec![
    Line::from(Span::raw(format!(
      "Are you sure you want to remove \"{}\"",
      track
    ))),
    Line::from(Span::raw(format!("from the playlist \"{}\"?", playlist))),
  ];
  let paragraph = Paragraph::new(text)
    .wrap(Wrap { trim: true })
    .alignment(Alignment::Center);
  f.render_widget(paragraph, vchunks[0]);
  let hchunks = Layout::default()
    .direction(Direction::Horizontal)
    .horizontal_margin(3)
    .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
    .split(vchunks[1]);
  // No is left, Yes is right. confirm==true means Yes is hovered.
  let no = Paragraph::new(Span::raw("No"))
    .style(Style::default().fg(if app.confirm {
      app.user_config.theme.inactive
    } else {
      app.user_config.theme.hovered
    }))
    .alignment(Alignment::Center);
  f.render_widget(no, hchunks[0]);
  let yes = Paragraph::new(Span::raw("Yes"))
    .style(Style::default().fg(if app.confirm {
      app.user_config.theme.hovered
    } else {
      app.user_config.theme.inactive
    }))
    .alignment(Alignment::Center);
  f.render_widget(yes, hchunks[1]);
}

// Playlist picker: the user's own playlists, arrows to move, Enter to add
// the captured track, q/Esc to cancel.
pub(super) fn draw_add_to_playlist_dialog(f: &mut Frame, app: &App) {
  let bounds = f.area();
  let width = std::cmp::min(bounds.width - 2, 45);
  let items = app.playlists.as_ref().map_or(0, |p| p.items.len());
  let count = items + 1; // trailing "+ New playlist" row
  let height = std::cmp::min(count.saturating_add(6), bounds.height as usize - 2) as u16;
  let left = (bounds.width - width) / 2;
  let top = bounds.height / 4;
  let rect = Rect::new(left, top, width, height);

  f.render_widget(Clear, rect);

  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
    .title(Span::styled(
      " Add to playlist ",
      Style::default().fg(app.user_config.theme.active),
    ))
    .border_style(Style::default().fg(app.user_config.theme.inactive));
  f.render_widget(block, rect);

  // Keep the selection visible in the panel viewport.
  let viewport = height.saturating_sub(4) as usize;
  let offset = app.playlist_picker_index.saturating_sub(viewport / 2);

  let mut lines: Vec<Line> = app
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
            if selected {
              // BOLD like every other list: color alone is invisible on
              // terminals where selected ≈ text.
              Style::default()
                .fg(app.user_config.theme.selected)
                .add_modifier(Modifier::BOLD)
            } else {
              Style::default().fg(app.user_config.theme.text)
            },
          ))
        })
        .collect()
    })
    .unwrap_or_default();
  // Trailing "+ New playlist" row (index == items). In name-entry mode it
  // shows the typed name with a block cursor.
  if offset <= items && items < offset + viewport {
    let selected = app.playlist_picker_index == items;
    let label = if app.naming_playlist {
      format!("> {}▌", app.new_playlist_name)
    } else {
      "+ New playlist".to_string()
    };
    let style = if selected {
      Style::default()
        .fg(app.user_config.theme.selected)
        .add_modifier(Modifier::BOLD)
    } else if app.naming_playlist {
      Style::default().fg(app.user_config.theme.active)
    } else {
      Style::default().fg(app.user_config.theme.text)
    };
    lines.push(Line::from(Span::styled(label, style)));
  }

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

  let hint_text = if app.naming_playlist {
    "type name   Enter create   Esc back"
  } else {
    "↑/↓ pick   Enter add   q cancel"
  };
  let hint = Paragraph::new(Line::from(Span::styled(
    hint_text,
    Style::default().fg(app.user_config.theme.inactive),
  )))
  .alignment(Alignment::Center);
  f.render_widget(
    hint,
    Rect::new(rect.x + 2, rect.y + rect.height - 2, rect.width - 4, 1),
  );
}

pub(super) fn draw_seek_dialog(f: &mut Frame, app: &App) {
  let bounds = f.area();
  let width = std::cmp::min(bounds.width - 2, 45);
  let height = 8;
  let left = (bounds.width - width) / 2;
  let top = bounds.height / 4;

  let rect = Rect::new(left, top, width, height);

  f.render_widget(Clear, rect);

  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(if app.user_config.terminal.unicode_borders {
      BorderType::Rounded
    } else {
      BorderType::Plain
    })
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
