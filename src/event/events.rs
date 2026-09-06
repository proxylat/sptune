use crate::event::Key;
use crossterm::event;
use std::{sync::mpsc, thread, time::Duration};

#[derive(Debug, Clone, Copy)]
/// Configuration for event handling.
pub struct EventConfig {
  /// The tick rate at which the application will sent an tick event.
  pub tick_rate: Duration,
  /// Windows-only calibration for mouse rows: some conhost/ConPTY builds
  /// deliver clicks offset from the drawn rows ("click lower than the
  /// button"). Positive values shift reported rows down. 0 = no correction.
  pub mouse_y_offset: i16,
}

impl Default for EventConfig {
  fn default() -> EventConfig {
    EventConfig {
      tick_rate: Duration::from_millis(250),
      mouse_y_offset: 0,
    }
  }
}

/// An occurred event.
pub enum Event<I> {
  /// An input event occurred.
  Input(I),
  /// An tick event occurred.
  Tick,
}

/// An input event: keyboard or mouse.
pub enum InputEvent {
  Key(Key),
  Mouse(crossterm::event::MouseEvent),
}

/// A small event handler that wrap crossterm input and tick event. Each event
/// type is handled in its own thread and returned to a common `Receiver`
pub struct Events {
  rx: mpsc::Receiver<Event<InputEvent>>,
  // Need to be kept around to prevent disposing the sender side.
  _tx: mpsc::Sender<Event<InputEvent>>,
}

impl Events {
  /// Constructs an new instance of `Events` with the default config.
  pub fn new(tick_rate: u64, mouse_y_offset: i16) -> Events {
    Events::with_config(EventConfig {
      tick_rate: Duration::from_millis(tick_rate),
      mouse_y_offset,
      ..Default::default()
    })
  }

  /// Constructs an new instance of `Events` from given config.
  pub fn with_config(config: EventConfig) -> Events {
    let (tx, rx) = mpsc::channel();

    let event_tx = tx.clone();
    thread::spawn(move || {
      let mut batch: Vec<Event<InputEvent>> = Vec::new();
      loop {
        // poll for tick rate duration, if no event, sent tick event.
        // poll/read failures mean terminal is closing; exit thread instead of panicking.
        let has_event = match event::poll(config.tick_rate) {
          Ok(v) => v,
          Err(_) => {
            if event_tx.send(Event::Tick).is_err() {
              break;
            }
            continue;
          }
        };
        if has_event {
          // Drain everything waiting in one batch. Every event triggers a full
          // redraw, so a busy pointer (drag/move flood) would otherwise queue
          // hundreds of frames and lag the screen by seconds.
          batch.clear();
          // Windows ConPTY/conhost builds can queue two identical Press
          // records for one physical keypress ("ride" -> "rriiddee"); an
          // identical Press read immediately after itself within one drain
          // is synthesized, so drop it. Real typing never lands two presses
          // in the same microsecond drain, and conhost coalesces held-key
          // auto-repeat into a single record, which keeps this safe.
          #[cfg(windows)]
          let mut prev_press: Option<(crossterm::event::KeyCode, crossterm::event::KeyModifiers)> = None;
          loop {
            let ev = match event::read() {
              Ok(ev) => match ev {
                event::Event::Key(key) => {
                  // crossterm 0.29 emits Press+Release; only handle Press/Repeat to avoid doubled chars ("ride" -> "rriiddee")
                  if matches!(
                    key.kind,
                    event::KeyEventKind::Release
                  ) {
                    continue;
                  }
                  #[cfg(windows)]
                  {
                    let id = (key.code, key.modifiers);
                    if matches!(key.kind, event::KeyEventKind::Press) {
                      if prev_press == Some(id) {
                        continue; // synthesized duplicate of the previous read
                      }
                      prev_press = Some(id);
                    } else {
                      prev_press = None;
                    }
                  }
                  Event::Input(InputEvent::Key(Key::from(key)))
                }
                event::Event::Mouse(mut mouse) => {
                  if config.mouse_y_offset != 0 {
                    mouse.row = (mouse.row as i16 + config.mouse_y_offset).max(0) as u16;
                  }
                  Event::Input(InputEvent::Mouse(mouse))
                }
                _ => continue,
              },
              Err(_) => break,
            };
            #[cfg(windows)]
            {
              if !matches!(&ev, Event::Input(InputEvent::Key(_))) {
                prev_press = None;
              }
            }
            push_coalesced(&mut batch, ev);
            match event::poll(Duration::from_millis(0)) {
              Ok(more) => {
                if !more {
                  break;
                }
              }
              Err(_) => break,
            }
          }
          for ev in batch.drain(..) {
            if event_tx.send(ev).is_err() {
              return;
            }
          }
        }

        if event_tx.send(Event::Tick).is_err() {
          break;
        }
      }
    });

    Events { rx, _tx: tx }
  }

