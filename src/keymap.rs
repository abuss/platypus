use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

use crate::app::Mode;

pub type KeySpec = (KeyCode, KeyModifiers);
pub type KeyMap = HashMap<KeySpec, Option<Command>>;

/// Normalize a key code and modifiers into canonical form.
/// 
/// Rules:
/// - For CONTROL (without ALT) + ASCII letter: lowercase letter, drop SHIFT
/// - For uppercase letter with SHIFT only (or bare uppercase): lowercase letter, keep SHIFT
/// - Otherwise: as-is
fn normalize(code: KeyCode, mods: KeyModifiers) -> KeySpec {
    match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() && mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) => {
            // Ctrl+letter: always lowercase, drop SHIFT
            (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL)
        }
        KeyCode::Char(c) if c.is_ascii_uppercase() && (mods == KeyModifiers::SHIFT || mods == KeyModifiers::NONE) => {
            // Uppercase letter (with SHIFT or bare): lowercase, add/keep SHIFT
            (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::SHIFT)
        }
        _ => (code, mods),
    }
}

pub fn canonical(key: &KeyEvent) -> KeySpec {
    normalize(key.code, key.modifiers)
}

pub fn parse_key_spec(spec: &str) -> Result<KeySpec, String> {
    if spec.is_empty() {
        return Err(format!("empty spec"));
    }

    let mut mods = KeyModifiers::NONE;
    let mut rest = spec;

    // Parse modifier prefixes
    loop {
        if rest.starts_with("C-") {
            mods |= KeyModifiers::CONTROL;
            rest = &rest[2..];
        } else if rest.starts_with("A-") {
            mods |= KeyModifiers::ALT;
            rest = &rest[2..];
        } else if rest.starts_with("S-") {
            mods |= KeyModifiers::SHIFT;
            rest = &rest[2..];
        } else {
            break;
        }
    }

    if rest.is_empty() {
        return Err(format!("incomplete spec: {spec}"));
    }

    // Parse the base key
    let code = if rest.len() == 1 {
        // Single character: keep case as-is for normalization
        let c = rest.chars().next().unwrap();
        if c.is_ascii() {
            KeyCode::Char(c)
        } else {
            return Err(format!("invalid base key in spec: {spec}"));
        }
    } else {
        // Named token: match case-insensitively
        match rest.to_ascii_lowercase().as_str() {
            "space" => KeyCode::Char(' '),
            "enter" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "esc" => KeyCode::Esc,
            "backspace" => KeyCode::Backspace,
            "delete" => KeyCode::Delete,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            _ => return Err(format!("invalid base key in spec: {spec}")),
        }
    };

    Ok(normalize(code, mods))
}

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

/// Two-step key capture state (f/F target char, Alt+- object char, g prefix for goto).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pending {
    JumpForward,
    JumpBackward,
    Object,
    GotoPrefix,
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
    NextBuffer,
    PrevBuffer,
    Quit,
    Suspend,
    Nop,
    // sidebar (Phase 1)
    OpenSidebarTree,
    OpenSidebarPicker,
    SidebarMove(Motion),
    SidebarExpand,
    SidebarCollapseOrParent,
    SidebarRefresh,
    SidebarInput(char),
    SidebarBackspace,
    SidebarSubmit,
    SidebarCancel,
    ResizeSidebar(i16),
    /// LSP diagnostic navigation (Task 5)
    PrevDiagnostic,
    NextDiagnostic,
    /// LSP completion trigger (Task 6-lsp)
    Completion,
    /// LSP goto-definition (Task 7)
    GotoDef,
    /// Jump back in navigation stack (Task 7)
    JumpBack,
    /// Enter goto-prefix mode (action 'g'): next key d=GotoDef, o=JumpBack
    GotoPrefix,
}

