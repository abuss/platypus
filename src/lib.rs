pub mod app;
pub mod config;
pub mod document;
pub mod git;
pub mod gitignore;
pub mod lsp;
pub mod picker;
pub mod preview;
pub mod search;
pub mod sidebar;
pub mod terminal;
pub mod tree;
pub mod input;
pub mod keymap;
pub mod prompt;
pub mod render;
pub mod syntax;
pub mod theme;
pub mod undo;
pub mod watch;

use terminal::encode_key;
fn bracketed_paste(on: bool) {
    use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
    let mut out = std::io::stdout();
    if on {
        let _ = crossterm::execute!(out, EnableBracketedPaste);
    } else {
        let _ = crossterm::execute!(out, DisableBracketedPaste);
    }
}

/// Mouse reporting on/off (#11). Disabling is always safe, enabling is
/// gated by [ui] mouse in the caller; with capture off the terminal keeps
/// its native click-drag selection.
fn mouse_capture(on: bool) {
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    let mut out = std::io::stdout();
    if on {
        let _ = crossterm::execute!(out, EnableMouseCapture);
    } else {
        let _ = crossterm::execute!(out, DisableMouseCapture);
    }
}

fn kitty_keyboard(on: bool) {
    // kitty keyboard protocol: DISAMBIGUATE_ESCAPE_CODES only — it splits the
    // legacy control-byte aliases (Shift-Ctrl chords arrive distinct, Ctrl-,
    // and Ctrl-\ stop colliding at 0x1c) without event flags, so press/repeat
    // semantics stay exactly as today. Unsupported terminals ignore both
    // sequences (CSI > u / CSI < u are unrecognized DCS-prefixed CSI — dropped).
    use crossterm::event::{
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    };
    if on {
        let _ = crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    } else {
        let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
}

pub fn run() {
    if std::env::var("PLAT_LSP_MOCK").is_ok() {
        lsp::mock::serve();
        std::process::exit(0);
    }

    // panic safety first: the terminal must come back before any panic prints
    std::panic::set_hook(Box::new(|info| {
        bracketed_paste(false);
        mouse_capture(false);
        kitty_keyboard(false);
        let _ = ratatui::restore();
        eprintln!("{info}");
    }));

    let paths: Vec<std::path::PathBuf> = std::env::args().skip(1).map(std::path::PathBuf::from).collect();
    let config = match config::load() {
        Ok(c) => c,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };
    let mut app = app::App::new_with_config(paths, config);
    let mut terminal = ratatui::init();
    kitty_keyboard(true); // push AFTER raw mode: distinct Shift-Ctrl codes on supporting terminals
    // Seed the real screen size NOW: crossterm never emits a Resize event at
    // startup, so app.last_size would stay at its (80,24) default and every
    // terminal-height clamp would compute against a phantom 24-row screen
    // (2026-11-06: dynamic pane resize ignored the configured height).
    app.last_size = terminal.size().map(|s| (s.width, s.height)).unwrap_or((80, 24));
    bracketed_paste(true); // pasted text arrives as Event::Paste, not typed keys
    let mouse_on = app.config.ui.mouse;
    if mouse_on {
        mouse_capture(true); // clicks/wheel arrive as Event::Mouse (#11)
    }
    if std::env::var("PLAT_PANIC").is_ok() {
        panic!("PLAT_PANIC=1 dev switch: injected panic to verify terminal restore (see README)");
    }
    let mut last_shape = None;
    let mut last_poll = std::time::Instant::now();
    loop {
        app.ensure_syntax(terminal.size().map(|s| s.height.saturating_sub(2) as usize).unwrap_or(22));
        app.refresh_branch();
        app.pump_git_marks();
        app.lsp_pump();
        app.lsp_sync_pass(std::time::Instant::now());
        // Pump PTY output every loop iteration so the terminal screen stays fresh.
        if let Some(term) = &mut app.terminal {
            let _ = term.pump_output();
        }
        app.search_pump(std::time::Instant::now());
        let _ = terminal.draw(|f| render::draw(f, &app));
        let shape = match app.mode {
            app::Mode::Edit => crossterm::cursor::SetCursorStyle::SteadyBar,
            app::Mode::Action => crossterm::cursor::SetCursorStyle::SteadyBlock,
        };
        if last_shape != Some(std::mem::discriminant(&shape)) {
            last_shape = Some(std::mem::discriminant(&shape));
            let _ = crossterm::execute!(std::io::stdout(), shape);
        }
        if let Some(event) = input::read_event(std::time::Duration::from_millis(10)) {
            match event {
                crossterm::event::Event::Key(key) => {
                    if app.key_debug {
                        app.status = Some((format!("{key:?}"), false));
                    }
                    // Priority: Prompt → Palette → Completion → Terminal → Sidebar → Normal.
                    match app.key_owner(&key) {
                        app::KeyOwner::Prompt => {
                            if let Some(cmd) = keymap::lookup_prompt(&key) {
                                app.handle_command(cmd);
                            } else {
                                app.pending = None;
                            }
                        }
                        app::KeyOwner::Palette => { app.handle_palette_key(&key); }
                        app::KeyOwner::Completion => { app.handle_completion_key(&key); }
                        app::KeyOwner::Terminal => {
                            // PgUp/PgDn: scroll the pane a full page at a time.
                            if matches!(key.code, crossterm::event::KeyCode::PageUp) {
                                if let Some(term) = &mut app.terminal {
                                    let page = term.height.max(1) as i16;
                                    term.scroll(-page);
                                }
                            } else if matches!(key.code, crossterm::event::KeyCode::PageDown) {
                                if let Some(term) = &mut app.terminal {
                                    let page = term.height.max(1) as i16;
                                    term.scroll(page);
                                }
                            } else if let Some(term) = &mut app.terminal {
                                if !term.exited {
                                    // Any key typed into the shell returns to live output.
                                    term.snap_to_live();
                                    let encoded = encode_key(key, term.keypad_mode());
                                    if !encoded.is_empty() { term.write_input(&encoded); }
                                }
                            }
                        }
                        app::KeyOwner::Sidebar => {
                            if let Some(cmd) = keymap::lookup_sidebar(&key) {
                                app.handle_command(cmd);
                            } else {
                                app.pending = None;
                            }
                        }
                        app::KeyOwner::LegacyToggle => {
                            app.handle_command(keymap::Command::ToggleTerminal);
                        }
                        app::KeyOwner::Normal => {
                            let cmd = keymap::lookup_with(&app.config.keys, app.mode, app.pending, &key);
                            if let Some(cmd) = cmd {
                                app.handle_command(cmd);
                            } else {
                                app.pending = None;
                            }
                        }
                    }
                }
                crossterm::event::Event::Resize(w, h) => render::apply_resize(&mut app, (w, h)),
                crossterm::event::Event::Mouse(m) => {
                    let (w, h) = terminal.size().map(|s| (s.width, s.height)).unwrap_or((80, 24));
                    app.handle_mouse(w, h, &m);
                }
                crossterm::event::Event::Paste(s) => {
                    // If terminal is visible, paste raw bytes to shell (no control filtering).
                    // Phase-1: mouse clicks over terminal area not routed (Task 9).
                    if let Some(term) = &mut app.terminal {
                        if term.visible && !term.exited {
                            term.write_input(s.as_bytes());
                            continue;
                        }
                    }
                    match app.prompt.as_mut() {
                        Some(p) => p.line.extend(s.chars().filter(|c| !c.is_control())),
                        None => app.handle_paste(&s),
                    }
                }
                _ => {}
            }
        }
        if app.suspend_requested {
            app.suspend_requested = false;
            drop(terminal);
            bracketed_paste(false);
            mouse_capture(false);
            kitty_keyboard(false); // the suspended shell must get plain encodings
            ratatui::restore();
            // mtime_cache intentionally left stale: entries from before suspend
            // make the next watcher poll flag anything external tools changed.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGTSTP);
            }
            terminal = ratatui::init();
            kitty_keyboard(true); // re-push for the resumed session
            bracketed_paste(true);
            if mouse_on {
                mouse_capture(true);
            }
            if let Ok((w, h)) = crossterm::terminal::size() {
                render::apply_resize(&mut app, (w, h));
            }
        }
        if app.quit {
            break;
        }
        if last_poll.elapsed() >= std::time::Duration::from_millis(1000) {
            last_poll = std::time::Instant::now();
            let changed = watch::changed_paths(&app.docs, &mut app.mtime_cache);
            for path in changed {
                app.handle_external_change(&path);
            }
        }
    }
    bracketed_paste(false);
    mouse_capture(false);
    kitty_keyboard(false);
    ratatui::restore();
    // Spec lifecycle: graceful shutdown+exit per server, kill after 300ms.
    // After restore so the bounded sleep never blocks terminal handoff visually.
    app.lsp.shutdown_all();
}
