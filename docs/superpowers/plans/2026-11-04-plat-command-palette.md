# Command Palette (#10) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a searchable command palette (`C-t`) that lists curated commands with descriptions and live key hints, and runs the selected command through the existing keypress command path.

**Architecture:** A `Command::CommandPalette` command opens `App.palette_menu: Option<PaletteMenu>`, a modal keyboard owner (like the completion menu) inserted into the lib.rs routing chain. A hand-curated `PALETTE_ITEMS` registry in `keymap.rs` supplies name/description/default-key; filtering reuses the picker's `subsequence_score`; Enter closes the palette then dispatches the command via `handle_command`. A new `palette_sel` theme role styles the selected row; the overlay is a centered boxed popup in `render.rs`.

**Tech Stack:** Rust, ratatui 0.26.3, crossterm (existing). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-04-plat-command-palette-design.md`

## Global Constraints

- No new dependencies (ratatui, crossterm, serde, toml already present).
- Full suite must stay green and **compiler-warning-free** after every task. Baseline at plan start: **326 unit + 13 flow + 19 lsp_flow**.
- Command dispatch must go through the existing `App::handle_command(cmd)` path — no parallel action implementations.
- Built-in `dark` palette stays a strict no-op upgrade for everything **outside** the new popup and its `palette_sel` role.
- Key-spec display strings match the `[keys]` parse format: `C-t`, `A-h`, `S-Tab`, `Enter`.
- Palette is keyboard-owned like the completion menu: while open it swallows control chords (Esc exits).

## Review Focus

Behaviors the spec implies but a single task's happy-path tests won't catch — each gets a pin in the owning task:

1. **Stale selection after the list shrinks** — filtering `save`→`s` changes row count; `sel`/`top` must clamp to the new length so neither render nor Enter indexes out of bounds. (Task 2)
2. **Empty filter, Enter pressed** — no rows match; Enter is a silent no-op (palette stays open, nothing dispatches), Esc still closes. (Task 2)
3. **User remap / unbind of `command_palette`** — a `[keys]` binding to another key moves the trigger and `[keys] … = "none"` unbinds it (no fallback to C-t opening the palette). (Task 1)
4. **Key-hint column tracks remaps, and default hints can't silently drift from the built-in bindings** — hint resolution prefers the user overlay, falls back to the registry's literal default, and a pin test proves every default hint actually produces that command via `lookup`. (Task 3)
5. **Control chords while the palette is open** — `C-q`/`C-c`/`C-g` do **not** quit/suspend/cancel from under the palette; the palette owns the keyboard (Esc is the exit). (Task 2)

---

## File Structure

- `src/keymap.rs` — `Command` enum gains `CommandPalette` + `SaveAsPrompt`; `command_by_name`; built-in `C-t` in `lookup` + `lookup_sidebar`; new `PaletteItem` struct + `PALETTE_ITEMS` const; `spec_label(KeySpec)->String`; `key_hint(...)`.
- `src/app.rs` — `PaletteMenu` struct; `App.palette_menu` field; `handle_command` arms for `CommandPalette`/`SaveAsPrompt`; `handle_palette_key(&KeyEvent)`.
- `src/lib.rs` — routing branch for `app.palette_menu.is_some()`.
- `src/picker.rs` — promote `subsequence_score` to `pub(crate)`.
- `src/theme.rs` — `palette_sel` role (struct field, `dark()`, `light()`, `UI_ROLES`, `apply_ui_overrides`).
- `src/render.rs` — palette overlay popup; popup render tests.
- `tests/flow.rs` — end-to-end palette flow.
- `README.md`, `PLAN.md` — docs + acceptance (Task 6).

---

## Task 1: Command surface — variants, names, `C-t` trigger, SaveAs trigger

**Files:**
- Modify: `src/keymap.rs` (`Command` enum ~:124-207; `command_by_name` ~:452-510; `lookup` global-Ctrl block ~:238-266; `lookup_sidebar` global-Ctrl block ~:355-365)
- Modify: `src/app.rs` (`handle_command` big `match cmd` ~:1066)
- Test: `src/keymap.rs` `mod tests`, `src/app.rs` `mod tests`

**Interfaces:**
- Produces:
  - `Command::CommandPalette`, `Command::SaveAsPrompt` (unit variants, `#[derive(Clone, Copy, PartialEq, Eq, Debug)]` already on `Command`).
  - `command_by_name("command_palette") -> Some(Command::CommandPalette)`, `command_by_name("save_as") -> Some(Command::SaveAsPrompt)`.
  - Built-in: `C-t` → `CommandPalette` in **both** `lookup` (any mode) and `lookup_sidebar` (any mode) global-Ctrl tables.
  - `handle_command(SaveAsPrompt)` opens `Prompt::new(PromptKind::SaveAs, String::new())` (reuses the existing-but-untriggered SaveAs prompt + `Document::save_as`).

