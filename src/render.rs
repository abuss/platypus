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

/// Rows the editor viewport actually paints: total height minus the status
/// row, the echo row, and the tab row (when >=2 buffers). The single source
/// of truth — draw(), apply_resize(), App::viewport_rows() and the scroll
/// clamp all derive from here so they cannot drift apart.
pub fn view_height(total_h: u16, tabs_on: bool) -> usize {
    (total_h as usize).saturating_sub(2 + tabs_on as usize)
}

/// The full-screen geometry for one frame: tab row, sidebar split, gutter
/// columns, editor viewport. draw() and mouse hit-testing both consume this,
/// so the two can never diverge (the scroll-height bug class, 2026-11-05).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub tabs_on: bool,
    pub ed_top: u16,
    pub view_rows: usize,
    /// sidebar width as drawn; the editor content starts at this column
    pub ed_x: u16,
    pub ed_w: u16,
    /// line-number column incl. its trailing space
    pub gutter_w: u16,
    /// diagnostic badge column (2 when the doc has any diagnostic)
    pub badge_w: u16,
    pub status_y: u16,
    /// terminal pane start row (0 when no terminal)
    pub terminal_y: u16,
    /// terminal pane height (0 when hidden or no terminal)
    pub terminal_h: u16,
}

pub fn layout(area: ratatui::layout::Rect, app: &crate::app::App) -> Layout {
    // a tab row costs one row only when there are at least two buffers to show
    let tabs_on = app.docs.len() >= 2;
    let ed_top: u16 = if tabs_on { 1 } else { 0 };
    // Terminal pane: visible rows directly above the status bar
    let (terminal_h, terminal_y) = if let Some(term) = &app.terminal {
        if term.visible {
            let max_h = (area.height.saturating_sub(2)) / 2;
            let h = term.height.min(max_h).max(1);
            let y = area.height.saturating_sub(2 + h);
            (h, y)
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };
    // Editor rows: total minus status(1) + echo(1) + tab row + terminal
    let base_view = view_height(area.height, tabs_on);
    let view_rows = base_view.saturating_sub(terminal_h as usize);
    // sidebar occupies the left edge; the editor gets what is left over
    let ed_x = app
        .sidebar
        .as_ref()
        .map(|s| s.width.min(area.width.saturating_sub(1)))
        .unwrap_or(0);
    let ed_w = area.width.saturating_sub(ed_x);
    // gutter width if line numbers enabled
    let doc = app.doc();
    let gutter_w = if app.show_line_numbers {
        let max_line = doc.rope.len_lines().saturating_sub(1);
        max_line.to_string().len().max(2) as u16 + 1 // +1 for space after gutter
    } else {
        0
    };
    // badge column stays reserved for ANY diagnostic, so text never shifts
    // while scrolling (spec 2026-11-04, gutter stability)
    let badge_w: u16 = if doc.lsp_diags.is_empty() { 0 } else { 2 };
    Layout {
        tabs_on,
        ed_top,
        view_rows,
        ed_x,
        ed_w,
        gutter_w,
        badge_w,
        status_y: ed_top + view_rows as u16 + terminal_h,
        terminal_y,
        terminal_h,
    }
}

/// After a terminal size change: recompute every doc's scroll so the cursor
/// stays visible; degenerate sizes clamp to (0,0).
pub fn apply_resize(app: &mut crate::app::App, size: (u16, u16)) {
    app.last_size = size;
    // Compute editor width accounting for sidebar (same as draw does)
    let sidebar_x = app
        .sidebar
        .as_ref()
        .map(|s| s.width.min(size.0.saturating_sub(1)))
        .unwrap_or(0);
    // Match layout()'s terminal_h formula exactly so scroll clamping and
    // painting can never disagree about how many editor rows are visible.
    let terminal_h = app.terminal.as_ref()
        .filter(|t| t.visible)
        .map(|t| t.height.min(((size.1.saturating_sub(2)) / 2).max(1)))
        .unwrap_or(0);
    let vh = view_height(size.1, app.docs.len() >= 2)
        .saturating_sub(terminal_h as usize) as u16;
    let view = (vh, size.0.saturating_sub(sidebar_x));
    for doc in app.docs.iter_mut() {
        doc.cursor = doc.cursor.min(doc.rope.len_chars());
        let row = doc.rope.char_to_line(doc.cursor);
        let col = doc.cursor - doc.rope.line_to_char(row);
        (doc.scroll_row, doc.scroll_col) = clamp_scroll(row, col, view, (doc.scroll_row, doc.scroll_col));
    }
    // Resize terminal pane cols to match screen width; clamp height to half the screen.
    if let Some(term) = &mut app.terminal {
        let max_h = ((size.1.saturating_sub(2)) / 2).max(3);
        let new_h = term.height.min(max_h).max(3);
        term.resize(size.0, new_h);
    }
}

/// Cell-safe char: control chars (e.g. a TAB in a filename or pasted
/// palette input) would flush to the TTY raw and be re-expanded there,
/// desyncing every later paint (2026-11-05 overlap bug). UI strings map
/// them to a space; the editor loop expands tabs deliberately instead.
fn cell_safe(ch: char) -> char {
    if ch.is_control() { ' ' } else { ch }
}

/// Char-index -> display-column table for one line (tab-expansion aware;
/// same 8-col stop rule the editor loop paints with). None = no tabs, so
/// display col == char col (fast path for the common case). Entry i is the
/// display column of char i; the last entry is the line's display width.
fn display_map(chars: &[char]) -> Option<Vec<u32>> {
    chars.iter().any(|c| *c == '\t').then(|| {
        let mut map = Vec::with_capacity(chars.len() + 1);
        let mut d = 0u32;
        for &ch in chars {
            map.push(d);
            if ch == '\t' {
                d += 8 - (d % 8);
            } else {
                d += 1;
            }
        }
        map.push(d);
        map
    })
}

fn dcol(map: &Option<Vec<u32>>, col: usize) -> usize {
    match map {
        None => col,
        Some(m) => m[col.min(m.len() - 1)] as usize,
    }
}

/// Paint char-column style spans (tree-sitter syntax or captured preview
/// styles) onto one editor line row.
#[allow(clippy::too_many_arguments)]
fn paint_line_spans(
    buf: &mut ratatui::buffer::Buffer,
    ed_x: u16,
    gutter_w: u16,
    y: u16,
    line_start: usize,
    avail: usize,
    dmap: &Option<Vec<u32>>,
    line_idx: usize,
    spans: &[crate::syntax::FrameSpan],
) {
    for span in spans {
        if span.line != line_idx {
            continue;
        }
        let a = dcol(dmap, span.col).max(line_start);
        let b = dcol(dmap, span.end_col).min(line_start + avail);
        for d in a..b {
            buf[(ed_x + (d - line_start) as u16 + gutter_w, y)].set_style(span.style);
        }
    }
}

/// Visible chars of a rope line (newline stripped) — input for display_map.
fn line_chars(doc: &crate::document::Document, line_idx: usize) -> Vec<char> {
    if line_idx >= doc.rope.len_lines() {
        return Vec::new();
    }
    doc.rope.line(line_idx).chars().take_while(|c| *c != '\n').collect()
}

/// Command-palette popup geometry — shared by render_palette and the click
/// hit-testing (#11) so a click always lands on what was painted.
pub(crate) struct PopGeom {
    pub(crate) x0: u16,
    pub(crate) y0: u16,
    pub(crate) w: u16,
    pub(crate) h: u16,
    pub(crate) show_n: usize,
}

pub(crate) fn popup_geom(
    area: ratatui::layout::Rect,
    app: &App,
    menu: &crate::app::PaletteMenu,
) -> PopGeom {
    let show_n = crate::app::PALETTE_MAX_ROWS
        .min(menu.rows.len())
        .min(area.height.saturating_sub(4) as usize); // border + filter + border = 3 extra rows
    // Popup width: longest content line, capped at area.width-2
    let mut content_w = format!("> {}", menu.filter).chars().count();
    for it in menu.rows[menu.top..].iter().take(show_n) {
        let desc: String = it.desc.chars().take(28).collect();
        let hint = crate::keymap::key_hint(it.cmd, &app.config.keys, it.default_hint)
            .unwrap_or_default();
        content_w = content_w.max(format!("{:<16}{:<28}{}", it.name, desc, hint).chars().count());
    }
    let w = (content_w as u16 + 2).min(area.width.saturating_sub(2)).max(4);
    // Height: border-top + filter + rows + border-bottom
    let h = (show_n as u16 + 3).min(area.height.saturating_sub(2)).max(3);
    PopGeom {
        // Center
        x0: (area.width.saturating_sub(w)) / 2,
        y0: (area.height.saturating_sub(h)) / 2,
        w,
        h,
        show_n,
    }
}

fn render_palette(buf: &mut ratatui::buffer::Buffer, area: ratatui::layout::Rect, app: &App) {
    let menu = match &app.palette_menu {
        Some(m) => m,
        None => return,
    };

    let g = popup_geom(area, app, menu);
    let (pop_w, pop_h, x0, y0, show_n) = (g.w, g.h, g.x0, g.y0, g.show_n);

    // Build row strings: name(16) + desc(28) + hint
    let row_strings: Vec<String> = menu.rows[menu.top..]
        .iter()
        .take(show_n)
        .map(|it| {
            let desc: String = it.desc.chars().take(28).collect();
            let hint = crate::keymap::key_hint(it.cmd, &app.config.keys, it.default_hint)
                .unwrap_or_default();
            format!("{:<16}{:<28}{}", it.name, desc, hint)
        })
        .collect();
    let filter_line = format!("> {}", menu.filter);

    let sel_style = app.palette.palette_sel;

    // Draw border
    let x1 = x0 + pop_w.saturating_sub(1);
    let y1 = y0 + pop_h.saturating_sub(1);
    // top row
    if y0 < area.height && x0 < area.width { buf[(x0, y0)].set_char('┌').set_style(ratatui::style::Style::default()); }
    for x in (x0 + 1)..x1 {
        if y0 < area.height && x < area.width { buf[(x, y0)].set_char('─').set_style(ratatui::style::Style::default()); }
    }
    if y0 < area.height && x1 < area.width { buf[(x1, y0)].set_char('┐').set_style(ratatui::style::Style::default()); }
    // bottom row
    if y1 < area.height && x0 < area.width { buf[(x0, y1)].set_char('└').set_style(ratatui::style::Style::default()); }
    for x in (x0 + 1)..x1 {
        if y1 < area.height && x < area.width { buf[(x, y1)].set_char('─').set_style(ratatui::style::Style::default()); }
    }
    if y1 < area.height && x1 < area.width { buf[(x1, y1)].set_char('┘').set_style(ratatui::style::Style::default()); }
    // sides
    for y in (y0 + 1)..y1 {
        if y < area.height && x0 < area.width { buf[(x0, y)].set_char('│').set_style(ratatui::style::Style::default()); }
        if y < area.height && x1 < area.width { buf[(x1, y)].set_char('│').set_style(ratatui::style::Style::default()); }
    }

    // Filter line (y0 + 1)
    let fy = y0 + 1;
    if fy < area.height {
        let inner_w = (pop_w.saturating_sub(2)) as usize;
        for (i, ch) in filter_line.chars().map(cell_safe).take(inner_w).enumerate() {
            let x = x0 + 1 + i as u16;
            if x < x1 { buf[(x, fy)].set_char(ch).set_style(ratatui::style::Style::default()); }
        }
        // clear remainder
        for i in filter_line.chars().count()..inner_w {
            let x = x0 + 1 + i as u16;
            if x < x1 { buf[(x, fy)].set_char(' ').set_style(ratatui::style::Style::default()); }
        }
    }

    // Item rows starting at y0+2
    for (idx, row_str) in row_strings.iter().enumerate() {
        let ry = y0 + 2 + idx as u16;
        if ry >= y1 || ry >= area.height { break; }
        let style = if menu.top + idx == menu.sel { sel_style } else { ratatui::style::Style::default() };
        let inner_w = (pop_w.saturating_sub(2)) as usize;
        for (i, ch) in row_str.chars().map(cell_safe).take(inner_w).enumerate() {
            let x = x0 + 1 + i as u16;
            if x < x1 { buf[(x, ry)].set_char(ch).set_style(style); }
        }
        // clear remainder
        for i in row_str.chars().count()..inner_w {
            let x = x0 + 1 + i as u16;
            if x < x1 { buf[(x, ry)].set_char(' ').set_style(style); }
        }
    }
}

/// One rendered tab row: the text plus each label's char span within it
/// (span k covers buffer lo + k). The active label ends with its '×'.
/// Shared by draw() and the tab-row click hit-testing (#11).
pub struct TabLine {
    pub text: String,
    pub lo: usize,
    pub spans: Vec<(usize, usize)>,
}

pub fn tab_line(app: &App, width: u16) -> TabLine {
    let labels: Vec<String> = app
        .docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let name = doc_display(d);
            let plus = if d.dirty { "+" } else { "" };
            if i == app.active {
                // Helix-style close affordance; wired to clicks by hit_test (#11)
                format!("[{name}{plus}] ×")
            } else {
                format!("{name}{plus}")
            }
        })
        .collect();
    let full = labels.join(" ");
    // (text, lo, hi) — lo/hi are the label indices rendered in the window
    let (text, lo, hi) = if full.chars().count() <= width as usize {
        let hi = labels.len().saturating_sub(1);
        (full, 0usize, hi)
    } else {
        // too many/too wide: window around the active buffer, '…' marks clipping
        let n = labels.len();
        let budget = (width as usize).saturating_sub(4); // room for the two markers
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
        (s, lo, hi)
    };
    let mut spans = Vec::new();
    let mut pos = if lo > 0 { "… ".chars().count() } else { 0 };
    for i in lo..=hi {
        let len = labels[i].chars().count();
        spans.push((pos, pos + len));
        pos += len + 1;
    }
    TabLine { text, lo, spans }
}

