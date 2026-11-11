//! Global workspace search (#15): spawns `rg --vimgrep` under the tree root
//! and streams parsed hits to the sidebar. Independent of the tree browser —
//! rg owns all scanning/ignore logic; plat only parses output lines.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Hard cap on collected hits (memory + render sanity). rg may keep running;
/// the app kills it once the cap is reached.
pub const MAX_HITS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf, // absolute
    pub line0: usize,  // 0-based, matches apply_goto_location
    pub col0: usize,   // 0-based; utf8 BYTE column (ponytail: good enough for
    // caret placement; switch to utf16 units if ever misaligned)
    pub text: String, // the matching source line, trimmed
}

/// Parse one `rg --vimgrep` line: `path:line:col:text`. Paths containing ':'
/// are handled by taking the LEFTMOST split where both numbers parse (a path
/// segment can't produce two consecutive integer fields).
pub fn parse_vimgrep_line(line: &str, root: &Path) -> Option<SearchHit> {
    let mut found: Option<(usize, usize, usize)> = None; // colon positions c1,c2,c3
    for (c1, _) in line.match_indices(':') {
        let after1 = &line[c1 + 1..];
        let j = after1.find(':')?; // line field has no ':' → deterministic
        let after2 = &after1[j + 1..];
        let k = after2.find(':')?; // col field likewise
        if !after1[..j].is_empty()
            && after1[..j].parse::<usize>().is_ok()
            && !after2[..k].is_empty()
            && after2[..k].parse::<usize>().is_ok()
        {
            found = Some((c1, c1 + 1 + j, c1 + 1 + j + 1 + k));
            break;
        }
    }
    let (c1, c2, c3) = found?;
    let line1: usize = line[c1 + 1..c2].parse().ok()?;
    let col1: usize = line[c2 + 1..c3].parse().ok()?;
    if line1 == 0 || col1 == 0 {
        return None; // rg fields are 1-based; 0 means malformed
    }
    let path = &line[..c1];
    let path = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let text = line[c3 + 1..].trim_end_matches('\r').trim().to_string();
    Some(SearchHit {
        path,
        line0: line1 - 1,
        col0: col1 - 1,
        text,
    })
}

pub struct SearchJob {
    child: Child,
    rx: Receiver<SearchHit>,
    pub done: bool,
}

impl SearchJob {
    /// Spawn rg in `root`. Err on spawn failure (e.g. rg not installed).
    pub fn spawn(root: &Path, query: &str) -> Result<Self, String> {
        let mut child = Command::new("rg")
            .args([
                "--vimgrep",
                "--no-heading",
                "-S",
                "--no-messages",
                "--color",
                "never",
            ])
            .arg("--")
            .arg(query)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("global search: {e} (is rg installed?)"))?;
        let out = child.stdout.take().expect("piped stdout");
        let (tx, rx) = channel();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(hit) = parse_vimgrep_line(&line, &root) {
                    if tx.send(hit).is_err() {
                        break; // app stopped listening
                    }
                }
            }
        });
        Ok(SearchJob {
            child,
            rx,
            done: false,
        })
    }

    /// Nonblocking drain of up to `max` parsed hits; second element = stream ended.
    pub fn drain(&mut self, max: usize) -> (Vec<SearchHit>, bool) {
        let mut v = Vec::new();
        for _ in 0..max {
            match self.rx.try_recv() {
                Ok(h) => v.push(h),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.done = true;
                    break;
                }
            }
        }
        (v, self.done)
    }

    /// Stop rg and reap it (restart/close paths).
    pub fn stop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_relative_hit() {
        let h = parse_vimgrep_line("src/app.rs:12:5:let foo = bar;", Path::new("/proj"))
            .expect("parsed");
        assert_eq!(h.path, PathBuf::from("/proj/src/app.rs"));
        assert_eq!((h.line0, h.col0), (11, 4));
        assert_eq!(h.text, "let foo = bar;");
    }

    #[test]
    fn parse_absolute_and_text_with_colons() {
        let h = parse_vimgrep_line(
            "/tmp/x/main.rs:1:1:https://example.com",
            Path::new("/ignored"),
        )
        .expect("parsed");
        assert_eq!(h.path, PathBuf::from("/tmp/x/main.rs"));
        assert_eq!(h.text, "https://example.com");
    }

    #[test]
    fn parse_path_containing_colon_picks_leftmost_numeric_split() {
        // Dir literally named "a:1" — the naive first-two-colons split would
        // read path="a", line=... garbage; the leftmost-valid-numeric rule wins.
        let h = parse_vimgrep_line("a:1/f.rs:7:2:x: y", Path::new("/r")).expect("parsed");
        assert_eq!(h.path, PathBuf::from("/r/a:1/f.rs"));
        assert_eq!((h.line0, h.col0), (6, 1));
        assert_eq!(h.text, "x: y");
    }

    #[test]
    fn parse_rejects_garbage_and_zero_fields() {
        assert!(parse_vimgrep_line("not a hit line", Path::new("/r")).is_none());
        assert!(parse_vimgrep_line("f.rs:0:3:x", Path::new("/r")).is_none());
        assert!(parse_vimgrep_line("f.rs:2:abc:x", Path::new("/r")).is_none());
    }

    #[test]
    fn spawn_drains_real_rg_and_reaps() {
        // rg on the dev machine is as much a fixture as git in flow tests.
        let dir = std::env::temp_dir().join(format!("plat_search_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "needle one\nkeep\n").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "needle two\n").unwrap();
        let mut job = SearchJob::spawn(&dir, "needle").expect("rg spawned");
        let mut hits = Vec::new();
        let mut done = false;
        let start = std::time::Instant::now();
        while !done && start.elapsed() < std::time::Duration::from_secs(10) {
            let (v, d) = job.drain(100);
            hits.extend(v);
            done = d;
            if !done {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert_eq!(hits.len(), 2, "hits: {hits:?}");
        assert!(
            hits.iter()
                .all(|h| h.path.starts_with(&dir) && h.text.starts_with("needle"))
        );
        job.stop(); // reaped; no zombie left
        std::fs::remove_dir_all(&dir).ok();
    }
}
