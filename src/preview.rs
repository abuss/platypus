//! Markdown preview (issue #2): render the buffer through `glow` into a
//! plain-text preview buffer. glow does all markdown work; plat only feeds
//! stdin, strips ANSI styling, and unroll-buffers the result.

use ratatui::style::{Color, Modifier, Style};
use std::io::Write;
use std::process::{Command, Stdio};

/// Render `source` markdown to styled-free plain text wrapped at `width`.
pub fn render_markdown(source: &str, width: usize) -> Result<String, String> {
    Ok(render_markdown_styled(source, width)?.0)
}

/// Same render, but glow's SGR colors/bold are captured as FrameSpan style
/// ranges (the same shape the syntax highlighter feeds the renderer) so the
/// preview buffer shows them instead of discarding them.
pub fn render_markdown_styled(source: &str, width: usize) -> Result<(String, Vec<crate::syntax::FrameSpan>), String> {
    render_spawns_with("glow", source, width)
}

/// Blocking: glow is a local <100 ms render (ponytail: move to a worker
/// thread + mpsc like search/ if huge docs ever make it lag the loop).
/// Binary is injectable so the missing-glow error path is testable.
fn render_spawns_with(binary: &str, source: &str, width: usize) -> Result<(String, Vec<crate::syntax::FrameSpan>), String> {
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
    Ok(clean_ansi_with_styles(&String::from_utf8_lossy(&out.stdout)))
}

/// Binary-injectable plain-text path (kept for the missing-glow error test).
pub fn render_markdown_with(binary: &str, source: &str, width: usize) -> Result<String, String> {
    Ok(render_spawns_with(binary, source, width)?.0)
}

/// Strip CSI sequences, trim the width-padding, AND capture SGR styling as
/// FrameSpan style ranges (char-column coords into the cleaned text).
pub fn clean_ansi(text: &str) -> String {
    clean_ansi_with_styles(text).0
}

pub fn clean_ansi_with_styles(text: &str) -> (String, Vec<crate::syntax::FrameSpan>) {
    // 1. scan: drop non-SGR CSIs, apply SGR to a running style, collect each
    //    line's text plus styled runs (char cols).
    let mut lines: Vec<(String, Vec<(usize, usize, Style)>)> = Vec::new();
    let mut style = Style::default();
    let mut line = String::new();
    let mut runs: Vec<(usize, usize, Style)> = Vec::new();
    let mut it = text.chars().peekable();
    let mut col: usize = 0;
    while let Some(c) = it.next() {
        if c == '\x1b' && it.peek() == Some(&'[') {
            it.next();
            // CSI: parameter bytes [0-9;?]*, intermediates [ -/]*, final byte @-~
            let mut params = String::new();
            while let Some(&p) = it.peek() {
                if p.is_ascii_digit() || p == ';' || p == '?' {
                    params.push(p);
                    it.next();
                } else {
                    break;
                }
            }
            while matches!(it.peek(), Some(c) if *c >= ' ' && *c <= '/') {
                it.next();
            }
            match it.next() {
                Some('m') => apply_sgr(&params, &mut style),
                Some(_) => {} // cursor move / erase: style untouched
                None => break,
            }
            continue;
        }
        if c == '\n' {
            line.push('\n');
            lines.push((std::mem::take(&mut line), std::mem::take(&mut runs)));
            col = 0;
            continue;
        }
        if style != Style::default() {
            // extend the previous run when contiguous and equal
            match runs.last_mut() {
                Some(r) if r.2 == style && r.1 == col => r.1 += 1,
                _ => runs.push((col, col + 1, style)),
            }
        }
        line.push(c);
        col += 1;
    }
    lines.push((line, runs));

    // 2. clean lines: trim trailing pad (clipping runs), collapse blank runs,
    //    then drop leading/trailing blanks — remapping span line indices.
    let mut kept: Vec<(String, Vec<crate::syntax::FrameSpan>)> = Vec::new();
    for (raw, runs) in lines {
        let s = raw.trim_end();
        let n = s.chars().count();
        let spans: Vec<crate::syntax::FrameSpan> = runs
            .into_iter()
            .filter(|(a, _, _)| *a < n)
            .map(|(a, b, st)| crate::syntax::FrameSpan {
                line: 0,
                col: a,
                end_col: b.min(n),
                style: st,
            })
            .collect();
        if s.is_empty() && kept.last().map(|k| k.0.is_empty()).unwrap_or(false) {
            continue; // collapse blank run
        }
        kept.push((s.to_string(), spans));
    }
    while kept.first().map(|k| k.0.is_empty()).unwrap_or(false) {
        kept.remove(0);
    }
    while kept.last().map(|k| k.0.is_empty()).unwrap_or(false) {
        kept.pop();
    }
    let mut spans = Vec::new();
    for (i, (_, ss)) in kept.iter().enumerate() {
        for sp in ss {
            spans.push(crate::syntax::FrameSpan { line: i, ..*sp });
        }
    }
    let text = kept
        .iter()
        .map(|(s, _)| format!("{s}\n"))
        .collect::<String>()
        .trim_matches('\n')
        .to_string();
    (text, spans)
}

