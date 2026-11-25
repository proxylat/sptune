use super::help::get_help_docs;
use super::layout;
use super::widgets::draw_scrollbar;
use crate::app::App;
use crate::user_config::theme_presets;
use ratatui::{
  layout::Constraint,
  style::{Color, Modifier, Style},
  text::Span,
  widgets::{Block, BorderType, Borders, List, ListItem, Row, Table},
  Frame,
};

pub fn draw_help_menu(f: &mut Frame, app: &App) {
  let help_menu_style = Style::default().fg(app.user_config.theme.text);
  if app.help_show_shortcuts {
    let rect = layout::help_full_rect(f.area());
    // cache formatted header, avoids 3 allocs per frame
    static HELP_HEADER: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let header = HELP_HEADER
      .get_or_init(|| {
        vec![format!(
          "{:50}{:40}{:20}",
          "Description", "Event", "Context"
        )]
      })
      .clone();
    let format_row =
      |r: Vec<String>| -> Vec<String> { vec![format!("{:50}{:40}{:20}", r[0], r[1], r[2])] };
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
          .border_type(if app.user_config.terminal.unicode_borders {
            BorderType::Rounded
          } else {
            BorderType::Plain
          })
          .style(help_menu_style)
          .title(Span::styled("Shortcuts (Esc to go back)", help_menu_style))
          .border_style(help_menu_style),
      )
      .style(help_menu_style)
      .widths([Constraint::Percentage(100)]);
    f.render_widget(help_menu, rect);
    let viewport = rect.height.saturating_sub(3) as usize;
    draw_scrollbar(
      f,
      app,
      rect,
      app.help_docs_size as usize,
      viewport,
      app.help_scroll_offset as usize,
    );
  } else {
    let rect = layout::help_full_rect(f.area());
    let all_rows = settings_rows_text(app, &help_menu_style);
    let count = all_rows.len();
    let viewport = rect.height.saturating_sub(2) as usize;
    let offset = (app.help_scroll_offset as usize).min(count.saturating_sub(viewport.min(count)));
    let visible = all_rows
      .into_iter()
      .skip(offset)
      .take(viewport)
      .collect::<Vec<_>>();
    let settings_list = List::new(visible)
      .block(
        Block::default()
          .borders(Borders::ALL)
          .border_type(if app.user_config.terminal.unicode_borders {
            BorderType::Rounded
          } else {
            BorderType::Plain
          })
          .style(help_menu_style)
          .title(Span::styled(
            "Settings (click row to toggle, Esc close \u{2022} Shortcuts \u{2192} Enter)",
            help_menu_style,
          ))
          .border_style(help_menu_style),
      )
      .style(help_menu_style);
    f.render_widget(settings_list, rect);
    draw_scrollbar(f, app, rect, count, viewport, offset);
  }
}

