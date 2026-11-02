use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::document::Document;

/// Flag docs whose on-disk mtime differs from the cached one; refreshes the
/// cache. A doc absent from the cache gets baselined (no event).
/// ponytail: mtime polling at 1s granularity in the main loop; switch to
/// inotify (notify crate) if sub-second latency or battery matters.
pub fn changed_paths(
    docs: &[Document],
    cache: &mut HashMap<PathBuf, SystemTime>,
) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    for doc in docs {
        if let Some(path) = &doc.path {
            match fs::metadata(path).map(|m| m.modified().unwrap_or(SystemTime::UNIX_EPOCH)) {
                Err(_) => {
                    cache.remove(path); // vanished; will flag if it reappears
                }
                Ok(mtime) => match cache.insert(path.clone(), mtime) {
                    Some(old) if old != mtime => changed.push(path.clone()),
                    _ => {}
                },
            }
        }
    }
    changed
}

/// Current on-disk mtime for rebaselining after a resolved conflict.
pub fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_mtime_change_exactly_once() {
        let dir = std::env::temp_dir().join(format!("plat-w10-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("w.txt");
        fs::write(&p, "a").unwrap();
        let doc = Document::open(&p).unwrap();
        let mut cache = HashMap::new();
        assert!(changed_paths(std::slice::from_ref(&doc), &mut cache).is_empty(), "baseline: no event");
        fs::write(&p, "bb").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(changed_paths(std::slice::from_ref(&doc), &mut cache), vec![p.clone()]);
        assert!(changed_paths(std::slice::from_ref(&doc), &mut cache).is_empty(), "flagged once only");
        fs::remove_file(&p).ok();
    }
}
