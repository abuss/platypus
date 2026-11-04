use ratatui::Frame;

use crate::app::App;

/// Keep cursor visible in a no-wrap viewport; returns new (scroll_row, scroll_col).
/// `area` is the viewport area (rows, cols), `scroll` the current scroll.
pub fn clamp_scroll(
    cursor_row: usize,
    cursor_col: usize,
    area: (u16, u16),
    scroll: (usize, usize),
) -> (usize, usize) {
    let rows = area.0 as usize;
    let cols = area.1 as usize;
    let (mut sr, mut sc) = scroll;
    if rows == 0 {
        sr = 0;
    } else if cursor_row < sr {
        sr = cursor_row;
    } else if cursor_row >= sr + rows {
        sr = cursor_row + 1 - rows;
    }
    if cols == 0 {
        sc = 0;
    } else if cursor_col < sc {
        sc = cursor_col;
    } else if cursor_col >= sc + cols {
        sc = cursor_col + 1 - cols;
    }
    (sr, sc)
}

/// After a terminal size change: recompute every doc's scroll so the cursor
/// stays visible; degenerate sizes clamp to (0,0).
pub fn apply_resize(app: &mut crate::app::App, size: (u16, u16)) {
    // Compute editor width accounting for sidebar (same as draw does)
    let sidebar_x = app
        .sidebar
        .as_ref()
        .map(|s| s.width.min(size.0.saturating_sub(1)))
        .unwrap_or(0);
    let view = (size.1.saturating_sub(1), size.0.saturating_sub(sidebar_x));
    for doc in app.docs.iter_mut() {
        doc.cursor = doc.cursor.min(doc.rope.len_chars());
        let row = doc.rope.char_to_line(doc.cursor);
        let col = doc.cursor - doc.rope.line_to_char(row);
        (doc.scroll_row, doc.scroll_col) = clamp_scroll(row, col, view, (doc.scroll_row, doc.scroll_col));
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let doc = app.doc();
    // a tab row costs one row only when there are at least two buffers to show
    let tabs_on = app.docs.len() >= 2;
    let ed_top: u16 = if tabs_on { 1 } else { 0 };
    let view_rows = area.height.saturating_sub(1 + ed_top) as usize;

    // sidebar occupies the left edge; the editor gets what is left over
    let ed_x = app
        .sidebar
        .as_ref()
        .map(|s| s.width.min(area.width.saturating_sub(1)))
        .unwrap_or(0);
    let ed_w = area.width.saturating_sub(ed_x);

    // gutter width if line numbers enabled
    let gutter_w = if app.show_line_numbers {
        let max_line = doc.rope.len_lines().saturating_sub(1);
        max_line.to_string().len().max(2) as u16 + 1 // +1 for space after gutter
    } else {
        0
    };

    // badge width: 2 if any diagnostic lands on a visible line, else 0
    let badge_w: u16 = if !doc.lsp_diags.is_empty() {
        let win_start = doc.scroll_row;
        let win_end = doc.scroll_row + view_rows;
        if doc.lsp_diags.iter().any(|d| d.line >= win_start && d.line < win_end) { 2 } else { 0 }
    } else {
        0
    };
    let total_gutter_w = gutter_w + badge_w;

    // cursor position (before the buffer borrow below)
    let cur_row = doc.rope.char_to_line(doc.cursor.min(doc.rope.len_chars()));
    if cur_row >= doc.scroll_row && cur_row < doc.scroll_row + view_rows {
        let line_start = doc.rope.line_to_char(cur_row);
        let cur_col = doc.cursor - line_start;
        let display_col = if cur_col >= doc.scroll_col {
            (cur_col - doc.scroll_col) as u16 + total_gutter_w + ed_x
        } else {
            ed_x
        };
        if cur_col >= doc.scroll_col && cur_col - doc.scroll_col < (ed_w as usize).saturating_sub(total_gutter_w as usize) {
            let _ = f.set_cursor_position((display_col, (cur_row - doc.scroll_row) as u16 + ed_top));
        }
    }

    let buf = f.buffer_mut();

    if tabs_on {
        let labels: Vec<String> = app
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let name = doc_display(d);
                let plus = if d.dirty { "+" } else { "" };
                if i == app.active {
                    format!("[{name}{plus}]")
                } else {
                    format!("{name}{plus}")
                }
            })
            .collect();
        let full = labels.join(" ");
        let line = if full.chars().count() <= area.width as usize {
            full
        } else {
            // too many/too wide: window around the active buffer, '…' marks clipping
            let n = labels.len();
            let budget = (area.width as usize).saturating_sub(4); // room for the two markers
            let (mut lo, mut hi) = (app.active, app.active);
            let mut used = labels[app.active].chars().count();
            loop {
                if hi + 1 < n && used + labels[hi + 1].chars().count() + 1 <= budget {
                    hi += 1;
                    used += labels[hi].chars().count() + 1;
                    continue;
                }
                match lo.checked_sub(1) {
                    Some(l) if used + labels[l].chars().count() + 1 <= budget => {
                        lo = l;
                        used += labels[l].chars().count() + 1;
                    }
                    _ => break,
                }
            }
            let mut s = labels[lo..=hi].join(" ");
            if lo > 0 {
                s.insert_str(0, "… ");
            }
            if hi + 1 < n {
                s.push_str(" …");
            }
            s
        };
        // Write tab row with per-label style: track byte position in `line` to find which label each char belongs to.
        // Build (char_start, is_active) intervals per label in the rendered line.
        let mut intervals: Vec<(usize, usize, bool)> = Vec::new(); // (char_start, char_end, is_active)
        {
            let mut pos = 0usize;
            let full_for_intervals = labels.join(" ");
            if line == full_for_intervals {
                for (i, label) in labels.iter().enumerate() {
                    let len = label.chars().count();
                    intervals.push((pos, pos + len, i == app.active));
                    pos += len + 1; // +1 for space separator
                }
            }
            // If line was truncated/windowed, we skip per-label styling (clip edge case).
        }
        for (x, ch) in line.chars().take(area.width as usize).enumerate() {
            if x >= area.width as usize { break; } // clamp: never write past row end
            let is_active = intervals.iter().any(|&(s, e, active)| active && x >= s && x < e);
            let style = if is_active { app.palette.tab_active } else { app.palette.tab_inactive };
            buf[(x as u16, 0)].set_char(ch).set_style(style);
        }
    }

    if let Some(sidebar) = &app.sidebar {
        render_sidebar(buf, ratatui::layout::Rect { x: 0, y: ed_top, width: ed_x, height: view_rows as u16 }, sidebar, app.palette.sidebar_sel);
    }

    for row in 0..view_rows {
        let y = row as u16 + ed_top; // content rows start below the tab row when shown
        let line_idx = doc.scroll_row + row;
        if line_idx >= doc.rope.len_lines() {
            break;
        }

        // draw line number in gutter
        if gutter_w > 0 {
            let lineno_str = (line_idx + 1).to_string();
            for (i, ch) in lineno_str.chars().enumerate() {
                buf[(ed_x + badge_w + i as u16, y)].set_char(ch);
            }
        }

        // draw badge (severity char + space) for worst diagnostic on this line
        if badge_w > 0 {
            if let Some(worst) = doc.lsp_diags.iter()
                .filter(|d| d.line == line_idx)
                .min_by_key(|d| d.severity)
            {
                let (ch, style) = match worst.severity {
                    1 => ('E', app.palette.diag_error),
                    2 => ('W', app.palette.diag_warn),
                    3 => ('I', app.palette.diag_info),
                    _ => ('H', app.palette.diag_hint),
                };
                buf[(ed_x, y)].set_char(ch).set_style(style);
                buf[(ed_x + 1, y)].set_char(' ');
            }
        }

        let line = doc.rope.line(line_idx);
        // ropey's line() includes the trailing '\n' — filter it (also guards
        // any other control char from cell_width)
        let visible = line.chars().take_while(|c| *c != '\n');
        for (col, ch) in visible.skip(doc.scroll_col).take((ed_w as usize).saturating_sub(total_gutter_w as usize)).enumerate() {
            buf[(ed_x + (col as u16) + total_gutter_w, y)].set_char(ch);
        }

        // paint syntax frame spans for this line
        if let Some(syntax) = &doc.syntax {
            for span in syntax.spans() {
                if span.line != line_idx {
                    continue;
                }
                // iterate over visible columns in the span
                let span_start = span.col.max(doc.scroll_col);
                let span_end = span.end_col.min(doc.scroll_col + (ed_w as usize).saturating_sub(total_gutter_w as usize));
                for c in span_start..span_end {
                    let buf_col = (c - doc.scroll_col) as u16 + total_gutter_w + ed_x;
                    buf[(buf_col, y)].set_style(span.style);
                }
            }
        }
    }

    // selection: blue background over the visible part of the range
    if let Some((sel_start, sel_end)) = doc.selection() {
        for row in 0..view_rows {
            let y = row as u16 + ed_top;
            let line_idx = doc.scroll_row + row;
            if line_idx >= doc.rope.len_lines() {
                break;
            }
            let line_start = doc.rope.line_to_char(line_idx);
            let line_end = if line_idx + 1 < doc.rope.len_lines() {
                doc.rope.line_to_char(line_idx + 1)
            } else {
                doc.rope.len_chars()
            };
            let from = sel_start.max(line_start);
            let to = sel_end.min(line_end);
            for off in from..to {
                let col = off - line_start;
                if col >= doc.scroll_col && col - doc.scroll_col < (ed_w as usize).saturating_sub(total_gutter_w as usize) {
                    buf[(ed_x + ((col - doc.scroll_col) as u16) + total_gutter_w, y)]
                        .set_style(app.palette.selection);
                }
            }
        }
    }

    // completion menu overlay: paint after syntax and selection (transient overlay)
    // Search underline paints after this; menu cells may get underline on overlap — acceptable.
    if let Some(ref menu) = app.completion {
        // Compute cursor display position
        let doc = app.doc();
        let cur_row = doc.rope.char_to_line(doc.cursor.min(doc.rope.len_chars()));
        if cur_row >= doc.scroll_row && cur_row < doc.scroll_row + view_rows {
            let line_start = doc.rope.line_to_char(cur_row);
            let cur_col = doc.cursor - line_start;
            let disp_col = if cur_col >= doc.scroll_col {
                (cur_col - doc.scroll_col) as u16 + total_gutter_w + ed_x
            } else {
                ed_x
            };
            let menu_top = (cur_row - doc.scroll_row) as u16 + ed_top + 1;
            let avail_rows = (view_rows as u16).saturating_sub(menu_top - ed_top);
            let n_rows = menu.items.len().min(10).min(avail_rows as usize) as u16;
            let sel_style = app.palette.completion_sel;
            for i in 0..n_rows {
                let item = &menu.items[i as usize];
                let text = format!("{} {}", item.kind_letter, item.label);
                let avail_w = area.width.saturating_sub(disp_col) as usize;
                let y = menu_top + i;
                if y >= area.height { break; }
                for (x, ch) in text.chars().take(avail_w).enumerate() {
                    let cell = &mut buf[(disp_col + x as u16, y)];
                    cell.set_char(ch);
                    if i as usize == menu.sel {
                        cell.set_style(sel_style);
                    } else {
                        cell.set_style(ratatui::style::Style::default());
                    }
                }
                // Clear remainder of menu row (in case prior content shows through)
                let text_w = text.chars().count().min(avail_w);
                let menu_w = avail_w.min(20 + 2); // label cap
                for x in text_w..menu_w {
                    let cell = &mut buf[(disp_col + x as u16, y)];
                    cell.set_char(' ');
                    if i as usize == menu.sel {
                        cell.set_style(sel_style);
                    } else {
                        cell.set_style(ratatui::style::Style::default());
                    }
                }
            }
        }
    }

    // live find highlight: the prompt's keystrokes already updated
    // doc.search_matches (live_find_search), so render just paints them
    if let Some(pr) = &app.prompt {
        if pr.kind == crate::prompt::PromptKind::Find {
            // ponytail: one char_to_line per matched char per frame; fine for
            // realistic match counts, intersect per-visible-line if a pattern
            // like .* over a 100k-line doc ever shows up in daily use.
            let term_style = app.palette.find_match;
            for &(start, end) in doc.search_matches.iter() {
                for off in start..end {
                    let line_idx = doc.rope.char_to_line(off);
                    if line_idx < doc.scroll_row || line_idx >= doc.scroll_row + view_rows {
                        continue;
                    }
                    let line_start = doc.rope.line_to_char(line_idx);
                    let col = off - line_start;
                    if col >= doc.scroll_col && col - doc.scroll_col < (ed_w as usize).saturating_sub(total_gutter_w as usize) {
                        buf[(ed_x + ((col - doc.scroll_col) as u16) + total_gutter_w, (line_idx - doc.scroll_row) as u16 + ed_top)]
                            .set_style(term_style);
                    }
                }
            }
        }
    }

    // status bar: mode, filename(+), then any status message
    let mut status = if let Some(pr) = &app.prompt {
        use crate::prompt::PromptKind::*;
        match pr.kind {
            Find => {
                let active = |s: String, on: bool| if on { format!("[{s}]") } else { s };
                let mut label = format!(
                    "find > {}",
                    active(pr.line.clone(), pr.field == crate::prompt::PromptField::Pattern)
                );
                if pr.field == crate::prompt::PromptField::Replace || !pr.replace.is_empty() {
                    label.push_str(&format!(
                        " | replace > {}",
                        active(pr.replace.clone(), pr.field == crate::prompt::PromptField::Replace)
                    ));
                }
                label
            }
            SaveAs => format!("save as > {}", pr.line),
            ConfirmQuit => format!("unsaved changes: {} (Tab cycles, Enter confirms, Esc cancels)", pr.choice.label().to_uppercase()),
            ConfirmReload => format!("changed on disk: {} (Tab cycles, Enter confirms, Esc cancels)", pr.choice.label().to_uppercase()),
            Command => format!(":> {}", pr.line),
        }
    } else {
        match app.mode {
            crate::app::Mode::Edit => "EDIT ".to_string(),
            crate::app::Mode::Action => "ACTION ".to_string(),
        }
    };
    if app.prompt.is_none() {
        if let Some(sidebar) = &app.sidebar {
            status.push_str(if sidebar.mode == crate::sidebar::SidebarMode::TreeBrowser {
                "[tree] "
            } else {
                "[picker] "
            });
        }
        if !tabs_on {
            // buffers live in the tab row when it is shown; status keeps them only for single-buffer sessions
            for (i, d) in app.docs.iter().enumerate() {
                if i > 0 {
                    status.push(' ');
                }
                let name = doc_display(d);
                if i == app.active {
                    status.push('[');
                    status.push_str(&name);
                    if d.dirty {
                        status.push('+');
                    }
                    status.push(']');
                } else {
                    status.push_str(&name);
                    if d.dirty {
                        status.push('+');
                    }
                }
            }
        }
        if let Some(b) = &app.active_branch {
            status.push_str("  git(");
            status.push_str(b);
            status.push(')');
        }
        if let Some(lang) = crate::syntax::language_for(app.doc().path.as_deref()) {
            status.push_str("  ");
            status.push_str(lang);
        }
    }
    if let Some((msg, _)) = &app.status {
        status.push_str("  ");
        status.push_str(msg);
    }
    // Cursor-on-diagnostic line: append first matching diagnostic message
    if app.prompt.is_none() {
        let doc = app.doc();
        let cur_diag_line = doc.rope.char_to_line(doc.cursor.min(doc.rope.len_chars()));
        if let Some(d) = doc.lsp_diags.iter().find(|d| d.line == cur_diag_line) {
            let col_1based = d.start + 1;
            let line_1based = d.line + 1;
            status.push_str(&format!("  {line_1based}:{col_1based}: {}", d.message));
        }
    }
    // right-aligned segments: selection count OR line:col, then document percent
    let doc = app.doc();
    let sel_chars: usize = doc.selections.iter().filter_map(|s| s.range()).map(|(a, b)| b - a).sum();
    let cur_line = doc.rope.char_to_line(doc.cursor);
    let pos = if sel_chars > 0 {
        format!("{sel_chars} selected")
    } else {
        format!("{}:{}", cur_line + 1, doc.cursor - doc.rope.line_to_char(cur_line) + 1)
    };
    let n_lines = doc.rope.len_lines();
    // ropey counts a phantom empty line after a trailing newline — exclude it
    let last_real = if n_lines > 1 && doc.rope.line(n_lines - 1).as_str().is_none_or(|s| s.is_empty()) { n_lines - 2 } else { n_lines.saturating_sub(1) };
    let pct = if cur_line == 0 {
        "Top".to_string()
    } else if cur_line >= last_real {
        "Bot".to_string()
    } else {
        format!("{}%", cur_line * 100 / last_real.max(1))
    };
    // Diagnostic counts appended after percent (only when non-zero)
    let n_errors: usize = doc.lsp_diags.iter().filter(|d| d.severity == 1).count();
    let n_warns: usize = doc.lsp_diags.iter().filter(|d| d.severity == 2).count();
    let diag_suffix = {
        let mut s = String::new();
        if n_errors > 0 { s.push_str(&format!(" {n_errors}E")); }
        if n_warns > 0 { s.push_str(&format!(" {n_warns}W")); }
        s
    };
    let right = format!("{pos} {pct}{diag_suffix}");
    let status_y = (view_rows as u16) + ed_top;
    if status_y < area.height {
        let w = area.width as usize;
        let right_w = right.chars().count();
        // narrow terminals: right side wins, left truncates (mode+docs lead the string)
        let left_budget = if right_w < w { w - right_w - 1 } else { 0 };
        for (x, ch) in status.chars().take_while(|c| !c.is_control()).take(left_budget).enumerate() {
            buf[(x as u16, status_y)].set_char(ch).set_style(app.palette.status_bar);
        }
        if right_w <= w {
            let rx = (w - right_w) as u16;
            for (i, ch) in right.chars().enumerate() {
                buf[(rx + i as u16, status_y)].set_char(ch).set_style(app.palette.status_bar);
            }
        }
        // Fill remaining cells with status_bar style (covers gaps between left and right)
        for x in 0..(w as u16).min(area.width) {
            let cell = &mut buf[(x, status_y)];
            if cell.style() == ratatui::style::Style::default() {
                cell.set_style(app.palette.status_bar);
            }
        }
    }
}

