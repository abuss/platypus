//! Git gutter marks (#13): which lines of a document differ from HEAD.
//! The diff is HEAD-vs-disk, refreshed on open and after every save; while a
//! buffer is dirty the marks are as-of the last save.
//! ponytail: upgrade path for live-buffer marks = temp file + `git diff
//! --no-index`, still no new dependencies.

use std::path::Path;
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkKind {
    Added,
    Modified,
    Deleted, // boundary marker at the line the removal happened before
}

/// One gutter mark: 0-based line in the NEW (current) file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GitMark {
    pub kind: MarkKind,
    pub line0: usize,
}

/// Parse `git diff --no-color -U0` output into gutter marks. Only the `@@`
/// hunk headers matter with -U0: `b` counts old-side removed lines, `n` the
/// new-side block. Paired = Modified, insertion-only = Added, removal-only =
/// Deleted (boundary row). Adjacent add+remove inside one hunk collapses to
/// Modified — acceptable for a gutter.
pub fn parse_unified_diff(diff: &str) -> Vec<GitMark> {
    let mut marks = Vec::new();
    for line in diff.lines() {
        let rest = match line.strip_prefix("@@") {
            Some(r) => r,
            None => continue,
        };
        let close = match rest.rfind("@@") {
            Some(i) => i,
            None => continue,
        };
        let mut parts = rest[..close].split_whitespace();
        let (_a, b) = parse_range(parts.next().unwrap_or(""));
        let (c, n) = parse_range(parts.next().unwrap_or(""));
        if n > 0 {
            let kind = if b > 0 {
                MarkKind::Modified
            } else {
                MarkKind::Added
            };
            let start = c.saturating_sub(1);
            for l in start..(start + n) {
                marks.push(GitMark { kind, line0: l });
            }
        } else if b > 0 {
            marks.push(GitMark {
                kind: MarkKind::Deleted,
                line0: c.saturating_sub(1),
            });
        }
    }
    marks.sort_by_key(|m| m.line0);
    marks
}

/// "-a,b" or "-a" (b=1) → (a, b); leading +/- stripped.
fn parse_range(s: &str) -> (usize, usize) {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    match s.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
        None => (s.parse().unwrap_or(0), if s.is_empty() { 0 } else { 1 }),
    }
}

/// Spawn `git diff HEAD -U0 -- <path>` off-thread; the receiver yields the
/// marks (empty on any failure: not a repo, git missing, untracked file).
pub fn spawn_diff(path: &Path) -> Receiver<Vec<GitMark>> {
    let (tx, rx) = channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        // `plat sub/file.txt` keeps a relative doc path: git resolves args
        // against the child cwd, so absolute-ize first (canonicalize also
        // fixes parent-of-bare-name). unwrap_or keeps odd paths best-effort.
        let p = p.canonicalize().unwrap_or(p);
        let dir = p
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| Path::new(".").into());
        let out = std::process::Command::new("git")
            .args(["diff", "--no-color", "--no-ext-diff", "-U0", "HEAD", "--"])
            .arg(&p)
            .current_dir(&dir)
            .output()
            .ok()
            .filter(|o| o.status.success());
        let marks = match out {
            Some(o) => parse_unified_diff(&String::from_utf8_lossy(&o.stdout)),
            None => Vec::new(),
        };
        let _ = tx.send(marks);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_added_lines() {
        let diff = "@@ -0,0 +1,2 @@\n+new one\n+new two\n";
        assert_eq!(
            parse_unified_diff(diff),
            vec![
                GitMark {
                    kind: MarkKind::Added,
                    line0: 0
                },
                GitMark {
                    kind: MarkKind::Added,
                    line0: 1
                },
            ]
        );
    }

    #[test]
    fn parse_modified_block() {
        let diff = "@@ -3,2 +3,4 @@\n";
        assert_eq!(
            parse_unified_diff(diff),
            (2..6)
                .map(|l| GitMark {
                    kind: MarkKind::Modified,
                    line0: l
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_deleted_marks_boundary_line() {
        // three lines removed; new file line 6 (1-based) follows the gap
        let diff = "@@ -5,3 +7,0 @@\n";
        assert_eq!(
            parse_unified_diff(diff),
            vec![GitMark {
                kind: MarkKind::Deleted,
                line0: 6
            }]
        );
    }

    #[test]
    fn parse_multi_hunk_sorted_and_skips_headers() {
        let diff = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -10,1 +10,2 @@\n+x\n@@ -20,2 +22,0 @@\n";
        assert_eq!(
            parse_unified_diff(diff),
            vec![
                GitMark {
                    kind: MarkKind::Modified,
                    line0: 9
                },
                GitMark {
                    kind: MarkKind::Modified,
                    line0: 10
                },
                GitMark {
                    kind: MarkKind::Deleted,
                    line0: 21
                },
            ]
        );
    }

    #[test]
    fn parse_empty_diff_yields_no_marks() {
        assert!(parse_unified_diff("").is_empty());
    }

    #[test]
    fn real_git_diff_end_to_end() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("plat_git_{}", std::process::id()));
        let f = dir.join("f.txt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&f, "keep\nchange me\ngone\nkeep2\n").unwrap();
        let g = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap()
        };
        g(&["init", "-q"]);
        g(&["add", "."]);
        g(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "base",
        ]);
        std::fs::write(&f, "keep\nchanged\nkeep2\nadded\n").unwrap();
        let marks = g(&["diff", "--no-color", "-U0", "HEAD", "--", "f.txt"]);
        let got = parse_unified_diff(&String::from_utf8_lossy(&marks.stdout));
        assert_eq!(
            got,
            vec![
                // git merges "change me"+gone -> changed into ONE hunk (-2,2 +2,1):
                // the collapsed Modified rule (see parse docs); no separate Deleted row
                GitMark {
                    kind: MarkKind::Modified,
                    line0: 1
                },
                GitMark {
                    kind: MarkKind::Added,
                    line0: 3
                },
            ],
            "raw diff was:\n{}",
            String::from_utf8_lossy(&marks.stdout)
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
