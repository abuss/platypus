//! Markdown preview (issue #2): render the buffer through `glow` into a
//! plain-text preview buffer. glow does all markdown work; plat only feeds
//! stdin, strips ANSI styling, and unroll-buffers the result.

use std::io::Write;
use std::process::{Command, Stdio};

/// Render `source` markdown to styled-free plain text wrapped at `width`.
/// Blocking: glow is a local <100 ms render (ponytail: move to a worker
/// thread + mpsc like search/ if huge docs ever make it lag the loop).
pub fn render_markdown(source: &str, width: usize) -> Result<String, String> {
    render_markdown_with("glow", source, width)
}

/// Binary is injectable so the missing-glow error path is testable.
pub fn render_markdown_with(binary: &str, source: &str, width: usize) -> Result<String, String> {
    // -s dark: explicit style (the default "auto" degrades to raw passthrough
    // on non-TTY stdout — pty-probed). SGR is stripped below anyway; the style
    // only decides which ANSI to emit. -p: no pager. --width: our pane width.
    let mut child = Command::new(binary)
        .args(["-s", "dark", "-p", "--width", &width.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("markdown preview: {e} (is glow installed?)"))?;
    {
        let mut w = child.stdin.take().expect("piped stdin");
        // glow reads the whole document before writing anything, so writing
        // before reading cannot deadlock on the pipe buffer.
        w.write_all(source.as_bytes())
            .map_err(|e| format!("markdown preview: {e}"))?;
    } // close stdin so glow finishes
    let out = child
        .wait_with_output()
        .map_err(|e| format!("markdown preview: {e}"))?;
    if !out.status.success() {
        return Err("markdown preview: glow failed".to_string());
    }
    Ok(clean_ansi(&String::from_utf8_lossy(&out.stdout)))
}

/// Strip CSI sequences (SGR colors, cursor moves) then trim the width-padding
/// glow adds. Full CSI range on purpose: glow may emit cursor moves too.
pub fn clean_ansi(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("valid regex");
    re.replace_all(text, "")
        .lines()
        .map(|l| l.trim_end())
        // glow's style boxes leave blank framed rows; collapse blank runs to one
        .fold(String::new(), |mut acc, l| {
            if l.is_empty() && acc.ends_with("\n\n") {
                return acc;
            }
            acc.push_str(l);
            acc.push('\n');
            acc
        })
        .trim_matches('\n')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_ansi_strips_sgr_and_pads() {
        // leading spaces survive (glow's box margin + markdown indentation is
        // meaningful); only styling and trailing pad are removed
        let raw = "\x1b[38;5;228;48;5;63;1m Title \x1b[0m   \n\x1b[38;5;252mtext\x1b[0m  \n\n\nend   \n";
        assert_eq!(clean_ansi(raw), " Title\ntext\n\nend");
    }

    #[test]
    fn clean_ansi_strips_cursor_moves_too() {
        assert_eq!(clean_ansi("\x1b[2;1Hreal\x1b[?25l"), "real");
    }

    #[test]
    fn render_via_real_glow_strips_styles() {
        // glow on the dev machine is a fixture like rg/git in other suites.
        let md = "# Head\n\nSome **bold** text\n\n- one\n- two\n";
        let out = render_markdown(md, 40).expect("glow rendered");
        assert!(out.contains("Head"), "got: {out:?}");
        assert!(out.contains("one") && out.contains("two"));
        assert!(!out.contains("**"), "markdown emphasis markers should be consumed");
        assert!(!out.contains('\x1b'), "no ANSI survives");
        assert!(out.lines().all(|l| l.chars().count() <= 42), "width respected: {out:?}");
    }

    #[test]
    fn render_reports_missing_binary() {
        let e = render_markdown_with("plat-no-such-binary", "# x", 40)
            .expect_err("missing glow must be an Err");
        assert!(e.contains("is glow installed?") || e.contains("plat-no-such-binary"), "{e}");
    }
}