pub(super) fn settings_rows_text<'a>(app: &App, style: &Style) -> Vec<ListItem<'a>> {
  let black_theme = app.user_config.theme.background == Color::Rgb(0, 0, 0);
  let on_off = |on: bool| if on { "on" } else { "off" };
  // 43 rows: 8 headers + 7 blanks + 27 toggles + Shortcuts link, matching mouse.rs ROW_TO_TOGGLE
  let header_style = Style::default()
    .fg(app.user_config.theme.text)
    .add_modifier(Modifier::BOLD);
  let danger_header_style = Style::default()
    .fg(app.user_config.theme.error_text)
    .add_modifier(Modifier::BOLD);
  let mk_header = |s: &str, st: Style| ListItem::new(Span::styled(s.to_string(), st));
  let mk_row = |s: String| ListItem::new(Span::styled(s, *style));
  let mk_clear = |s: String| {
    ListItem::new(Span::styled(
      s,
      Style::default().fg(app.user_config.theme.error_text),
    ))
  };
  let theme_name = app.theme_preset_index.map_or_else(
    || "Custom".to_string(),
    |i| theme_presets()[i].0.to_string(),
  );
  let mut rows: Vec<ListItem<'a>> = Vec::with_capacity(43);
  rows.push(mk_header("─ Appearance ─", header_style));
  rows.push(mk_row(format!("Black theme: {}", on_off(black_theme))));
  rows.push(mk_row(format!("Theme: {} (P)", theme_name)));
  rows.push(mk_row(format!(
    "Animations: {}",
    on_off(app.user_config.behavior.enable_animations)
  )));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Layout & Columns ─", header_style));
  rows.push(mk_row(format!(
    "Library block: {}",
    on_off(app.show_library)
  )));
  rows.push(mk_row(format!(
    "Playlists block: {}",
    on_off(app.show_playlists)
  )));
  rows.push(mk_row(format!(
    "Column Album: {}",
    on_off(app.user_config.behavior.show_album_column)
  )));
  rows.push(mk_row(format!(
    "Column Artist: {}",
    on_off(app.user_config.behavior.show_artist_column)
  )));
  rows.push(mk_row(format!(
    "Column Length: {}",
    on_off(app.user_config.behavior.show_length_column)
  )));
  rows.push(mk_row(format!(
    "Column Date Added: {}",
    on_off(app.user_config.behavior.show_date_added_column)
  )));
  rows.push(mk_row(format!(
    "Max name length: {}",
    if app.user_config.behavior.max_display_length == 0 {
      "off".to_string()
    } else {
      app.user_config.behavior.max_display_length.to_string()
    }
  )));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Playback & Audio ─", header_style));
  rows.push(mk_row(format!(
    "Volume step: {}%",
    app.user_config.behavior.volume_increment
  )));
  rows.push(mk_row(format!(
    "Timestamp by typing: {}",
    on_off(app.user_config.behavior.seek_by_typing)
  )));
  rows.push(mk_row(format!(
    "Playback poll: {}s",
    app.user_config.behavior.playback_poll_secs
  )));
  rows.push(mk_row(format!(
    "Desktop notifications: {}",
    on_off(app.desktop_notifications)
  )));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Soloist ─", header_style));
  rows.push(mk_row(format!(
    "Soloist headless audio: {}",
    on_off(app.user_config.soloist.enabled)
  )));
  rows.push(mk_row(format!(
    "Keep Soloist running on exit: {}",
    on_off(app.user_config.soloist.keep_running)
  )));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Controls & Tools ─", header_style));
  rows.push(mk_row(format!(
    "Mouse interactions: {} (m)",
    on_off(app.user_config.behavior.enable_mouse)
  )));
  rows.push(mk_row(format!(
    "Add to playlist: {} (a)",
    on_off(app.user_config.behavior.enable_add_to_playlist)
  )));
  rows.push(mk_row(format!(
    "Remove from playlist: {}",
    on_off(app.user_config.behavior.enable_remove_from_playlist)
  )));
  rows.push(mk_row(format!(
    "Suspend/Resume Process: {}",
    on_off(app.show_process_suspend)
  )));
  rows.push(mk_row(format!("Requests View: {}", on_off(app.dev_view))));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Spotify Desktop ─", header_style));
  rows.push(mk_row(format!(
    "Auto launch with sptune: {}",
    on_off(app.user_config.spotify.auto_launch)
  )));
  rows.push(mk_row(format!(
    "Enable Chromium flags: {}",
    on_off(app.user_config.spotify.use_chromium_flags)
  )));
  rows.push(mk_row(format!(
    "Auto suspend child processes: {}",
    on_off(app.user_config.spotify.suspend_children)
  )));
  rows.push(mk_row(format!(
    "Auto trim WorkingSet: {}",
    on_off(app.user_config.spotify.trim_working_set)
  )));
  rows.push(mk_row(format!(
    "Memory limit per child process: {}",
    if app.user_config.spotify.memory_limit_mb == 0 {
      "off".to_string()
    } else {
      format!("{} MB", app.user_config.spotify.memory_limit_mb)
    }
  )));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Navigation ─", header_style));
  rows.push(ListItem::new(Span::styled(
    "Shortcuts \u{2192}".to_string(),
    Style::default()
      .fg(app.user_config.theme.text)
      .add_modifier(Modifier::UNDERLINED),
  )));
  rows.push(ListItem::new(Span::raw("".to_string())));
  rows.push(mk_header("─ Danger Zone ─", danger_header_style));
  rows.push(mk_clear(match app.user_config.keys.clear_cache {
    Some(key) => format!("Clear cache ({})", key),
    None => "Clear cache".to_string(),
  }));
  rows
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn settings_rows_match_layout_count() {
    let app = App::default();
    let rows = settings_rows_text(&app, &Style::default());
    assert_eq!(rows.len(), crate::tui::layout::SETTINGS_ROW_COUNT as usize);
  }
}
