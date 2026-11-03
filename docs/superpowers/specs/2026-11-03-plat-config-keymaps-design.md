# plat — TOML Config + Remappable Keymaps Design

Date: 2026-11-03
Status: draft for user review
Parent doc: PLAN.md — Work order #3 (retires the `PLAT_*` env switches; folds in
the Phase 1 sidebar follow-ups from the review ledger)

## Purpose

One config file covers all customization (success criterion: ~50 TOML lines
cover everything a user changes), and **any plat command can be rebound to
another key**. The `PLAT_LINENO` / `PLAT_KEYDEBUG` env switches are deleted in
favor of config options.

Agreed decisions (user): options **and** key remaps in one task; single user
file at `$XDG_CONFIG_HOME/platypus/config.toml` with a `PLAT_CONFIG` override;
`toml` + `serde` crates; **approach A** — the Rust tables in `keymap.rs` stay
the source of truth for defaults, config is a sparse overlay consulted before
them.

## Non-goals

- No project-local config file, no config merge story.
- Prompt and sidebar ownership key tables are **not** remappable (their keys
  are structural: Esc-cancel, Enter-submit, Tab-cycle). Remapping them lets a
  user lock themselves out of the tool they are using.
- No theme/colors section (work order #6 does that when highlighting exists).
- No watcher-driven tree auto-rescan; re-scan is explicit (`Ctrl-R`).
- No `n`/`N` while the sidebar is open: in the picker those keys are filter
  typing; a bare key event cannot disambiguate. Step through matches after
  closing the sidebar.

---

## Config file

Path: `$XDG_CONFIG_HOME/platypus/config.toml`, falling back to
`~/.config/platypus/config.toml`. `PLAT_CONFIG=<path>` overrides it (and is
what tests use). A missing file means defaults, silently.

```toml
[editor]
line_numbers = true       # gutter on/off (replaces PLAT_LINENO)
indent_width = 2          # Tab/Indent width; clamp 0..=8
key_debug = false         # raw KeyEvent echo in status (replaces PLAT_KEYDEBUG)

[sidebar]
width = 32                # seeds the session-persistent width; clamp 10..=50
ignore = ["dist", "*.min.js"]   # ADDED to built-in defaults
                              # (.git, node_modules, target, __pycache__, .egg-info)

[keys]
"C-Space" = "select_line"  # overlay: consulted before the default tables
"C-q" = "none"             # unbind quit (Ctrl-W still closes the buffer)
"N" = "none"               # bare uppercase = shift; plain letters bind action mode
```

Every section uses `#[serde(deny_unknown_fields)]`: `line_number = 1` is a
startup error naming the field, plus a did-you-mean against that section's
fields (~10-line helper, no new deps).

`PLAT_PANIC` stays: it is a test hook for the panic-restore check, not user
config.

---

## Module: `src/config.rs` (~120 lines)

```rust
pub struct Config {
    pub editor: EditorCfg,        // line_numbers, indent_width, key_debug
    pub sidebar: SidebarCfg,      // width, ignore: Vec<String>
    pub keys: KeyMap,             // BTreeMap<KeySpec, Option<Command>>
}

pub type KeySpec = (crossterm::event::KeyCode, crossterm::event::KeyModifiers);
pub type KeyMap = std::collections::BTreeMap<KeySpec, Option<Command>>;

impl Config {
    pub fn defaults() -> Config;                    // no file read; tests use this
    pub fn from_str(toml: &str) -> Result<Config, ConfigError>; // exposed for tests
}
pub fn path() -> PathBuf;                           // PLAT_CONFIG or XDG fallback
pub fn load() -> Result<Config, ConfigError>;       // missing file -> defaults
```

- `ConfigError` carries the path and serde/toml's line+column when available;
  `Display` is the single stderr line the user sees.
- `keys` is built at load: each `keys` table entry is parsed with
  `parse_key_spec` and `command_by_name`, so bad specs and unknown command
  names are config-load errors, never runtime surprises. Duplicate specs after
  normalization (both `"N"` and `"S-n"`) are an error.
- `App::new(paths)` keeps **defaults, no file read** (tests must not pick up a
  personal config). `App::new_with_config(paths, cfg)` is used by `run()`,
  which calls `config::load()` **before** `crossterm::init` — failure is plain
  stderr + exit 1, the terminal is never initialized.
- `App` owns `pub config: Config`. Existing fields (`show_line_numbers`,
  `sidebar_width`, `key_debug`) are seeded from it at startup instead of read
  per-frame; `tree::FileTree::build` receives `&SidebarCfg` for its ignore
  rules (defaults + config patterns).

---

## Remap mechanics

### Key spec grammar

`[C-][A-][S-]` prefixes + one base token: a printable character, or
`Space Enter Tab Esc Backspace Delete Up Down Left Right Home End PageUp
PageDown`. Parser ~40 lines with unit tests.

Normalization (applied identically to specs and to incoming `KeyEvent`s, so
what a terminal actually sends matches what the file says):

- For a `C-` chord the letter lowercases and any `S-` is dropped: `C-S-q` and
  `C-q` are one spec (crossterm reports Ctrl+letter as the shifted glyph).
- Bare uppercase normalizes to lowercase + SHIFT: `"N"` ≡ `"S-n"`.
- `A-` keeps the character as reported.

### Command vocabulary

`command_by_name(&str) -> Option<Command>` — one match, ~45 arms, snake_case
names for every command the default key tables expose:

- editing: `indent`, `insert_newline`, `backspace`, `delete`
- motions: `move.left`, `move.right`, `move.up`, `move.down`, `move.pageup`,
  `move.pagedown`, `move.word_next`, `move.word_prev`, `move.line_start`,
  `move.line_end`, and the same ten with `.extend`
- selection/objects: `begin_select_char`, `select_line`, `expand_line`,
  `jump_char`, `jump_char_back`, `object_prefix`
- clipboard/ops: `yank`, `paste_after`, `paste_before`, `paste`,
  `delete_selection`, `change`
- find: `find`, `find_next`, `find_prev`
- globals: `save`, `undo`, `redo`, `quit`, `suspend`, `close_buffer`
- sidebar: `open_sidebar_tree`, `open_sidebar_picker`, `resize_sidebar.grow`,
  `resize_sidebar.shrink`
- modes: `enter_action_mode`, `enter_edit_mode`

Not addressable (documented in README): commands whose argument comes from a
runtime capture (`jump_to_char`, `select_object`) and the prompt/sidebar
structural commands. The full name list ships in README next to the key tables.
An unknown name is a config error: `unknown command 'selct_line'`.

### Lookup order

```rust
pub fn lookup_with(keys: &KeyMap, mode: Mode, pending: Option<Pending>,
                   key: &KeyEvent) -> Option<Command>
```

1. If `pending` is set (f/F/object capture), the capture completes first — a
   binding can never hijack the captured character.
2. Consult the overlay (canonicalized incoming key → `KeyMap` lookup). A
   `"none"` binding stores `None` and lookup returns `Nop`: an unbound key
   does nothing instead of falling through to its default.
3. Otherwise the compiled default tables (`lookup` unchanged, still the
   defaults-only entry point the existing ~40 test call sites use).

`lookup_sidebar` / `lookup_prompt` never consult the overlay. `run()` routes
through `lookup_with(&app.config.keys, …)` for the main layer only.

Within the main layer, unbinding `Esc` (`enter_action_mode`) is allowed —
Ctrl chords keep plat usable, and README says so in one line.

---

## Folded follow-ups (from the Phase 1 ledger / PLAN #3 line)

1. `apply_resize()` (render.rs): clamp editor scroll against
   `width - sidebar_width`, not full width.
2. Tree arrows: `→` expands; `←` collapses an expanded dir, else moves the
   cursor to its parent dir. (Today both toggle.)
3. `Ctrl-R` in the sidebar: re-scan the tree. Cursor stays on the same path
   when it still exists, else lands on the nearest row. Re-scan failure reuses
   the existing `sidebar: <error>` status path.
4. Tree descends into symlinked directories; a visited set of canonicalized
   paths along the current branch prevents loops, `MAX_DEPTH = 20` stays, a
   `canonicalize()` failure increments the existing `skipped` counter.
5. Picker ranking: subsequence scoring — start-of-name and after `/ _ . -`
   beat mid-word, gaps penalized, shorter name wins ties. The substring check
   stays as the pre-filter; empty term still lists everything. ~40 lines.
6. `lookup_sidebar`'s global Ctrl list gains `f` → `Command::Find`: the find
   prompt owns the keyboard while the sidebar stays visible (existing
   prompt-before-sidebar ordering in `run()`), Esc returns to the sidebar.
7. `[sidebar] width` seeds `App::sidebar_width`; `[sidebar] ignore` patterns
   append to the built-in defaults and run through the existing
   gitignore-lite matcher (no compile step; a junk pattern simply never
   matches, same as today).
8. `[editor] indent_width` is what `Indent` inserts (hardcoded 2 today);
   `[editor] line_numbers` seeds the gutter; `[editor] key_debug` replaces the
   `PLAT_KEYDEBUG` env var, which is deleted along with `PLAT_LINENO`.

---

## Error handling

- Config failures happen **before the terminal is initialized**: bad TOML,
  unknown field, wrong type, bad key spec, duplicate spec, unknown command
  name → one stderr line
  (`plat: config error in <path>: line 7: unknown field 'line_number'`) + exit 1.
- Missing file → defaults silently. Existing-but-unreadable file → same
  startup failure (a half-read config is worse than none).
- Range nonsense clamps silently: `width` 10–50, `indent_width` 0–8. Type
  errors are hard errors; clamps are for cosmetic values only.

---

## Testing

Repo conventions: unit tests per module, decode-layer integration in
`tests/flow.rs`, a release pty smoke recorded in README.

- `config.rs`: the example file above parses end to end; `from_str`; missing
  file → defaults; `PLAT_CONFIG` wins over XDG; unknown field names the field
  and suggests the right one; wrong type reports the line; bad spec;
  both-spellings duplicate (`"N"` + `"S-n"`); unknown command name; width and
  indent clamps; ignore list merges with defaults.
- `keymap.rs`: overlay beats defaults; `"x" = "none"` produces no command (no
  fallthrough); pending capture not hijacked; canonicalization matches real
  events (`C-S-q`↔`C-q`, `"N"`↔shift+n, `"A-Left"`↔alt+left); prompt/sidebar
  lookups ignore the overlay.
- `app.rs`: `new_with_config` seeds gutter/width/indent; `indent_width = 3` →
  Tab inserts 3 spaces; `Ctrl-F` from the sidebar opens find and Esc returns
  to a still-open sidebar.
- `sidebar.rs`/`tree.rs`: `←` collapse-then-parent; `Ctrl-R` keeps the cursor
  by path (and degrades sanely when the path is gone); self-referencing
  symlink (`a/link → a`) terminates bounded; `fltr` ranks `filter.rs` above a
  far-away match.
- `render.rs`: `apply_resize` with the sidebar open clamps to width-minus-sidebar.
- `tests/flow.rs`: TOML text → `Config::from_str` → `lookup_with` →
  `handle_command` with real `KeyEvent`s, including an unbind.
- pty smoke (release, `PLAT_CONFIG` temp file): action-mode `x` remapped to
  `begin_select_char`, verified on a real terminal — plain letters encode
  identically everywhere, so the smoke avoids terminal-dependent chords.

Performance: config is read once at startup; the overlay is a BTreeMap lookup
per key event (negligible). No new render paths.

## Risks

1. Chord encoding varies by terminal (`C-Space`, alt-arrows): the diagnostic
   is `[editor] key_debug = true`, shipped in config rather than an env var.
2. Self-inflicted lockout by unbinding main-layer structural keys (Esc):
   allowed by design, one README line.
3. Two sources of truth for bindings (compiled defaults + overlay) — the
   accepted cost of approach A; `key_debug` is the inspector.

## Acceptance criteria

- `cargo test` green with no Phase 1 regressions (138 unit + 8 flow baseline).
- The example config above loads and every field visibly takes effect.
- A rebound chord and an unbind behave identically through the decoder tests
  and the pty smoke.
- `PLAT_LINENO`, `PLAT_KEYDEBUG` gone from code and README; `PLAT_PANIC` documented as a test hook.
- All 8 folded follow-ups covered by tests.
