# plat Themes (#6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make all plat colors configurable via named theme roles: built-in `dark`/`light`, external theme files, and `[theme]` config overrides.

**Architecture:** One new module `src/theme.rs` owns the color grammar, style-spec merge semantics, resolved `Palette` struct, and layered `resolve()`. `SyntaxState` gains a palette at build time; `render.rs` swaps five inline color sites plus new status-row/tab-row styling for palette field reads. Palette resolves once at `App::new_with_config` — caches can never go stale.

**Tech Stack:** Rust, ratatui `Style`/`Color`, serde + toml (already deps — no new ones).

**Spec:** `docs/superpowers/specs/2026-11-04-plat-themes-design.md`

## Global Constraints

- Colors parse from: names (`red`..`bright-white`, `gray`→LightGray, `dark-gray`→DarkGray, `white`/`bright-white`→White), `reset`, `#rrggbb`, integer 0..=255 → Ansi256.
- Built-in `dark` palette MUST equal today's colors verbatim (upgrade is visually a no-op; pinned by test).
- Layer precedence: dark base → built-in named delta (`light` only) → theme file → `[theme.ui]`/`[theme.syntax]`.
- Theme file dir: `themes/` sibling of the config file, i.e. `config_path().parent()` (refines spec's fixed path; keeps PLAT_CONFIG/XDG overrides working and testability).
- Config-TOML type errors stay fatal (existing `deny_unknown_fields` semantics); *value-level* theme problems (unknown name, bad theme-file TOML, bad color string, unknown `[ui]` key) are status notes, never fatal.
- No new dependencies. No live theme switching. Suite stays warning-free; final counts recorded in README + PLAN.

## Review Focus

- A bad color string in a theme file or in `[theme]` → that one key ignored (layer below survives) + one counted status note; it must not blank other colors and must not crash (Task 3).
- `theme = "foo"` with no file, or a file with bad TOML / unreadable → dark everything + one note (Task 3).
- User `[theme.syntax]` overrides must not break prefix hierarchy: overriding `function` must not re-color `function.method` (longest-prefix wins) (Task 3).
- The `dark` upgrade no-op: every previously-colored scope and chrome site renders byte-identically with default config (Task 3 pins scope table; Task 4 pins chrome cell styles).
- StyleSpec must NOT deserialize colors eagerly — a bad color would become a fatal config parse error otherwise; parse at resolve time so it degrades to a warning (Task 1/3).

---

### Task 1: Color grammar + style specs (`src/theme.rs` skeleton)

**Files:**
- Create: `src/theme.rs`
- Modify: `src/lib.rs` (add `pub mod theme;` after `pub mod syntax;`)
- Test: `src/theme.rs` `mod tests`

**Interfaces:**
- Consumes: nothing (leaf module).
- Produces:
  - `pub fn parse_color(s: &str) -> Result<Color, String>`
  - `#[derive(Clone, Debug, Deserialize)] #[serde(untagged)] pub enum StyleSpec { Color(String), Index(i64), Full(StyleTable) }`
  - `#[derive(Clone, Debug, Deserialize, Default)] pub struct StyleTable { #[serde(alias = "color", default)] pub fg: Option<String>, #[serde(default)] pub bg: Option<String>, #[serde(default)] pub bold: Option<bool>, #[serde(default)] pub italic: Option<bool>, #[serde(default)] pub underline: Option<bool> }`
  - `impl StyleSpec { pub fn apply(&self, base: Style, warn: &mut Vec<String>) -> Style }`
  - Colors stay as raw strings inside `StyleSpec` until `apply` (Review Focus 5).

- [ ] **Step 1: Failing tests** in `src/theme.rs mod tests`:
  - `parse_color_names`: `"red"→Color::Red` … all 8 base names, `bright-red`→LightRed … `bright-yellow`→LightYellow, `gray`/`grey`→LightGray, `dark-gray`→DarkGray, `white`→White, `bright-white`→White, `reset`→Color::Reset.
  - `parse_color_hex_and_index`: `"#ff00af"→Color::Rgb(255,0,175)`, `"240"→Color::Ansi256(240)`, `"0"→Ansi256(0)`, `"255"→Ansi256(255)`.
  - `parse_color_junk_errs`: `"notacolor"`, `"#gg0000"`, `"256"`, `"-1"`, `""` → `Err(_)` (message contains the offending text).
  - `style_spec_apply`: parse TOML fragments with `toml::from_str::<std::collections::BTreeMap<String, StyleSpec>>` — `k = "blue"` over a base of `Style::default().fg(DarkGray).italic()` keeps italic, replaces fg; `{ italic = false }` clears italic; `{ fg = "red", bg = "#010203", bold = true }` sets exactly those; bad fg string pushes exactly one warning containing the key context and returns base unchanged.
- [ ] **Step 2: Run** `cargo test theme:: 2>&1 | tail -5` — expect compile failure (module empty) → then implement.
- [ ] **Step 3: Implement** `src/theme.rs`: name table as a `match` on lowercased input; hex = 6 hex digits; integer strings 0..=255. `apply`: `Color(String)`/`Index(i64)` set fg only; `Full` replaces each present field (`Some(false)` → `rem_modifier`, `Some(true)` → `add_modifier`); every color parse failure pushes one `warn` string naming the value.
- [ ] **Step 4: Run** `cargo test theme:: 2>&1 | tail -5` — PASS.
- [ ] **Step 5: Commit** `git add src/theme.rs src/lib.rs && git commit -m "theme: color grammar + style-spec merge semantics"`

### Task 2: Config surface — `[editor] theme` + `[theme]` tables

**Files:**
- Modify: `src/config.rs` (`RawEditor`, `RawConfig`, `EditorCfg` + `Default`, `Config`, `from_str` apply step, `all_editor_fields` at :318)
- Modify: `src/theme.rs` (add `pub struct ThemeSection` — the serde container for `[theme]` and theme files)
- Test: `src/config.rs` `mod tests`

**Interfaces:**
- Consumes: Task 1's `StyleSpec`.
- Produces:
  - `ThemeSection` (in `src/theme.rs`): `#[derive(Clone, Debug, Deserialize, Default)] pub struct ThemeSection { #[serde(default)] pub ui: BTreeMap<String, StyleSpec>, #[serde(default)] pub syntax: BTreeMap<String, StyleSpec> }` — one serde container serving both the config `[theme]` table and (Task 3) theme files.
  - `EditorCfg.theme_name: String` (default `"dark"`), `Config.theme: theme::ThemeSection` (default empty), both readable by Task 3's `resolve`.

- [ ] **Step 1: Failing tests** in `src/config.rs mod tests` (follow existing `from_str` test style):
  - `theme_keys_parse`: `[editor]` `theme = "light"` → `cfg.editor.theme_name == "light"`; `[theme.ui]` `selection = "#ff0000"` and `[theme.syntax]` `keyword = { fg = "blue", bold = true }` → `cfg.theme.ui["selection"]` / `cfg.theme.syntax["keyword"]` are the right `StyleSpec` variants.
  - `theme_defaults`: empty config → `theme_name == "dark"`, both maps empty.
  - `theme_typo_in_editor_suggests`: `[editor] thme = "x"` → error message contains `theme` (extends `all_editor_fields`).
- [ ] **Step 2: Run** `cargo test config::tests::theme 2>&1 | tail -5` — FAIL (unknown fields).
- [ ] **Step 3: Implement:** add `ThemeSection` to `src/theme.rs`; `RawEditor.theme: Option<String>`; `RawConfig.theme: Option<ThemeSection>` with `#[serde(default)]`; apply in `from_str`; `all_editor_fields` += `"theme"`.
- [ ] **Step 4: Run** — PASS. Then `cargo test 2>&1 | grep "test result"` — full suite green (deny_unknown_fields elsewhere untouched).
- [ ] **Step 5: Commit** `git commit -am "config: [editor] theme name + [theme.ui]/[theme.syntax] override tables"`

### Task 3: `Palette`, layered `resolve`, built-ins

**Files:**
- Modify: `src/theme.rs`
- Test: `src/theme.rs mod tests`

**Interfaces:**
- Consumes: Task 2's `Config` fields; Task 1's `StyleSpec::apply`.
- Produces:
  - `#[derive(Clone, Debug)] pub struct Palette { pub status_bar, pub tab_active, pub tab_inactive, pub selection, pub completion_sel, pub sidebar_sel, pub find_match, pub diag_error, pub diag_warn, pub diag_info, pub diag_hint: Style, pub syntax: SyntaxPalette }`
  - `impl Palette { pub fn dark() -> Palette }` — UI roles = today's verbatim (`selection/completion_sel/sidebar_sel` bg Blue; `find_match` UNDERLINED; diag badges Red/Yellow/DarkGray/DarkGray; `status_bar`/`tab_*` `Style::default()`), syntax = the exact 13-entry table moved from `syntax.rs:36-49` (that table stays in syntax.rs until Task 4 — copy it; Task 4 deletes the original).
  - `pub fn light() -> Palette` = dark base with spec's light values applied as `StyleSpec`s.
  - `#[derive(Clone, Debug)] pub struct SyntaxPalette { entries: Vec<(String, Style)> }` + `pub fn style_for_scope(&self, scope: &str) -> Option<Style>` — entries sorted by key length DESC so longest prefix wins; same `starts_with` first-match loop as today.
  - `pub fn resolve(cfg: &Config) -> (Palette, Vec<String>)` — layers per Global Constraints; theme file path = `config::config_path().parent()` join `"themes"` join `format!("{name}.toml")`; file loaded for ANY named theme incl. `dark`/`light` (shadow); `[theme.ui]` unknown key vs the 11-role name set → note listing the key; every bad color → one counted note ("N invalid color value(s) in theme config"); unknown name → note `theme "x" not found`; file read/TOML error → note, base survives.

- [ ] **Step 1: Failing tests** (unit, `theme::tests`):
  - `dark_syntax_table_pins_today`: dark palette answers `keyword.function`→LightMagenta fg, `comment`→DarkGray+ITALIC, `function.method`→LightCyan, `variable`→None, `punctuation.bracket`→None (the exact assertions deleted from `syntax.rs` tests in Task 4 — copy them here now).
  - `longest_prefix_wins_under_overrides`: cfg `[theme.syntax] function = "red"` → `function.method` still LightCyan, `function` now Red.
  - `per_key_merge_inherits_base_style`: `[theme.syntax] comment = "blue"` → fg Blue, ITALIC kept (merged onto the base entry at the same key).
  - `unknown_theme_name_falls_back_with_note`: cfg theme_name `"nope"` (XDG_CONFIG_HOME → tmp with no themes dir) → palette == dark + exactly one note containing `"nope"`.
  - `file_shadows_builtin`: tmp XDG dir, `themes/light.toml` with `[syntax] keyword = "red"` → `resolve` keyword → Red + dark for everything else.
  - `bad_values_warn_not_fatal`: theme file with `[ui]` `selection = "nosuchcolor"` + `bogus_role = "red"` → selection stays dark-base bg Blue, notes mention `bogus_role` and count 1 invalid color; everything else still applied.
  - `light_palette_spot`: fg of `status_bar` == Black, bg == Gray; `tab_active` has BOLD; `selection` bg == LightGray.
- [ ] **Step 2: Run** `cargo test theme:: 2>&1 | tail -5` — FAIL.
- [ ] **Step 3: Implement** `Palette`/`SyntaxPalette`/`resolve` + merge helper `merge_syntax(base: BTreeMap<String, Style>, over: &BTreeMap<String, StyleSpec>, warn) -> SyntaxPalette` (per-key: `StyleSpec.apply` onto the base entry at the SAME key if present, else onto `Style::default()`).
- [ ] **Step 4: Run** — PASS; full suite green.
- [ ] **Step 5: Commit** `git commit -am "theme: resolved Palette with layered resolve(); built-in dark (verbatim pin) + light"`

### Task 4: Wire palette into App, syntax, render

**Files:**
- Modify: `src/app.rs` (App struct field + `new_with_config` + `ensure_syntax` :915), `src/syntax.rs` (delete `style_for_scope`, `SyntaxState::build(lang, text, palette)`, store `SyntaxPalette`, use in refresh :275; move the 4 scope tests to theme.rs assertions from Task 3), `src/document.rs` tests (:794,:807 pass `&Palette::dark()`), `src/render.rs` (5 sites + status row + tab labels)
- Test: `src/render.rs mod tests` (TestBackend cell-style asserts)

**Interfaces:**
- Consumes: Task 3 `Palette`, `resolve`.
- Produces: `App.palette: Palette` (public — flow tests read it); `SyntaxState::build(lang: &'static str, text: &str, palette: &Palette) -> Option<SyntaxState>`.

- [ ] **Step 1: Failing render tests** in `src/render.rs mod tests` (pattern: existing tab_row test at :861 + TestBackend):
  - `dark_chrome_unchanged`: default-config app with a selection → selected cell bg == Blue (exact today-behavior pin).
  - `light_theme_recolores`: app built via `Config` from `config::from_str("[editor]\ntheme=\"light\"")` + `new_with_config` → selected cell bg == LightGray; status-row first cell style == palette.status_bar (fg Black bg Gray).
  - `config_override_cell`: `[theme.ui]` `selection = "#ff0000"` → selected cell bg == `Color::Rgb(255,0,0)`.
  - `tab_active_styled`: two docs, light theme → active label cell has BOLD, inactive cell does not.
- [ ] **Step 2: Run** `cargo test render::tests 2>&1 | tail -5` — FAIL.
- [ ] **Step 3: Implement wiring** — App field + `let (palette, notes) = theme::resolve(&config); if !notes.is_empty() { status = Some((notes.join("; "), true)); }` in `new_with_config`; `build(lang, &text, &self.palette)` in `ensure_syntax`; syntax.rs internal swap (tests there updated to `&Palette::dark()`); render.rs: diag badge match arms read `palette.diag_*` fg, selection/completion/sidebar cells take the palette Style, find-match uses `palette.find_match`, after writing the status row set its style to `palette.status_bar` for the full row width, tab labels styled per active/inactive (the `[…]`/`+` characters unchanged).
- [ ] **Step 4: Run** full `cargo test 2>&1 | grep -E "test result|warning"` — all green, warning-free. Old syntax-table tests in syntax.rs now live in theme.rs (Task 3) — delete originals, assert counts didn't silently drop coverage (net ≥ −1).
- [ ] **Step 5: Commit** `git commit -am "theme: palette resolved at startup; syntax + all chrome render sites read it"`

### Task 5: End-to-end, smoke, docs

**Files:**
- Test: `tests/flow.rs`
- Modify: `README.md` (Theme section: name, file dir, role table, color grammar, overrides), `PLAN.md` (#6 shipped marker + test counts)
- Create (throwaway harness, not committed): `/tmp/opencode/theme_smoke.py`

- [ ] **Step 1: Failing flow test** in `tests/flow.rs`: write a real theme file at `XDG-configured tmp/themes/e2e.toml` (`[syntax] keyword = "red"`, `[ui] selection = "#00ff00"`), config with `theme = "e2e"` + `[theme.ui] completion_sel = "magenta"`, render a buffer with `fn` + a selection + open completion → assert keyword span cell fg Red, selection cell bg Rgb(0,255,0), menu-selected bg Magenta. Run → FAIL (until Task 4 already passes it — if green immediately, that's the acceptance pin: still commit the test).
- [ ] **Step 2: pty smoke** (release build): `python3 /tmp/opencode/theme_smoke.py` — run `./target/release/plat` with XDG tmp + theme file; assert SGR codes for one ui color (`48;2;0;255;0`) and syntax Red fg in raw stream. Record the transcript excerpt in README's acceptance list.
- [ ] **Step 3:** README "Theme" section + acceptance bullets; `PLAN.md` #6 marked shipped with counts; run full suite + existing benches once to confirm perf baseline untouched (startup-only cost, no keystroke path changes) — record counts.
- [ ] **Step 4: Commit** `git commit -am "docs+e2e: themes shipped — flow test, pty smoke, README/PLAN"`

---

Self-review note (kept inline for executors): spec's themes-dir wording (`~/.config/platypus/themes`) is implemented as *sibling of the resolved config file* so PLAT_CONFIG/XDG test isolation works; under default config these are the same directory.