/// Overlay lookup: pending capture FIRST, then user keymap, then builtin lookup.
/// `Some(cmd)` in the overlay wins; `None` value in the overlay → `Command::Nop` (unbound).
/// Falls back to `lookup` when the key is not in the overlay.
pub fn lookup_with(keys: &KeyMap, mode: Mode, pending: Option<Pending>, key: &KeyEvent) -> Option<Command> {
    // Pending capture always wins — don't let an overlay binding intercept it.
    if pending.is_some() {
        return lookup(mode, pending, key);
    }
    match keys.get(&canonical(key)) {
        Some(Some(c)) => Some(*c),
        Some(None) => Some(Command::Nop),
        None => lookup(mode, pending, key),
    }
}

pub fn lookup(mode: Mode, pending: Option<Pending>, key: &KeyEvent) -> Option<Command> {
    if let Some(p) = pending {
        // capture state: next printable char completes the command; anything
        // else cancels (app clears pending on None).
        return match (p, key.code) {
            (Pending::JumpForward, KeyCode::Char(c)) => Some(Command::JumpToChar { ch: c, backward: false }),
            (Pending::JumpBackward, KeyCode::Char(c)) => Some(Command::JumpToChar { ch: c, backward: true }),
            (Pending::Object, KeyCode::Char(c)) => Some(Command::SelectObject(c)),
            (Pending::GotoPrefix, KeyCode::Char('d')) => Some(Command::GotoDef),
            (Pending::GotoPrefix, KeyCode::Char('o')) => Some(Command::JumpBack),
            (Pending::GotoPrefix, _) => None, // cancels quietly
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
                // Ctrl-G starts the goto prefix from ANY mode (byte 0x07 arrives
                // as Char('g')+CONTROL, pty-probed): Ctrl-G d = goto-def,
                // Ctrl-G o = jump back. Pending captures are mode-agnostic.
                'g' => return Some(Command::GotoPrefix),
                // Ctrl-Space (arrives as Char(' ')+CONTROL, verified by pty probe)
                // and Ctrl-@ (same byte 0x00 on legacy terminals)
                ' ' | '@' => return Some(Command::Completion),
                ']' => return Some(Command::GotoDef), // kitty-protocol form
                // crossterm maps the Ctrl-] byte (0x1d) to Char('5')+CONTROL on
                // non-kitty terminals (pty-probed: 0x1d -> Char('5')); Ctrl+5 on
                // xterm sends 0x15 -> Char('U'), so '5'+CTRL is unambiguous.
                '5' => return Some(Command::GotoDef),
                _ => {} // Ctrl-\ reserved; Ctrl-a/e fall through to mode-specific below
            }
        }
    }
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);
    let plain = key.modifiers.is_empty();
    // Alt+arrows resize the sidebar (App ignores the command when it is closed).
    // Buffer cycling uses Alt+letters: Alt+[ / Alt+] arrive as ESC[ / ESC],
    // which terminal parsers eat as CSI/OSC sequence starts — never bind those.
    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Left => return Some(Command::ResizeSidebar(-2)),
            KeyCode::Right => return Some(Command::ResizeSidebar(2)),
            KeyCode::Char('h') => return Some(Command::PrevBuffer),
            KeyCode::Char('l') => return Some(Command::NextBuffer),
            KeyCode::Char(']') => return Some(Command::NextBuffer),
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
            KeyCode::Char('[') if plain => Some(Command::PrevDiagnostic),
            KeyCode::Char(']') if plain => Some(Command::NextDiagnostic),
            KeyCode::Char('g') if plain => Some(Command::GotoPrefix),
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
                'r' => Some(Command::SidebarRefresh),
                'f' => Some(Command::Find),
                _ => None,
            };
        }
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return match key.code {
            KeyCode::Left => Some(Command::ResizeSidebar(-2)),
            KeyCode::Right => Some(Command::ResizeSidebar(2)),
            KeyCode::Char('h') => Some(Command::PrevBuffer),
            KeyCode::Char('l') => Some(Command::NextBuffer),
            KeyCode::Char(']') => Some(Command::NextBuffer),
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
        KeyCode::Right => Some(Command::SidebarExpand),
        KeyCode::Left => Some(Command::SidebarCollapseOrParent),
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

#[test]
fn every_documented_name_maps_to_a_command() {
    for name in ["select_line", "expand_line", "begin_select_char", "jump_char",
                 "jump_char_back", "object_prefix", "yank", "paste_after", "paste_before",
                 "paste", "delete_selection", "change", "find", "find_next", "find_prev",
                 "save", "undo", "redo", "quit", "suspend", "close_buffer",
                 "next_buffer", "prev_buffer",
                 "open_sidebar_tree", "open_sidebar_picker",
                 "resize_sidebar.grow", "resize_sidebar.shrink", "indent",
                 "insert_newline", "backspace", "delete", "enter_action_mode",
                 "enter_edit_mode",
                 "prev_diagnostic", "next_diagnostic", "completion",
                 "goto_def", "jump_back"] {
        assert!(command_by_name(name).is_some(), "missing {name}");
    }
    let motions = ["left","right","up","down","pageup","pagedown","word_next",
                   "word_prev","line_start","line_end"];
    for m in motions {
        assert!(command_by_name(&format!("move.{m}")).is_some(), "missing move.{m}");
        assert!(command_by_name(&format!("move.{m}.extend")).is_some(), "missing move.{m}.extend");
    }
}

#[test]
fn runtime_arg_and_ownership_commands_are_not_bindable() {
    for name in ["jump_to_char", "select_object", "prompt_submit", "prompt_cancel",
                 "sidebar_move", "sidebar_submit", "insert_char", "nop"] {
        assert!(command_by_name(name).is_none(), "{name} must not be bindable");
    }
    assert!(command_by_name("selct_line").is_none());
}

pub fn command_by_name(name: &str) -> Option<Command> {
    match name {
        "select_line" => Some(Command::SelectLine),
        "expand_line" => Some(Command::ExpandLine),
        "begin_select_char" => Some(Command::BeginSelectChar),
        "jump_char" => Some(Command::JumpCharCapture { backward: false }),
        "jump_char_back" => Some(Command::JumpCharCapture { backward: true }),
        "object_prefix" => Some(Command::ObjectPrefix),
        "yank" => Some(Command::Yank),
        "paste_after" => Some(Command::PasteAfter),
        "paste_before" => Some(Command::PasteBefore),
        "paste" => Some(Command::Paste),
        "delete_selection" => Some(Command::DeleteSelection),
        "change" => Some(Command::Change),
        "find" => Some(Command::Find),
        "find_next" => Some(Command::FindNext),
        "find_prev" => Some(Command::FindPrev),
        "save" => Some(Command::Save),
        "undo" => Some(Command::Undo),
        "redo" => Some(Command::Redo),
        "quit" => Some(Command::Quit),
        "suspend" => Some(Command::Suspend),
        "close_buffer" => Some(Command::CloseBuffer),
        "next_buffer" => Some(Command::NextBuffer),
        "prev_buffer" => Some(Command::PrevBuffer),
        "open_sidebar_tree" => Some(Command::OpenSidebarTree),
        "open_sidebar_picker" => Some(Command::OpenSidebarPicker),
        "resize_sidebar.grow" => Some(Command::ResizeSidebar(2)),
        "resize_sidebar.shrink" => Some(Command::ResizeSidebar(-2)),
        "indent" => Some(Command::Indent),
        "insert_newline" => Some(Command::InsertNewline),
        "backspace" => Some(Command::Backspace),
        "delete" => Some(Command::Delete),
        "enter_action_mode" => Some(Command::EnterActionMode),
        "enter_edit_mode" => Some(Command::EnterEditMode),
        "move.left" => Some(Command::Move(Motion::Left, Select::Move)),
        "move.right" => Some(Command::Move(Motion::Right, Select::Move)),
        "move.up" => Some(Command::Move(Motion::Up, Select::Move)),
        "move.down" => Some(Command::Move(Motion::Down, Select::Move)),
        "move.pageup" => Some(Command::Move(Motion::PageUp, Select::Move)),
        "move.pagedown" => Some(Command::Move(Motion::PageDown, Select::Move)),
        "move.word_next" => Some(Command::Move(Motion::WordForward, Select::Move)),
        "move.word_prev" => Some(Command::Move(Motion::WordBackward, Select::Move)),
        "move.line_start" => Some(Command::Move(Motion::LineStart, Select::Move)),
        "move.line_end" => Some(Command::Move(Motion::LineEnd, Select::Move)),
        "move.left.extend" => Some(Command::Move(Motion::Left, Select::Extend)),
        "move.right.extend" => Some(Command::Move(Motion::Right, Select::Extend)),
        "move.up.extend" => Some(Command::Move(Motion::Up, Select::Extend)),
        "move.down.extend" => Some(Command::Move(Motion::Down, Select::Extend)),
        "move.pageup.extend" => Some(Command::Move(Motion::PageUp, Select::Extend)),
        "move.pagedown.extend" => Some(Command::Move(Motion::PageDown, Select::Extend)),
        "move.word_next.extend" => Some(Command::Move(Motion::WordForward, Select::Extend)),
        "move.word_prev.extend" => Some(Command::Move(Motion::WordBackward, Select::Extend)),
        "move.line_start.extend" => Some(Command::Move(Motion::LineStart, Select::Extend)),
        "move.line_end.extend" => Some(Command::Move(Motion::LineEnd, Select::Extend)),
        "prev_diagnostic" => Some(Command::PrevDiagnostic),
        "next_diagnostic" => Some(Command::NextDiagnostic),
        "completion" => Some(Command::Completion),
        "goto_def" => Some(Command::GotoDef),
        "jump_back" => Some(Command::JumpBack),
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

    #[test]
    fn specs_cover_letters_chords_and_named_keys() {
        assert_eq!(parse_key_spec("x"), Ok((KeyCode::Char('x'), KeyModifiers::NONE)));
        assert_eq!(parse_key_spec("C-q"), Ok((KeyCode::Char('q'), KeyModifiers::CONTROL)));
        assert_eq!(parse_key_spec("A-Left"), Ok((KeyCode::Left, KeyModifiers::ALT)));
        assert_eq!(parse_key_spec("S-Tab"), Ok((KeyCode::Tab, KeyModifiers::SHIFT)));
        assert_eq!(parse_key_spec("C-Space"), Ok((KeyCode::Char(' '), KeyModifiers::CONTROL)));
        assert_eq!(parse_key_spec("PageUp"), Ok((KeyCode::PageUp, KeyModifiers::NONE)));
    }

    #[test]
    fn hyphen_is_the_remainder_after_known_prefixes() {
        assert_eq!(parse_key_spec("-"), Ok((KeyCode::Char('-'), KeyModifiers::NONE)));
        assert_eq!(parse_key_spec("A--"), Ok((KeyCode::Char('-'), KeyModifiers::ALT)));
        assert_eq!(parse_key_spec("C--"), Ok((KeyCode::Char('-'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn specs_normalize_like_incoming_events() {
        // C-S-q == C-q; bare uppercase == shift+lowercase
        assert_eq!(parse_key_spec("C-S-q"), parse_key_spec("C-q"));
        assert_eq!(parse_key_spec("N"), parse_key_spec("S-n"));
        assert!(parse_key_spec("C-S").is_err() == false); // letters lowercase under C-
    }

    #[test]
    fn bad_specs_are_rejected_with_an_explanation() {
        for bad in ["", "C-", "Hyper-X", "Foo", "C-Foo", "Ab"] {
            let e = parse_key_spec(bad).unwrap_err();
            assert!(!e.is_empty() && e.contains(bad), "message names {bad:?}: {e}");
        }
    }

    #[test]
    fn canonical_matches_what_crossterm_actually_sends() {
        // Ctrl+S arrives as Char('S') + CONTROL|SHIFT (existing test pins the wire fact)
        let ctrl_s = key(KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(canonical(&ctrl_s), parse_key_spec("C-s").unwrap());
        // plain uppercase arrives as Char('N') + SHIFT
        let shift_n = key(KeyCode::Char('N'), KeyModifiers::SHIFT);
        assert_eq!(canonical(&shift_n), parse_key_spec("N").unwrap());
        // alt+left: Left + ALT (verified by the Phase 1 keydebug pty smoke)
        let alt_left = key(KeyCode::Left, KeyModifiers::ALT);
        assert_eq!(canonical(&alt_left), parse_key_spec("A-Left").unwrap());
        // plain 'x' stays plain
        assert_eq!(canonical(&key(KeyCode::Char('x'), KeyModifiers::NONE)), parse_key_spec("x").unwrap());
    }

    #[test]
    fn named_tokens_are_case_insensitive() {
        // Named tokens must work in any case
        assert_eq!(parse_key_spec("space"), parse_key_spec("Space"));
        assert_eq!(parse_key_spec("TAB"), parse_key_spec("Tab"));
        assert_eq!(parse_key_spec("pageup"), parse_key_spec("PageUp"));
        assert_eq!(parse_key_spec("C-space"), parse_key_spec("C-Space"));
        // But bare uppercase letters still normalize to SHIFT+lowercase
        assert_eq!(parse_key_spec("N"), parse_key_spec("S-n"));
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
        assert_eq!(s(KeyCode::Right, KeyModifiers::NONE), Some(Command::SidebarExpand));
        assert_eq!(s(KeyCode::Left, KeyModifiers::NONE), Some(Command::SidebarCollapseOrParent));
        // Ctrl-R rescans, Ctrl-F opens find
        assert_eq!(s(KeyCode::Char('r'), KeyModifiers::CONTROL), Some(Command::SidebarRefresh));
        assert_eq!(s(KeyCode::Char('f'), KeyModifiers::CONTROL), Some(Command::Find));
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

#[cfg(test)]
mod task5_tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyCode, KeyModifiers};
    use crate::config;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    fn overlay(pairs: &[(&str, &str)]) -> KeyMap {
        let mut lines = String::from("[keys]\n");
        for (spec, cmd) in pairs {
            lines.push_str(&format!("{spec:?} = {cmd:?}\n"));
        }
        config::from_str(&lines).unwrap().keys
    }

    #[test]
    fn overlay_beats_defaults_and_none_unbinds() {
        let keys = overlay(&[("C-Space", "select_line"), ("x", "none")]);
        assert_eq!(lookup_with(&keys, Mode::Action, None, &key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
                   Some(Command::SelectLine));
        // unbound default key: nothing, and NO fallthrough to the old command
        assert_eq!(lookup_with(&keys, Mode::Action, None, &key(KeyCode::Char('x'), KeyModifiers::NONE)),
                   Some(Command::Nop));
    }

    #[test]
    fn pending_capture_completes_before_the_overlay() {
        let keys = overlay(&[("q", "save")]);
        // `f` then q must jump to 'q', not run save
        assert_eq!(lookup_with(&keys, Mode::Action, Some(Pending::JumpForward), &key(KeyCode::Char('q'), KeyModifiers::NONE)),
                   Some(Command::JumpToChar { ch: 'q', backward: false }));
    }

    #[test]
    fn normalized_specs_match_real_events() {
        let keys = overlay(&[("C-S-q", "save"), ("N", "redo")]);
        // Ctrl+Q on a real terminal is Char('Q')+CONTROL|SHIFT, matching C-S-q spelling
        assert_eq!(lookup_with(&keys, Mode::Edit, None, &key(KeyCode::Char('Q'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
                   Some(Command::Save));
        assert_eq!(lookup_with(&keys, Mode::Action, None, &key(KeyCode::Char('N'), KeyModifiers::SHIFT)),
                   Some(Command::Redo));
    }
}

#[cfg(test)]
mod buffer_cycle_tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyCode, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn alt_letter_buffer_cycling_in_both_modes_and_sidebar() {
        // Alt+[ / Alt+] removed: ESC[ / ESC] are CSI/OSC sequence starts that
        // terminal parsers swallow; Alt+letters are unambiguous.
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('h'), KeyModifiers::ALT)), Some(Command::PrevBuffer));
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('l'), KeyModifiers::ALT)), Some(Command::NextBuffer));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('h'), KeyModifiers::ALT)), Some(Command::PrevBuffer));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('l'), KeyModifiers::ALT)), Some(Command::NextBuffer));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char(']'), KeyModifiers::ALT)), Some(Command::NextBuffer));
        assert_eq!(lookup_sidebar(&key(KeyCode::Char('l'), KeyModifiers::ALT)), Some(Command::NextBuffer));
        // Alt-] stays as a best-effort alias (works in crossterm pty, not all terminals)
    }

    #[test]
    fn buffer_cycle_command_names_resolve() {
        assert_eq!(command_by_name("next_buffer"), Some(Command::NextBuffer));
        assert_eq!(command_by_name("prev_buffer"), Some(Command::PrevBuffer));
    }
}

