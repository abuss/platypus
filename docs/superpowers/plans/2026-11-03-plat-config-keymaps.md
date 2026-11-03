# TOML Config + Remappable Keymaps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One TOML config file (`$XDG_CONFIG_HOME/platypus/config.toml`) covering editor options, sidebar options, and a key-remap overlay, retiring the `PLAT_LINENO`/`PLAT_KEYDEBUG` env switches and folding in the Phase 1 sidebar follow-ups.

**Architecture:** Approach A — the compiled `keymap.rs` tables stay the source of truth; the config contributes a sparse `KeyMap` overlay consulted after pending-capture and before the default tables. A single `config.rs` module parses/validates everything at startup, before the terminal is initialized. Follow-up fixes (tree arrows, re-scan, symlinks, picker scoring, resize clamp) ride along in their own modules.

**Tech Stack:** Rust (edition 2024), ratatui 0.30, crossterm 0.29, ropey 1.6; new deps: `toml`, `serde` (derive).

**Spec:** `docs/superpowers/specs/2026-11-03-plat-config-keymaps-design.md` (read it alongside this plan)

## Global Constraints

- New dependencies: `toml` and `serde` (with `derive`) only. No others.
- All config errors are reported to **stderr before `crossterm::init`**, then exit 1; a missing config file means defaults silently.
- `#[serde(deny_unknown_fields)]` on every config section.
- Ranges clamp silently: `sidebar.width` 10–50, `editor.indent_width` 0–8. Type errors are hard errors.
- `PLAT_LINENO` and `PLAT_KEYDEBUG` are deleted (code + README). `PLAT_PANIC` stays, documented as a test hook.
- Prompt (`lookup_prompt`) and sidebar (`lookup_sidebar`) key tables never consult the overlay.
- `App::new(paths)` must NOT read the config file (tests use defaults); `run()` uses `App::new_with_config`.
- Test gates: `cargo test` green (baseline 138 unit + 8 flow at plan time), zero warnings from `cargo build`.
- Key canonicalization in specs and incoming events uses identical rules.

## Review Focus

1. **A typo'd config key** (`line_number = false`) → startup error naming the field and the right name, with the file path and line; plat must never start with half-loaded settings. (Task 1)
2. **The same binding spelled two ways** (`"N"` and `"S-n"`, or `"C-S-q"` and `"C-q"`) → duplicate-spec error, never last-wins. (Task 4)
3. **A config binding on a structural ownership key** (`"Tab"`, `"Esc"` while a prompt or sidebar is open) → prompt/sidebar keep their own keys; only the main layer is overlayed. (Task 5)
4. **A binding that would hijack a pending char capture** (`f` then a bound char) → the capture completes first, always. (Task 5)
5. **An existing but unreadable config file** (bad permissions) → startup failure like a bad file, not silent defaults. (Task 1)

---

### Task 1: `config.rs` core — path, load, parse, errors

**Files:**
- Create: `src/config.rs`
- Modify: `Cargo.toml` (add `toml`, `serde` with derive), `src/lib.rs` (add `pub mod config;`)
- Test: `src/config.rs` (unit tests in-module, repo convention)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub struct Config { pub editor: EditorCfg, pub sidebar: SidebarCfg, pub keys: std::collections::BTreeMap<String, String> }` — `keys` is the **raw** table; Task 4 converts it to the parsed overlay.
  - `pub struct EditorCfg { pub line_numbers: bool, pub indent_width: u8, pub key_debug: bool }` (defaults true, 2, false)
  - `pub struct SidebarCfg { pub width: u16, pub ignore: Vec<String> }` (defaults 32, empty)
  - `impl Config { pub fn defaults() -> Config }`
  - `pub fn config_path() -> PathBuf` — `$PLAT_CONFIG` if set, else `$XDG_CONFIG_HOME/platypus/config.toml`, else `~/.config/platypus/config.toml`
  - `pub fn load() -> Result<Config, ConfigError>` — missing file → `Config::defaults()`
  - `pub fn load_from(path: &Path) -> Result<Config, ConfigError>`
  - `pub fn from_str(text: &str) -> Result<Config, ConfigError>`
  - `pub struct ConfigError` implementing `Display` as `plat: config error in <path>: line <n>: <message>` (line omitted when unknown)

- [ ] **Step 1: Add dependencies**

```bash
cargo add toml serde --features serde/derive
```

- [ ] **Step 2: Write the failing tests** — in `src/config.rs`, the spec's example config parses; clamps; errors name the field:

```rust
#[test]
fn example_config_parses_end_to_end() {
    let cfg = from_str(r#"
[editor]
line_numbers = false
indent_width = 4

[sidebar]
width = 40
ignore = ["dist", "*.min.js"]

[keys]
"C-Space" = "select_line"
"C-q" = "none"
"#).unwrap();
    assert!(!cfg.editor.line_numbers);
    assert_eq!(cfg.editor.indent_width, 4);
    assert_eq!(cfg.sidebar.width, 40);
    assert_eq!(cfg.sidebar.ignore, vec!["dist", "*.min.js"]);
    assert_eq!(cfg.keys.get("C-Space").map(String::as_str), Some("select_line"));
}

#[test]
fn defaults_apply_for_missing_sections_and_file() {
    let cfg = from_str("").unwrap();
    assert!(cfg.editor.line_numbers);
    assert_eq!(cfg.editor.indent_width, 2);
    assert_eq!(cfg.sidebar.width, 32);
    assert!(cfg.keys.is_empty());
}

#[test]
fn cosmetic_ranges_clamp_silently() {
    let cfg = from_str("[sidebar]\nwidth = 900\n[editor]\nindent_width = 99\n").unwrap();
    assert_eq!(cfg.sidebar.width, crate::sidebar::MAX_WIDTH);
    assert_eq!(cfg.editor.indent_width, 8);
}

#[test]
fn unknown_field_is_named_with_a_suggestion_and_line() {
    let err = from_str("[editor]\nline_number = false\n").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("line_number"), "names the field: {msg}");
    assert!(msg.contains("line_numbers"), "suggests the right one: {msg}");
    assert!(msg.contains("line 2"), "reports the line: {msg}");
}