fn draw_terminal(buf: &mut ratatui::buffer::Buffer, pane: &crate::terminal::TerminalPane, area: ratatui::layout::Rect) {
    use ratatui::style::{Color, Modifier, Style};
    let grid = pane.draw();
    for (row_idx, row) in grid.iter().enumerate() {
        let y = area.y + row_idx as u16;
        if y >= area.y + area.height {
            break;
        }
        for (col_idx, cell) in row.iter().enumerate() {
            let x = area.x + col_idx as u16;
            if x >= area.x + area.width {
                break;
            }
            let ch = if cell.ch.is_control() { ' ' } else { cell.ch };
            let mut style = Style::default();
            if cell.fg != Color::Reset {
                style = style.fg(cell.fg);
            }
            if cell.bg != Color::Reset {
                style = style.bg(cell.bg);
            }
            // attrs: bit 0 = bold, bit 1 = italic, bit 2 = underline
            if cell.attrs & 1 != 0 { style = style.add_modifier(Modifier::BOLD); }
            if cell.attrs & 2 != 0 { style = style.add_modifier(Modifier::ITALIC); }
            if cell.attrs & 4 != 0 { style = style.add_modifier(Modifier::UNDERLINED); }
            buf[(x, y)].set_char(ch).set_style(style);
        }
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    // ALL geometry comes from layout() — the single source shared with mouse
    // hit-testing (#11), so clicks can never disagree with what was painted.
    let lay = layout(area, app);
    let tabs_on = lay.tabs_on;
    let ed_top = lay.ed_top;
    let view_rows = lay.view_rows;
    let ed_x = lay.ed_x;
    let ed_w = lay.ed_w;
    let gutter_w = lay.gutter_w;
    let badge_w = lay.badge_w;
    let doc = app.doc();
    let total_gutter_w = gutter_w + badge_w;

    // The prompt input lives on the bottom echo line (Helix-style command
    // line); prompt_cursor_col marks where the caret belongs in that string.
    let (prompt_line, prompt_cursor_col): (Option<String>, usize) = {
        use crate::prompt::PromptKind::*;
        match &app.prompt {
            None => (None, 0),
            Some(pr) => {
                let (label, cursor_at) = match pr.kind {
                    Find => {
                        let active = |s: String, on: bool| if on { format!("[{s}]") } else { s };
                        let pat_on = pr.field == crate::prompt::PromptField::Pattern;
                        let mut label = format!(
                            "find > {}",
                            active(pr.line.clone(), pat_on)
                        );
                        let pat_end = label.len();
                        if pr.field == crate::prompt::PromptField::Replace || !pr.replace.is_empty() {
                            label.push_str(&format!(
                                " | replace > {}",
                                active(pr.replace.clone(), !pat_on)
                            ));
                        }
                        let at = if pat_on { pat_end } else { label.len() };
                        (label, at)
                    }
                    SaveAs => {
                        let label = format!("save as > {}", pr.line);
                        let end = label.len();
                        (label, end)
                    }
                    ConfirmQuit | ConfirmReload => {
                        let head = if matches!(pr.kind, ConfirmQuit) { "unsaved changes: " } else { "changed on disk: " };
                        let word = pr.choice.label().to_uppercase();
                        let label = format!("{head}{word} (Tab cycles, Enter confirms, Esc cancels)");
                        let at = head.len() + 2;
                        (label, at)
                    }
                    Command => {
                        let label = format!(":> {}", pr.line);
                        let end = label.len();
                        (label, end)
                    }
                };
                (Some(label), cursor_at)
            }
        }
    };

    // cursor position (before the buffer borrow below)
    let cur_row = doc.rope.char_to_line(doc.cursor.min(doc.rope.len_chars()));
    if app.prompt.is_some() {
        // The caret belongs on the echo line at the prompt's input position.
        let echo_y = lay.status_y + 1;
        if echo_y < area.height {
            let col = prompt_cursor_col.min(area.width.saturating_sub(1) as usize) as u16;
            let _ = f.set_cursor_position((col, echo_y));
        }
    } else if app
        .terminal
        .as_ref()
        .is_some_and(|t| t.visible && lay.terminal_h > 0)
    {
        // Terminal owns the keys while visible: park the caret on the
        // emulator's cursor inside the pane (the vt100 cursor is already
        // viewport-relative, so it stays correct after scrolling). While
        // scrolled back there is no live caret: not calling
        // set_cursor_position hides the hardware cursor (ratatui default).
        let term = app.terminal.as_ref().unwrap();
        if term.view_scroll == 0 {
            let (cx, cy) = term.emulator.cursor();
            let x = cx.min(area.width.saturating_sub(1));
            let y = (lay.terminal_y + cy).min(lay.terminal_y + lay.terminal_h - 1);
            let _ = f.set_cursor_position((x, y));
        }
    } else if app
        .sidebar
        .as_ref()
        .is_some_and(|sb| sb.mode != crate::sidebar::SidebarMode::TreeBrowser || !sb.input.is_empty())
        && ed_x > 0
    {
        // The sidebar owns the keyboard while its filter row is up: park the
        // caret just past the typed text instead of in the document.
        let sb = app.sidebar.as_ref().unwrap();
        let col = (2 + sb.input.chars().count()).min(ed_x.saturating_sub(1) as usize) as u16;
        let _ = f.set_cursor_position((col, ed_top));
    } else if cur_row >= doc.scroll_row && cur_row < doc.scroll_row + view_rows {
        let line_start = doc.rope.line_to_char(cur_row);
        let cur_col = doc.cursor - line_start;
        let vis = line_chars(&doc, cur_row);
        let dmap = display_map(&vis);
        let s = dcol(&dmap, doc.scroll_col.min(vis.len()));
        let d = dcol(&dmap, cur_col.min(vis.len()));
        let avail = (ed_w as usize).saturating_sub(total_gutter_w as usize);
        if d >= s && (d - s) < avail {
            let _ = f.set_cursor_position((
                ed_x + (d - s) as u16 + total_gutter_w,
                (cur_row - doc.scroll_row) as u16 + ed_top,
            ));
        }
    }

    let buf = f.buffer_mut();

    if tabs_on {
        // labels, windowing and char spans live in tab_line() so the click
        // hit-testing for this row (#11) sees the exact same layout
        let tl = tab_line(app, area.width);
        for (x, ch) in tl.text.chars().map(cell_safe).take(area.width as usize).enumerate() {
            let is_active = tl
                .spans
                .iter()
                .enumerate()
                .any(|(k, &(s, e))| tl.lo + k == app.active && x >= s && x < e);
            let style = if is_active { app.palette.tab_active } else { app.palette.tab_inactive };
            buf[(x as u16, 0)].set_char(ch).set_style(style);
        }
    }

    if let Some(sidebar) = &app.sidebar {
        render_sidebar(buf, ratatui::layout::Rect { x: 0, y: ed_top, width: ed_x, height: view_rows as u16 }, sidebar, app.palette.sidebar_sel, app.palette.sidebar_dir);
    }

    // base fg/bg for the whole editor content area (gutter incl., cells past
    // EOL); everything below patches on top. Style::default() in dark = no-op.
    buf.set_style(
        ratatui::layout::Rect { x: ed_x, y: ed_top, width: ed_w, height: view_rows as u16 },
        app.palette.default,
    );

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
                let cell = &mut buf[(ed_x + badge_w + i as u16, y)];
                cell.set_char(ch);
                cell.set_style(app.palette.line_no);
                if line_idx == cur_row {
                    cell.set_style(app.palette.current_line_no);
                }
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
        // Tabs are EXPANDED into cells here, never passed through: a raw '\t'
        // flushes as a real TAB, the terminal jumps to the next 8-col stop
        // while the app counted one cell, and every later diff frame paints
        // at wrong columns — the "text overlap/scramble" on tab-indented Go
        // (2026-11-05). Spans/selection/caret/find map through the same
        // per-line table so their colors and positions follow the text.
        let vis: Vec<char> = line.chars().take_while(|c| *c != '\n').collect();
        let dmap = display_map(&vis);
        let avail = (ed_w as usize).saturating_sub(total_gutter_w as usize);
        let s = dcol(&dmap, doc.scroll_col.min(vis.len()));
        {
            let mut d0 = 0u32; // display column of the current char's first cell
            for &ch in &vis {
                let cells = if ch == '\t' { 8 - (d0 % 8) } else { 1 };
                for k in 0..cells {
                    let d = (d0 + k) as usize;
                    if d >= s && (d - s) < avail {
                        buf[(ed_x + (d - s) as u16 + total_gutter_w, y)]
                            .set_char(if ch == '\t' { ' ' } else { ch });
                    }
                }
                d0 += cells;
            }
        }

        // paint syntax frame spans for this line
        if let Some(syntax) = &doc.syntax {
            paint_line_spans(buf, ed_x, total_gutter_w, y, s, avail, &dmap, line_idx, syntax.spans());
        }
        // preview buffers carry glow's colors as captured style ranges; once
        // the buffer is edited the coords no longer match the render, so drop
        // them (they come back on the next :preview re-render)
        if !doc.dirty {
            paint_line_spans(buf, ed_x, total_gutter_w, y, s, avail, &dmap, line_idx, &doc.preview_styles);
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
            let vis = line_chars(&doc, line_idx);
            let dmap = display_map(&vis);
            let s = dcol(&dmap, doc.scroll_col.min(vis.len()));
            let avail = (ed_w as usize).saturating_sub(total_gutter_w as usize);
            for off in from..to {
                let d = dcol(&dmap, off - line_start);
                if d >= s && (d - s) < avail {
                    buf[(ed_x + ((d - s) as u16) + total_gutter_w, y)]
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
            let vis = line_chars(doc, cur_row);
            let dmap = display_map(&vis);
            let s = dcol(&dmap, doc.scroll_col.min(vis.len()));
            let d = dcol(&dmap, cur_col.min(vis.len()));
            let avail = (ed_w as usize).saturating_sub(total_gutter_w as usize);
            let disp_col = ed_x + (d - s).min(avail.saturating_sub(1)) as u16 + total_gutter_w;
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
                for (x, ch) in text.chars().map(cell_safe).take(avail_w).enumerate() {
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

    render_palette(buf, area, app);

    // draw terminal pane if visible
    if let Some(term) = &app.terminal {
        if term.visible && lay.terminal_h > 0 {
            let term_area = ratatui::layout::Rect {
                x: 0,
                y: lay.terminal_y,
                width: area.width,
                height: lay.terminal_h,
            };
            draw_terminal(buf, term, term_area);
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
                    let vis = line_chars(&doc, line_idx);
                    let dmap = display_map(&vis);
                    let s = dcol(&dmap, doc.scroll_col.min(vis.len()));
                    let d = dcol(&dmap, (off - line_start).min(vis.len()));
                    if d >= s && (d - s) < (ed_w as usize).saturating_sub(total_gutter_w as usize) {
                        buf[(ed_x + ((d - s) as u16) + total_gutter_w, (line_idx - doc.scroll_row) as u16 + ed_top)]
                            .set_style(term_style);
                    }
                }
            }
        }
    }

    let mut status = match app.mode {
        crate::app::Mode::Edit => format!("EDIT{} ", if app.doc().dirty { "+" } else { "" }),
        crate::app::Mode::Action => format!("ACTION{} ", if app.doc().dirty { "+" } else { "" }),
    };
    // status_mode covers the mode word (incl. dirty '+' and trailing space)
    let mode_cells = match app.mode {
        crate::app::Mode::Edit => if app.doc().dirty { 6 } else { 5 },
        crate::app::Mode::Action => if app.doc().dirty { 8 } else { 7 },
    };
    if let Some(sidebar) = &app.sidebar {
        use crate::sidebar::SidebarMode;
        match sidebar.mode {
            SidebarMode::TreeBrowser => status.push_str("[tree] "),
            SidebarMode::FuzzyPicker => status.push_str("[picker] "),
            SidebarMode::SearchResults => {
                let trunc = if sidebar.search_truncated() { "+" } else { "" };
                status.push_str(&format!("[search {}{}{}] ", sidebar.search_hit_count(), trunc, if app.search_job.is_some() { "…" } else { "" }));
            }
        }
    }
    // Terminal status: show when pane exists and is visible OR exited.
    // Hidden-and-alive keeps the bar quiet (per task-7 ruling).
    if let Some(term) = &app.terminal {
        if term.visible || term.exited {
            status.push_str("  ");
            status.push_str(&term.status_line());
        }
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
        // file-format indicator: line ending + encoding (plat only reads UTF-8)
        status.push_str(if app.doc().line_ending == crate::document::LineEnding::CRLF { "  CRLF utf-8" } else { "  LF utf-8" });
    // right-aligned segments: selection count OR line:col, then document percent
    let doc = app.doc();
    let sel_chars: usize = doc.selections.iter().filter_map(|s| s.range()).map(|(a, b)| b - a).sum();
    let cursor = doc.cursor.min(doc.rope.len_chars()); // safety net: never trust callers to have remapped
    let cur_line = doc.rope.char_to_line(cursor);
    let pos = if sel_chars > 0 {
        format!("{sel_chars} selected")
    } else {
        format!("{}:{}", cur_line + 1, cursor - doc.rope.line_to_char(cur_line) + 1)
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
    // Echo line: prompt input while active, else transient messages
    // (status message, then a diagnostic message when the cursor sits on one).
    let echo: String = match &prompt_line {
        Some(l) => l.clone(),
        None => {
            let mut e = String::new();
            if let Some((msg, _)) = &app.status {
                e.push_str(msg);
            }
            let cur_diag_line = doc.rope.char_to_line(doc.cursor.min(doc.rope.len_chars()));
            if let Some(d) = doc.lsp_diags.iter().find(|d| d.line == cur_diag_line) {
                if !e.is_empty() {
                    e.push_str("  ");
                }
                e.push_str(&format!("{}:{}: {}", d.line + 1, d.start + 1, d.message));
            }
            e
        }
    };
    let echo_err = prompt_line.is_none()
        && app.status.as_ref().map(|(_, is_err)| *is_err).unwrap_or(false);
    let status_y = lay.status_y;
    let echo_y = status_y + 1; // last terminal row: echo/command line
    if status_y < area.height {
        let w = area.width as usize;
        let right_w = right.chars().count();
        // narrow terminals: right side wins, left truncates (mode+docs lead the string)
        let left_budget = if right_w < w { w - right_w - 1 } else { 0 };
        for (x, ch) in status.chars().take_while(|c| !c.is_control()).take(left_budget).enumerate() {
            let cell = &mut buf[(x as u16, status_y)];
            cell.set_char(ch).set_style(app.palette.status_bar);
            if x < mode_cells {
                cell.set_style(app.palette.status_mode);
            }
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
    // Echo line: full-width status_bar backdrop; prompt text, messages
    // (errors in diag_error color).
    if echo_y < area.height {
        let echo_fg = if echo_err {
            app.palette.diag_error.fg.unwrap_or(ratatui::style::Color::Red)
        } else {
            app.palette.status_bar.fg.unwrap_or(ratatui::style::Color::Reset)
        };
        for x in 0..area.width {
            buf[(x, echo_y)].set_style(app.palette.status_bar);
        }
        for (x, ch) in echo.chars().take_while(|c| !c.is_control()).take(area.width as usize).enumerate() {
            let cell = &mut buf[(x as u16, echo_y)];
            cell.set_char(ch).set_style(app.palette.status_bar).set_fg(echo_fg);
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
/// Filter-row + scroll-window math for the sidebar, shared by render_sidebar
/// and click hit-testing (#11). Returns (header rows above the list, first
/// visible global item index, list capacity in rows).
fn sidebar_view(sidebar: &crate::sidebar::Sidebar, rows: usize) -> (usize, usize, usize) {
    use crate::sidebar::SidebarMode;
    // Tree/picker: filter row appears when typing (picker always). Search mode
    // ALWAYS has the query row — it IS the input.
    let header = if sidebar.mode != SidebarMode::TreeBrowser || !sidebar.input.is_empty() {
        1
    } else {
        0
    };
    let capacity = rows.saturating_sub(header);
    // keep the cursor row on screen by scrolling the list head
    let head = if sidebar.cursor >= capacity { sidebar.cursor + 1 - capacity } else { 0 };
    (header, head, capacity)
}

fn render_sidebar(buf: &mut ratatui::buffer::Buffer, area: ratatui::layout::Rect, sidebar: &crate::sidebar::Sidebar, sel_style: ratatui::style::Style, dir_style: ratatui::style::Style) {
    use crate::sidebar::SidebarMode;
    let rows = area.height as usize;
    if area.width == 0 || rows == 0 {
        return;
    }
    let picker = sidebar.mode == SidebarMode::FuzzyPicker;
    let search = sidebar.mode == SidebarMode::SearchResults;
    // header/scroll-window math shared with hit_test via sidebar_view()
    let (header, head, capacity) = sidebar_view(sidebar, rows);
    let items = sidebar.visible_items();
    let ranges = sidebar.match_ranges();

    if header > 0 {
        let text = format!("> {}", sidebar.input);
        for (x, ch) in text.chars().map(cell_safe).take(area.width as usize).enumerate() {
            buf[(area.x + x as u16, area.y)].set_char(ch);
        }
    } else if items.is_empty() {
        for (x, ch) in "no files".chars().take(area.width as usize).enumerate() {
            buf[(area.x + x as u16, area.y)].set_char(ch);
        }
    }
    if search && items.is_empty() && header > 0 {
        // empty search: hint under the query row
        let hint = if sidebar.input.is_empty() { "type to search…" } else { "no matches" };
        for (x, ch) in hint.chars().map(cell_safe).take(area.width as usize).enumerate() {
            buf[(area.x + x as u16, area.y + 1)].set_char(ch);
        }
    }

    // Tree connector prefixes ("│  "-style guides + ├─/└─), one per row.
    let prefixes: Vec<String> = if picker || search {
        vec![String::new(); items.len()]
    } else {
        tree_connectors(&items)
    };

    // keep the cursor row on screen by scrolling the list head
    for (i, item) in items.iter().skip(head).take(capacity).enumerate() {
        let gi = head + i; // global row index (ranges/prefixes alignment)
        let y = area.y + (header + i) as u16;
        let is_root_dot = sidebar.root_dot().is_some_and(|rd| item.path == rd);
        let name = if is_root_dot {
            "..".to_string()
        } else if search {
            // "name:line text…" label from the hit (see Sidebar::search_label)
            sidebar.search_label(gi).unwrap_or_default()
        } else {
            let n = item.path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            if item.is_dir { format!("{n}/") } else { n }
        };
        let prefix = prefixes.get(gi).map(|p| p.as_str()).unwrap_or("");
        let text = format!("{prefix}{name}");
        for (x, ch) in text.chars().map(cell_safe).take(area.width as usize).enumerate() {
            let cell = &mut buf[(area.x + x as u16, y)];
            cell.set_char(ch);
            if item.is_dir {
                cell.set_style(dir_style);
            }
        }
        // Underline the filter-hit bytes inside the name (byte offsets map
        // onto the char cells covering them; name starts after the prefix).
        if let Some(hits) = ranges.get(gi) {
            if !hits.is_empty() {
                let prefix_chars = prefix.chars().count() as u16;
                let mut byte_idx = 0usize;
                for (ci, ch) in name.chars().enumerate() {
                    let ch_len = ch.len_utf8();
                    let hit = (byte_idx..byte_idx + ch_len).any(|b| hits.contains(&b));
                    if hit {
                        buf[(area.x + prefix_chars + ci as u16, y)]
                            .set_style(ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::UNDERLINED));
                    }
                    byte_idx += ch_len;
                }
            }
        }
        if gi == sidebar.cursor {
            for x in 0..area.width {
                buf[(area.x + x, y)].set_style(sel_style);
            }
        }
    }
}

/// Connector prefix per tree row, computed from the flat visible list:
/// ancestor-guide columns ("│ " while that ancestor has later siblings, else
/// "  ") plus the row's own branch ("├─ " / "└─ "). depth-0 rows get "".
fn tree_connectors(items: &[&crate::tree::TreeItem]) -> Vec<String> {
    fn has_more_at(items: &[&crate::tree::TreeItem], i: usize, level: usize) -> bool {
        for j in (i + 1)..items.len() {
            let d = items[j].depth;
            if d < level {
                return false;
            }
            if d == level {
                return true;
            }
        }
        false
    }
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            if it.depth == 0 {
                return String::new();
            }
            let mut p = String::new();
            for l in 1..it.depth {
                p.push_str(if has_more_at(items, i, l) { "│ " } else { "  " });
            }
            p.push_str(if has_more_at(items, i, it.depth) { "├─ " } else { "└─ " });
            p
        })
        .collect()
}

/// Inverse of display_map: a click's display column -> char index in the
/// line. Clicks inside a tab's expansion or past EOL land on the tab char /
/// line end respectively.
fn char_at_col(chars: &[char], target: usize) -> usize {
    let mut d = 0usize;
    for (i, &ch) in chars.iter().enumerate() {
        if d >= target {
            return i;
        }
        if ch == '\t' {
            let w = 8 - d % 8;
            if target < d + w {
                return i; // click inside the tab's blank: it covers this char
            }
            d += w;
        } else {
            d += 1;
        }
    }
    chars.len() // past EOL -> line end
}

/// What a screen coordinate refers to; hit_test and draw() share layout(),
/// tab_line(), popup_geom() and sidebar_view() so this can never drift from
/// the painted frame.
#[derive(Debug, PartialEq)]
pub enum Hit {
    Nothing,
    Tab { doc: usize, close: bool },
    /// global index into PaletteMenu.rows
    PaletteRow(usize),
    PaletteOutside,
    SidebarHeader,
    /// global row index into Sidebar::visible_items()
    SidebarRow(usize),
    /// row/char position in the active document
    EditorCell { row: usize, col: usize },
}

pub fn hit_test(area: ratatui::layout::Rect, app: &App, x: u16, y: u16) -> Hit {
    let lay = layout(area, app);

    // The palette popup is topmost: it owns any click it covers, and clicks
    // outside it report PaletteOutside so the caller can dismiss it.
    if let Some(menu) = &app.palette_menu {
        let g = popup_geom(area, app, menu);
        if x < g.x0 || x >= g.x0 + g.w || y < g.y0 || y >= g.y0 + g.h {
            return Hit::PaletteOutside;
        }
        if y >= g.y0 + 2 && (y - g.y0 - 2) < g.show_n as u16 {
            return Hit::PaletteRow(menu.top + (y - g.y0 - 2) as usize);
        }
        return Hit::Nothing; // border or filter row
    }

    if lay.tabs_on && y == 0 {
        let tl = tab_line(app, area.width);
        for (k, &(s, e)) in tl.spans.iter().enumerate() {
            if (x as usize) >= s && (x as usize) < e {
                let doc = tl.lo + k;
                // the active label's last char is its '×' close affordance
                let close = doc == app.active && (x as usize) == e - 1;
                return Hit::Tab { doc, close };
            }
        }
        return Hit::Nothing; // gap between labels
    }

    let in_view = y >= lay.ed_top && ((y - lay.ed_top) as usize) < lay.view_rows;

    // Clicks in the terminal pane area are no-ops in Phase 1 (not editor cells)
    if lay.terminal_h > 0 && y >= lay.terminal_y && y < lay.terminal_y + lay.terminal_h {
        return Hit::Nothing;
    }
    if let Some(sb) = &app.sidebar {
        if x < lay.ed_x && in_view {
            let (header, head, capacity) = sidebar_view(sb, lay.view_rows);
            let rel = (y - lay.ed_top) as usize;
            if rel < header {
                return Hit::SidebarHeader;
            }
            let gi = head + (rel - header);
            if rel - header < capacity && gi < sb.visible_items().len() {
                return Hit::SidebarRow(gi);
            }
            return Hit::Nothing; // past the end of the list
        }
    }

    if x >= lay.ed_x && in_view {
        let gutter = (lay.gutter_w + lay.badge_w) as usize;
        let rel = (x - lay.ed_x) as usize;
        if rel < gutter {
            return Hit::Nothing; // line-number / badge columns
        }
        let doc = app.doc();
        let row = doc.scroll_row + (y - lay.ed_top) as usize;
        if row >= doc.rope.len_lines() {
            return Hit::Nothing;
        }
        let chars = line_chars(doc, row);
        let map = display_map(&chars);
        let start = dcol(&map, doc.scroll_col.min(chars.len()));
        Hit::EditorCell { row, col: char_at_col(&chars, start + (rel - gutter)) }
    } else {
        Hit::Nothing
    }
}

/// Cell under the pointer for selection DRAGS (unlike hit_test's strict
/// EditorCell): coordinates clamp into the editor viewport, so a drag past
/// an edge grabs that edge's cell instead of dropping the selection.
pub fn editor_cell(area: ratatui::layout::Rect, app: &App, x: u16, y: u16) -> Option<(usize, usize)> {
    let lay = layout(area, app);
    if lay.view_rows == 0 {
        return None;
    }
    let doc = app.doc();
    let gutter = (lay.gutter_w + lay.badge_w) as usize;
    let rel = ((x.max(lay.ed_x) - lay.ed_x) as usize).max(gutter);
    let y = y.clamp(lay.ed_top, lay.ed_top + lay.view_rows as u16 - 1);
    let row = (doc.scroll_row + (y - lay.ed_top) as usize).min(doc.rope.len_lines().saturating_sub(1));
    let chars = line_chars(doc, row);
    let map = display_map(&chars);
    let start = dcol(&map, doc.scroll_col.min(chars.len()));
    // same mapping as hit_test's EditorCell: start is already the DISPLAY col
    // of scroll_col, so the click target is start + (rel - gutter)
    Some((row, char_at_col(&chars, start + (rel - gutter))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Mode};
    use ratatui::backend::TestBackend;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn buffer_of(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().buffer().clone()
    }

    fn row_string(buf: &ratatui::buffer::Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect()
    }

    #[test]
    fn syntax_highlight_follows_tab_expanded_text() {
        // Spans are char-columns, text is drawn at tab-EXPANDED display cols;
        // without mapping, keywords sit 7 cells left of their cells per tab
        // (the 2026-11-05 "keyword has no color / two colors" report).
        use ratatui::style::Color;
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str("func main() {\n\t\treturn true\n}\n");
        app.docs[0].path = Some(std::path::PathBuf::from("/tmp/t.go"));
        app.show_line_numbers = false;
        app.ensure_syntax(22);
        let buf = buffer_of(&app, 80, 24);
        let r = row_string(&buf, 1, 80);
        let c = r.find("return").expect("'return' must be rendered");
        for i in 0..6u16 {
            assert_eq!(
                buf[(c as u16 + i, 1)].fg,
                Color::LightMagenta,
                "keyword cell at x={} must be colored (row={:?})",
                c as u16 + i,
                r
            );
        }
        // and the indent must NOT carry the keyword's color
        assert_ne!(buf[(0, 1)].fg, Color::LightMagenta, "stale span painted the indent");
    }

    #[test]
    fn selection_and_caret_follow_tab_expanded_text() {
        use ratatui::style::Color;
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str("\treturn true\n");
        app.show_line_numbers = false;
        // select char range 0..7: the TAB (8 display cells) plus "retur"
        app.doc_mut().selections = vec![crate::document::Selection { anchor: 0, head: 7 }];
        let buf = buffer_of(&app, 80, 24);
        let r = row_string(&buf, 0, 80);
        let c = r.find("return").expect("rendered");
        // chars 1..7 of the source are inside "return": its first 6 cells
        // ("return" is chars 1-6) must carry the selection background
        for i in 0..6u16 {
            assert_eq!(buf[(c as u16 + i, 0)].bg, Color::Blue, "selection drift at x={} (row={:?})", c as u16 + i, r);
        }
    }

    #[test]
    fn filename_with_a_tab_never_leaks_control_chars() {
        // A raw \t cell flushes as a real TAB and the terminal re-expands it,
        // desyncing all later paints (2026-11-05 overlap bug). Same hazard
        // as the editor-loop fix, for tree rows / palette filter input.
        use crate::tree::TreeItem;
        let items = vec![TreeItem {
            path: std::path::PathBuf::from("we\tird.rs"),
            is_dir: false,
            depth: 0,
        }];
        let mut app = App::new(vec![]);
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 20));
        app.sidebar.as_mut().unwrap().input = "a\tb".into();
        let buf = buffer_of(&app, 80, 24);
        for y in 0..24 {
            for x in 0..80 {
                let s = buf[(x, y)].symbol();
                assert!(
                    s.chars().all(|c| !c.is_control()),
                    "control char {:?} leaked into cell ({x},{y})",
                    s
                );
            }
        }
    }

    #[test]
    fn tab_in_content_expands_and_never_reaches_the_buffer() {
        // Regression: a raw \t cell got flushed to the TTY as a real TAB —
        // the terminal expanded it to the next 8-col stop while the app
        // counted one column, so later diff frames wrote at wrong x and
        // text overlapped (garbled tab-indented Go, screenshot 2026-11-05).
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str("\tsamlib\n");
        app.show_line_numbers = false;
        let buf = buffer_of(&app, 80, 24);
        for y in 0..24 {
            for x in 0..80 {
                assert_ne!(buf[(x, y)].symbol(), "\t", "raw TAB leaked into cell ({x},{y})");
            }
        }
        assert_eq!(buf[(8, 0)].symbol(), "s", "'s' must sit at the 8-col tab stop");
        assert_eq!(buf[(0, 0)].symbol(), " ");
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

        // --- status row: fg/bg Reset; the mode-word block is intentionally
        // REVERSED (richer status line #8), every other status cell plain ---
        assert_eq!(buf[(0u16, 22u16)].fg, Color::Reset, "dark: status fg Reset");
        assert_eq!(buf[(0u16, 22u16)].bg, Color::Reset, "dark: status bg Reset");
        assert_eq!(buf[(0u16, 22u16)].modifier, Modifier::REVERSED, "dark: mode block intentionally REVERSED (#8)");
        assert!(buf[(20u16, 22u16)].modifier.is_empty(), "dark: status non-mode cells no modifiers");

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
    fn palette_render_selected_row_and_hint() {
        use ratatui::style::Color;
        let mut app = App::new(vec![]);
        app.handle_command(crate::keymap::Command::CommandPalette);
        for c in "save".chars() { app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE)); }
        let buf = buffer_of(&app, 80, 24);
        // find a cell with palette_sel bg (Blue in dark theme)
        let blue = (0..80u16).flat_map(|x| (0..24u16).map(move |y| (x, y)))
            .find(|(x, y)| buf[(*x, *y)].bg == Color::Blue);
        assert!(blue.is_some(), "a palette_sel (Blue) selected row must be painted");
        let row_y = blue.unwrap().1;
        let row_text: String = (0..80u16).map(|x| buf[(x, row_y)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row_text.contains("save") && row_text.contains("C-s"), "row shows name + hint: {row_text:?}");
    }

    #[test]
    fn palette_render_narrow_no_panic() {
        let mut app = App::new(vec![]);
        app.handle_command(crate::keymap::Command::CommandPalette);
        let buf = buffer_of(&app, 8, 6);
        let _ = buf[(0u16, 0u16)].symbol();
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
        assert_eq!(buf[(0u16, 22u16)].fg, ratatui::style::Color::Black, "light: status bar fg must be Black");
        assert_eq!(buf[(0u16, 22u16)].bg, ratatui::style::Color::Gray, "light: status bar bg must be Gray");
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
    fn preview_styles_paint_and_vanish_when_dirty() {
        use crate::syntax::FrameSpan;
        let mut app = App::new(vec![]);
        app.show_line_numbers = false;
        app.docs[0] = crate::document::Document::from_str("red text\n");
        app.docs[0].preview_styles = vec![FrameSpan {
            line: 0,
            col: 0,
            end_col: 3,
            style: ratatui::style::Style::default().fg(ratatui::style::Color::Red),
        }];
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0u16, 0u16)].fg, ratatui::style::Color::Red, "preview style must paint");
        assert_ne!(buf[(3u16, 0u16)].fg, ratatui::style::Color::Red, "end_col is exclusive");
        // edited buffer: captured coords no longer match the render, drop them
        app.docs[0].dirty = true;
        let buf = buffer_of(&app, 80, 24);
        assert_ne!(buf[(0u16, 0u16)].fg, ratatui::style::Color::Red, "stale styles must vanish when dirty");
    }

    #[test]
    fn default_role_paints_text_area() {
        // [theme.ui] default paints the editor area's base fg/bg; syntax spans
        // patch fg on top while keeping the default bg.
        let cfg = crate::config::from_str("[theme.ui]\ndefault = { fg = \"#cdd6f4\", bg = \"#1e1e2e\" }\n").unwrap();
        let mut app = App::new_with_config(vec![], cfg);
        app.show_line_numbers = false;
        app.docs[0] = crate::document::Document::from_str("fn main() {}\n");
        app.docs[0].path = Some(std::path::PathBuf::from("/tmp/t.rs")); // trigger rust syntax
        app.show_line_numbers = false;
        app.ensure_syntax(22);
        let buf = buffer_of(&app, 80, 24);
        // cell past end-of-line inside the editor area still gets base colors
        assert_eq!(buf[(40u16, 0u16)].bg, ratatui::style::Color::Rgb(0x1e, 0x1e, 0x2e), "default bg must paint empty editor-area cells");
        assert_eq!(buf[(40u16, 0u16)].fg, ratatui::style::Color::Rgb(0xcd, 0xd6, 0xf4), "default fg must paint empty editor-area cells");
        // keyword cell: syntax fg wins, default bg survives
        assert_eq!(buf[(0u16, 0u16)].fg, ratatui::style::Color::LightMagenta, "syntax fg must patch on top of default");
        assert_eq!(buf[(0u16, 0u16)].bg, ratatui::style::Color::Rgb(0x1e, 0x1e, 0x2e), "default bg must survive under syntax spans");
    }

    #[test]
    fn current_line_number_emphasized_in_gutter() {
        // cursor on display row 1 → that gutter number carries BOLD|UNDERLINED,
        // its neighbors stay plain.
        let mut app = App::new(vec![]);
        app.show_line_numbers = true;
        app.docs[0] = crate::document::Document::from_str("a\nb\nc\n");
        app.docs[0].cursor = 2; // start of line 2 ("b") → line_idx 1
        let buf = buffer_of(&app, 80, 24);
        let cur = buf[(0u16, 1u16)].style();
        assert!(cur.add_modifier.contains(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED), "cursor-line number must be emphasized, got {cur:?}");
        for row in [0u16, 2] {
            let s = buf[(0u16, row)].style();
            assert!(!s.add_modifier.contains(ratatui::style::Modifier::BOLD) && !s.add_modifier.contains(ratatui::style::Modifier::UNDERLINED), "non-cursor gutter row {row} must stay plain, got {s:?}");
        }
    }

    #[test]
    fn line_no_role_paints_gutter_numbers() {
        let mut app = App::new(vec![]);
        app.show_line_numbers = true;
        app.docs[0] = crate::document::Document::from_str("a\nb\nc\n");
        app.palette.line_no = ratatui::style::Style::default().bg(ratatui::style::Color::Red);
        let buf = buffer_of(&app, 80, 24);
        for row in [0u16, 1, 2] {
            assert_eq!(buf[(0u16, row)].bg, ratatui::style::Color::Red,
                       "gutter number on row {row} must carry line_no style");
        }
        assert_ne!(buf[(30u16, 0u16)].bg, ratatui::style::Color::Red,
                   "line_no must not bleed past the gutter");
    }

    /// Buffer + TestBackend for a frame (for backend state like the terminal
    /// cursor position).
    fn buffer_terminal(app: &App, w: u16, h: u16) -> (ratatui::buffer::Buffer, TestBackend) {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        (term.backend().buffer().clone(), term.backend().clone())
    }

    /// (x, y) of the first occurrence of `needle` on buffer row `line`.
    fn line_of_row(buf: &ratatui::buffer::Buffer, line: usize, needle: &str) -> Option<(u16, u16)> {
        for x in 0..buf.area.width {
            let mut ok = true;
            for (i, ch) in needle.chars().enumerate() {
                if buf[(x + i as u16, line as u16)].symbol() != ch.to_string() {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some((x, line as u16));
            }
        }
        None
    }

    #[test]
    fn prompt_renders_on_echo_line_not_status_bar() {
        use crate::prompt::{Prompt, PromptKind};
        let mut app = doc_app("hello world\n");
        app.prompt = Some(Prompt::new(PromptKind::Find, "foo".into()));
        let buf = buffer_of(&app, 80, 24);
        let echo = row_string(&buf, 23, 80);
        assert!(echo.contains("find > [foo]"), "prompt on echo line: {echo:?}");
        let status = row_string(&buf, 22, 80);
        assert!(status.starts_with("EDIT"), "status bar keeps its segments: {status:?}");
        assert!(!status.contains("find"), "prompt must leave the status bar: {status:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn save_as_and_command_prompts_render_on_echo_line() {
        use crate::prompt::{Prompt, PromptKind};
        let mut app = doc_app("x\n");
        app.prompt = Some(Prompt::new(PromptKind::SaveAs, "out.txt".into()));
        let mut echo = row_string(&buffer_of(&app, 80, 24), 23, 80);
        assert!(echo.contains("save as > out.txt"), "save-as prompt on echo line: {echo:?}");
        app.prompt = Some(Prompt::new(PromptKind::Command, "w".into()));
        echo = row_string(&buffer_of(&app, 80, 24), 23, 80);
        assert!(echo.starts_with(":> w"), "command prompt on echo line: {echo:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn confirm_prompt_renders_on_echo_line() {
        use crate::prompt::{Choice, Prompt, PromptKind};
        let mut app = doc_app("x\n");
        let mut p = Prompt::new(PromptKind::ConfirmQuit, String::new());
        p.choice = Choice::Save;
        app.prompt = Some(p);
        let echo = row_string(&buffer_of(&app, 80, 24), 23, 80);
        assert!(echo.contains("unsaved changes:") && echo.contains("SAVE"), "confirm prompt on echo line: {echo:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn echo_line_error_messages_use_diag_error_color() {
        let mut app = doc_app("x\n");
        app.status = Some(("saved".to_string(), false));
        let buf = buffer_of(&app, 80, 24);
        let (x, y) = line_of_row(&buf, 23, "saved").expect("info message on echo line (row 23)");
        let expected = app.palette.status_bar.fg.unwrap_or(ratatui::style::Color::Reset);
        assert_eq!(buf[(x, y)].fg, expected, "info message uses the status_bar fg");

        app.status = Some(("boom".to_string(), true));
        let buf = buffer_of(&app, 80, 24);
        let (x, y) = line_of_row(&buf, 23, "boom").expect("error message on echo line");
        assert_eq!(buf[(x, y)].fg, ratatui::style::Color::Red, "error messages use diag_error fg");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn terminal_cursor_sits_on_echo_line_during_prompt() {
        use crate::prompt::{Prompt, PromptKind};
        let mut app = doc_app("hello world\nsecond line\n");
        app.prompt = Some(Prompt::new(PromptKind::Find, "foo".into()));
        let (buf, backend) = buffer_terminal(&app, 80, 24);
        let pos = backend.cursor_position();
        let (cx, cy) = (pos.x, pos.y);
        assert_eq!(cy, 23, "caret must sit on the echo line during prompts, got y={cy}");
        let want = line_of_row(&buf, 23, "[foo]")
            .map(|(x, _)| (x + "[foo]".chars().count() as u16) as usize)
            .expect("bracketed pattern on echo line");
        assert_eq!(cx as usize, want, "caret must sit right after the active field text");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn terminal_cursor_lands_in_terminal_pane() {
        use crate::terminal::Vt100Emulator;
        let mut app = doc_app("hello world\n");
        app.handle_command(crate::keymap::Command::ToggleTerminal);
        let h = {
            let pane = app.terminal.as_mut().expect("terminal opened");
            // deterministic emulator: known bytes → known cursor (3,0)
            pane.emulator = Box::new(Vt100Emulator::new(80, pane.height));
            pane.emulator.write(b"abc");
            pane.height
        };
        // layout() geometry mirrored: pane occupies rows directly above status+echo
        let h = h.min((24 - 2) / 2).max(1);
        let pane_y = 24 - 2 - h;
        let (_, backend) = buffer_terminal(&app, 80, 24);
        let pos = backend.cursor_position();
        assert_eq!(
            (pos.x, pos.y),
            (3, pane_y),
            "caret must track the emulator cursor inside the terminal pane, got ({},{})",
            pos.x,
            pos.y
        );
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_mode_block_dirty_marker_and_format() {
        use ratatui::style::Modifier;
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str("a\nb\n"); // loader detects CRLF; set the field directly here
        app.docs[0].line_ending = crate::document::LineEnding::CRLF;
        let buf = buffer_of(&app, 80, 24);
        // mode block "EDIT " (cols 0..5) carries REVERSE; col 5 (past block) does not
        for x in 0..5u16 {
            assert!(buf[(x, 22u16)].modifier.contains(Modifier::REVERSED), "mode block REVERSE at col {x}");
        }
        assert!(!buf[(5u16, 22u16)].modifier.contains(Modifier::REVERSED), "block ends after 'EDIT '");
        // file-format indicator on the left
        let row: String = (0..60u16).map(|x| buf[(x, 22u16)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row.contains("CRLF utf-8"), "expected format indicator, got {row:?}");
        // dirty → '+' right after EDIT, inside the styled block
        app.docs[0].dirty = true;
        let buf2 = buffer_of(&app, 80, 24);
        assert_eq!(buf2[(4u16, 22u16)].symbol(), "+", "dirty marker after mode word");
        assert!(buf2[(4u16, 22u16)].modifier.contains(Modifier::REVERSED), "dirty '+' inside block");
        // clean doc shows LF
        let mut app2 = App::new(vec![]);
        app2.docs[0] = crate::document::Document::from_str("a\nb\n");
        let buf3 = buffer_of(&app2, 80, 24);
        let row3: String = (0..60u16).map(|x| buf3[(x, 22u16)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row3.contains("LF utf-8") && !row3.contains("CRLF"), "LF indicator for unix newlines, got {row3:?}");
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
        // #9 layout: active label carries a visual close affordance "×" and
        // the joined row "[aa.rs] × bb.rs" (15 chars) is LEFT-aligned (user
        // preference 2026-11-05). Active span = cols 0..9 (incl. the ' ×').
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0u16, 0u16)].symbol(), "[", "row must start at the left edge");
        assert!(buf[(0u16, 0u16)].modifier.contains(ratatui::style::Modifier::BOLD), "active tab must be BOLD");
        assert_eq!(buf[(8u16, 0u16)].symbol(), "×", "active label must show close affordance");
        assert!(buf[(8u16, 0u16)].modifier.contains(ratatui::style::Modifier::BOLD), "close '×' belongs to the active span");
        // inactive label 'b' starts after "[aa.rs] × " → col 10, must NOT be BOLD
        assert_eq!(buf[(10u16, 0u16)].symbol(), "b", "inactive label follows the active one");
        assert!(!buf[(10u16, 0u16)].modifier.contains(ratatui::style::Modifier::BOLD), "inactive tab must NOT be BOLD");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_survives_out_of_range_cursor() {
        // defense in depth: even if a caller leaves the cursor past the end,
        // the status bar must clamp instead of panicking the frame
        let dir = std::env::temp_dir().join(format!("plat-t3r-oob-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tiny.txt");
        std::fs::write(&p, "z\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.doc_mut().cursor = 999;
        let buf = buffer_of(&app, 80, 24);
        assert!(row_string(&buf, 22, 80).contains("EDIT"), "frame rendered without panic");
        std::fs::remove_file(&p).ok();
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
        let status = row_string(&buf, 22, 80);
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
        assert_eq!(row_string(&buf, 0, 20).trim_end(), "src/", "collapsed dir row shows a trailing slash");
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
    fn tree_rows_use_connectors_and_dir_slash() {
        use std::path::PathBuf;
        let mut app = doc_app("x\n");
        let items = vec![
            sb_item("src", true, 0), sb_item("src/a.rs", false, 1),
            sb_item("src/sub", true, 1), sb_item("src/sub/deep.rs", false, 2),
            sb_item("top", true, 0),
        ];
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 40));
        let sb = app.sidebar.as_mut().unwrap();
        sb.expand_path(&PathBuf::from("src"));
        sb.expand_path(&PathBuf::from("src/sub"));
        let buf = buffer_of(&app, 80, 24);
        let r0 = row_string(&buf, 0, 20);
        assert!(r0.starts_with("src/"), "top-level dir gets a slash: {r0:?}");
        let r1 = row_string(&buf, 1, 20);
        assert!(r1.starts_with("├─ a.rs"), "first sibling ├─: {r1:?}");
        let r2 = row_string(&buf, 2, 20);
        assert!(r2.starts_with("└─ sub/"), "last sibling └─: {r2:?}");
        let r3 = row_string(&buf, 3, 20);
        assert!(r3.starts_with("  └─ deep.rs"), "nested under a last-child parent: {r3:?}");
        let r4 = row_string(&buf, 4, 20);
        assert!(r4.starts_with("top/"), "next top-level row: {r4:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn tree_with_nested_sibling_shows_pipe_guide() {
        use std::path::PathBuf;
        let mut app = doc_app("x\n");
        let items = vec![
            sb_item("src", true, 0), sb_item("src/sub", true, 1),
            sb_item("src/sub/deep.rs", false, 2), sb_item("src/other", true, 1),
            sb_item("top", true, 0),
        ];
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 40));
        app.sidebar.as_mut().unwrap().expand_path(&PathBuf::from("src"));
        app.sidebar.as_mut().unwrap().expand_path(&PathBuf::from("src/sub"));
        let buf = buffer_of(&app, 80, 24);
        let r2 = row_string(&buf, 2, 20);
        assert!(r2.starts_with("│ └─ deep.rs"), "guide pipe while the ancestor's dir still has siblings: {r2:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn tree_filter_shows_header_and_underlines_name_hits() {
        use ratatui::style::Modifier;
        use std::path::PathBuf;
        let mut app = doc_app("x\n");
        let items = vec![sb_item("src", true, 0), sb_item("src/main.rs", false, 1)];
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 30));
        app.sidebar.as_mut().unwrap().expand_path(&PathBuf::from("src"));
        app.sidebar.as_mut().unwrap().filter_items("ain");
        let buf = buffer_of(&app, 80, 24);
        assert!(row_string(&buf, 0, 20).starts_with("> ain"), "filter line shows for the tree too: {:?}", row_string(&buf, 0, 20));
        // rows: [src (ancestor), src/main.rs]; name starts after "└─ " at x=3;
        // "ain" hits bytes 1..=3 of "main.rs" → x 4..=6
        for x in 4..=6u16 {
            assert!(buf[(x, 2u16)].modifier.contains(Modifier::UNDERLINED), "x={x} must be underlined");
        }
        assert!(!buf[(3u16, 2u16)].modifier.contains(Modifier::UNDERLINED), "'m' stays plain");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn sidebar_dirs_are_bold_and_root_dot_leads() {
        use ratatui::style::Modifier;
        let root = std::env::temp_dir().join(format!("plat-sbrender-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = doc_app("x\n");
        let items = vec![
            crate::tree::TreeItem { path: root.join("a.rs"), is_dir: false, depth: 0 },
            crate::tree::TreeItem { path: root.join("adir"), is_dir: true, depth: 0 },
        ];
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 30));
        let buf = buffer_of(&app, 80, 24);
        assert!(row_string(&buf, 0, 10).starts_with(".."), "the .. row leads absolute trees");
        assert!(buf[(0u16, 0u16)].modifier.contains(Modifier::BOLD), ".. is styled like a dir");
        assert!(!buf[(0u16, 1u16)].modifier.contains(Modifier::BOLD), "file names stay plain");
        assert!(buf[(0u16, 2u16)].modifier.contains(Modifier::BOLD), "dir names carry the sidebar_dir style");
        assert!(row_string(&buf, 2, 10).starts_with("adir/"), "dir slash");
        std::fs::remove_dir_all(&root).ok();
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
        let status = row_string(&buffer_of(&app, 80, 24), 22, 80);
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
        let status = row_string(&buf, 22, 80);
        assert!(status.contains("3:1"), "line:col missing: {status:?}");
        assert!(status.trim_end().ends_with("3:1 50%"), "right side misaligned: {status:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_top_bot_and_selection_replace_position() {
        let mut app = doc_app_named("tb.txt", "a\nb\nc\nd\ne");
        let status_at = |app: &mut App| {
            let buf = buffer_of(app, 80, 24);
            row_string(&buf, 22, 80)
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
        let status = row_string(&buf, 22, 20);
        assert!(status.starts_with("EDIT"), "mode must survive truncation: {status:?}");
        assert!(status.ends_with("1:1 Top"), "right side must stay pinned: {status:?}");
        assert!(!status.contains("fairly"), "messages no longer share the status bar: {status:?}");
        let echo = row_string(&buf, 23, 20);
        assert!(echo.starts_with("a fairly long"), "message lives on the echo line: {echo:?}");
        std::fs::remove_dir_all(app.doc().path.as_ref().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn status_bar_shows_branch_and_language() {
        let mut app = doc_app_named("t.rs", "fn main() {}\n");
        app.active_branch = Some("feat-x".to_string());
        let buf = buffer_of(&app, 80, 24);
        let status = row_string(&buf, 22, 80);
        assert!(status.contains("git(feat-x)"), "branch missing: {status:?}");
        assert!(status.contains("rust"), "language missing: {status:?}");
        // .txt files: language segment absent
        let app2 = doc_app_named("plain.txt", "x");
        let status2 = row_string(&buffer_of(&app2, 80, 24), 22, 80);
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
        let status = row_string(&buf, 22, 80);
        assert!(status.starts_with("EDIT"), "status kept mode: {status:?}");
        assert!(!status.contains("a.rs"), "status must not duplicate the tab row: {status:?}");
        assert_eq!(row_string(&buf, 23, 80).trim_end(), "", "echo line blank without messages");
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
    fn badge_column_reserved_even_when_diag_offscreen() {
        // Gutter stability spec (2026-11-04): the badge column is reserved for
        // ANY diag in the document — the text must not jump sideways when
        // scrolling into or out of a diagnostic.
        use crate::document::Diag;
        let mut app = doc_app("line one\nline two\n");
        app.show_line_numbers = false;
        // Put diag at line 40 (not visible in a 24-row window starting at 0).
        app.doc_mut().lsp_diags = vec![Diag { line: 40, start: 0, end: 1, severity: 1, message: "x".into() }];
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0, 0)].symbol(), " ", "badge column stays reserved (blank here)");
        assert_eq!(buf[(2, 0)].symbol(), "l", "text starts at col 2 even with the diag off-screen");
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
        let status = row_string(&buf, 22, 80);
        assert!(status.contains("2E"), "error count missing: {status:?}");
        assert!(status.contains("1W"), "warning count missing: {status:?}");
    }

    #[test]
    fn status_count_absent_when_no_diags() {
        let app = doc_app_named("t.rs", "fn main() {}\n");
        let buf = buffer_of(&app, 80, 24);
        let status = row_string(&buf, 22, 80);
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
        let echo = row_string(&buf, 23, 120); // diag messages live on the echo line
        assert!(echo.contains("expected ;"), "diag message on echo line: {echo:?}");
        assert!(echo.contains("1:4"), "diag line:col should appear: {echo:?}");
    }

    // ── Task 7: terminal pane layout & drawing ────────────────────────────────

    #[test]
    fn layout_without_terminal_areas_unchanged() {
        // Regression: adding terminal=None must not change any existing geometry.
        let mut app = App::new(vec![]);
        app.docs[0] = crate::document::Document::from_str("a\nb\n");
        app.show_line_numbers = false;
        let a = ratatui::layout::Rect::new(0, 0, 80, 24);
        let lay = layout(a, &app);
        assert_eq!(lay.view_rows, 22, "no terminal: all rows to editor");
        assert_eq!(lay.ed_top, 0);
        assert_eq!(lay.status_y, 22);
        assert_eq!(lay.terminal_h, 0, "no terminal: terminal_h = 0");
    }

    #[test]
    fn terminal_pane_shrinks_editor_area() {
        // Without PTY available the test is vacuous; with PTY it verifies geometry.
        use crate::terminal::TerminalPane;
        let cwd = std::env::temp_dir();
        match TerminalPane::new(8, 80, cwd) {
            Ok(mut pane) => {
                pane.visible = true;
                let mut app = App::new(vec![]);
                app.docs[0] = crate::document::Document::from_str("x\n");
                app.show_line_numbers = false;
                let a = ratatui::layout::Rect::new(0, 0, 80, 24);
                let baseline = layout(a, &app);
                app.terminal = Some(pane);
                let lay = layout(a, &app);
                assert!(lay.view_rows < baseline.view_rows, "editor must shrink when terminal visible");
                assert!(lay.terminal_h > 0, "terminal_h must be > 0");
                assert_eq!(
                    lay.view_rows + lay.terminal_h as usize + 2,
                    24,
                    "rows must sum to screen height: view_rows={} terminal_h={}",
                    lay.view_rows, lay.terminal_h
                );
            }
            Err(_) => {} // PTY unavailable — skip
        }
    }

    #[test]
    fn status_bar_shows_shell_when_terminal_visible() {
        use crate::terminal::TerminalPane;
        let cwd = std::env::temp_dir();
        match TerminalPane::new(10, 80, cwd) {
            Ok(mut pane) => {
                pane.visible = true;
                let mut app = App::new(vec![]);
                app.docs[0] = crate::document::Document::from_str("x\n");
                app.show_line_numbers = false;
                app.terminal = Some(pane);
                let buf = buffer_of(&app, 80, 24);
                let status = row_string(&buf, 22, 80);
                assert!(
                    status.contains("shell:") || status.contains("shell exited"),
                    "status must contain shell info when terminal visible: {status:?}"
                );
            }
            Err(_) => {}
        }
    }

    #[test]
    fn status_bar_silent_when_terminal_hidden() {
        use crate::terminal::TerminalPane;
        let cwd = std::env::temp_dir();
        match TerminalPane::new(10, 80, cwd) {
            Ok(mut pane) => {
                pane.visible = false;
                let mut app = App::new(vec![]);
                app.docs[0] = crate::document::Document::from_str("x\n");
                app.show_line_numbers = false;
                app.terminal = Some(pane);
                let buf = buffer_of(&app, 80, 24);
                let status = row_string(&buf, 22, 80);
                assert!(
                    !status.contains("shell:") && !status.contains("shell exited"),
                    "status must be silent when terminal hidden: {status:?}"
                );
            }
            Err(_) => {}
        }
    }

    #[test]
    fn terminal_layout_arithmetic() {
        // Direct arithmetic test: no PTY needed.
        // base_view = view_height(30, false) = 28
        // terminal_h = min(5, (30-2)/2=14) = 5
        // view_rows = 28 - 5 = 23
        let base = view_height(30, false);
        assert_eq!(base, 28);
        let term_h: usize = 5_usize.min((30usize.saturating_sub(2)) / 2);
        assert_eq!(term_h, 5);
        let view_rows = base.saturating_sub(term_h);
        assert_eq!(view_rows, 23);
        assert_eq!(view_rows + term_h + 2, 30, "all rows accounted for");
    }

    #[test]
    fn apply_resize_scroll_clamp_honors_visible_terminal_height() {
        // Regression: apply_resize used to clamp scroll against base view_rows,
        // ignoring the terminal pane. With a visible 10-row pane on a 24-row
        // screen, real editor rows = 24-2-10 = 12. A cursor at row 13 (0-based)
        // must become visible after resize, i.e. scroll_row <= 13 < scroll_row+12.
        use crate::terminal::TerminalPane;
        let cwd = std::env::temp_dir();
        match TerminalPane::new(10, 80, cwd) {
            Ok(mut pane) => {
                pane.visible = true;
                // Build a doc with 40 lines; put cursor on row 13.
                let content: String = (0..40).map(|i| format!("line{i}\n")).collect();
                let mut app = App::new(vec![]);
                app.docs[0] = crate::document::Document::from_str(&content);
                app.show_line_numbers = false;
                app.docs[0].cursor = app.docs[0].rope.line_to_char(13);
                app.terminal = Some(pane);
                apply_resize(&mut app, (80, 24));
                let doc = &app.docs[0];
                // layout() gives view_rows = 24-2-10=12 for a 10-row visible pane.
                // Cursor row 13 must be visible: scroll_row <= 13 < scroll_row + 12.
                let a = ratatui::layout::Rect::new(0, 0, 80, 24);
                let lay = layout(a, &app);
                assert!(doc.scroll_row <= 13, "scroll_row must be <= cursor row");
                assert!(doc.scroll_row + lay.view_rows > 13,
                    "cursor row 13 must be inside the shrunken editor (view_rows={}, scroll_row={})",
                    lay.view_rows, doc.scroll_row);
            }
            Err(_) => {} // PTY unavailable — skip
        }
    }

    #[test]
    fn layout_clamps_oversized_pane_on_window_shrink() {
        // Screen shrinks from 30→10 rows; pane height was 14 (more than half of 30).
        // After resize to height=10: max_h = (10-2)/2 = 4; pane clamped to max(3,4)=4.
        // Editor must still get at least 1 visible row.
        use crate::terminal::TerminalPane;
        let cwd = std::env::temp_dir();
        match TerminalPane::new(14, 80, cwd) {
            Ok(mut pane) => {
                pane.visible = true;
                let mut app = App::new(vec![]);
                app.show_line_numbers = false;
                app.terminal = Some(pane);
                // First size: 30 rows (pane height 14 fits within half=14)
                apply_resize(&mut app, (80, 30));
                // Shrink to 10 rows — pane height must clamp, editor must survive
                apply_resize(&mut app, (80, 10));
                let area = ratatui::layout::Rect::new(0, 0, 80, 10);
                let lay = layout(area, &app);
                assert!(lay.view_rows >= 1,
                    "editor must have >= 1 row after shrink (view_rows={})", lay.view_rows);
                assert!(lay.terminal_h >= 1,
                    "terminal pane must still show (terminal_h={})", lay.terminal_h);
                // total must not exceed screen height
                assert!(
                    lay.ed_top as usize + lay.view_rows + lay.terminal_h as usize + 2 <= 10,
                    "layout exceeds screen: ed_top={} view_rows={} terminal_h={}",
                    lay.ed_top, lay.view_rows, lay.terminal_h
                );
            }
            Err(_) => {} // PTY unavailable — skip
        }
    }
}


#[cfg(test)]
mod mouse_tests {
    use super::*;
    use crate::app::App;
    use crate::document::{Diag, Document};
    use crate::tree::TreeItem;
    use ratatui::layout::Rect;

    fn named_doc(name: &str, content: &str) -> Document {
        let mut d = Document::from_str(content);
        d.path = Some(std::path::PathBuf::from(name));
        d
    }

    fn tall_content() -> String {
        (0..40).map(|i| format!("line{i}\n")).collect()
    }

    #[test]
    fn layout_is_the_single_geometry_source() {
        let mut app = App::new(vec![]);
        app.docs[0] = Document::from_str("a\nb\nc\nd\n");
        let a = Rect::new(0, 0, 80, 24);
        let l = layout(a, &app);
        assert!(!l.tabs_on);
        assert_eq!((l.ed_top, l.view_rows, l.ed_x, l.ed_w), (0, 22, 0, 80));
        assert_eq!(l.gutter_w, 3); // 4 lines -> width 2 + trailing space
        assert_eq!(l.badge_w, 0);
        assert_eq!(l.status_y, 22);
        // a second buffer costs the top row
        app.docs.push(Document::from_str("x\n"));
        let l = layout(a, &app);
        assert!(l.tabs_on);
        assert_eq!((l.ed_top, l.view_rows, l.ed_w), (1, 21, 80));
        // the sidebar eats width from the left
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(vec![], 30));
        let l = layout(a, &app);
        assert_eq!((l.ed_x, l.ed_w), (30, 50));
        // diagnostics reserve the badge column
        app.docs[0].lsp_diags = vec![Diag { line: 0, start: 0, end: 1, severity: 1, message: "e".into() }];
        let l = layout(a, &app);
        assert_eq!(l.badge_w, 2);
        // a too-wide sidebar cannot eat the last column
        app.sidebar.as_mut().unwrap().width = 200;
        let l = layout(a, &app);
        assert_eq!((l.ed_x, l.ed_w), (79, 1));
    }

    #[test]
    fn char_at_col_maps_display_click_to_char() {
        let chars: Vec<char> = "\treturn".chars().collect();
        assert_eq!(char_at_col(&chars, 0), 0); // tab start
        assert_eq!(char_at_col(&chars, 3), 0); // inside the tab's expansion
        assert_eq!(char_at_col(&chars, 8), 1); // 'r' begins at display col 8
        assert_eq!(char_at_col(&chars, 99), chars.len()); // past EOL -> line end
        assert_eq!(char_at_col(&"abc".chars().collect::<Vec<char>>(), 2), 2);
    }

    #[test]
    fn hit_test_editor_cell_maps_through_gutter_badge_and_scroll() {
        let mut app = App::new(vec![]);
        app.docs[0] = Document::from_str(&tall_content());
        let a = Rect::new(0, 0, 80, 24);
        // gutter is 3 wide: click at x=5 -> char col 2 of scroll_row + y
        assert_eq!(hit_test(a, &app, 5, 2), Hit::EditorCell { row: 2, col: 2 });
        // gutter clicks are ignored
        assert_eq!(hit_test(a, &app, 1, 2), Hit::Nothing);
        // vertical scroll shifts rows
        app.doc_mut().scroll_row = 10;
        assert_eq!(hit_test(a, &app, 5, 2), Hit::EditorCell { row: 12, col: 2 });
        // horizontal scroll shifts cols
        app.doc_mut().scroll_col = 2;
        assert_eq!(hit_test(a, &app, 5, 2), Hit::EditorCell { row: 12, col: 4 });
        // the badge column also shifts content
        app.doc_mut().scroll_row = 0;
        app.doc_mut().scroll_col = 0;
        app.doc_mut().lsp_diags = vec![Diag { line: 0, start: 0, end: 1, severity: 1, message: "e".into() }];
        assert_eq!(hit_test(a, &app, 5, 2), Hit::EditorCell { row: 2, col: 0 });
        // status and echo rows are never editor cells
        assert_eq!(hit_test(a, &app, 40, 22), Hit::Nothing);
        assert_eq!(hit_test(a, &app, 40, 23), Hit::Nothing);
    }

    #[test]
    fn hit_test_editor_cell_respects_tab_row_offset() {
        let mut app = App::new(vec![]);
        app.docs[0] = named_doc("one.rs", &tall_content());
        app.docs.push(named_doc("two.rs", "b\n"));
        let a = Rect::new(0, 0, 80, 24);
        // row 0 is the tab row; the first editor row sits at y=1
        assert_eq!(hit_test(a, &app, 5, 1), Hit::EditorCell { row: 0, col: 2 });
    }

    #[test]
    fn hit_test_tab_row_activate_and_close() {
        let mut app = App::new(vec![]);
        app.docs[0] = named_doc("one.rs", "a\n");
        app.docs.push(named_doc("two.rs", "b\n"));
        app.active = 1;
        let a = Rect::new(0, 0, 80, 24);
        // labels: "one.rs" at 0..6, "[two.rs] ×" at 7..17 (× at col 16)
        assert_eq!(hit_test(a, &app, 2, 0), Hit::Tab { doc: 0, close: false });
        assert_eq!(hit_test(a, &app, 8, 0), Hit::Tab { doc: 1, close: false });
        assert_eq!(hit_test(a, &app, 16, 0), Hit::Tab { doc: 1, close: true });
        // the gap between labels is Nothing
        assert_eq!(hit_test(a, &app, 6, 0), Hit::Nothing);
    }

    #[test]
    fn tab_line_windows_labels_and_spans_map_to_buffers() {
        let mut app = App::new(vec![]);
        app.docs.clear();
        for i in 0..10 {
            app.docs.push(named_doc(&format!("{i}.rs"), "a\n"));
        }
        app.active = 9;
        let tl = tab_line(&app, 30);
        assert!(tl.text.starts_with('…'), "clipped window marker: {:?}", tl.text);
        assert!(tl.lo > 0);
        assert_eq!(tl.spans[0].0, 2, "first span starts after '… '");
        // span k corresponds to buffer lo + k
        let last = tl.spans.last().unwrap();
        assert_eq!(tl.lo + tl.spans.len() - 1, 9);
        assert!(last.1 <= 30 + 1, "window stays inside the width: {:?}", tl.text);
    }

    #[test]
    fn hit_test_sidebar_rows_follow_header_and_scroll_head() {
        let mut app = App::new(vec![]);
        app.docs[0] = named_doc("c.rs", &tall_content());
        let items: Vec<TreeItem> = (0..30)
            .map(|i| TreeItem { path: format!("f{i}.rs").into(), is_dir: false, depth: 0 })
            .collect();
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 20));
        let a = Rect::new(0, 0, 80, 24);
        assert_eq!(hit_test(a, &app, 5, 3), Hit::SidebarRow(3));
        // the filter row leads while the tree filter has text
        app.sidebar.as_mut().unwrap().input = "f".into();
        assert_eq!(hit_test(a, &app, 5, 0), Hit::SidebarHeader);
        assert_eq!(hit_test(a, &app, 5, 3), Hit::SidebarRow(2));
        app.sidebar.as_mut().unwrap().input.clear();
        // a cursor below capacity scrolls the head: 22 rows, cursor 25 -> head 4
        app.sidebar.as_mut().unwrap().cursor = 25;
        assert_eq!(hit_test(a, &app, 5, 0), Hit::SidebarRow(4));
        // past the item list is Nothing
        assert_eq!(hit_test(a, &app, 5, 25), Hit::Nothing);
        // the editor keeps its own geometry with the sidebar open
        assert_eq!(hit_test(a, &app, 25, 3), Hit::EditorCell { row: 3, col: 2 });
    }

    #[test]
    fn hit_test_palette_popup_inside_and_outside() {
        let mut app = App::new(vec![]);
        app.handle_command(crate::keymap::Command::CommandPalette);
        let a = Rect::new(0, 0, 80, 24);
        let menu = app.palette_menu.as_ref().unwrap();
        let g = popup_geom(a, &app, menu);
        assert_eq!(hit_test(a, &app, g.x0, g.y0), Hit::Nothing); // border
        assert_eq!(hit_test(a, &app, g.x0 + 1, g.y0 + 1), Hit::Nothing); // filter row
        assert_eq!(hit_test(a, &app, g.x0 + 1, g.y0 + 2), Hit::PaletteRow(0));
        assert_eq!(hit_test(a, &app, g.x0 + 1, g.y0 + 1 + g.show_n as u16), Hit::PaletteRow(g.show_n - 1));
        assert_eq!(hit_test(a, &app, g.x0 + 1, g.y0 + 2 + g.show_n as u16), Hit::Nothing); // bottom border
        assert_eq!(hit_test(a, &app, 0, 0), Hit::PaletteOutside);
    }
}

