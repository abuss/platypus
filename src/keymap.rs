use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Mode;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Select {
    Move,
    Extend,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Command {
    // edit-mode editing
    InsertChar(char),
    InsertNewline,
    Indent,
    Backspace,
    Delete,
    // motions
    GoToLineStart,
    GoToLineEnd,
    Move(Motion, Select),
    // mode switching
    EnterActionMode,
    EnterEditMode,
    // global (later tasks own Find/Open/Close/Suspend behavior)
    Save,
    Undo,
    Redo,
    Find,
    Open,
    CloseBuffer,
    Quit,
    Suspend,
    Nop,
}

pub fn lookup(mode: Mode, key: &KeyEvent) -> Option<Command> {
    // Global Ctrl table first: works in every mode. Ctrl+letter arrives with
    // the shifted glyph, so lowercase and mask SHIFT before matching.
    if key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(c) = key.code {
            match c.to_ascii_lowercase() {
                's' => return Some(Command::Save),
                'z' => return Some(Command::Undo),
                'y' => return Some(Command::Redo),
                'f' => return Some(Command::Find),
                'p' => return Some(Command::Open),
                'w' => return Some(Command::CloseBuffer),
                'q' | 'c' => return Some(Command::Quit),
                'o' => return Some(Command::Suspend),
                _ => {} // Ctrl-\ reserved; Ctrl-a/e fall through to mode-specific below
            }
        }
    }
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);
    let plain = key.modifiers.is_empty();
    match mode {
        Mode::Edit => match key.code {
            KeyCode::Esc => Some(Command::EnterActionMode),
            KeyCode::Char(c) if plain && !c.is_control() => Some(Command::InsertChar(c)),
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => Some(Command::GoToLineStart),
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => Some(Command::GoToLineEnd),
            KeyCode::Enter if plain => Some(Command::InsertNewline),
            KeyCode::Tab if plain => Some(Command::Indent),
            KeyCode::Backspace if plain => Some(Command::Backspace),
            KeyCode::Delete if plain => Some(Command::Delete),
            KeyCode::Home => Some(Command::GoToLineStart),
            KeyCode::End => Some(Command::GoToLineEnd),
            KeyCode::Left => Some(Command::Move(Motion::Left, sel(extend))),
            KeyCode::Right => Some(Command::Move(Motion::Right, sel(extend))),
            KeyCode::Up => Some(Command::Move(Motion::Up, sel(extend))),
            KeyCode::Down => Some(Command::Move(Motion::Down, sel(extend))),
            KeyCode::PageUp => Some(Command::Move(Motion::PageUp, sel(extend))),
            KeyCode::PageDown => Some(Command::Move(Motion::PageDown, sel(extend))),
            _ => None,
        },
        Mode::Action => match key.code {
            KeyCode::Esc => Some(Command::EnterEditMode),
            KeyCode::Home => Some(Command::GoToLineStart),
            KeyCode::End => Some(Command::GoToLineEnd),
            KeyCode::Left => Some(Command::Move(Motion::Left, sel(extend))),
            KeyCode::Right => Some(Command::Move(Motion::Right, sel(extend))),
            KeyCode::Up => Some(Command::Move(Motion::Up, sel(extend))),
            KeyCode::Down => Some(Command::Move(Motion::Down, sel(extend))),
            KeyCode::PageUp => Some(Command::Move(Motion::PageUp, sel(extend))),
            KeyCode::PageDown => Some(Command::Move(Motion::PageDown, sel(extend))),
            _ => None, // action-mode letter bindings land in Tasks 5-8
        },
    }
}

fn sel(extend: bool) -> Select {
    if extend { Select::Extend } else { Select::Move }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn edit_mode_typing() {
        let m = Mode::Edit;
        assert_eq!(lookup(m, &key(KeyCode::Char('a'), KeyModifiers::NONE)), Some(Command::InsertChar('a')));
        assert_eq!(lookup(m, &key(KeyCode::Enter, KeyModifiers::NONE)), Some(Command::InsertNewline));
        assert_eq!(lookup(m, &key(KeyCode::Backspace, KeyModifiers::NONE)), Some(Command::Backspace));
        // Ctrl masks Shift for letter matching (Ctrl+S arrives as 'S')
        assert_eq!(lookup(m, &key(KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(Command::Save));
    }

    #[test]
    fn global_ctrls_work_in_both_modes() {
        for m in [Mode::Edit, Mode::Action] {
            assert_eq!(lookup(m, &key(KeyCode::Char('s'), KeyModifiers::CONTROL)), Some(Command::Save));
            assert_eq!(lookup(m, &key(KeyCode::Char('z'), KeyModifiers::CONTROL)), Some(Command::Undo));
            assert_eq!(lookup(m, &key(KeyCode::Char('y'), KeyModifiers::CONTROL)), Some(Command::Redo));
            assert_eq!(lookup(m, &key(KeyCode::Char('q'), KeyModifiers::CONTROL)), Some(Command::Quit));
            assert_eq!(lookup(m, &key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(Command::Quit));
        }
    }

    #[test]
    fn ctrl_a_line_start_edit_only() {
        let k = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Mode::Edit, &k), Some(Command::GoToLineStart));
        assert_eq!(lookup(Mode::Action, &k), None);
    }

    #[test]
    fn unbound_action_key_is_none_and_reserved_ctrls_unbound() {
        assert_eq!(lookup(Mode::Action, &key(KeyCode::Char('q'), KeyModifiers::NONE)), None);
        // Ctrl-\ reserved for the future terminal overlay
        assert_eq!(lookup(Mode::Edit, &key(KeyCode::Char('\\'), KeyModifiers::CONTROL)), None);
        assert_eq!(lookup(Mode::Action, &key(KeyCode::Char('\\'), KeyModifiers::CONTROL)), None);
    }

    #[test]
    fn esc_switches_modes() {
        assert_eq!(lookup(Mode::Edit, &key(KeyCode::Esc, KeyModifiers::NONE)), Some(Command::EnterActionMode));
        assert_eq!(lookup(Mode::Action, &key(KeyCode::Esc, KeyModifiers::NONE)), Some(Command::EnterEditMode));
    }

    #[test]
    fn arrows_move_in_both_modes_shift_extends() {
        for m in [Mode::Edit, Mode::Action] {
            assert_eq!(lookup(m, &key(KeyCode::Left, KeyModifiers::NONE)), Some(Command::Move(Motion::Left, Select::Move)));
            assert_eq!(lookup(m, &key(KeyCode::Down, KeyModifiers::SHIFT)), Some(Command::Move(Motion::Down, Select::Extend)));
        }
    }
}
