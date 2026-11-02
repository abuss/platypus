mod app;
mod document;
mod input;
mod keymap;
mod render;
mod undo;

use std::path::PathBuf;
use std::time::Duration;

fn main() {
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let mut app = app::App::new(paths);
    let mut terminal = ratatui::init();
    let mut last_shape = None;
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
            if let Some(cmd) = keymap::lookup(app.mode, &key) {
                app.handle_command(cmd);
                if app.quit {
                    break;
                }
            }
        }
    }
    ratatui::restore();
}