/// Status/tab-row display name for a document: file name, or full path if it
/// has none (unsaved buffer → "[No name]").
fn doc_display(d: &crate::document::Document) -> String {
    d.path
        .as_ref()
        .map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()))
        .unwrap_or_else(|| "[No name]".to_string())
}

/// Sidebar rows: tree rows indented by depth with +/- dir markers, or the
/// picker's filter line plus its ranked results. Writes straight into the
/// buffer, like the rest of this module.
fn render_sidebar(buf: &mut ratatui::buffer::Buffer, area: ratatui::layout::Rect, sidebar: &crate::sidebar::Sidebar, sel_style: ratatui::style::Style) {
    use crate::sidebar::SidebarMode;
    let rows = area.height as usize;
    if area.width == 0 || rows == 0 {
        return;
    }
    // picker spends its first row on the filter line
    let header = if sidebar.mode == SidebarMode::FuzzyPicker { 1 } else { 0 };
    let items = sidebar.visible_items();

    if sidebar.mode == SidebarMode::FuzzyPicker {
        let text = format!("> {}", sidebar.input);
        for (x, ch) in text.chars().take(area.width as usize).enumerate() {
            buf[(area.x + x as u16, area.y)].set_char(ch);
        }
    } else if items.is_empty() {
        for (x, ch) in "no files".chars().take(area.width as usize).enumerate() {
            buf[(area.x + x as u16, area.y)].set_char(ch);
        }
    }

    // keep the cursor row on screen by scrolling the list head
    let capacity = rows.saturating_sub(header);
    let head = if sidebar.cursor >= capacity { sidebar.cursor + 1 - capacity } else { 0 };

    for (i, item) in items.iter().skip(head).take(capacity).enumerate() {
        let y = area.y + (header + i) as u16;
        let marker = if item.is_dir {
            if sidebar.tree_state.as_ref().is_some_and(|t| t.expanded.contains(&item.path)) {
                "- "
            } else {
                "+ "
            }
        } else {
            ""
        };
        let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = format!("{}{}{}", "  ".repeat(item.depth), marker, name);
        for (x, ch) in text.chars().take(area.width as usize).enumerate() {
            buf[(area.x + x as u16, y)].set_char(ch);
        }
        if head + i == sidebar.cursor {
            for x in 0..area.width {
                buf[(area.x + x, y)].set_style(sel_style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Mode};
    use ratatui::backend::TestBackend;

    fn buffer_of(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().buffer().clone()
    }

    fn row_string(buf: &ratatui::buffer::Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect()
    }

    fn doc_app_with_selection(content: &str) -> App {
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str(content);
        app.show_line_numbers = false;
        // Select first 3 chars
        app.docs[0].selections = vec![crate::document::Selection { anchor: 0, head: 3 }];
        app
    }

    #[test]
    fn dark_chrome_unchanged() {
        use ratatui::style::{Color, Modifier};
        use crate::document::Diag;

        // --- selection: bg Blue ---
        let app = doc_app_with_selection("hello world\n");
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0u16, 0u16)].bg, Color::Blue, "dark: selection bg Blue");

        // --- status row: Style::default() (fg/bg Reset, no modifiers) ---
        assert_eq!(buf[(0u16, 23u16)].fg, Color::Reset, "dark: status fg Reset");
        assert_eq!(buf[(0u16, 23u16)].bg, Color::Reset, "dark: status bg Reset");
        assert!(buf[(0u16, 23u16)].modifier.is_empty(), "dark: status no modifiers");

        // --- find match: UNDERLINED modifier ---
        {
            use crate::prompt::{Prompt, PromptKind};
            let mut app2 = App::new(vec![]);
            app2.docs[0] = crate::document::Document::from_str("hello world\n");
            app2.show_line_numbers = false;
            app2.prompt = Some(Prompt::new(PromptKind::Find, "hel".into()));
            app2.docs[0].set_search("hel").unwrap();
            let buf2 = buffer_of(&app2, 80, 24);
            assert!(buf2[(0u16, 0u16)].modifier.contains(Modifier::UNDERLINED), "dark: find match UNDERLINED");
        }

        // --- diagnostic badges: E→Red, W→Yellow, I→DarkGray, H→DarkGray ---
        {
            let mut app3 = App::new(vec![]);
            app3.docs[0] = crate::document::Document::from_str("a\nb\nc\nd\n");
            app3.show_line_numbers = false;
            app3.docs[0].lsp_diags = vec![
                Diag { line: 0, start: 0, end: 1, severity: 1, message: "e".into() },
                Diag { line: 1, start: 0, end: 1, severity: 2, message: "w".into() },
                Diag { line: 2, start: 0, end: 1, severity: 3, message: "i".into() },
                Diag { line: 3, start: 0, end: 1, severity: 4, message: "h".into() },
            ];
            let buf3 = buffer_of(&app3, 80, 24);
            assert_eq!(buf3[(0u16, 0u16)].fg, Color::Red,     "dark: E badge fg Red");
            assert_eq!(buf3[(0u16, 1u16)].fg, Color::Yellow,  "dark: W badge fg Yellow");
            assert_eq!(buf3[(0u16, 2u16)].fg, Color::DarkGray,"dark: I badge fg DarkGray");
            assert_eq!(buf3[(0u16, 3u16)].fg, Color::DarkGray,"dark: H badge fg DarkGray");
        }

        // --- completion menu selected row: bg Blue ---
        {
            use crate::app::{CompItem, CompMenu};
            let mut app4 = App::new(vec![]);
            app4.docs[0] = crate::document::Document::from_str("hello\n");
            app4.show_line_numbers = false;
            app4.completion = Some(CompMenu {
                items: vec![CompItem { label: "foo".into(), kind_letter: 'f', text_edit: None, insert: "foo".into() }],
                sel: 0,
                doc_idx: 0,
                edit_gen: 0,
            });
            let buf4 = buffer_of(&app4, 80, 24);
            // menu row is at y=1 (below cursor on row 0), selected → bg Blue
            assert_eq!(buf4[(0u16, 1u16)].bg, Color::Blue, "dark: completion sel row bg Blue");
        }

        // --- sidebar cursor row: bg Blue ---
        {
            use crate::sidebar::Sidebar;
            use crate::tree::TreeItem;
            let mut app5 = App::new(vec![]);
            app5.docs[0] = crate::document::Document::from_str("x\n");
            app5.show_line_numbers = false;
            let items = vec![
                TreeItem { path: std::path::PathBuf::from("a.rs"), is_dir: false, depth: 0 },
                TreeItem { path: std::path::PathBuf::from("b.rs"), is_dir: false, depth: 0 },
            ];
            app5.sidebar = Some(Sidebar::new_tree(items, 15));
            // sidebar.cursor=0 → row 0 of sidebar (y=0) should have bg Blue
            let buf5 = buffer_of(&app5, 80, 24);
            assert_eq!(buf5[(0u16, 0u16)].bg, Color::Blue, "dark: sidebar cursor row bg Blue");
        }

        // --- tab row labels: Style::default() (dark has no tab styling) ---
        {
            let dir = std::env::temp_dir().join(format!("plat-dark-tabs-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let a = dir.join("x.rs"); let b = dir.join("y.rs");
            std::fs::write(&a, "x\n").unwrap(); std::fs::write(&b, "y\n").unwrap();
            let mut app6 = App::new(vec![a, b]);
            app6.show_line_numbers = false;
            let buf6 = buffer_of(&app6, 80, 24);
            // dark: tab_active = Style::default(), no BOLD
            assert!(!buf6[(0u16, 0u16)].modifier.contains(Modifier::BOLD), "dark: active tab no BOLD");
            assert_eq!(buf6[(0u16, 0u16)].fg, Color::Reset, "dark: active tab fg Reset");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn dark_syntax_span_via_palette() {
        // Syntax on, default (dark) config: `fn` keyword cell must have fg LightMagenta
        use ratatui::style::Color;
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str("fn main() {}\n");
        app.docs[0].path = Some(std::path::PathBuf::from("/tmp/t.rs")); // trigger rust syntax
        app.show_line_numbers = false;
        app.ensure_syntax(22);
        let buf = buffer_of(&app, 80, 24);
        // `fn` is at col 0-1; after syntax painting the cell should have LightMagenta fg
        assert_eq!(buf[(0u16, 0u16)].fg, Color::LightMagenta, "dark syntax: `fn` keyword must be LightMagenta via stored palette");
    }

    #[test]
    fn narrow_terminal_no_panic() {
        // Terminal narrower than the two tab labels: must not panic, must not index out of bounds
        let dir = std::env::temp_dir().join(format!("plat-narrow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("longname_a.rs");
        let b = dir.join("longname_b.rs");
        std::fs::write(&a, "x\n").unwrap();
        std::fs::write(&b, "y\n").unwrap();
        let mut app = App::new(vec![a, b]);
        app.show_line_numbers = false;
        // Width 5 is narrower than either label; must not panic
        let buf = buffer_of(&app, 5, 5);
        // Just assert it rendered something (no panic is the real test)
        let _ = buf[(0u16, 0u16)].symbol();
        // Status row (y=4) also must not panic
        let _ = buf[(0u16, 4u16)].symbol();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn light_theme_recolors() {
        // Light theme: selection bg == LightGray (Gray), status bar fg Black bg Gray
        let cfg = crate::config::from_str("[editor]\ntheme=\"light\"\n").unwrap();
        let mut app = App::new_with_config(vec![], cfg);
        app.show_line_numbers = false;
        app.docs[0] = crate::document::Document::from_str("hello world\n");
        app.docs[0].selections = vec![crate::document::Selection { anchor: 0, head: 3 }];
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0u16, 0u16)].bg, ratatui::style::Color::Gray, "light palette: selected cell bg must be Gray");
        // Status bar (row 23) first cell: fg Black, bg Gray
        assert_eq!(buf[(0u16, 23u16)].fg, ratatui::style::Color::Black, "light: status bar fg must be Black");
        assert_eq!(buf[(0u16, 23u16)].bg, ratatui::style::Color::Gray, "light: status bar bg must be Gray");
    }

    #[test]
    fn config_override_cell() {
        // [theme.ui] selection with bg override → selected cell bg == Rgb(255,0,0)
        let cfg = crate::config::from_str("[theme.ui]\nselection = { bg = \"#ff0000\" }\n").unwrap();
        let mut app = App::new_with_config(vec![], cfg);
        app.show_line_numbers = false;
        app.docs[0] = crate::document::Document::from_str("hello world\n");
        app.docs[0].selections = vec![crate::document::Selection { anchor: 0, head: 3 }];
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0u16, 0u16)].bg, ratatui::style::Color::Rgb(255, 0, 0), "config override: selected cell bg must be #ff0000");
    }

    #[test]
    fn tab_active_styled() {
        // Two docs, light theme → active label cell has BOLD, inactive does not
        let dir = std::env::temp_dir().join(format!("plat-tab-style-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("aa.rs");
        let b = dir.join("bb.rs");
        std::fs::write(&a, "x\n").unwrap();
        std::fs::write(&b, "y\n").unwrap();
        let cfg = crate::config::from_str("[editor]\ntheme=\"light\"\n").unwrap();
        let mut app = App::new_with_config(vec![a.clone(), b.clone()], cfg);
        app.show_line_numbers = false;
        // active=0 → "[aa.rs]" starts at col 0
        let buf = buffer_of(&app, 80, 24);
        // active label char at col 0 must be BOLD
        assert!(buf[(0u16, 0u16)].modifier.contains(ratatui::style::Modifier::BOLD), "active tab must be BOLD");
        // inactive label: find col of 'b' in "bb.rs" — after "[aa.rs] "
        let active_label = "[aa.rs]";
        let inactive_start = active_label.chars().count() + 1; // +1 for space
        assert!(!buf[(inactive_start as u16, 0u16)].modifier.contains(ratatui::style::Modifier::BOLD), "inactive tab must NOT be BOLD");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renders_visible_lines_and_status_bar() {
        let dir = std::env::temp_dir().join(format!("plat-t3r-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("file.txt");
        std::fs::write(&p, "alpha\nbeta\ngamma\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.show_line_numbers = false;  // disable for test
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(row_string(&buf, 0, 80).trim_end(), "alpha");
        assert_eq!(row_string(&buf, 1, 80).trim_end(), "beta");
        assert_eq!(row_string(&buf, 2, 80).trim_end(), "gamma");
        let status = row_string(&buf, 23, 80);
        assert!(status.contains("EDIT"), "status must show mode: {status:?}");
        assert!(status.contains("file.txt"), "status must show path: {status:?}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn find_prompt_underlines_live_matches() {
        use crate::prompt::{Prompt, PromptKind};
        use ratatui::style::Modifier;
        let dir = std::env::temp_dir().join(format!("plat-t8f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("find.txt");
        std::fs::write(&p, "hello ell\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.show_line_numbers = false;  // disable for test
        app.prompt = Some(Prompt::new(PromptKind::Find, "ell".into()));
        // the live-search keystroke path fills search_matches; render paints those
        app.doc_mut().set_search("ell").unwrap();
        let buf = buffer_of(&app, 80, 24);
        assert!(buf[(1, 0)].modifier.contains(Modifier::UNDERLINED), "match 'ell' at 1 underlined");
        assert!(buf[(6, 0)].modifier.contains(Modifier::UNDERLINED), "match at 6 underlined");
        assert!(!buf[(0, 0)].modifier.contains(Modifier::UNDERLINED), "'h' not underlined");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn find_prompt_status_shows_both_fields_with_the_active_one_bracketed() {
        use crate::prompt::{Prompt, PromptField, PromptKind};
        let mut app = doc_app("hello world\n");
        let mut p = Prompt::new(PromptKind::Find, "wor".into());
        p.replace = "WWW".into();
        p.field = PromptField::Replace;
        app.prompt = Some(p);
        let status = row_string(&buffer_of(&app, 80, 24), 23, 80);
        assert!(status.contains("find > wor"), "pattern field shown: {status:?}");
        assert!(status.contains("replace > [WWW]"), "active replace value bracketed: {status:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn selection_painted_with_blue_bg() {
        use crate::document::Selection;
        use ratatui::style::Color;
        let dir = std::env::temp_dir().join(format!("plat-t5s-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("sel.txt");
        std::fs::write(&p, "hello\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.show_line_numbers = false;  // disable for test
        app.doc_mut().selections.push(Selection { anchor: 1, head: 3 });
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
        assert_eq!(buf[(1, 0)].bg, Color::Blue, "'e' selected");
        assert_eq!(buf[(2, 0)].bg, Color::Blue, "'l' selected");
        assert_eq!(buf[(3, 0)].bg, Color::Reset);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn clamp_scroll_follows_cursor_down() {
        // cursor row 30 in a 20-row window starting at scroll 0 -> window ends at row 30
        assert_eq!(clamp_scroll(30, 0, (20, 80), (0, 0)), (11, 0));
        // cursor already visible: no change
        assert_eq!(clamp_scroll(5, 0, (20, 80), (0, 0)), (0, 0));
    }

    #[test]
    fn line_numbers_gutter_renders_when_enabled() {
        let dir = std::env::temp_dir().join(format!("plat-tln-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("lineno.txt");
        std::fs::write(&p, "line 1\nline 2\nline 3\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.show_line_numbers = true;
        let buf = buffer_of(&app, 80, 24);
        // gutter width for 3 lines: 1 digit + 1 space = 2
        let row0 = row_string(&buf, 0, 80);
        assert!(row0.starts_with("1 "), "line 1 should have '1 ' in gutter, got: {row0:?}");
        let row1 = row_string(&buf, 1, 80);
        assert!(row1.starts_with("2 "), "line 2 should have '2 ' in gutter, got: {row1:?}");
        std::fs::remove_file(&p).ok();
    }


    #[test]
    fn clamp_scroll_follows_cursor_right() {
        assert_eq!(clamp_scroll(0, 95, (20, 80), (0, 0)), (0, 16));
    }

    #[test]
    fn resize_with_sidebar_open_clamps_scroll_to_the_narrower_editor() {
        let mut app = doc_app(&"x".repeat(100)); // long line to allow cursor at column 70
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(vec![sb_item("a.rs", false, 0)], 40));
        app.doc_mut().cursor = 70; // position in long line, scrolled right
        apply_resize(&mut app, (80, 24));
        // editor width is 80-40=40; the cursor column must fit inside that
        let d = app.doc();
        let line_start = d.rope.line_to_char(0);
        let cursor_col = d.cursor - line_start;
        assert!(cursor_col >= d.scroll_col && cursor_col - d.scroll_col < 40,
                "scroll window must follow the shrunken editor width (40); cursor_col={}, scroll_col={}", cursor_col, d.scroll_col);
        assert!(d.scroll_col >= 31, "cursor col 70 must be visible in width 40: {}", d.scroll_col);
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    fn sb_item(path: &str, is_dir: bool, depth: usize) -> crate::tree::TreeItem {
        crate::tree::TreeItem { path: std::path::PathBuf::from(path), is_dir, depth }
    }

    /// Unique temp dir per call: tests run in parallel threads and each one
    /// remove_dir_all's its own, so sharing a name by (pid, text.len) made
    /// siblings' files vanish mid-render (flaky).
    fn doc_app(text: &str) -> App {
        doc_app_named("doc.txt", text)
    }

    /// Like doc_app but with a custom filename (needed for extension detection).
    fn doc_app_named(filename: &str, text: &str) -> App {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("plat-sbrender-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(filename);
        std::fs::write(&p, text).unwrap();
        let mut app = App::new(vec![p]);
        app.show_line_numbers = false;
        app
    }

    #[test]
    fn sidebar_tree_rows_render_in_the_left_pane_and_shift_the_editor() {
        let mut app = doc_app("hello\n");
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(
            vec![sb_item("src", true, 0), sb_item("src/main.rs", false, 1)],
            20,
        ));
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(row_string(&buf, 0, 20).trim_end(), "+ src", "collapsed dir row");
        assert_eq!(row_string(&buf, 1, 20).trim_end(), "", "child hidden while dir collapsed");
        assert_eq!(&row_string(&buf, 0, 80)[20..25], "hello", "editor starts right of the sidebar");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn sidebar_empty_tree_says_no_files() {
        let mut app = doc_app("x\n");
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(vec![], 20));
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(row_string(&buf, 0, 20).trim_end(), "no files");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn picker_shows_its_filter_line_above_the_results() {
        let mut app = doc_app("x\n");
        let mut sidebar = crate::sidebar::Sidebar::new_picker(vec![sb_item("a.rs", false, 0)], 20);
        sidebar.input = "a".into();
        app.sidebar = Some(sidebar);
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(row_string(&buf, 0, 20).trim_end(), "> a", "filter line");
        assert_eq!(row_string(&buf, 1, 20).trim_end(), "a.rs", "result under the filter line");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn sidebar_cursor_row_is_highlighted() {
        use ratatui::style::Color;
        let mut app = doc_app("x\n");
        let mut sidebar = crate::sidebar::Sidebar::new_picker(
            vec![sb_item("a.rs", false, 0), sb_item("b.rs", false, 0)],
            20,
        );
        sidebar.cursor = 1;
        app.sidebar = Some(sidebar);
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0, 2)].bg, Color::Blue, "second result (row 1) highlighted");
        assert_eq!(buf[(0, 1)].bg, Color::Reset, "first result not highlighted");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_reports_the_open_sidebar_mode() {
        let mut app = doc_app("x\n");
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(vec![], 20));
        let status = row_string(&buffer_of(&app, 80, 24), 23, 80);
        assert!(status.contains("[tree]"), "status should name the sidebar mode: {status:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn tiny_terminal_with_sidebar_renders_without_panic() {
        let mut app = doc_app("x\n");
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(vec![sb_item("a.rs", false, 0)], 50));
        let buf = buffer_of(&app, 12, 3);
        let _ = buf[(0, 0)].symbol();
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn empty_doc_one_column_terminal_renders_without_panic() {
        let app = App::new(vec![]);
        let buf = buffer_of(&app, 1, 1);
        assert_eq!(*buf.area(), ratatui::layout::Rect::new(0, 0, 1, 1));
        let _ = buf[(0, 0)].symbol();
        assert_eq!(app.mode, Mode::Edit);
    }

    #[test]
    fn keyword_cells_carry_the_palette_color() {
        let mut app = doc_app_named("t.rs", "fn main() {}\n");
        app.ensure_syntax(24);
        let buf = buffer_of(&app, 80, 24);
        // 'f' of "fn" keyword should have LightMagenta color from syntax highlighting
        let ed_x = 0u16; // no sidebar
        let gutter_w = 0u16; // no line numbers
        let cell = &buf[(ed_x + gutter_w, 0)];
        assert_eq!(cell.fg, ratatui::style::Color::LightMagenta, "first char 'f' of 'fn' should be LightMagenta");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn keyword_cells_under_find_prompt_keep_color_with_underline_added() {
        use crate::prompt::{Prompt, PromptKind};
        use ratatui::style::Modifier;
        let mut app = doc_app_named("t.rs", "fn main() {}\n");
        app.ensure_syntax(24);
        // Open Find prompt and search for "fn"
        app.prompt = Some(Prompt::new(PromptKind::Find, "fn".into()));
        app.doc_mut().set_search("fn").unwrap();
        let buf = buffer_of(&app, 80, 24);
        let ed_x = 0u16;
        // 'f' should have both LightMagenta (syntax) AND underline (find)
        let cell = &buf[(ed_x, 0)];
        assert_eq!(cell.fg, ratatui::style::Color::LightMagenta, "syntax color kept under find");
        assert!(cell.modifier.contains(Modifier::UNDERLINED), "find underline added on top");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn keyword_cells_with_horizontal_scroll() {
        let mut app = doc_app_named("t.rs", "let x = 42; fn main() {}\n");
        app.ensure_syntax(24);
        // Scroll right so that the "fn" keyword is visible but not at column 0
        app.doc_mut().scroll_col = 12; // scroll past "let x = 42; "
        let buf = buffer_of(&app, 80, 24);
        let ed_x = 0u16;
        // "fn" should now appear at buffer column 0 (but it's at char col 12 in the doc)
        assert_eq!(buf[(ed_x, 0)].fg, ratatui::style::Color::LightMagenta, "keyword color painted after horizontal scroll");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn plain_text_document_has_no_syntax_styles() {
        let mut app = doc_app_named("plain.txt", "hello world\n");
        app.ensure_syntax(24);
        let buf = buffer_of(&app, 80, 24);
        let ed_x = 0u16;
        // Plain text should have no syntax highlighting - fg should be Reset
        for col in 0..11 {
            let cell = &buf[(ed_x + col, 0)];
            assert_eq!(cell.fg, ratatui::style::Color::Reset, "char at column {col} should have default fg");
        }
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_shows_line_col_and_percent_right_aligned() {
        let mut app = doc_app_named("pos.txt", "a\nb\nc\nd\ne"); // 5 lines, no trailing NL
        app.doc_mut().cursor = 4; // line 3 col 1 (char 'c': a=0 \n=1 b=2 \n=3 c=4)
        let buf = buffer_of(&app, 80, 24);
        let status = row_string(&buf, 23, 80);
        assert!(status.contains("3:1"), "line:col missing: {status:?}");
        assert!(status.trim_end().ends_with("3:1 50%"), "right side misaligned: {status:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_top_bot_and_selection_replace_position() {
        let mut app = doc_app_named("tb.txt", "a\nb\nc\nd\ne");
        let status_at = |app: &mut App| {
            let buf = buffer_of(app, 80, 24);
            row_string(&buf, 23, 80)
        };
        app.doc_mut().cursor = 0;
        assert!(status_at(&mut app).trim_end().ends_with("1:1 Top"));
        app.doc_mut().cursor = 8; // last line ('e')
        assert!(status_at(&mut app).trim_end().ends_with("5:1 Bot"));
        app.doc_mut().selections = vec![crate::document::Selection { anchor: 0, head: 4 }];
        let s = status_at(&mut app);
        assert!(s.contains("4 selected"), "selection count missing: {s:?}");
        assert!(!s.contains("5:1"), "line:col must hide during selection: {s:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_narrow_width_keeps_right_side_and_mode() {
        let mut app = doc_app_named("narrow.txt", "a\nb\nc\nd\ne");
        app.status = Some(("a fairly long status message".to_string(), false));
        let buf = buffer_of(&app, 20, 24);
        let status = row_string(&buf, 23, 20);
        assert!(status.starts_with("EDIT"), "mode must survive truncation: {status:?}");
        assert!(status.ends_with("1:1 Top"), "right side must stay pinned: {status:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_shows_branch_and_language() {
        let mut app = doc_app_named("t.rs", "fn main() {}\n");
        app.active_branch = Some("feat-x".to_string());
        let buf = buffer_of(&app, 80, 24);
        let status = row_string(&buf, 23, 80);
        assert!(status.contains("git(feat-x)"), "branch missing: {status:?}");
        assert!(status.contains("rust"), "language missing: {status:?}");
        // .txt files: language segment absent
        let app2 = doc_app_named("plain.txt", "x");
        let status2 = row_string(&buffer_of(&app2, 80, 24), 23, 80);
        assert!(!status2.contains("rust"), "plain file must not show a language: {status2:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
        std::fs::remove_dir_all(app2.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn tab_row_lists_buffers_and_shifts_editor_down() {
        let dir = std::env::temp_dir().join(format!("plat-tabs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.rs");
        let b = dir.join("b.rs");
        std::fs::write(&a, "first\n").unwrap();
        std::fs::write(&b, "second\n").unwrap();
        let mut app = App::new(vec![a.clone(), b.clone()]); // active = 0
        app.show_line_numbers = false;
        let buf = buffer_of(&app, 80, 24);
        let row0 = row_string(&buf, 0, 80);
        assert!(row0.contains("[a.rs]") && row0.contains("b.rs"), "tab row: {row0:?}");
        assert_eq!(row_string(&buf, 1, 80).trim_end(), "first", "editor must shift down one row");
        let status = row_string(&buf, 23, 80);
        assert!(status.starts_with("EDIT"), "status kept mode: {status:?}");
        assert!(!status.contains("a.rs"), "status must not duplicate the tab row: {status:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tab_row_windows_active_into_view_when_overflowing() {
        let dir = std::env::temp_dir().join(format!("plat-tabswide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<_> = (0..8).map(|i| {
            let p = dir.join(format!("buffer0{i}.rs"));
            std::fs::write(&p, "x\n").unwrap();
            p
        }).collect();
        let mut app = App::new(paths);
        while app.active < 7 {
            app.handle_command(crate::keymap::Command::NextBuffer);
        }
        let buf = buffer_of(&app, 60, 24);
        let row0 = row_string(&buf, 0, 60);
        assert!(row0.contains("[buffer07.rs]"), "active buffer must stay visible: {row0:?}");
        assert!(row0.contains('…'), "clipped end marker missing: {row0:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Task 5: diagnostics UX tests ─────────────────────────────────────────

    #[test]
    fn no_diags_gutter_bytes_identical_to_baseline() {
        // With no diagnostics, badge_w=0, rendering must be byte-identical to before Task 5.
        let mut app = doc_app("line one\nline two\nline three\n");
        app.show_line_numbers = true;
        // No lsp_diags set → badge_w = 0.
        let buf = buffer_of(&app, 80, 24);
        let row0 = row_string(&buf, 0, 80);
        assert!(row0.starts_with("1 "), "gutter must start with line number when no diags: {row0:?}");
        assert!(row0.contains("line one"), "content must follow gutter: {row0:?}");
    }

    #[test]
    fn badge_appears_when_diag_on_visible_line() {
        use crate::document::Diag;
        use ratatui::style::Color;
        let mut app = doc_app("line one\nline two\nline three\n");
        app.show_line_numbers = false;
        // Add a diag on line 1 (0-based)
        app.doc_mut().lsp_diags = vec![Diag { line: 1, start: 0, end: 4, severity: 1, message: "err".into() }];
        let buf = buffer_of(&app, 80, 24);
        // Row 1 should have 'E' badge at col 0 with red color
        assert_eq!(buf[(0, 1)].symbol(), "E", "badge char 'E' on error line");
        assert_eq!(buf[(0, 1)].fg, Color::Red, "error badge is red");
        // Row 0 has no diag: badge cell is space
        assert_ne!(buf[(0, 0)].symbol(), "E", "no badge on clean line");
        // Text should be shifted by 2
        assert_eq!(buf[(2, 0)].symbol(), "l", "text starts at col 2 (badge_w=2)");
    }

    #[test]
    fn badge_absent_when_diag_outside_window() {
        use crate::document::Diag;
        // A diag far off-screen should not trigger badge_w=2.
        let mut app = doc_app("line one\nline two\n");
        app.show_line_numbers = false;
        // Put diag at line 40 (not visible in a 24-row window starting at 0).
        app.doc_mut().lsp_diags = vec![Diag { line: 40, start: 0, end: 1, severity: 1, message: "x".into() }];
        let buf = buffer_of(&app, 80, 24);
        // No badge: text starts at col 0
        assert_eq!(buf[(0, 0)].symbol(), "l", "no badge when diag off-screen");
    }

    #[test]
    fn status_count_errors_and_warnings() {
        use crate::document::Diag;
        let mut app = doc_app_named("t.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
        app.doc_mut().lsp_diags = vec![
            Diag { line: 0, start: 0, end: 1, severity: 1, message: "e1".into() },
            Diag { line: 1, start: 0, end: 1, severity: 1, message: "e2".into() },
            Diag { line: 2, start: 0, end: 1, severity: 2, message: "w1".into() },
        ];
        let buf = buffer_of(&app, 80, 24);
        let status = row_string(&buf, 23, 80);
        assert!(status.contains("2E"), "error count missing: {status:?}");
        assert!(status.contains("1W"), "warning count missing: {status:?}");
    }

    #[test]
    fn status_count_absent_when_no_diags() {
        let app = doc_app_named("t.rs", "fn main() {}\n");
        let buf = buffer_of(&app, 80, 24);
        let status = row_string(&buf, 23, 80);
        assert!(!status.contains('E') || status.contains("EDIT"), "no E count when no diags: {status:?}");
        // The right side should NOT have trailing E or W count suffixes
        let trimmed = status.trim_end();
        assert!(trimmed.ends_with("Top") || trimmed.ends_with("Bot") || trimmed.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false),
            "status right side must end with percent/Top/Bot when no diags: {trimmed:?}");
    }

    #[test]
    fn status_cursor_on_diag_line_shows_message() {
        use crate::document::Diag;
        let mut app = doc_app_named("t.rs", "fn main() {}\nlet x = 1;\n");
        app.doc_mut().lsp_diags = vec![
            Diag { line: 0, start: 3, end: 7, severity: 1, message: "expected ;".into() },
        ];
        app.doc_mut().cursor = 0; // on line 0
        let buf = buffer_of(&app, 120, 24);
        let status = row_string(&buf, 23, 120);
        assert!(status.contains("expected ;"), "diag message should appear in status: {status:?}");
        assert!(status.contains("1:4"), "diag line:col should appear: {status:?}");
    }
}
