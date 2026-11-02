use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

/// Read one event, honoring `timeout`. Key Release events are filtered;
/// everything else (Press/Repeat keys, Resize, …) passes through.
pub fn read_event(timeout: Duration) -> Option<Event> {
    if !event::poll(timeout).ok()? {
        return None;
    }
    match event::read().ok()? {
        Event::Key(k) if k.kind == KeyEventKind::Release => None,
        e => Some(e),
    }
}
