mod app;
mod document;
mod input;
mod keymap;
mod prompt;
mod render;
mod undo;
mod watch;

use std::path::PathBuf;
use std::time::Duration;

fn main() {
    // panic safety first: the terminal must come back before any panic prints
    std::panic::set_hook(Box::new(|info| {
        let _ = ratatui::restore();
        eprintln!("{info}");
    }));

    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let mut app = app::App::new(paths);
    let mut terminal = ratatui::init();
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
        if let Some(event) = input::read_event(Duration::from_millis(10)) {
            match event {
                crossterm::event::Event::Key(key) => {
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
                _ => {}
            }
        }
        if app.suspend_requested {
            app.suspend_requested = false;
            drop(terminal);
            ratatui::restore();
            // mtime_cache intentionally left stale: entries from before suspend
            // make the next watcher poll flag anything external tools changed.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGTSTP);
            }
            terminal = ratatui::init();
            if let Ok((w, h)) = crossterm::terminal::size() {
                render::apply_resize(&mut app, (w, h));
            }
        }
        if app.quit {
            break;
        }
        if last_poll.elapsed() >= Duration::from_millis(1000) {
            last_poll = std::time::Instant::now();
            let changed = watch::changed_paths(&app.docs, &mut app.mtime_cache);
            for path in changed {
                app.handle_external_change(&path);
            }
        }
    }
    ratatui::restore();
}