/// Apply an SGR parameter string ("38;5;228;48;5;63;1") to a running style.
/// Only the subset glow emits; unknown params are ignored.
fn apply_sgr(params: &str, style: &mut Style) {
    let nums: Vec<i64> = if params.is_empty() {
        vec![0]
    } else {
        params.split(';').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => *style = Style::default(),
            1 => style.add_modifier |= Modifier::BOLD,
            2 => style.add_modifier |= Modifier::DIM,
            3 => style.add_modifier |= Modifier::ITALIC,
            4 => style.add_modifier |= Modifier::UNDERLINED,
            22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *style = style.remove_modifier(Modifier::ITALIC),
            24 => *style = style.remove_modifier(Modifier::UNDERLINED),
            39 => style.fg = Some(Color::Reset),
            49 => style.bg = Some(Color::Reset),
            c @ (30..=37 | 90..=97) => {
                let idx = if c < 38 { (c - 30) as u8 } else { (c - 90 + 8) as u8 };
                style.fg = Some(Color::Indexed(idx));
            }
            c @ (40..=47 | 100..=107) => {
                let idx = if c < 48 { (c - 40) as u8 } else { (c - 100 + 8) as u8 };
                style.bg = Some(Color::Indexed(idx));
            }
            f @ (38 | 48) => {
                let is_fg = f == 38;
                if nums.get(i + 1) == Some(&5) {
                    if let Some(&n) = nums.get(i + 2) {
                        let c = Color::Indexed(n as u8);
                        if is_fg { style.fg = Some(c) } else { style.bg = Some(c) }
                        i += 2;
                    }
                } else if nums.get(i + 1) == Some(&2) {
                    if nums.len() >= i + 5 {
                        let c = Color::Rgb(nums[i + 2] as u8, nums[i + 3] as u8, nums[i + 4] as u8);
                        if is_fg { style.fg = Some(c) } else { style.bg = Some(c) }
                        i += 4;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_map_sgr_runs_to_char_cols() {
        let (text, spans) = clean_ansi_with_styles("\x1b[38;5;228;1mab\x1b[0mc\n");
        assert_eq!(text, "abc");
        assert_eq!(spans.len(), 1, "only the styled run gets a span");
        assert_eq!((spans[0].line, spans[0].col, spans[0].end_col), (0, 0, 2));
        assert_eq!(spans[0].style.fg, Some(Color::Indexed(228)));
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn styles_survive_collapse_remap_and_trailing_clip() {
        let (text, spans) = clean_ansi_with_styles("plain\n\n\n\n\x1b[32mgreen  \n");
        assert_eq!(text, "plain\n\ngreen");
        assert_eq!(spans.len(), 1);
        // blank run collapsed to one => the styled line moved to index 2,
        // trailing pad trimmed => span clipped to 5
        assert_eq!((spans[0].line, spans[0].col, spans[0].end_col), (2, 0, 5));
        assert_eq!(spans[0].style.fg, Some(Color::Indexed(2)));
    }

    #[test]
    fn cursor_moves_dont_reset_style_and_join_runs() {
        let (text, spans) = clean_ansi_with_styles("\x1b[32ma\x1b[2Jb\x1b[0m");
        assert_eq!(text, "ab");
        assert_eq!(spans.len(), 1, "the mid-run cursor move must not split or reset");
        assert_eq!((spans[0].col, spans[0].end_col), (0, 2));
    }

    #[test]
    fn render_captures_glow_styles_as_spans() {
        let (text, spans) = render_markdown_styled("# Head\n\n- one\n", 40).expect("glow rendered");
        assert!(!text.contains('\x1b'));
        assert!(!spans.is_empty(), "glow -s dark must yield style spans");
        let nlines = text.lines().count();
        assert!(spans.iter().all(|s| s.line < nlines));
        assert!(spans
            .iter()
            .all(|s| s.end_col <= text.lines().nth(s.line).unwrap().chars().count()),
            "spans must stay inside the cleaned text");
    }

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