  /// Attempts to read an event.
  /// This function will block the current thread.
  pub fn next(&self) -> Result<Event<InputEvent>, mpsc::RecvError> {
    self.rx.recv()
  }

  /// Non-blocking read of an event already queued behind `next()`.
  /// Lets the UI apply a whole burst of input before drawing one frame.
  pub fn try_next(&self) -> Result<Event<InputEvent>, mpsc::TryRecvError> {
    self.rx.try_recv()
  }
}

// Collapse a run of consecutive mouse drag/move events into the latest one:
// only the newest pointer position matters, and each queued event costs one
// full screen redraw. Keys, wheel notches and click/release events stay 1:1.
fn push_coalesced(batch: &mut Vec<Event<InputEvent>>, ev: Event<InputEvent>) {
  if let Event::Input(InputEvent::Mouse(mouse)) = &ev {
    let is_motion = matches!(
      mouse.kind,
      event::MouseEventKind::Drag(_) | event::MouseEventKind::Moved
    );
    if is_motion {
      if let Some(Event::Input(InputEvent::Mouse(prev))) = batch.last_mut() {
        if matches!(
          prev.kind,
          event::MouseEventKind::Drag(_) | event::MouseEventKind::Moved
        ) {
          *prev = mouse.clone();
          return;
        }
      }
    }
  }
  batch.push(ev);
}

#[cfg(test)]
mod tests {
  use super::*;
  use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

  fn mouse(kind: MouseEventKind) -> Event<InputEvent> {
    Event::Input(InputEvent::Mouse(MouseEvent {
      kind,
      column: 1,
      row: 1,
      modifiers: KeyModifiers::NONE,
    }))
  }

  fn key() -> Event<InputEvent> {
    Event::Input(InputEvent::Key(Key::Enter))
  }

  #[test]
  fn drag_run_collapses_to_latest() {
    let mut batch = Vec::new();
    push_coalesced(&mut batch, mouse(MouseEventKind::Drag(MouseButton::Left)));
    push_coalesced(&mut batch, mouse(MouseEventKind::Drag(MouseButton::Left)));
    push_coalesced(&mut batch, mouse(MouseEventKind::Drag(MouseButton::Left)));
    assert_eq!(batch.len(), 1);
    // The surviving event is the newest one.
    if let Event::Input(InputEvent::Mouse(m)) = &batch[0] {
      assert_eq!(m.column, 1);
    } else {
      panic!("expected a mouse event");
    }
  }

  #[test]
  fn wheel_and_keys_survive_unchanged() {
    let mut batch = Vec::new();
    push_coalesced(&mut batch, key());
    push_coalesced(&mut batch, mouse(MouseEventKind::ScrollDown));
    push_coalesced(&mut batch, mouse(MouseEventKind::Drag(MouseButton::Left)));
    push_coalesced(&mut batch, key());
    assert_eq!(batch.len(), 4);
  }

  #[test]
  fn motion_and_drag_merge_but_release_does_not() {
    let mut batch = Vec::new();
    push_coalesced(&mut batch, mouse(MouseEventKind::Moved));
    push_coalesced(&mut batch, mouse(MouseEventKind::Moved));
    push_coalesced(&mut batch, mouse(MouseEventKind::Up(MouseButton::Left)));
    push_coalesced(&mut batch, mouse(MouseEventKind::Moved));
    assert_eq!(batch.len(), 3);
  }
}
