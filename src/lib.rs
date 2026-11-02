pub mod app;
pub mod document;
pub mod input;
pub mod keymap;
pub mod prompt;
pub mod render;
pub mod undo;
pub mod watch;

fn bracketed_paste(on: bool) {
    use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
    let mut out = std::io::stdout();
    if on {
        let _ = crossterm::execute!(out, EnableBracketedPaste);
    } else {
        let _ = crossterm::execute!(out, DisableBracketedPaste);
    }
}

pub fn run() {
    // panic safety first: the terminal must come back before any panic prints
    std::panic::set_hook(Box::new(|info| {
        bracketed_paste(false);
        let _ = ratatui::restore();
        eprintln!("{info}");
    }));

    let paths: Vec<std::path::PathBuf> = std::env::args().skip(1).map(std::path::PathBuf::from).collect();
    let mut app = app::App::new(paths);
    let mut terminal = ratatui::init();
    bracketed_paste(true); // pasted text arrives as Event::Paste, not typed keys
    if std::env::var("PLAT_PANIC").is_ok() {
        panic!("PLAT_PANIC=1 dev switch: injected panic to verify terminal restore (see README)");
    }
    let mut last_shape = None;
    let mut last_poll = std::time::Instant::now();
    loop {
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
                    let cmd = if app.prompt.is_some() {
                        keymap::lookup_prompt(&key)
                    } else {
                        keymap::lookup(app.mode, app.pending, &key)
                    };
                    if let Some(cmd) = cmd {
                        app.handle_command(cmd);
                    } else {
                        app.pending = None; // any unbound key cancels a pending capture
                    }
                }
                crossterm::event::Event::Resize(w, h) => render::apply_resize(&mut app, (w, h)),
                crossterm::event::Event::Paste(s) => match app.prompt.as_mut() {
                    Some(p) => p.line.extend(s.chars().filter(|c| !c.is_control())),
                    None => app.handle_paste(&s),
                },
                _ => {}
            }
        }
        if app.suspend_requested {
            app.suspend_requested = false;
            drop(terminal);
            bracketed_paste(false);
            ratatui::restore();
            // mtime_cache intentionally left stale: entries from before suspend
            // make the next watcher poll flag anything external tools changed.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGTSTP);
            }
            terminal = ratatui::init();
            bracketed_paste(true);
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
    ratatui::restore();
}
