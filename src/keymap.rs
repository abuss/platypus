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
    WordForward,
    WordBackward,
}

/// Two-step key capture state (f/F target char, Alt+- object char).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pending {
    JumpForward,
    JumpBackward,
    Object,
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
    // selection (Task 5)
    BeginSelectChar,
    SelectLine,
    ExpandLine,
    // motions with capture (Task 6)
    JumpCharCapture {
        backward: bool,
    },
    JumpToChar {
        ch: char,
        backward: bool,
    },
    ObjectPrefix,
    SelectObject(char),
    // selection ops (Task 7); DeleteSelection to avoid clash with Delete key
    Yank,
    PasteAfter,
    PasteBefore,
    DeleteSelection,
    Change,
    // find (Task 8)
    FindNext,
    FindPrev,
    // prompt routing (Task 8)
    PromptChar(char),
    PromptBackspace,
    PromptHistoryUp,
    PromptHistoryDown,
    PromptSubmit,
    PromptCancel,
    PromptCycle,
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

pub fn lookup(mode: Mode, pending: Option<Pending>, key: &KeyEvent) -> Option<Command> {
    if let Some(p) = pending {
        // capture state: next printable char completes the command; anything
        // else cancels (app clears pending on None).
        return match (p, key.code) {
            (Pending::JumpForward, KeyCode::Char(c)) => Some(Command::JumpToChar { ch: c, backward: false }),
            (Pending::JumpBackward, KeyCode::Char(c)) => Some(Command::JumpToChar { ch: c, backward: true }),
            (Pending::Object, KeyCode::Char(c)) => Some(Command::SelectObject(c)),
            _ => None,
        };
    }
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
            KeyCode::Char(' ') if plain => Some(Command::BeginSelectChar),
            KeyCode::Char('x') if plain => Some(Command::SelectLine),
            KeyCode::Char('X') if key.modifiers == KeyModifiers::SHIFT => Some(Command::ExpandLine),
            // Task 6 bindings
            KeyCode::Char('w') if plain => Some(Command::Move(Motion::WordForward, Select::Move)),
            KeyCode::Char('b') if plain => Some(Command::Move(Motion::WordBackward, Select::Move)),
            KeyCode::Char('f') if plain => Some(Command::JumpCharCapture { backward: false }),
            KeyCode::Char('F') if key.modifiers == KeyModifiers::SHIFT => Some(Command::JumpCharCapture { backward: true }),
            KeyCode::Char('-') if key.modifiers == KeyModifiers::ALT => Some(Command::ObjectPrefix),
            // selection ops (Task 7)
            KeyCode::Char('y') if plain => Some(Command::Yank),
            KeyCode::Char('p') if plain => Some(Command::PasteAfter),
            KeyCode::Char('P') if key.modifiers == KeyModifiers::SHIFT => Some(Command::PasteBefore),
            KeyCode::Char('d') if plain => Some(Command::DeleteSelection),
            KeyCode::Char('c') if plain => Some(Command::Change),
            KeyCode::Char('/') if plain => Some(Command::Find),
            KeyCode::Char('n') if plain => Some(Command::FindNext),
            KeyCode::Char('N') if key.modifiers == KeyModifiers::SHIFT => Some(Command::FindPrev),
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

/// Key decoding while a prompt is open: the prompt owns the keyboard.
pub fn lookup_prompt(key: &KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Esc => Some(Command::PromptCancel),
        KeyCode::Enter => Some(Command::PromptSubmit),
        KeyCode::Backspace | KeyCode::Delete => Some(Command::PromptBackspace),
        KeyCode::Up => Some(Command::PromptHistoryUp),
        KeyCode::Down => Some(Command::PromptHistoryDown),
        KeyCode::Tab => Some(Command::PromptCycle),
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            match c.to_ascii_lowercase() {
                // Ctrl-G/Ctrl-C/Ctrl-Q cancel a prompt (for confirms, cancel
                // is the safe no-op choice — they never quit mid-prompt)
                'g' | 'c' | 'q' => Some(Command::PromptCancel),
                _ => None,
            }
        }
        KeyCode::Char(c) if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) == false => {
            Some(Command::PromptChar(c))
        }
        _ => None,
    }
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
        assert_eq!(lookup(m, None, &key(KeyCode::Char('a'), KeyModifiers::NONE)), Some(Command::InsertChar('a')));
        assert_eq!(lookup(m, None, &key(KeyCode::Enter, KeyModifiers::NONE)), Some(Command::InsertNewline));
        assert_eq!(lookup(m, None, &key(KeyCode::Backspace, KeyModifiers::NONE)), Some(Command::Backspace));
        // Ctrl masks Shift for letter matching (Ctrl+S arrives as 'S')
        assert_eq!(lookup(m, None, &key(KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(Command::Save));
    }

    #[test]
    fn global_ctrls_work_in_both_modes() {
        for m in [Mode::Edit, Mode::Action] {
            assert_eq!(lookup(m, None, &key(KeyCode::Char('s'), KeyModifiers::CONTROL)), Some(Command::Save));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('z'), KeyModifiers::CONTROL)), Some(Command::Undo));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('y'), KeyModifiers::CONTROL)), Some(Command::Redo));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('q'), KeyModifiers::CONTROL)), Some(Command::Quit));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(Command::Quit));
        }
    }

    #[test]
    fn ctrl_a_line_start_edit_only() {
        let k = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Mode::Edit, None, &k), Some(Command::GoToLineStart));
        assert_eq!(lookup(Mode::Action, None, &k), None);
    }

    #[test]
    fn unbound_action_key_is_none_and_reserved_ctrls_unbound() {
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('q'), KeyModifiers::NONE)), None);
        // Ctrl-\ reserved for the future terminal overlay
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('\\'), KeyModifiers::CONTROL)), None);
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('\\'), KeyModifiers::CONTROL)), None);
    }

    #[test]
    fn action_selection_bindings() {
        let m = Mode::Action;
        assert_eq!(lookup(m, None, &key(KeyCode::Char(' '), KeyModifiers::NONE)), Some(Command::BeginSelectChar));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('x'), KeyModifiers::NONE)), Some(Command::SelectLine));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('X'), KeyModifiers::SHIFT)), Some(Command::ExpandLine));
        // x types in Edit mode
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('x'), KeyModifiers::NONE)), Some(Command::InsertChar('x')));
    }

    #[test]
    fn esc_switches_modes() {
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Esc, KeyModifiers::NONE)), Some(Command::EnterActionMode));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Esc, KeyModifiers::NONE)), Some(Command::EnterEditMode));
    }

    #[test]
    fn arrows_move_in_both_modes_shift_extends() {
        for m in [Mode::Edit, Mode::Action] {
            assert_eq!(lookup(m, None, &key(KeyCode::Left, KeyModifiers::NONE)), Some(Command::Move(Motion::Left, Select::Move)));
            assert_eq!(lookup(m, None, &key(KeyCode::Down, KeyModifiers::SHIFT)), Some(Command::Move(Motion::Down, Select::Extend)));
        }
    }
}

