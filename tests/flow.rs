// plat flow test — the plan's spec test stood in for event injection at the
// DECODE layer: real KeyEvents through keymap::lookup + App::handle_command
// (crossterm has no stdin-injection API; the real-terminal path is covered by
// the manual pty smoke recorded in README).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use plat::app::{App, Mode};
use plat::keymap;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use std::path::PathBuf;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let k = key(code, mods);
    // mirrors lib.rs::run() routing: prompt owns keys, then sidebar, else mode+overlay
    let cmd = if app.prompt.is_some() {
        keymap::lookup_prompt(&k)
    } else if app.sidebar.is_some() {
        keymap::lookup_sidebar(&k)
    } else {
        keymap::lookup_with(&app.config.keys, app.mode, app.pending, &k)
    };
    if let Some(cmd) = cmd {
        app.handle_command(cmd);
    } else {
        app.pending = None;
    }
    app.ensure_syntax(22);
    app.refresh_branch();
}

fn type_text(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn buffer_of(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| plat::render::draw(f, app)).unwrap();
    term.backend().buffer().clone()
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

// --- Phase 1: file sidebar (integration through the real decode layer) ---

#[test]
fn ctrl_p_picker_filters_and_opens_a_file() {
    let mut app = App::new(vec![]);
    press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert!(app.sidebar.is_some(), "Ctrl-P opens the picker");
    assert!(!app.sidebar.as_ref().unwrap().visible_items().is_empty(), "cwd has files");

    type_text(&mut app, "render");
    let visible = app.sidebar.as_ref().unwrap().visible_items().to_vec();
    assert!(visible.iter().any(|i| i.path.to_string_lossy().contains("render")), "filtered to render*: {visible:?}");
    assert!(!visible.iter().any(|i| i.path.to_string_lossy().contains("flow")), "non-matches filtered out");

    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.sidebar.is_none(), "sidebar closes after opening a file");
    let opened = app.doc().path.as_ref().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    assert_eq!(opened, Some("render.rs".to_string()), "top-ranked match opened as the active buffer");
}

#[test]
fn ctrl_b_tree_expands_a_dir_and_opens_a_child() {
    let mut app = App::new(vec![]);
    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert!(app.sidebar.is_some(), "Ctrl-B opens the tree");

    // walk to the "src" dir (dirs sort first), expand it, step into a child
    let src = loop {
        let sel = app.sidebar.as_ref().unwrap().selected_path().unwrap();
        if sel.file_name().unwrap() == "src" {
            break sel;
        }
        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    };
    press(&mut app, KeyCode::Right, KeyModifiers::NONE);
    assert!(
        app.sidebar.as_ref().unwrap().visible_items().iter().any(|i| i.path.starts_with(&src) && !i.is_dir),
        "expanding src lists its children"
    );

    press(&mut app, KeyCode::Down, KeyModifiers::NONE); // first child of src
    let child = app.sidebar.as_ref().unwrap().selected_path().unwrap();
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.sidebar.is_none(), "sidebar closes after Enter");
    assert_eq!(app.doc().path.as_ref().unwrap(), &child, "the selected child became the active buffer");
}

#[test]
fn esc_closes_the_sidebar_without_opening_anything() {
    let mut app = App::new(vec![]);
    let docs = app.docs.len();
    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.sidebar.is_none(), "Esc closes the sidebar");
    assert_eq!(app.docs.len(), docs, "Esc opens nothing");
    assert_eq!(app.mode, Mode::Edit, "editor keeps its mode");
    // and the editor is back in charge of the keyboard
    type_text(&mut app, "hi");
    assert_eq!(app.doc().rope.to_string(), "hi");
}

#[test]
fn alt_arrows_resize_and_the_width_persists() {
    let mut app = App::new(vec![]);
    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(app.sidebar.as_ref().unwrap().width, 32, "default width");
    press(&mut app, KeyCode::Right, KeyModifiers::ALT);
    assert_eq!(app.sidebar.as_ref().unwrap().width, 34);
    for _ in 0..20 {
        press(&mut app, KeyCode::Left, KeyModifiers::ALT);
    }
    assert_eq!(app.sidebar.as_ref().unwrap().width, 10, "clamped to the minimum");
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert_eq!(app.sidebar.as_ref().unwrap().width, 10, "width persists across close/reopen");
}

