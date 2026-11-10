// Persistent most-recently-opened files (issue #37).
//
// A plain JSON array of absolute paths under the XDG data dir. Writes are
// write-through (tiny file) so a crash keeps the history. Missing entries are
// dropped at load — files get moved/deleted, the list heals itself.

use std::path::{Path, PathBuf};

pub const MAX_RECENT: usize = 25;

#[derive(Debug, Default, Clone)]
pub struct Recent {
    pub paths: Vec<PathBuf>,
}

impl Recent {
    pub fn default_file() -> PathBuf {
        Self::data_dir().join("platypus/recent.json")
    }

    fn data_dir() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from(".local/share"))
    }

    pub fn load_at(file: &Path) -> Recent {
        let Ok(bytes) = std::fs::read(file) else { return Recent::default() };
        // tolerate corrupt/foreign JSON: start fresh rather than fail the boot
        let raw: Vec<String> = serde_json::from_slice(&bytes).unwrap_or_default();
        Recent {
            paths: raw.into_iter().map(PathBuf::from).filter(|p| p.is_file()).take(MAX_RECENT).collect(),
        }
    }

    pub fn save_at(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = serde_json::to_vec(&self.paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(file, body)
    }

    /// canonicalize when possible (new files not on disk yet fall back to an
    /// absolute path), dedup, front-insert, cap
    pub fn record(&mut self, path: &Path) {
        let abs = path.canonicalize().unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().map(|c| c.join(path)).unwrap_or_else(|_| path.to_path_buf())
            }
        });
        self.paths.retain(|p| *p != abs);
        self.paths.insert(0, abs);
        self.paths.truncate(MAX_RECENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_moves_to_front_dedups_and_caps() {
        let dir = std::env::temp_dir().join(format!("plat_recent_mod_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = (0..(MAX_RECENT + 5))
            .map(|i| {
                let p = dir.join(format!("f{i}.rs"));
                std::fs::write(&p, "x").unwrap();
                p
            })
            .collect();
        let mut r = Recent::default();
        for f in &files {
            r.record(f);
        }
        assert_eq!(r.paths.len(), MAX_RECENT, "capped");
        assert_eq!(r.paths[0], *files.last().unwrap().canonicalize().unwrap(), "newest first");
        r.record(&files[3]);
        assert_eq!(r.paths.iter().filter(|p| **p == files[3].canonicalize().unwrap()).count(), 1, "deduped");
        assert_eq!(r.paths[0], files[3].canonicalize().unwrap(), "re-recorded path jumps to front");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip_drops_missing_and_survives_garbage() {
        let dir = std::env::temp_dir().join(format!("plat_recent_io_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("nested/recent.json"); // parent must be auto-created
        let keep = dir.join("keep.rs");
        std::fs::write(&keep, "x").unwrap();
        let gone = dir.join("gone.rs");
        let mut r = Recent::default();
        r.paths = vec![gone.clone(), keep.clone()];
        r.save_at(&file).unwrap();
        let loaded = Recent::load_at(&file);
        assert_eq!(loaded.paths, vec![keep], "missing paths self-heal away at load");
        std::fs::write(&file, b"not json at all").unwrap();
        assert_eq!(Recent::load_at(&file).paths, Vec::<PathBuf>::new(), "corrupt file: fresh start, no crash");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