#[test]
fn wrong_type_reports_the_line() {
    let err = from_str("[editor]\nline_numbers = \"yes\"\n").unwrap_err();
    assert!(err.to_string().contains("line 2"), "{err}");
}

#[test]
fn unreadable_file_is_an_error_not_defaults() {
    // chmod 000 on a real file; same pattern as document.rs save-failure tests
    ...
}

#[test]
fn plconfig_env_overrides_xdg_path() {
    // set PLAT_CONFIG to a temp file, then load(); remember: env mutation is
    // `unsafe` in edition 2024 and this test must be the only one touching PLAT_CONFIG
    ...
}
```

- [ ] **Step 3: Run to verify they fail:** `cargo test config:: 2>&1 | head` — expected: compile error (module missing).

- [ ] **Step 4: Implement `src/config.rs`**

- serde structs: private `RawConfig`/`RawEditor`/`RawSidebar` with `#[derive(Deserialize)] #[serde(deny_unknown_fields, default)]`, public `Config`/`EditorCfg`/`SidebarCfg` with manual mapping from the raw form (that's where clamps happen); `#[serde(default)]` on every field so partial sections work.
- `ConfigError { file: Option<PathBuf>, line: Option<usize>, message: String }` with `Display` per the interface above.
- Convert `toml::de::Error` via `.line_col()` into `line`.
- Unknown-field "did you mean": ~10-line Levenshtein (or `edit_distance <= 2`) over that section's known field names, appended as `, did you mean 'line_numbers'?`.
- `load_from`: `fs::read_to_string` error → if `kind() == NotFound` return defaults, else `ConfigError` (Review Focus 5).
- `config_path()`: `std::env::var("PLAT_CONFIG")` → `var("XDG_CONFIG_HOME")` → `home_dir()` fallbacks as spec'd.

- [ ] **Step 5: Run `cargo test config::` — pass. Then `cargo test` — whole suite still green.**
- [ ] **Step 6: Commit** `git add -A && git commit -m "config: TOML loader with strict sections, clamps, did-you-mean errors (spec task 1)"`

---

### Task 2: Key-spec parsing and key canonicalization (`keymap.rs`)

**Files:**
- Modify: `src/keymap.rs` (append functions + tests)

**Interfaces:**
- Consumes: `crossterm::event::{KeyCode, KeyEvent, KeyModifiers}`.
- Produces (used by Tasks 4, 5):
  - `pub type KeySpec = (KeyCode, KeyModifiers);`
  - `pub fn parse_key_spec(spec: &str) -> Result<KeySpec, String>`
  - `pub fn canonical(key: &KeyEvent) -> KeySpec`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn specs_cover_letters_chords_and_named_keys() {
    assert_eq!(parse_key_spec("x"), Ok((KeyCode::Char('x'), KeyModifiers::NONE)));
    assert_eq!(parse_key_spec("C-q"), Ok((KeyCode::Char('q'), KeyModifiers::CONTROL)));
    assert_eq!(parse_key_spec("A-Left"), Ok((KeyCode::Left, KeyModifiers::ALT)));
    assert_eq!(parse_key_spec("S-Tab"), Ok((KeyCode::Tab, KeyModifiers::SHIFT)));
    assert_eq!(parse_key_spec("C-Space"), Ok((KeyCode::Char(' '), KeyModifiers::CONTROL)));
    assert_eq!(parse_key_spec("PageUp"), Ok((KeyCode::PageUp, KeyModifiers::NONE)));
}