#[cfg(test)]
mod task7_lsp_tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyCode, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn ctrl_bracket_fires_goto_def_in_both_modes() {
        // Ctrl-] arrives as Char(']')+CONTROL (0x1D on most terminals)
        let k = key(KeyCode::Char(']'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Mode::Edit, None, &k), Some(Command::GotoDef));
        assert_eq!(lookup(Mode::Action, None, &k), Some(Command::GotoDef));
        // ...and as Char('5')+CONTROL on NON-kitty terminals (crossterm maps the
        // 0x1d byte to '5'+CTRL; pty-probed 2026-11-04)
        let k5 = key(KeyCode::Char('5'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Mode::Edit, None, &k5), Some(Command::GotoDef));
        assert_eq!(lookup(Mode::Action, None, &k5), Some(Command::GotoDef));
    }

    #[test]
    fn ctrl_g_starts_goto_prefix_in_both_modes() {
        // Ctrl-G (byte 0x07, pty-probed as Char('g')+CONTROL) starts the same
        // g-prefix capture from ANY mode, so Ctrl-G d / Ctrl-G o work in Edit.
        let k = key(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(lookup(Mode::Edit, None, &k), Some(Command::GotoPrefix));
        assert_eq!(lookup(Mode::Action, None, &k), Some(Command::GotoPrefix));
        let k_shift = key(KeyCode::Char('G'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(lookup(Mode::Edit, None, &k_shift), Some(Command::GotoPrefix), "kitty Ctrl-Shift-G");
        // The capture resolves in Edit mode too (pending wins over InsertChar).
        assert_eq!(lookup(Mode::Edit, Some(Pending::GotoPrefix), &key(KeyCode::Char('d'), KeyModifiers::NONE)), Some(Command::GotoDef));
        assert_eq!(lookup(Mode::Edit, Some(Pending::GotoPrefix), &key(KeyCode::Char('o'), KeyModifiers::NONE)), Some(Command::JumpBack));
    }

    #[test]
    fn action_g_enters_goto_prefix() {
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('g'), KeyModifiers::NONE)), Some(Command::GotoPrefix));
        // g types in Edit mode
        assert_eq!(lookup(Mode::Edit, None, &key(KeyCode::Char('g'), KeyModifiers::NONE)), Some(Command::InsertChar('g')));
    }

    #[test]
    fn goto_prefix_d_and_o_and_cancel() {
        // After GotoPrefix, d → GotoDef, o → JumpBack, other → None (cancel)
        assert_eq!(lookup(Mode::Action, Some(Pending::GotoPrefix), &key(KeyCode::Char('d'), KeyModifiers::NONE)), Some(Command::GotoDef));
        assert_eq!(lookup(Mode::Action, Some(Pending::GotoPrefix), &key(KeyCode::Char('o'), KeyModifiers::NONE)), Some(Command::JumpBack));
        assert_eq!(lookup(Mode::Action, Some(Pending::GotoPrefix), &key(KeyCode::Char('x'), KeyModifiers::NONE)), None, "any other char cancels");
        assert_eq!(lookup(Mode::Action, Some(Pending::GotoPrefix), &key(KeyCode::Esc, KeyModifiers::NONE)), None, "Esc cancels");
    }

    #[test]
    fn goto_def_jump_back_names_resolve() {
        assert_eq!(command_by_name("goto_def"), Some(Command::GotoDef));
        assert_eq!(command_by_name("jump_back"), Some(Command::JumpBack));
    }
}
