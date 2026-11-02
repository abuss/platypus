mod app;
mod document;
mod render;
mod undo;

use std::path::PathBuf;

fn main() {
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let app = app::App::new(paths);
    let mut terminal = ratatui::init();
    let _ = terminal.draw(|f| render::draw(f, &app));
    ratatui::restore();
}