#[test]
fn hyphen_is_the_remainder_after_known_prefixes() {
    assert_eq!(parse_key_spec("-"), Ok((KeyCode::Char('-'), KeyModifiers::NONE)));
    assert_eq!(parse_key_spec("A--"), Ok((KeyCode::Char('-'), KeyModifiers::ALT)));
    assert_eq!(parse_key_spec("C--"), Ok((KeyCode::Char('-'), KeyModifiers::CONTROL)));
}

#[test]
fn specs_normalize_like_incoming_events() {
    // C-S-q == C-q; bare uppercase == shift+lowercase
    assert_eq!(parse_key_spec("C-S-q"), parse_key_spec("C-q"));
    assert_eq!(parse_key_spec("N"), parse_key_spec("S-n"));
    assert!(parse_key_spec("C-S").is_err() == false); // letters lowercase under C-
}

#[test]
fn bad_specs_are_rejected_with_an_explanation() {
    for bad in ["", "C-", "Hyper-X", "Foo", "C-Foo", "Ab"] {
        let e = parse_key_spec(bad).unwrap_err();
        assert!(!e.is_empty() && e.contains(bad), "message names {bad:?}: {e}");
    }
}

#[test]
fn canonical_matches_what_crossterm_actually_sends() {
    // Ctrl+S arrives as Char('S') + CONTROL|SHIFT (existing test pins the wire fact)
    let ctrl_s = key(KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert_eq!(canonical(&ctrl_s), parse_key_spec("C-s").unwrap());
    // plain uppercase arrives as Char('N') + SHIFT
    let shift_n = key(KeyCode::Char('N'), KeyModifiers::SHIFT);
    assert_eq!(canonical(&shift_n), parse_key_spec("N").unwrap());
    // alt+left: Left + ALT (verified by the Phase 1 keydebug pty smoke)
    let alt_left = key(KeyCode::Left, KeyModifiers::ALT);
    assert_eq!(canonical(&alt_left), parse_key_spec("A-Left").unwrap());
    // plain 'x' stays plain
    assert_eq!(canonical(&key(KeyCode::Char('x'), KeyModifiers::NONE)), parse_key_spec("x").unwrap());
}
```

(`key(...)` is the existing test helper — extend the tests module or duplicate it there.)

- [ ] **Step 2: Verify they fail** (`parse_key_spec` undefined).
- [ ] **Step 3: Implement**

- `parse_key_spec`: loop consuming literal `"C-"`/`"A-"`/`"S-"` prefixes from the front, accumulating `KeyModifiers`; the remainder is the base token: one of the named tokens (`Space Enter Tab Esc Backspace Delete Up Down Left Right Home End PageUp PageDown`, case-insensitive) or a single `char`.
- Normalize the parsed spec by calling the *same* normalization used by `canonical` on the built `(KeyCode, KeyModifiers)` — one shared private `fn normalize(KeyCode, KeyModifiers) -> KeySpec` so specs and events can never drift: for `CONTROL` (and not `ALT`) with an ASCII letter, lowercase and drop `SHIFT`; for a bare uppercase letter (no CONTROL/ALT), lowercase and add `SHIFT`; otherwise as-is. Note `S-Tab` keeps SHIFT (not a letter).
- `canonical(key)` = `normalize(key.code, key.modifiers)`.
- Errors: empty spec, trailing prefix, multi-char base that is not a named token, unknown modifier prefix — message contains the offending spec string.

- [ ] **Step 4: Run `cargo test keymap::` — pass; full `cargo test` green.**
- [ ] **Step 5: Commit** `git commit -am "keymap: key-spec parser + shared canonicalization of specs and events"`

---

### Task 3: Command-name table (`keymap.rs`)

**Files:**
- Modify: `src/keymap.rs`

**Interfaces:**
- Consumes: `Command` enum (existing).
- Produces: `pub fn command_by_name(name: &str) -> Option<Command>` — exact name list from the spec ("Command vocabulary" section); unknown names → `None`; `jump_to_char`/`select_object` and prompt/sidebar commands must NOT be present.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_documented_name_maps_to_a_command() {
    for name in ["select_line", "expand_line", "begin_select_char", "jump_char",
                 "jump_char_back", "object_prefix", "yank", "paste_after", "paste_before",
                 "paste", "delete_selection", "change", "find", "find_next", "find_prev",
                 "save", "undo", "redo", "quit", "suspend", "close_buffer",
                 "open_sidebar_tree", "open_sidebar_picker",
                 "resize_sidebar.grow", "resize_sidebar.shrink", "indent",
                 "insert_newline", "backspace", "delete", "enter_action_mode",
                 "enter_edit_mode"] {
        assert!(command_by_name(name).is_some(), "missing {name}");
    }
    let motions = ["left","right","up","down","pageup","pagedown","word_next",
                   "word_prev","line_start","line_end"];
    for m in motions {
        assert!(command_by_name(&format!("move.{m}")).is_some(), "missing move.{m}");
        assert!(command_by_name(&format!("move.{m}.extend")).is_some(), "missing move.{m}.extend");
    }
}

#[test]
fn runtime_arg_and_ownership_commands_are_not_bindable() {
    for name in ["jump_to_char", "select_object", "prompt_submit", "prompt_cancel",
                 "sidebar_move", "sidebar_submit", "insert_char", "nop"] {
        assert!(command_by_name(name).is_none(), "{name} must not be bindable");
    }
    assert!(command_by_name("selct_line").is_none());
}
```

- [ ] **Step 2: Verify fail; Step 3: implement** — one `match name { ... }`, `resize_sidebar.grow => ResizeSidebar(2)`, `.shrink => ResizeSidebar(-2)`, `move.X.extend => Move(X, Select::Extend)`.
- [ ] **Step 4: `cargo test keymap::` green. Step 5: Commit** `git commit -am "keymap: command-name table for config bindings"`

---

### Task 4: Build the `KeyMap` overlay from config

**Files:**
- Modify: `src/config.rs` (replace raw `keys` map with the parsed overlay)

**Interfaces:**
- Consumes: `parse_key_spec`, `command_by_name`, `KeySpec` (Tasks 2–3).
- Produces:
  - `pub type KeyMap = std::collections::BTreeMap<crate::keymap::KeySpec, Option<crate::keymap::Command>>;` (defined in `keymap.rs`, re-exported through config)
  - `Config.keys: KeyMap` — `"none"` → `None`.
  - Config-load errors for: unparseable spec (propagated from `parse_key_spec`), `unknown command 'X'`, and duplicate specs after normalization (Review Focus 2).

- [ ] **Step 1: Write the failing tests** (replace Task 1's raw-map assertions about `cfg.keys` with parsed ones)

```rust
#[test]
fn keys_table_builds_the_parsed_overlay() {
    let cfg = from_str(r#"[keys]
"C-Space" = "select_line"
"C-q" = "none"
"N" = "find_prev""#).unwrap();
    use crate::keymap::{Command, parse_key_spec};
    assert_eq!(cfg.keys.get(&parse_key_spec("C-Space").unwrap()), Some(&Some(Command::SelectLine)));
    assert_eq!(cfg.keys.get(&parse_key_spec("C-q").unwrap()), Some(&None));
    assert_eq!(cfg.keys.get(&parse_key_spec("S-n").unwrap()), Some(&Some(Command::FindPrev)));
}

#[test]
fn the_same_binding_spelled_two_ways_is_a_duplicate_error() {
    let err = from_str("[keys]\n\"N\" = \"find_prev\"\n\"S-n\" = \"find_next\"").unwrap_err();
    assert!(err.to_string().contains("duplicate"), "{err}");
    let err2 = from_str("[keys]\n\"C-S-q\" = \"quit\"\n\"C-q\" = \"save\"").unwrap_err();
    assert!(err2.to_string().contains("duplicate"), "{err2}");
}

#[test]
fn unknown_command_and_bad_spec_are_named() {
    let e = from_str("[keys]\n\"C-q\" = \"selct_line\"").unwrap_err();
    assert!(e.to_string().contains("unknown command 'selct_line'"), "{e}");
    let e = from_str("[keys]\n\"Hyper-X\" = \"save\"").unwrap_err();
    assert!(e.to_string().contains("Hyper-X"), "{e}");
}
```

- [ ] **Step 2: Verify fail; Step 3: implement** — in `config.rs`, convert the raw `BTreeMap<String,String>` during mapping: parse spec → parse value (`"none"` → `None`, else `command_by_name`) → `entry`-style insert erroring on an existing key.
- [ ] **Step 4: `cargo test` green. Step 5: Commit** `git commit -am "config: [keys] parses to the KeySpec->Command overlay map, duplicates rejected"`

---

### Task 5: Overlay lookup, App wiring, editor options; retire the env switches

**Files:**
- Modify: `src/keymap.rs` (`lookup_with`), `src/app.rs` (`new_with_config`, `config` field, `Indent`), `src/lib.rs` (`run()` loads config before `ratatui::init`; routing through `lookup_with`)
- Test: `src/app.rs`, `src/keymap.rs`, `tests/flow.rs`

**Interfaces:**
- Consumes: `Config` (Task 1/4), `KeyMap`/`canonical` (Tasks 2/4).
- Produces:
  - `pub fn lookup_with(keys: &KeyMap, mode: Mode, pending: Option<Pending>, key: &KeyEvent) -> Option<Command>`
  - `pub fn App::new_with_config(paths: Vec<PathBuf>, config: Config) -> App`; `App.pub config: Config`; `App::new(paths)` == `new_with_config(paths, Config::defaults())`.
  - Seeded: `show_line_numbers`, `key_debug`, `sidebar_width`, and `Indent` inserts `config.editor.indent_width as usize` spaces.

- [ ] **Step 1: Write the failing tests** (keymap)

```rust
#[test]
fn overlay_beats_defaults_and_none_unbinds() {
    let keys = overlay(&[("C-Space", "select_line"), ("x", "none")]); // helper builds a KeyMap via from_str on a [keys] table
    assert_eq!(lookup_with(&keys, Mode::Action, None, &key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
               Some(Command::SelectLine));
    // unbound default key: nothing, and NO fallthrough to the old command
    assert_eq!(lookup_with(&keys, Mode::Action, None, &key(KeyCode::Char('x'), KeyModifiers::NONE)),
               Some(Command::Nop));
}

#[test]
fn pending_capture_completes_before_the_overlay() {   // Review Focus 4
    let keys = overlay(&[("q", "save")]);
    // `f` then q must jump to 'q', not run save
    assert_eq!(lookup_with(&keys, Mode::Action, Some(Pending::JumpForward), &key(KeyCode::Char('q'), KeyModifiers::NONE)),
               Some(Command::JumpToChar { ch: 'q', backward: false }));
}

#[test]
fn normalized_specs_match_real_events() {
    let keys = overlay(&[("C-S-q", "save"), ("N", "redo")]);
    // Ctrl+Q on a real terminal is Char('Q')+CONTROL|SHIFT, matching C-S-q spelling
    assert_eq!(lookup_with(&keys, Mode::Edit, None, &key(KeyCode::Char('Q'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
               Some(Command::Save));
    assert_eq!(lookup_with(&keys, Mode::Action, None, &key(KeyCode::Char('N'), KeyModifiers::SHIFT)),
               Some(Command::Redo));
}
```

(app tests)

```rust
#[test]
fn config_seeds_options_and_indent_width() {
    let cfg = config::from_str("[editor]\nindent_width = 4\nline_numbers = false\n").unwrap();
    let mut app = App::new_with_config(vec![], cfg);
    assert!(!app.show_line_numbers);
    app.handle_command(Command::Indent);
    assert_eq!(app.doc().rope.to_string(), "    ");
}

#[test]
fn prompt_and_sidebar_ignore_the_overlay() {          // Review Focus 3
    let cfg = config::from_str("[keys]\n\"Enter\" = \"none\"\n\"Esc\" = \"none\"").unwrap();
    let mut app = App::new_with_config(vec![], cfg);
    app.handle_command(Command::Find);            // prompt opens
    // Enter/Esc still work because run() routes the prompt via lookup_prompt,
    // which never consults keys; assert the routing decision instead:
    assert_eq!(keymap::lookup_prompt(&key(KeyCode::Enter, KeyModifiers::NONE)), Some(Command::PromptSubmit));
    assert_eq!(keymap::lookup_sidebar(&key(KeyCode::Esc, KeyModifiers::NONE)), Some(Command::SidebarCancel));
}
```

Also update the existing `lookup` test module call sites that go through `run()`-equivalent routing (they keep calling plain `lookup` — unchanged), and delete the `PLAT_LINENO`/`PLAT_KEYDEBUG` env reads from `App::new` (Review Focus line lands in README in Task 12).

- [ ] **Step 2: Verify fail; Step 3: implement**

- `lookup_with` exactly as the spec's pseudocode: pending → plain `lookup`; else `match keys.get(&canonical(key)) { Some(Some(c)) => Some(*c), Some(None) => Some(Command::Nop), None => lookup(...) }`.
- `lib.rs::run()`: load config **before** `ratatui::init`:

```rust
let config = match config::load() {
    Ok(c) => c,
    Err(e) => { eprintln!("{e}"); std::process::exit(1); }
};
let mut app = app::App::new_with_config(paths, config);
```

and route: `keymap::lookup_with(&app.config.keys, app.mode, app.pending, &key)`.
- `App::new_with_config` seeds the fields listed above; `Indent` uses `self.config.editor.indent_width`.

- [ ] **Step 4: `cargo test` green (138+8 baseline + new); `cargo build` warning-free.**
- [ ] **Step 5: Manual check** — `PLAT_CONFIG=/nonexistent-dir/x ./target/debug/plat` must print the config error and exit 1 **with no alternate-screen artifacts** (`echo $?` = 1; terminal still sane).
- [ ] **Step 6: Commit** `git commit -am "app+keymap: config overlay in the event loop, options seeded, PLAT_LINENO/PLAT_KEYDEBUG retired"`

---

### Task 6: `apply_resize` clamps against width-minus-sidebar

**Files:** Modify: `src/render.rs` (function `apply_resize`), tests in-module.

**Interfaces:** Consumes `App.sidebar_width`/`sidebar` (existing). Produces nothing new.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn resize_with_sidebar_open_clamps_scroll_to_the_narrower_editor() {
    let mut app = doc_app("x\n");
    app.sidebar = Some(crate::sidebar::Sidebar::new_tree(vec![sb_item("a.rs", false, 0)], 40));
    app.doc_mut().cursor = 70; // long line, scrolled right
    apply_resize(&mut app, (80, 24));
    // editor width is 80-40=40; the cursor column must fit inside that
    let d = app.doc();
    assert!(d.cursor - d.rope.line_to_char(0) >= d.scroll_col
            && d.cursor - d.scroll_col < d.scroll_col + 40,
            "scroll window must follow the shrunken editor width");
    assert!(d.scroll_col >= 31, "cursor col 70 must be visible in width 40: {}", d.scroll_col);
}
```

(Adjust the assertion arithmetic to `clamp_scroll`'s contract; the point is `view = (width - sidebar_width, height)` instead of `width - 1`.)

- [ ] **Step 2: Verify fail; Step 3: implement** — in `apply_resize`, `let view = (size.1.saturating_sub(1 + sidebar_x(app)), size.0);` where `sidebar_x(app)` is `app.sidebar.as_ref().map(|s| s.width.min(...)).unwrap_or(0)` — the same expression `draw` uses.
- [ ] **Step 4: `cargo test` green. Step 5: Commit** `git commit -am "render: apply_resize clamps scroll against width-minus-sidebar (Phase 1 ledger bug)"`

---

### Task 7: Tree arrows — `→` expand, `←` collapse-else-parent

**Files:** Modify: `src/keymap.rs` (`lookup_sidebar` Left/Right → two commands; add `Command::SidebarExpand`, `Command::SidebarCollapseOrParent`), `src/sidebar.rs` (`expand_path`, `collapse_or_parent`), `src/app.rs` (arms), tests in both files + the existing sidebar keymap test updated.

**Interfaces:** Consumes existing `TreeState.expanded`. Produces the two `Command` variants + `pub fn Sidebar::expand_path(&mut self, path: &PathBuf)`, `pub fn Sidebar::collapse_or_parent(&mut self, path: &PathBuf)` (collapses when expanded; else moves `cursor` to the row of `path.parent()` in `visible_items()` if present).

- [ ] **Step 1: Failing tests**

```rust
// keymap
assert_eq!(lookup_sidebar(&key(KeyCode::Right, KeyModifiers::NONE)), Some(Command::SidebarExpand));
assert_eq!(lookup_sidebar(&key(KeyCode::Left, KeyModifiers::NONE)), Some(Command::SidebarCollapseOrParent));

// sidebar: cursor lands on the parent dir row when collapsing is impossible
#[test]
fn left_arrow_collapses_then_walks_to_parent() {
    let items = vec![sb_item("src", true, 0), sb_item("src/app.rs", false, 1)];
    let mut sb = Sidebar::new_tree(items, 30);
    sb.cursor = 1;                       // on src/app.rs
    let p = PathBuf::from("src");
    sb.expand_path(&p);                  // → expands
    assert!(sb.tree_state.as_ref().unwrap().expanded.contains(&p));
    sb.collapse_or_parent(&PathBuf::from("src/app.rs")); // ← on a file: jump to parent row "src"
    assert_eq!(sb.selected_path(), Some(PathBuf::from("src")));
    sb.collapse_or_parent(&p);           // ← on an expanded dir: collapse it
    assert!(!sb.tree_state.as_ref().unwrap().expanded.contains(&p));
}
```

- [ ] **Step 2: Verify fail; Step 3: implement** (app arms call the sidebar methods with `selected_path()`; `SidebarToggleExpand` variant is deleted — grep shows its only producer is the Left/Right arm.)
- [ ] **Step 4: `cargo test` green (fix the Phase 1 `sidebar_owns_the_keyboard_when_open` expectation accordingly). Step 5: Commit**

---

### Task 8: Sidebar re-scan (`Ctrl-R`) and `Ctrl-F` find from the sidebar

**Files:** Modify: `src/keymap.rs` (`lookup_sidebar` ctrl arm: `'r' => SidebarRefresh`, `'f' => Find`; add the two Commands — `Find` already exists), `src/app.rs` (`open_sidebar` refactor with a `refresh_sidebar` that preserves state), tests.

**Interfaces:** Consumes `FileTree::build` (existing for now; Task 11 changes its signature — call it through one private helper `fn build_tree(&self) -> Result<FileTree, String>` so Task 11 edits one place). Produces `pub fn App::refresh_sidebar(&mut self)`; behavior: rebuild the tree, keep mode/width/`expanded` set/picker `input`, and keep the cursor on the same path when it still exists, else the nearest row.

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn ctrl_r_rescans_and_keeps_cursor_by_path() {
    // temp dir with one file; open tree sidebar; create a second file on disk;
    // handle_command(SidebarRefresh); assert the new file is now listed and
    // the cursor still sits on the first file's path.
    ...
}

#[test]
fn ctrl_r_when_the_cursor_path_is_gone_lands_on_the_nearest_row() { ... }

#[test]
fn ctrl_f_from_the_sidebar_opens_find_and_esc_returns_to_the_sidebar() {
    // open tree sidebar, handle Ctrl-F => app.prompt is Some(Find) and sidebar still Some;
    // PromptCancel => prompt None, sidebar Some (routing order verified at the
    // keymap layer too: lookup_sidebar(Char('f')+CONTROL) == Some(Command::Find))
    ...
}
```

- [ ] **Step 2: Verify fail; Step 3: implement** — `refresh_sidebar`: capture `(mode, selected_path(), expanded, input)`, rebuild via `build_tree()`, re-open in the same mode, restore `expanded` (intersected with real dirs), re-`filter_items(input)` for the picker, then set `cursor` = index of the saved path in `visible_items()` (else clamp).
- [ ] **Step 4: `cargo test` green. Step 5: Commit**

---

### Task 9: Symlinked directories are browsable, loops stay bounded

**Files:** Modify: `src/tree.rs` (`scan` follows symlinked dirs with a visited set), tests in-module.

**Interfaces:** Consumes nothing new. Produces unchanged `FileTree::build(root)` signature (Task 11 changes it).

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn symlinked_dir_is_listed_and_traversed() {
    // real/ subdirs with a file; symlink link -> real; build(root);
    // assert an item under link/ appears with is_dir chain expanded
}

#[test]
fn self_referencing_symlink_terminates() {
    // a/link -> a itself; build must return; item count stays bounded by MAX_DEPTH
    // (assert items.len() < 1000 and the test does not hang: it either passes
    // fast or the suite hangs -- the latter is a loud failure)
}

#[test]
fn unresolvable_symlink_target_counts_as_skipped() {
    // dangling symlink pointing nowhere: is_dir false, no crash; canonicalize
    // failure on a *dir* symlink increments skipped
}
```

- [ ] **Step 2: Verify fail; Step 3: implement** — in `scan`, when `file_type.is_symlink()`: `std::fs::metadata(&path)` (follows) to decide dir-ness; if dir: `let canon = path.canonicalize()`; `Err` → count into the existing `skipped` and skip recursion (the entry itself is still listed as a file); `Ok(c)` → pass `visited: &HashSet<PathBuf>` down (threaded as a new `scan` parameter), recurse only when `visited.insert(c)` returns true, and remove `c` after the recursion (branch-local visited, not global, so sibling symlinks to the same dir both list but the current branch can't loop). `MAX_DEPTH` unchanged.
- [ ] **Step 4: `cargo test tree::` green + full suite. Step 5: Commit**

---

### Task 10: Picker ranking — subsequence scoring

**Files:** Modify: `src/picker.rs`; tests in-module (update `search_results_sorted_by_score_descending` and add the ranking tests below).

**Interfaces:** Produces private `fn subsequence_score(hay: &str, term: &str) -> Option<u32>`; `FuzzyMatcher::search` keeps its signature and the empty-term path; substring tiers stay as the fast path, subsequence is the fallback that admits new matches (e.g. `fltr` → `filter.rs`).

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn subsequence_ranks_fltr_over_a_far_path_match() {
    let m = FuzzyMatcher { items: vec![
        item("src/filter.rs"), item("src/lorem/flitster.rs"), item("docs/report_flitster_long_name.md") ] };
    let r = m.search("fltr");
    assert_eq!(r[0].item.path.to_string_lossy(), "src/filter.rs", "gap-penalty + shorter-name wins: {r:?}");
}

#[test]
fn word_start_beats_mid_word() {
    let m = FuzzyMatcher { items: vec![item("src/my_filter.rs"), item("src/xfilter.rs")] };
    assert_eq!(m.search("fil")[0].item.path.to_string_lossy(), "src/my_filter.rs");
}

#[test]
fn non_subsequence_still_scores_zero() {
    let m = FuzzyMatcher { items: vec![item("src/render.rs")] };
    assert!(m.search("zq").is_empty());
}
```

- [ ] **Step 2: Verify fail; Step 3: implement** — on the filename first, else the relative path (path scores are divided by 4, integer): scan `term` chars in order; per matched char bonus `+8` at start-of-name or after `/ _ . -`, `+2` elsewhere; each gap char `-1`; then `- (hay.len() - term.len()) / 8` name-length tiebreak. `Some(score.max(1))` on a full subsequence else `None`. Keep the existing exact/prefix/contains tiers above it so current tests stay valid.
- [ ] **Step 4: `cargo test picker::` green + full suite. Step 5: Commit**

---

### Task 11: `[sidebar] ignore` reaches the scanner

**Files:** Modify: `src/tree.rs` (`build` gains `extra_ignore: &[String]`), `src/app.rs` (`build_tree` helper passes `&self.config.sidebar.ignore`), `src/gitignore.rs` (one new constructor), tests.

**Interfaces:** Produces `pub fn FileTree::build(root: &Path, extra_ignore: &[String]) -> Result<Self, String>` and `impl GitignoreRules { pub fn from_patterns(patterns: &[String]) -> Self }` (patterns appended to `.gitignore` rules and matched with the existing `match_rule`, so `dist` and `*.min.js` behave exactly like gitignore-lite rules).

- [ ] **Step 1: Failing tests** (tree module, following the existing temp-dir test style)

```rust
#[test]
fn config_ignore_patterns_are_skipped_like_gitignore_rules() {
    // dirs: dist/, vendor/lib.min.js, src/keep.rs
    let tree = FileTree::build(&root, &["dist".into(), "*.min.js".into()]).unwrap();
    let paths: Vec<_> = tree.items.iter().map(|i| i.path.strip_prefix(&root).unwrap().to_path_buf()).collect();
    assert!(!paths.iter().any(|p| p == Path::new("dist")));
    assert!(!paths.iter().any(|p| p.to_string_lossy().ends_with(".min.js")));
    assert!(paths.iter().any(|p| p == Path::new("src/keep.rs")));
}

#[test]
fn built_in_defaults_still_skipped_with_empty_extra() { /* node_modules absent */ }
```

- [ ] **Step 2: Verify fail; Step 3: implement** — in `build`, merge: `let extra = GitignoreRules::from_patterns(extra_ignore);` and check it next to `gitignore` in `scan` (pass both or concatenate their rule vectors — concatenating keeps one matcher and one pass).
- [ ] **Step 4: `cargo test` green. Step 5: Commit**

---

### Task 12: End-to-end flow test, pty smoke, README, PLAN

**Files:** Modify: `tests/flow.rs` (new chain test), `README.md` (config section: the example TOML, the full command-name list, `[editor] key_debug` as the chord-diagnostic, `PLAT_PANIC` documented as a test hook, `PLAT_LINENO`/`PLAT_KEYDEBUG` rows removed, `Ctrl-F`/`Ctrl-R`/arrow semantics updated), `PLAN.md` (work order #3 ✅ + acceptance).

- [ ] **Step 1: Write the decode-layer chain test** in `tests/flow.rs`

```rust
#[test]
fn config_remap_and_unbind_work_through_real_key_events() {
    let cfg = plat::config::from_str("[keys]\n\"C-Space\" = \"select_line\"\n\"x\" = \"none\"").unwrap();
    let mut app = App::new_with_config(vec![], cfg);
    type_text(&mut app, "hello");
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);          // ACTION
    // the default `x` (select_line) is gone:
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(app.doc().selection(), None, "unbind must not fall through");
    // and C-Space now selects the line:
    press(&mut app, KeyCode::Char(' '), KeyModifiers::CONTROL);
    assert_eq!(app.doc().selection(), Some((0, 5)));
}
```

- [ ] **Step 2: Run `cargo test --test flow` — pass.**
- [ ] **Step 3: pty smoke (release build)** — script at `/tmp/opencode/config_smoke.py` modeled on `repl_smoke.py`: a temp `PLAT_CONFIG` with `[keys]` remapping action-mode `x` → `begin_select_char`; open plat in a pty, Esc, press `x`, and assert the buffer got a char-selection (screenshot the status/grid), then Esc, Ctrl-Q quit, terminal restored. Record the run + the startup-error check (`PLAT_CONFIG=/nonexistent plat` exits 1 without touching the alternate screen) in README's new Acceptance (config) section: test counts (baseline 138+8 plus what the plan added), the 5 Review Focus behaviors each pinned by a named test, and `PLAT_CONFIG` smoke results.
- [ ] **Step 4: Update README + PLAN.md; `cargo test` one final time; commit** `git commit -am "docs+flow: config/keymaps acceptance, README rewrite of the config section; PLAN work order #3"`

---

## Self-review notes (kept for the record)

- Spec coverage: config file/format + errors → T1/T4/T5(+Review Focus 1,5); key specs/canonicalization → T2; command names → T3; overlay semantics incl. unbind/pending/ownership → T5(+Review Focus 2,3,4); follow-ups 1–8 → T6,T7,T8,T9,T10,T11,T5,T12; README/env-switch retirement → T12; acceptance → T12.
- Review Focus lines are pinned: 1 → T1 `unknown_field_is_named_with_a_suggestion_and_line`; 2 → T4 `the_same_binding_spelled_two_ways_is_a_duplicate_error`; 3 → T5 `prompt_and_sidebar_ignore_the_overlay`; 4 → T5 `pending_capture_completes_before_the_overlay`; 5 → T1 `unreadable_file_is_an_error_not_defaults`.
- Naming stays consistent: `lookup_with`, `KeyMap`, `KeySpec`, `canonical`, `parse_key_spec`, `command_by_name`, `new_with_config`, `config_path`, `load_from`, `from_str`, `defaults()`.