- [ ] **Step 1: Write failing keymap tests**

Add to `src/keymap.rs` `mod tests`:

```rust
#[test]
fn command_palette_trigger() {
    for m in [Mode::Edit, Mode::Action] {
        assert_eq!(lookup(m, None, &key(KeyCode::Char('t'), KeyModifiers::CONTROL)),
                   Some(Command::CommandPalette), "C-t opens the palette in every mode");
    }
    // sidebar: global Ctrl table reaches it too
    assert_eq!(lookup_sidebar(&key(KeyCode::Char('t'), KeyModifiers::CONTROL)),
               Some(Command::CommandPalette));
    // names resolve for [keys] remapping
    assert_eq!(command_by_name("command_palette"), Some(Command::CommandPalette));
    assert_eq!(command_by_name("save_as"), Some(Command::SaveAsPrompt));
}
```

- [ ] **Step 2: Run it, confirm it fails**

Run: `cargo test command_palette_trigger 2>&1 | grep -E "test result|no variant|error\["`
Expected: FAIL — compile error `no variant named CommandPalette`.

- [ ] **Step 3: Add the two `Command` variants and name mappings**

In the `Command` enum, add unit variants `CommandPalette,` and `SaveAsPrompt,` (place near the global commands). In `command_by_name`, add `"command_palette" => Some(Command::CommandPalette),` and `"save_as" => Some(Command::SaveAsPrompt),`. In `lookup`'s global-Ctrl `match c.to_ascii_lowercase()` add `'t' => return Some(Command::CommandPalette),`; do the same in `lookup_sidebar`'s global-Ctrl table.

- [ ] **Step 4: Run the keymap test, confirm pass**

Run: `cargo test command_palette_trigger 2>&1 | grep -E "test result|warning"`
Expected: PASS; no warnings.

- [ ] **Step 5: Write the failing SaveAsPrompt app test**

Add to `src/app.rs` `mod tests`:

```rust
#[test]
fn save_as_command_opens_saveas_prompt() {
    let mut app = App::new(vec![]);
    app.handle_command(crate::keymap::Command::SaveAsPrompt);
    let p = app.prompt.as_ref().expect("SaveAsPrompt must open a prompt");
    assert_eq!(p.kind, crate::prompt::PromptKind::SaveAs);
}
```

- [ ] **Step 6: Run it, confirm it fails**

Run: `cargo test save_as_command_opens 2>&1 | grep -E "test result|panicked|expect"`
Expected: FAIL — no prompt opened (arm missing).

- [ ] **Step 7: Add the `handle_command` arm**

In `handle_command`'s `match cmd`, add:

```rust
Command::SaveAsPrompt => {
    self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::SaveAs, String::new()));
}
```

- [ ] **Step 8: Run full suite, confirm green + warning-free**

