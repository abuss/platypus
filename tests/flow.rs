// plat flow test — the plan's spec test stood in for event injection at the
// DECODE layer: real KeyEvents through keymap::lookup + App::handle_command
// (crossterm has no stdin-injection API; the real-terminal path is covered by
// the manual pty smoke recorded in README).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use plat::app::{App, Mode};
use plat::keymap;
use std::path::PathBuf;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let k = key(code, mods);
    let cmd = if app.prompt.is_some() {
        keymap::lookup_prompt(&k)
    } else {
        keymap::lookup(app.mode, app.pending, &k)
    };
    if let Some(cmd) = cmd {
        app.handle_command(cmd);
    } else {
        app.pending = None;
    }
}

fn type_text(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

#[test]
fn start_typing_and_escape() {
    let mut app = App::new(vec![]);
    type_text(&mut app, "hi"); // start typing works with zero Esc
    assert_eq!(app.doc().rope.to_string(), "hi");
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.mode, Mode::Action);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.mode, Mode::Edit);
    // Ctrl-Q on a dirty buffer must ask, never silently discard
    press(&mut app, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(!app.quit);
    assert!(app.prompt.is_some());
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE); // Cancel -> Save
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE); // Save -> Discard
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.quit);
}

#[test]
fn full_edit_flow_saves_expected_bytes() {
    let dir = std::env::temp_dir().join(format!("plat-flow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("flow.txt");
    std::fs::write(&path, "hello world").unwrap();

    let mut app = App::new(vec![PathBuf::from(&path)]);
    // start at end: type_text of existing file? cursor starts 0 — jump to End
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE); // ACTION
    press(&mut app, KeyCode::Char('b'), KeyModifiers::NONE); // w back: 'world' start
    for _ in 0..5 {
        press(&mut app, KeyCode::Right, KeyModifiers::SHIFT); // select 'world'
    }
    assert_eq!(app.doc().selection(), Some((6, 11)));
    press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE); // cut
    assert_eq!(app.doc().rope.to_string(), "hello ");
    press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE); // paste after
    assert_eq!(app.doc().rope.to_string(), "hello world");
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE); // EDIT
    type_text(&mut app, "!");
    press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL); // save
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world!");
    assert!(!app.doc().dirty);
    press(&mut app, KeyCode::Char('q'), KeyModifiers::CONTROL); // clean -> quit no prompt
    assert!(app.quit && app.prompt.is_none());
    std::fs::remove_file(&path).ok();
}
