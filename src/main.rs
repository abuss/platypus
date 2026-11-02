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
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let mut app = app::App::new(paths);
    let mut terminal = ratatui::init();
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
        if let Some(key) = input::read_key(Duration::from_millis(10)) {
            let cmd = if app.prompt.is_some() {
                keymap::lookup_prompt(&key)
            } else {
                keymap::lookup(app.mode, app.pending, &key)
            };
            if let Some(cmd) = cmd {
                app.handle_command(cmd);
                if app.quit {
                    break;
                }
            } else {
                app.pending = None; // any unbound key cancels a pending capture
            }
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
