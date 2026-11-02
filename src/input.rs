use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};

/// Read one key event, honoring `timeout`. Filters non-key and Release events.
pub fn read_key(timeout: Duration) -> Option<KeyEvent> {
    if !event::poll(timeout).ok()? {
        return None;
    }
    match event::read().ok()? {
        Event::Key(k) if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat => Some(k),
        _ => None,
    }
}