Run: `cargo test 2>&1 | grep -E "test result|warning|error\["`
Expected: all pass; no warnings.

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "keymap+app: CommandPalette + SaveAsPrompt commands, C-t trigger"
```

---

## Task 2: Palette state, keyboard handling, routing, filtering

**Files:**
- Modify: `src/app.rs` (add `PaletteMenu` struct ~after `CompMenu` ~:86; `App` field + ctor ~:61/:169; `handle_command` add `CommandPalette` arm; new `handle_palette_key`)
- Modify: `src/lib.rs` (routing chain ~:74-88)
- Modify: `src/picker.rs` (`subsequence_score` ~:14 → `pub(crate)`)
- Modify: `src/keymap.rs` — add `pub struct PaletteItem { cmd, name, desc }` + `pub const PALETTE_ITEMS: &[PaletteItem]` (18 entries)
- Test: `src/app.rs` `mod tests`

**Interfaces:**
- Consumes: `Command::CommandPalette` (Task 1); `crate::picker::subsequence_score(hay: &str, term: &str) -> Option<u32>`; `Command::SaveAsPrompt`.
- Produces:
  - `pub struct PaletteItem { pub cmd: Command, pub name: &'static str, pub desc: &'static str }` and `pub const PALETTE_ITEMS: &[PaletteItem]` (exact 18 entries listed in Step 1).
  - `pub struct PaletteMenu { pub filter: String, pub sel: usize, pub top: usize, pub rows: Vec<&'static PaletteItem> }`.
  - `App.palette_menu: Option<PaletteMenu>`.
  - `pub fn handle_palette_key(&mut self, key: &crossterm::event::KeyEvent)` — the keyboard owner.

- [ ] **Step 1: Add `PaletteItem` + `PALETTE_ITEMS` (exact table)**

In `src/keymap.rs`:

```rust
#[derive(Debug, Clone, Copy)]
pub struct PaletteItem {
    pub cmd: Command,
    pub name: &'static str,
    pub desc: &'static str,
}

pub const PALETTE_ITEMS: &[PaletteItem] = &[
    PaletteItem { cmd: Command::Save,            name: "save",             desc: "save current buffer" },
    PaletteItem { cmd: Command::SaveAsPrompt,    name: "save_as",          desc: "save with a new file name" },
    PaletteItem { cmd: Command::CloseBuffer,     name: "close_buffer",     desc: "close active buffer" },
    PaletteItem { cmd: Command::NextBuffer,      name: "next_buffer",      desc: "switch to next buffer" },
    PaletteItem { cmd: Command::PrevBuffer,      name: "prev_buffer",      desc: "switch to previous buffer" },
    PaletteItem { cmd: Command::Undo,            name: "undo",             desc: "undo last edit" },
    PaletteItem { cmd: Command::Redo,            name: "redo",             desc: "redo last undone edit" },
    PaletteItem { cmd: Command::Find,            name: "find",             desc: "search (regex, live preview)" },
    PaletteItem { cmd: Command::OpenSidebarPicker, name: "open_file",      desc: "fuzzy file picker" },
    PaletteItem { cmd: Command::OpenSidebarTree, name: "file_tree",        desc: "browse project files" },
    PaletteItem { cmd: Command::GotoDef,         name: "goto_definition",  desc: "jump to definition (LSP)" },
    PaletteItem { cmd: Command::JumpBack,        name: "jump_back",        desc: "jump back in nav stack" },
    PaletteItem { cmd: Command::NextDiagnostic,  name: "next_diagnostic",  desc: "next LSP diagnostic" },
    PaletteItem { cmd: Command::PrevDiagnostic,  name: "prev_diagnostic",  desc: "previous LSP diagnostic" },
    PaletteItem { cmd: Command::Yank,            name: "yank",             desc: "copy selection (or line)" },
    PaletteItem { cmd: Command::Paste,           name: "paste",            desc: "paste at cursor" },
    PaletteItem { cmd: Command::CommandPrefix,   name: "command_mode",     desc: "colon commands (:q :w :wq)" },
    PaletteItem { cmd: Command::Quit,            name: "quit",             desc: "exit (prompts to save if dirty)" },
];
```

- [ ] **Step 2: Promote `subsequence_score`**

In `src/picker.rs`, change `fn subsequence_score(` to `pub(crate) fn subsequence_score(`.

- [ ] **Step 3: Write the failing state/keyboard tests**

Add to `src/app.rs` `mod tests`. Use the existing test key ctor (see `fn key(code, mods)` in app tests):

```rust
fn palette(app: &mut App) { app.handle_command(crate::keymap::Command::CommandPalette); }

#[test]
fn palette_opens_and_filters() {
    let mut app = App::new(vec![]);
    palette(&mut app);
    assert!(app.palette_menu.is_some(), "CommandPalette opens the palette");
    assert_eq!(app.palette_menu.as_ref().unwrap().rows.len(), 18, "empty filter shows all 18");
    for c in "sav".chars() {
        app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let rows = &app.palette_menu.as_ref().unwrap().rows;
    assert!(!rows.is_empty() && rows[0].name == "save", "filter 'sav' ranks save first");
}

#[test]
fn palette_enter_runs_then_closes() {
    let p = tmp_path("pal_save.rs"); std::fs::write(&p, "x\n").unwrap();
    let mut app = App::new(vec![p.clone()]);
    app.doc_mut().edit(0, "z", 0); // make dirty
    assert!(app.doc().dirty);
    palette(&mut app);
    for c in "save".chars() { app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE)); }
    app.handle_palette_key(&key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.palette_menu.is_none(), "palette closes on Enter");
    assert!(!app.doc().dirty, "the dispatched Save cleared dirty");
    std::fs::remove_file(&p).ok();
}

#[test]
fn palette_shrinking_filter_clamps_selection() {
    // Review Focus #1: move sel far, then filter hard — no panic, sel valid
    let mut app = App::new(vec![]);
    palette(&mut app);
    for _ in 0..15 { app.handle_palette_key(&key(KeyCode::Down, KeyModifiers::NONE)); }
    for c in "quit".chars() { app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE)); }
    let m = app.palette_menu.as_ref().unwrap();
    assert!(m.sel < m.rows.len(), "sel within bounds after shrink (sel {} rows {})", m.sel, m.rows.len());
    // Enter must not panic and must run 'quit'? quit prompts (dirty check) — instead assert no panic path:
    app.handle_palette_key(&key(KeyCode::Enter, KeyModifiers::NONE)); // must not panic
}

#[test]
fn palette_empty_filter_enter_is_noop() {
    // Review Focus #2
    let mut app = App::new(vec![]);
    palette(&mut app);
    for c in "zzzz".chars() { app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE)); }
    assert!(app.palette_menu.as_ref().unwrap().rows.is_empty());
    app.handle_palette_key(&key(KeyCode::Enter, KeyModifiers::NONE)); // no-op
    assert!(app.palette_menu.is_some(), "Enter with no rows keeps the palette open");
    app.handle_palette_key(&key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.palette_menu.is_none(), "Esc closes");
}

#[test]
fn palette_swallows_ctrl_quit() {
    // Review Focus #5: C-q does not quit while palette is open
    let mut app = App::new(vec![]);
    palette(&mut app);
    app.handle_palette_key(&key(KeyCode::Char('q'), KeyModifiers::CONTROL));
    assert!(!app.quit, "C-q must not quit under the palette");
    assert!(app.palette_menu.is_some(), "C-q ignored, palette still open");
}
```

(If `tmp_path` / `key` helpers differ, mirror the existing app-test helpers already in the file.)

- [ ] **Step 4: Run them, confirm they fail**

Run: `cargo test palette_ 2>&1 | grep -E "test result|no field|no method|error\["`
Expected: FAIL — `no field palette_menu` / `no method handle_palette_key`.

- [ ] **Step 5: Implement `PaletteMenu`, field, open, keyboard owner, routing**

`src/app.rs` (after `CompMenu`):

```rust
pub struct PaletteMenu {
    pub filter: String,
    pub sel: usize,
    pub top: usize,
    pub rows: Vec<&'static crate::keymap::PaletteItem>,
}

const PALETTE_MAX_ROWS: usize = 10; // visible window; render clamps to screen too

impl App {
    fn palette_refilter(&mut self) {
        let Some(m) = self.palette_menu.as_mut() else { return };
        let term = m.filter.clone();
        let mut scored: Vec<(u32, &'static crate::keymap::PaletteItem)> = crate::keymap::PALETTE_ITEMS
            .iter()
            .filter_map(|it| crate::picker::subsequence_score(it.name, &term).map(|s| (s, it)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0)); // stable: ties keep registry order
        m.rows = scored.into_iter().map(|(_, it)| it).collect();
        m.sel = 0;
        m.top = 0;
    }

    pub fn handle_palette_key(&mut self, key: &crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        let Some(m) = self.palette_menu.as_mut() else { return };
        match key.code {
            KeyCode::Esc => { self.palette_menu = None; return; }
            KeyCode::Enter => {
                if m.rows.is_empty() { return; }
                let cmd = m.rows[m.sel].cmd;
                self.palette_menu = None;
                self.handle_command(cmd);
                return;
            }
            KeyCode::Backspace => { m.filter.pop(); self.palette_refilter(); return; }
            KeyCode::Up => { let n = m.rows.len(); if n > 0 { m.sel = (m.sel + n - 1) % n; } }
            KeyCode::Down => { let n = m.rows.len(); if n > 0 { m.sel = (m.sel + 1) % n; } }
            KeyCode::Char(c) if !key.modifiers.intersects(crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT) && !c.is_control() => {
                m.filter.push(c); self.palette_refilter(); return;
            }
            _ => return, // swallow everything else (Review Focus #5)
        }
        // scroll keep-selection: window [top, top+PALETTE_MAX_ROWS)
        let n = PALETTE_MAX_ROWS.min(m.rows.len().max(1));
        if m.sel < m.top { m.top = m.sel; }
        if m.sel >= m.top + n { m.top = m.sel + 1 - n; }
    }
}
```

In `handle_command`, add:

```rust
Command::CommandPalette => {
    self.completion = None;
    self.sidebar = None;
    self.palette_menu = Some(PaletteMenu { filter: String::new(), sel: 0, top: 0,
        rows: crate::keymap::PALETTE_ITEMS.iter().collect() });
}
```

Add the field `pub palette_menu: Option<PaletteMenu>,` to `App` and `palette_menu: None,` to the `App { … }` ctor. In `src/lib.rs`, insert before the `else if app.sidebar.is_some()` branch:

```rust
} else if app.palette_menu.is_some() {
    app.handle_palette_key(&key);
    None
}
```

- [ ] **Step 6: Run palette tests, confirm pass**

Run: `cargo test palette_ 2>&1 | grep -E "test result|warning|FAILED"`
Expected: all PASS; no warnings.

- [ ] **Step 7: Run full suite; commit**

Run: `cargo test 2>&1 | grep -E "test result|warning|error\["` then
`git add -A && git commit -m "app+keymap: PaletteMenu modal state, live fuzzy filtering, C-t routing"`

---

## Task 3: Key-hint resolution (`spec_label` + `key_hint`)

**Files:**
- Modify: `src/keymap.rs` (add `pub fn spec_label(code: KeyCode, mods: KeyModifiers) -> String`; add `pub fn key_hint(cmd: Command, user_keys: &KeyMap, default_hint: Option<&'static str>) -> Option<String>`; add `default_hint: Option<&'static str>` to `PaletteItem` and set it on the 18 rows)
- Test: `src/keymap.rs` `mod tests`

**Interfaces:**
- Consumes: `KeyMap = HashMap<KeySpec, Option<Command>>`, `parse_key_spec`, `canonical`.
- Produces:
  - `pub fn spec_label(code: KeyCode, mods: KeyModifiers) -> String` — display form inverse of `parse_key_spec`.
  - `pub fn key_hint(cmd, user_keys, default_hint) -> Option<String>` — first user binding for `cmd` (deterministic), else `default_hint.map(str::to_string)`.
  - `PaletteItem` gains field `pub default_hint: Option<&'static str>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn spec_label_round_trips_parser() {
    for spec in ["C-s","C-t","A-h","A-l","S-Tab","Enter","C-]","C-space","Up","Escape"] {
        let (code, mods) = parse_key_spec(spec).unwrap();
        let label = spec_label(code, mods);
        assert_eq!(parse_key_spec(&label).ok(), Some((code, mods)),
                   "spec_label({spec:?}) -> {label:?} must re-parse identically");
    }
}

#[test]
fn default_hints_match_builtin_bindings() {
    // Review Focus #4: default hints can't drift from the built-in lookup table.
    // Check each entry that declares a default_hint, in Action mode.
    for it in PALETTE_ITEMS {
        let Some(h) = it.default_hint else { continue };
        let (code, mods) = parse_key_spec(h).unwrap();
        let k = key(code, mods);
        assert_eq!(lookup(Mode::Action, None, &k), Some(it.cmd),
                   "hint {h:?} must actually bind {}", it.name);
    }
}

#[test]
fn key_hint_prefers_user_remap() {
    // user binds save to "C-g"? C-g is goto prefix builtin; overlay wins in lookup_with,
    // and key_hint scans the overlay map:
    let mut keys = KeyMap::new();
    keys.insert(parse_key_spec("M-s").unwrap(), Some(Command::Save));
    assert_eq!(key_hint(Command::Save, &keys, Some("C-s")).as_deref(), Some("M-s"));
    // no user binding → builtin default
    assert_eq!(key_hint(Command::Save, &KeyMap::new(), Some("C-s")).as_deref(), Some("C-s"));
}
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test spec_label_round 2>&1 | grep -E "test result|no function|error\["`
Expected: FAIL — `no function spec_label`.

- [ ] **Step 3: Add `default_hint` to the 18 rows**

Add the field to `PaletteItem` and set these built-in defaults (blank for the two prefix-only / unbound commands):

| name | default_hint |
|---|---|
| save | `Some("C-s")` |
| save_as | `None` |
| close_buffer | `Some("C-w")` |
| next_buffer | `Some("A-l")` |
| prev_buffer | `Some("A-h")` |
| undo | `Some("C-z")` |
| redo | `Some("C-y")` |
| find | `Some("C-f")` |
| open_file | `Some("C-p")` |
| file_tree | `Some("C-b")` |
| goto_definition | `Some("C-]")` |
| jump_back | `None` |
| next_diagnostic | `Some("]")` |
| prev_diagnostic | `Some("[")` |
| yank | `Some("C-c")` |
| paste | `Some("C-v")` |
| command_mode | `Some(":")` |
| quit | `Some("C-q")` |

- [ ] **Step 4: Implement `spec_label` and `key_hint`**

`spec_label`: prefix `C-`/`A-`/`S-` for CONTROL/ALT/SHIFT, then the key token; map named keys (`Enter`, `Escape`, `Tab`, `BackTab`→ emitted as `S-Tab`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `Up/Down/Left/Right`, `Char(' ')`→`Space`, otherwise the char itself). Keep it the exact inverse of `parse_key_spec` (the round-trip test is the contract). `key_hint`: iterate `user_keys` for entries `(spec, Some(c))` where `*c == cmd`; among matches pick the lexicographically-smallest `spec_label` for determinism; else `default_hint.map(|s| s.to_string())`.

- [ ] **Step 5: Run the three tests, confirm pass**

Run: `cargo test "spec_label_round|default_hints_match|key_hint_prefers" 2>&1 | grep -E "test result|warning|FAILED"`
Expected: all PASS; no warnings.

- [ ] **Step 6: Full suite; commit**

Run: `cargo test 2>&1 | grep -E "test result|warning|error\["` then
`git add -A && git commit -m "keymap: spec_label + key_hint with builtin-default pin"`

---

## Task 4: Theme role `palette_sel`

**Files:**
- Modify: `src/theme.rs` (`Palette` struct ~:181-194; `dark()` ~:200-229; `light()` light_ui delta array ~:231-245; `UI_ROLES` ~:285; `apply_ui_overrides` match ~:324-336)
- Test: `src/theme.rs` `mod tests`

**Interfaces:**
- Produces: `Palette.palette_sel: Style` — dark `bg Blue`, light `bg Gray`; known role name `"palette_sel"`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn palette_sel_role() {
    assert_eq!(Palette::dark().palette_sel.bg, Some(Color::Blue), "dark: bg Blue");
    assert_eq!(Palette::light().palette_sel.bg, Some(Color::Gray), "light mirrors completion_sel: bg Gray");
    let cfg = crate::config::from_str("[theme.ui]\npalette_sel = { bg = \"#ff0000\" }\n").unwrap();
    let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
    assert_eq!(p.palette_sel.bg, Some(Color::Rgb(255,0,0)));
    assert!(notes.is_empty(), "palette_sel must be a known role, got {notes:?}");
}
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test palette_sel_role 2>&1 | grep -E "test result|no field|error\["`
Expected: FAIL — `no field palette_sel`.

- [ ] **Step 3: Add the field + built-ins + role wiring**

`Palette` struct: add `pub palette_sel: Style,` (place after `diag_hint`). `dark()`: `palette_sel: Style::default().bg(Color::Blue),`. `light()`: append `("palette_sel", StyleSpec::Full(StyleTable { fg: None, bg: Some("light-gray".to_string()), bold: None, italic: None, underline: None }))` to `light_ui` and add its `"palette_sel" => p.palette_sel = spec.apply(p.palette_sel, &mut warns),` dispatch arm. Add `"palette_sel"` to `UI_ROLES`. Add `"palette_sel" => palette.palette_sel = spec.apply(palette.palette_sel, &mut w),` to `apply_ui_overrides`.

- [ ] **Step 4: Run test + full suite, confirm green**

Run: `cargo test 2>&1 | grep -E "test result|warning|error\["`
Expected: all PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "theme: palette_sel role (dark Blue / light Gray)"
```

---

## Task 5: Overlay rendering (centered popup + selection + hints)

**Files:**
- Modify: `src/render.rs` (add `render_palette(...)` and call it from `draw` after the completion-menu block ~:318, so the palette paints on top; import nothing new)
- Test: `src/render.rs` `mod tests`

**Interfaces:**
- Consumes: `App.palette_menu` (`PaletteMenu { filter, sel, top, rows }`, Task 2); `Palette.palette_sel` (Task 4); `crate::keymap::{PALETTE_ITEMS, key_hint}`; `App.config.keys`.
- Produces: visible popup with border, `> <filter>` line, and `name  description  hint` rows; selected row styled `palette_sel`.

- [ ] **Step 1: Write the failing render tests**

```rust
#[test]
fn palette_render_selected_row_and_hint() {
    let mut app = App::new(vec![]);
    app.handle_command(crate::keymap::Command::CommandPalette);
    // filter so 'save' is first, then check its hint column shows C-s
    for c in "save".chars() { app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE)); }
    let buf = buffer_of(&app, 80, 24);
    // some cell carries 'save' text and the selected row uses palette_sel (bg Blue in dark)
    let blue = (0..80u16).flat_map(|x| (0..24u16).map(move |y| (x,y)))
        .find(|(x,y)| buf[(*x,*y)].bg == ratatui::style::Color::Some(ratatui::style::Color::Blue));
    assert!(blue.is_some(), "a palette_sel (Blue) selected row must be painted");
    let row_text: String = (0..80u16).map(|x| buf[(x, blue.unwrap().1)].symbol().chars().next().unwrap_or(' ')).collect();
    assert!(row_text.contains("save") && row_text.contains("C-s"), "row shows name + hint: {row_text:?}");
}

#[test]
fn palette_render_narrow_no_panic() {
    // Review Focus / tiny screen: must not panic or index OOB
    let mut app = App::new(vec![]);
    app.handle_command(crate::keymap::Command::CommandPalette);
    let buf = buffer_of(&app, 8, 6);
    let _ = buf[(0u16, 0u16)].symbol(); // just prove it rendered
}
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test palette_render 2>&1 | grep -E "test result|panicked|assert"`
Expected: FAIL — nothing painted / no Blue row.

- [ ] **Step 3: Implement `render_palette` and call it**

Add a function drawing a centered boxed popup: compute rows to show = `menu.rows[menu.top .. menu.top + min(len, screen_h-4)]`; build each row string `format!("{:<16}{:<28}{}", name, desc, hint)` (desc truncated at cap, hint from `key_hint(it.cmd, &app.config.keys, it.default_hint)`); popup width = longest row capped at `area.width.saturating_sub(2)`; center x,y. Paint a border using `buf[(x,y)].set_char('─'/'│'/'┌'…)`; a `> {filter}` line; then the item rows, setting each cell's style to `palette_sel` when its item index equals `menu.sel`, else `Style::default()`. Clamp every write with `if y < area.height && x < area.width`. Call `render_palette(buf, area, app)` in `draw` right after the completion-menu block (~:318), before the find-highlight block.

- [ ] **Step 4: Run render tests + full suite, confirm green**

Run: `cargo test 2>&1 | grep -E "test result|warning|error\[|FAILED"`
Expected: all PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "render: centered command palette popup with hints + palette_sel row"
```

---

## Task 6: End-to-end flow test + docs + acceptance

**Files:**
- Test: `tests/flow.rs` (palette flow)
- Modify: `README.md` (command-palette section + `[keys]` mention + acceptance), `PLAN.md` (#10 → ✅)

- [ ] **Step 1: Write the flow test**

Add to `tests/flow.rs`, mirroring the in-process flow helpers (TestBackend + a dirty file):

```rust
#[test]
fn flow_command_palette_opens_filters_runs_save() {
    let dir = unique_tmp("flow-palette");
    let path = dir.join("flow_pal.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let mut app = App::new_with_config(vec![path.clone()], plat::config::Config::defaults());
    // dirty the buffer through a real edit command
    app.handle_command(plat::keymap::Command::EnterEditMode);
    app.handle_command(plat::keymap::Command::InsertChar('x'));
    assert!(app.doc().dirty);
    // open palette, filter to save, Enter → dispatches Save
    app.handle_command(plat::keymap::Command::CommandPalette);
    assert!(app.palette_menu.is_some());
    for c in "save".chars() { app.handle_palette_key(&key(KeyCode::Char(c), KeyModifiers::NONE)); }
    app.handle_palette_key(&key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.palette_menu.is_none(), "palette closed after Enter");
    assert!(!app.doc().dirty, "palette-dispatched Save wrote the file");
    // cleanup: force-quit
    app.quit = true;
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] [ ] **Step 2: Run it, confirm it fails before the wiring is present, then passes**

Run: `cargo test --test flow flow_command_palette 2>&1 | tail -5`
Expected: PASS (wiring exists from Tasks 1-5); if any assertion fails, fix the owning task, not the test.

- [ ] **Step 3: pty smoke (throwaway, not committed)**

Model on `/tmp/opencode/theme_smoke.py`: launch the release binary on a temp `.rs` file, send `C-t`, pump, assert the raw stream contains `save current buffer` (a description unique to the popup) and the popup border char `┌`; then send `save` + Enter and confirm the file is written (dirty cleared → no unsaved-changes prompt). Save the script at `/tmp/opencode/palette_smoke.py`. Record the observed strings in the README acceptance.

- [ ] **Step 4: README**

Add a "Command palette" section (terse house style): `C-t` opens; type to filter (fuzzy, matches command names); Up/Down select, Enter runs, Esc closes; every action runs through the normal command path so key hints reflect your `[keys]` remaps; `command_palette` is remappable, `"none"` unbinds. Add an acceptance bullet: flow test name, pty smoke strings, test counts (update the running total line).

- [ ] **Step 5: PLAN**

Mark item #10 ✅ with a one-line shipped blurb + pointer to README acceptance.

- [ ] **Step 6: Full suite + release build, then commit**

Run: `cargo test 2>&1 | grep -E "test result|warning|error\["` and `cargo build --release 2>&1 | tail -1`
Expected: green, warning-free, builds. Then:
`git add -A && git commit -m "docs+e2e: command palette shipped — flow test, pty smoke, README/PLAN"`

---

## Self-Review

- **Spec coverage:** trigger (T1), registry 18 entries (T2), filtering via subsequence_score (T2), keyboard owner + routing (T2), Enter-through-handle_command (T2), spec_label + key hints + drift pin (T3), palette_sel role (T4), centered popup + clamp (T5), SaveAsPrompt reusing existing SaveAs prompt (T1), flow + pty + docs (T6). No orphan spec requirements.
- **Type consistency:** `PaletteMenu{filter,sel,top,rows}`, `handle_palette_key(&KeyEvent)`, `PaletteItem{cmd,name,desc,default_hint}`, `spec_label(KeyCode,KeyModifiers)->String`, `key_hint(Command,&KeyMap,Option<&str>)->Option<String>`, `Palette.palette_sel` — used identically across T2-T5.
- **Review Focus:** #1 stale-sel (T2 clamp test), #2 empty-enter (T2), #3 remap/unbind trigger (T1 command_by_name + lookup; remap of the trigger is exercised by `lookup_with` overlay semantics already tested, plus T3 hint), #4 hint drift + remap (T3), #5 control chords swallowed (T2). Each pinned.
- **Proportion:** bodies only for the algorithms the signatures don't fix (refilter/sort, keyboard match, popup layout sketch); hint table + registry table are the spec's pinned copy.
