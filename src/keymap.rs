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
    LineStart,
    LineEnd,
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
    /// CUA paste (Ctrl-V): char register lands at the caret; line register below
    Paste,
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
    CommandPrefix,
    // global
    Save,
    Undo,
    Redo,
    Find,
    CloseBuffer,
    Quit,
    Suspend,
    Nop,
    // sidebar (Phase 1)
    OpenSidebarTree,
    OpenSidebarPicker,
    SidebarMove(Motion),
    SidebarToggleExpand,
    SidebarInput(char),
    SidebarBackspace,
    SidebarSubmit,
    SidebarCancel,
    ResizeSidebar(i16),
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
                'p' => return Some(Command::OpenSidebarPicker),
                'b' => return Some(Command::OpenSidebarTree),
                'w' => return Some(Command::CloseBuffer),
                'q' => return Some(Command::Quit),
                // CUA clipboard trio; Ctrl-C is deliberately NOT quit (Ctrl-Q quits)
                'c' => return Some(Command::Yank),
                'x' => return Some(Command::DeleteSelection),
                'v' => return Some(Command::Paste),
                'o' => return Some(Command::Suspend),
                _ => {} // Ctrl-\ reserved; Ctrl-a/e fall through to mode-specific below
            }
        }
    }
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);
    let plain = key.modifiers.is_empty();
    // Alt+arrows resize the sidebar (App ignores the command when it is closed)
    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Left => return Some(Command::ResizeSidebar(-2)),
            KeyCode::Right => return Some(Command::ResizeSidebar(2)),
            _ => {}
        }
    }
    match mode {
        Mode::Edit => match key.code {
            KeyCode::Esc => Some(Command::EnterActionMode),
            // self-insert: any printable glyph regardless of SHIFT (crossterm
            // tags uppercase with SHIFT); Ctrl/Alt combos are handled elsewhere
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && !c.is_control() => Some(Command::InsertChar(c)),
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => Some(Command::Move(Motion::LineStart, Select::Move)),
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => Some(Command::Move(Motion::LineEnd, Select::Move)),
            KeyCode::Enter if plain => Some(Command::InsertNewline),
            KeyCode::Tab if plain => Some(Command::Indent),
            KeyCode::Backspace if plain => Some(Command::Backspace),
            KeyCode::Delete if plain => Some(Command::Delete),
            KeyCode::Home => Some(Command::Move(Motion::LineStart, sel(extend))),
            KeyCode::End => Some(Command::Move(Motion::LineEnd, sel(extend))),
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
            KeyCode::Char('u') if plain => Some(Command::Undo),
            KeyCode::Char('U') if key.modifiers == KeyModifiers::SHIFT => Some(Command::Redo),
            KeyCode::Char('/') if plain => Some(Command::Find),
            KeyCode::Char('n') if plain => Some(Command::FindNext),
            KeyCode::Char('N') if key.modifiers == KeyModifiers::SHIFT => Some(Command::FindPrev),
            KeyCode::Char(':') if plain => Some(Command::CommandPrefix),
            KeyCode::Home => Some(Command::Move(Motion::LineStart, sel(extend))),
            KeyCode::End => Some(Command::Move(Motion::LineEnd, sel(extend))),
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

/// Key decoding while the sidebar is open: it owns the keyboard, like a prompt.
/// Global Ctrl chords stay global so save/quit still work with the sidebar up.
pub fn lookup_sidebar(key: &KeyEvent) -> Option<Command> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(c) = key.code {
            return match c.to_ascii_lowercase() {
                'b' => Some(Command::OpenSidebarTree),
                'p' => Some(Command::OpenSidebarPicker),
                's' => Some(Command::Save),
                'z' => Some(Command::Undo),
                'y' => Some(Command::Redo),
                'w' => Some(Command::CloseBuffer),
                'q' => Some(Command::Quit),
                'o' => Some(Command::Suspend),
                _ => None,
            };
        }
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return match key.code {
            KeyCode::Left => Some(Command::ResizeSidebar(-2)),
            KeyCode::Right => Some(Command::ResizeSidebar(2)),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Esc => Some(Command::SidebarCancel),
        KeyCode::Enter => Some(Command::SidebarSubmit),
        KeyCode::Backspace => Some(Command::SidebarBackspace),
        KeyCode::Up => Some(Command::SidebarMove(Motion::Up)),
        KeyCode::Down => Some(Command::SidebarMove(Motion::Down)),
        KeyCode::PageUp => Some(Command::SidebarMove(Motion::PageUp)),
        KeyCode::PageDown => Some(Command::SidebarMove(Motion::PageDown)),
        KeyCode::Home => Some(Command::SidebarMove(Motion::LineStart)),
        KeyCode::End => Some(Command::SidebarMove(Motion::LineEnd)),
        KeyCode::Left | KeyCode::Right => Some(Command::SidebarToggleExpand),
        KeyCode::Char(c) if key.modifiers.is_empty() && !c.is_control() => Some(Command::SidebarInput(c)),
        _ => None,
    }
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
    fn shift_home_end_extend_selection_via_move() {
        // Shift+End/Home must EXTEND a selection (copy-to-line-end flow), not just move
        let m = Mode::Edit;
        assert_eq!(lookup(m, None, &key(KeyCode::End, KeyModifiers::SHIFT)), Some(Command::Move(Motion::LineEnd, Select::Extend)));
        assert_eq!(lookup(m, None, &key(KeyCode::Home, KeyModifiers::NONE)), Some(Command::Move(Motion::LineStart, Select::Move)));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('e'), KeyModifiers::CONTROL)), Some(Command::Move(Motion::LineEnd, Select::Move)));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Home, KeyModifiers::SHIFT)), Some(Command::Move(Motion::LineStart, Select::Extend)));
    }

    #[test]
    fn uppercase_self_insert_works_with_crossterm_shift_tag() {
        // crossterm attaches SHIFT to uppercase Char events on real terminals
        // (parse.rs: KeyCode::Char(c) if c.is_uppercase() => SHIFT)
        let k = key(KeyCode::Char('X'), KeyModifiers::SHIFT);
        assert_eq!(lookup(Mode::Edit, None, &k), Some(Command::InsertChar('X')), "uppercase must type");
        // and Ctrl-A must still win over self-insert
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('a'), KeyModifiers::CONTROL)), Some(Command::Move(Motion::LineStart, Select::Move)));
    }

    #[test]
    fn global_ctrls_work_in_both_modes() {
        for m in [Mode::Edit, Mode::Action] {
            assert_eq!(lookup(m, None, &key(KeyCode::Char('s'), KeyModifiers::CONTROL)), Some(Command::Save));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('z'), KeyModifiers::CONTROL)), Some(Command::Undo));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('y'), KeyModifiers::CONTROL)), Some(Command::Redo));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('q'), KeyModifiers::CONTROL)), Some(Command::Quit));
        }
    }

    #[test]
    fn global_cua_clipboard_bindings() {
        // CUA: Ctrl-C copy, Ctrl-X cut, Ctrl-V paste — in every mode
        for m in [Mode::Edit, Mode::Action] {
            assert_eq!(lookup(m, None, &key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(Command::Yank));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('x'), KeyModifiers::CONTROL)), Some(Command::DeleteSelection));
            assert_eq!(lookup(m, None, &key(KeyCode::Char('v'), KeyModifiers::CONTROL)), Some(Command::Paste));
            // Ctrl+Shift+C (terminal copy combo) decodes to the same thing
            assert_eq!(lookup(m, None, &key(KeyCode::Char('C'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(Command::Yank));
        }
    }

    #[test]
    fn ctrl_a_line_start_edit_only() {
        let k = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Mode::Edit, None, &k), Some(Command::Move(Motion::LineStart, Select::Move)));
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
    fn action_undo_redo_bindings() {
        let m = Mode::Action;
        assert_eq!(lookup(m, None, &key(KeyCode::Char('u'), KeyModifiers::NONE)), Some(Command::Undo));
        assert_eq!(lookup(m, None, &key(KeyCode::Char('U'), KeyModifiers::SHIFT)), Some(Command::Redo));
        // u types in Edit mode
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('u'), KeyModifiers::NONE)), Some(Command::InsertChar('u')));
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

    #[test]
    fn ctrl_b_and_ctrl_p_open_the_sidebar() {
        // Ctrl-P replaces the old open prompt
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('b'), KeyModifiers::CONTROL)), Some(Command::OpenSidebarTree));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('B'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(Command::OpenSidebarTree));
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('p'), KeyModifiers::CONTROL)), Some(Command::OpenSidebarPicker));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('P'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(Command::OpenSidebarPicker));
    }

    #[test]
    fn sidebar_owns_the_keyboard_when_open() {
        let s = |code: KeyCode, mods: KeyModifiers| lookup_sidebar(&key(code, mods));
        assert_eq!(s(KeyCode::Esc, KeyModifiers::NONE), Some(Command::SidebarCancel));
        assert_eq!(s(KeyCode::Enter, KeyModifiers::NONE), Some(Command::SidebarSubmit));
        assert_eq!(s(KeyCode::Backspace, KeyModifiers::NONE), Some(Command::SidebarBackspace));
        assert_eq!(s(KeyCode::Char('a'), KeyModifiers::NONE), Some(Command::SidebarInput('a')));
        assert_eq!(s(KeyCode::Up, KeyModifiers::NONE), Some(Command::SidebarMove(Motion::Up)));
        assert_eq!(s(KeyCode::Down, KeyModifiers::NONE), Some(Command::SidebarMove(Motion::Down)));
        assert_eq!(s(KeyCode::PageUp, KeyModifiers::NONE), Some(Command::SidebarMove(Motion::PageUp)));
        assert_eq!(s(KeyCode::PageDown, KeyModifiers::NONE), Some(Command::SidebarMove(Motion::PageDown)));
        assert_eq!(s(KeyCode::Home, KeyModifiers::NONE), Some(Command::SidebarMove(Motion::LineStart)));
        assert_eq!(s(KeyCode::End, KeyModifiers::NONE), Some(Command::SidebarMove(Motion::LineEnd)));
        // left/right expand/collapse the selected dir
        assert_eq!(s(KeyCode::Right, KeyModifiers::NONE), Some(Command::SidebarToggleExpand));
        assert_eq!(s(KeyCode::Left, KeyModifiers::NONE), Some(Command::SidebarToggleExpand));
    }

    #[test]
    fn alt_arrows_resize_the_sidebar() {
        assert_eq!(lookup_sidebar(&key(KeyCode::Left, KeyModifiers::ALT)), Some(Command::ResizeSidebar(-2)));
        assert_eq!(lookup_sidebar(&key(KeyCode::Right, KeyModifiers::ALT)), Some(Command::ResizeSidebar(2)));
        // resize also works when the key arrives while the editor has focus
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Left, KeyModifiers::ALT)), Some(Command::ResizeSidebar(-2)));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Right, KeyModifiers::ALT)), Some(Command::ResizeSidebar(2)));
    }
}