#[cfg(test)]
mod task8_tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyCode, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn action_find_bindings() {
        let m = Mode::Action;
        assert_eq!(lookup(m, None, &key(KeyCode::Char('/'), KeyModifiers::NONE)), Some(Command::Find));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('n'), KeyModifiers::NONE)), Some(Command::FindNext));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('N'), KeyModifiers::SHIFT)), Some(Command::FindPrev));
    }

    #[test]
    fn prompt_decode_owns_keyboard() {
        assert_eq!(lookup_prompt(&key(KeyCode::Char('e'), KeyModifiers::NONE)), Some(Command::PromptChar('e')));
        assert_eq!(lookup_prompt(&key(KeyCode::Char('E'), KeyModifiers::SHIFT)), Some(Command::PromptChar('E')));
        assert_eq!(lookup_prompt(&key(KeyCode::Enter, KeyModifiers::NONE)), Some(Command::PromptSubmit));
        assert_eq!(lookup_prompt(&key(KeyCode::Esc, KeyModifiers::NONE)), Some(Command::PromptCancel));
        assert_eq!(lookup_prompt(&key(KeyCode::Backspace, KeyModifiers::NONE)), Some(Command::PromptBackspace));
        assert_eq!(lookup_prompt(&key(KeyCode::Tab, KeyModifiers::NONE)), Some(Command::PromptCycle));
        // Ctrl-C in a confirm prompt must NOT quit: it cancels
        assert_eq!(lookup_prompt(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(Command::PromptCancel));
    }
}

#[cfg(test)]
mod task6_tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyCode, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn action_word_jump_and_capture_bindings() {
        let m = Mode::Action;
        assert_eq!(lookup(m, None, &key(KeyCode::Char('w'), KeyModifiers::NONE)), Some(Command::Move(Motion::WordForward, Select::Move)));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('b'), KeyModifiers::NONE)), Some(Command::Move(Motion::WordBackward, Select::Move)));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('f'), KeyModifiers::NONE)), Some(Command::JumpCharCapture { backward: false }));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('F'), KeyModifiers::SHIFT)), Some(Command::JumpCharCapture { backward: true }));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('-'), KeyModifiers::ALT)), Some(Command::ObjectPrefix));
        // w still types in Edit mode
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('w'), KeyModifiers::NONE)), Some(Command::InsertChar('w')));
    }

    #[test]
    fn pending_capture_completes() {
        assert_eq!(lookup(Mode::Action, Some(Pending::JumpForward), &key(KeyCode::Char(')'), KeyModifiers::NONE)), Some(Command::JumpToChar { ch: ')', backward: false }));
        assert_eq!(lookup(Mode::Action, Some(Pending::JumpBackward), &key(KeyCode::Char('('), KeyModifiers::NONE)), Some(Command::JumpToChar { ch: '(', backward: true }));
        assert_eq!(lookup(Mode::Action, Some(Pending::Object), &key(KeyCode::Char('('), KeyModifiers::NONE)), Some(Command::SelectObject('(')));
        assert_eq!(lookup(Mode::Action, Some(Pending::JumpForward), &key(KeyCode::Esc, KeyModifiers::NONE)), None, "Esc cancels pending");
    }
}
