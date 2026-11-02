use std::path::PathBuf;

use crate::document::Document;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    Edit,
    Action,
}

pub struct App {
    pub docs: Vec<Document>,
    pub active: usize,
    pub mode: Mode,
    /// (message, is_error)
    pub status: Option<(String, bool)>,
}

impl App {
    pub fn new(paths: Vec<PathBuf>) -> App {
        let mut docs = Vec::new();
        let mut status = None;
        for path in paths {
            match Document::open(&path) {
                Ok(doc) => docs.push(doc),
                Err(e) => status = Some((format!("{}: {e:?}", path.display()), true)),
            }
        }
        if docs.is_empty() {
            docs.push(Document::empty());
        }
        App { docs, active: 0, mode: Mode::Edit, status }
    }

    pub fn doc(&self) -> &Document {
        &self.docs[self.active]
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.docs[self.active]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_opens_paths_or_empty_buffer() {
        let dir = std::env::temp_dir().join(format!("plat-t3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.txt");
        std::fs::write(&p, "x").unwrap();
        let app = App::new(vec![p.clone()]);
        assert_eq!(app.docs.len(), 1);
        assert_eq!(app.mode, Mode::Edit);
        let empty = App::new(vec![dir.join("missing.txt")]);
        assert_eq!(empty.docs.len(), 1);
        assert!(empty.status.as_ref().unwrap().1, "failed open must set error status");
        std::fs::remove_file(&p).ok();
    }
}