#[test]
fn a_closed_mode_switch_does_not_leak_state_and_globals_still_work() {
    let dir = std::env::temp_dir().join(format!("plat-sb-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("keep.txt");
    std::fs::write(&path, "").unwrap();

    let mut app = App::new(vec![path.clone()]);
    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    type_text(&mut app, "abc"); // tree mode ignores typing
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL); // switch to picker

    let sidebar = app.sidebar.as_ref().unwrap();
    assert_eq!(sidebar.mode, plat::sidebar::SidebarMode::FuzzyPicker, "mode switched");
    assert_eq!(sidebar.input, "", "tree-mode keystrokes did not leak into the filter");
    assert_eq!(sidebar.cursor, 0, "cursor did not leak");

    press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL); // Ctrl-S still saves
    assert!(!app.doc().dirty);
    assert_eq!(app.sidebar.as_ref().unwrap().mode, plat::sidebar::SidebarMode::FuzzyPicker, "save did not close the sidebar");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_remap_and_unbind_work_through_real_key_events() {
    let cfg = plat::config::from_str("[keys]\n\"C-Space\" = \"select_line\"\n\"x\" = \"none\"").unwrap();
    let mut app = App::new_with_config(vec![], cfg);
    type_text(&mut app, "hello");
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);          // ACTION
    // the default `x` (select_line) is gone:
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(app.doc().selection(), None, "unbind must not fall through");
    // and C-Space now selects the line:
    press(&mut app, KeyCode::Char(' '), KeyModifiers::CONTROL);
    assert_eq!(app.doc().selection(), Some((0, 5)));
}

#[test]
fn regex_replace_walkthrough_end_to_end() {
    let dir = std::env::temp_dir().join(format!("plat-flow-repl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("repl.txt");
    std::fs::write(&path, "due 2026-11-03 and 2026-11-04\n").unwrap();

    let mut app = App::new(vec![PathBuf::from(&path)]);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE); // ACTION
    press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    type_text(&mut app, r"(\d+)-(\d+)-(\d+)");
    assert_eq!(app.doc().search_matches.len(), 2, "live regex preview while typing");
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE); // Tab -> replacement field
    type_text(&mut app, "$3/$1/$2");
    assert_eq!(app.prompt.as_ref().unwrap().line, r"(\d+)-(\d+)-(\d+)", "Tab switched fields");
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE); // walkthrough starts
    assert!(app.replace.is_some());
    press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE); // first date only
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE); // rest in one batch
    assert_eq!(app.doc().rope.to_string(), "due 03/2026/11 and 04/2026/11\n");
    press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL); // save
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "due 03/2026/11 and 04/2026/11\n");
    press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL); // one undo = the batch
    assert_eq!(app.doc().rope.to_string(), "due 03/2026/11 and 2026-11-04\n");
    std::fs::remove_file(&path).ok();
}

#[test]
fn syntax_survives_edit_undo_and_reload() {
    let dir = std::env::temp_dir().join(format!("plat-flow-syntax-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.rs");
    std::fs::write(&path, "fn a(){}\n").unwrap();

    let mut app = App::new(vec![PathBuf::from(&path)]);
    app.show_line_numbers = false;  // Disable line numbers for clean testing
    app.ensure_syntax(22);
    let buf = buffer_of(&app, 80, 24);
    // 'f' of "fn" keyword should have LightMagenta color from syntax highlighting
    let cell = &buf[(0, 0)];
    assert_eq!(cell.fg, Color::LightMagenta, "keyword 'fn' should be LightMagenta after open");

    // Type an edit: add a space
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    type_text(&mut app, " // test");
    let buf = buffer_of(&app, 80, 24);
    let cell = &buf[(0, 0)];
    assert_eq!(cell.fg, Color::LightMagenta, "keyword 'fn' should still be LightMagenta after edit");

    // Undo the edit
    press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    let buf = buffer_of(&app, 80, 24);
    let cell = &buf[(0, 0)];
    assert_eq!(cell.fg, Color::LightMagenta, "keyword 'fn' should still be LightMagenta after undo");

    // Save the document so it's clean for the reload test
    press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    
    // Write new content to disk
    std::fs::write(&path, "fn zz(){} // hello world\n").unwrap();
    app.handle_external_change(&path);
    app.ensure_syntax(22);
    let buf = buffer_of(&app, 80, 24);
    
    // The new keyword 'fn' should still be colored
    let cell = &buf[(0, 0)];
    assert_eq!(cell.fg, Color::LightMagenta, "keyword 'fn' should be LightMagenta in reloaded text");
    
    // The new text should have different content
    assert!(app.doc().rope.to_string().contains("zz"), "reloaded content should show new function name");
    
    // Verify syntax was rebuilt (query_count should have increased)
    let spans_count = app.doc().syntax.as_ref().map(|s| s.spans().len()).unwrap_or(0);
    assert!(spans_count > 0, "syntax should have spans after rebuild");

    std::fs::remove_dir_all(&dir).ok();
}
