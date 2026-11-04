# plat Themes (#6) — design

## Objective

Make every color plat paints configurable: the tree-sitter syntax palette
(the seam `style_for_scope` was built for) and all editor chrome. Themes are
external files (`~/.config/platypus/themes/<name>.toml`) selected by name,
individually overridable from config.toml. Two built-ins: `dark` (today's
palette verbatim — the upgrade is visually a no-op) and `light`.

Agreed during brainstorming (2026-11-04): external theme files + overrides;
syntax + all chrome in scope; built-ins dark+light with dark as default and
fallback; color syntax = names + hex + 256-index.

## Config surface

- `[editor] theme = "name"` — default `"dark"`. `theme = "dark"`/`"light"`
  use built-ins; any other name loads `~/.config/platypus/themes/<name>.toml`.
  A themes-dir file named `dark` or `light` **shadows** the built-in.
- `[theme.ui]` and `[theme.syntax]` tables in config.toml — same shape as a
  theme file's `[ui]`/`[syntax]` tables; applied as the outermost override
  layer, so you can keep a shared theme file and tweak it locally.

Theme file shape:

```toml
# ~/.config/platypus/themes/monokai.toml — every key optional, deltas only
[ui]
selection = "#4a4a4a"                       # bare string = fg
status_bar = { fg = "#f8f8f2", bg = "#272822" }
diag_warn = { color = "#fd971f" }           # `color` is an alias of `fg`

[syntax]
keyword = "magenta"
function = "#a6e22e"
comment = { fg = "#75715e", italic = true }
```

All themes (file or none) **layer on top of the built-in dark base**: a theme
file only lists deltas; unlisted keys keep dark values. Resolution:
`dark base → theme file (if named ≠ dark, or shadowing) → [theme.ui]/[theme.syntax]`.

## Value grammar

A *style spec* is either

- a bare string → sets **fg** only (everything else inherited), or
- a table with any of: `fg` (or alias `color`), `bg`, `bold`, `italic`,
  `underline`. Present fields replace; absent fields inherit from the layer
  below. `false` on a modifier explicitly clears it.

A *color string* parses to `ratatui::style::Color`:

| form | example | maps to |
|---|---|---|
| name | `"red"` | Color::Red |
| bright name | `"bright-cyan"` | Color::LightCyan |
| grays | `"gray"`→LightGray, `"dark-gray"`→DarkGray, `"white"`/`"bright-white"`→White | |
| reset | `"reset"` | Color::Reset (terminal default) |
| hex | `"#ff00af"` | Color::Rgb |
| 256 index | `"240"` (also bare TOML integer `240`) | Color::Ansi256 |

`[theme.syntax]` scope names are open-ended (tree-sitter namespace; prefix
hierarchy applies exactly like `style_for_scope` today — `keyword` covers
`keyword.function`). Unknown `[ui]` keys are a **warning** (finite known set,
typo protection); unknown `[syntax]` scopes are **legal** (can't enumerate
every grammar's scopes) — a typo is simply a dead entry, documented.

## Architecture

New `src/theme.rs`:

- `parse_color(&str) -> Result<Color, String>` — the grammar above.
- `StyleSpec` (deserializable from string-or-table) → merges onto a base
  `Style`: `apply(self, base: Style) -> Style`.
- `ThemeFile` = `{ ui: BTreeMap<String, StyleSpec>, syntax: BTreeMap<String,
  StyleSpec> }` — one serde struct serving BOTH theme files and the config
  `[theme]` subtree (single parser, single validation path).
- `Palette` — resolved, compile-time-named UI roles + the syntax layer:

  ```rust
  pub struct Palette {
      pub status_bar: Style,
      pub selection: Style,
      pub completion_sel: Style,
      pub sidebar_sel: Style,
      pub find_match: Style,
      pub diag_error: Style,
      pub diag_warn: Style,
      pub diag_info: Style,
      pub diag_hint: Style,
      syntax: SyntaxPalette, // merged scope→Style table, prefix lookup
  }
  ```

- `resolve(cfg: &Config) -> (Palette, Vec<String> warnings)`.

Wiring (every existing choke point, no new plumbing paths):

- `Config` gains `theme: ThemeFile` (the raw `[theme]` tables) and
  `EditorCfg.theme_name: String`. `App::new` resolves once → `App.palette:
  Palette` + startup status notes (same channel as existing config warnings).
- `syntax.rs`: `style_for_scope(scope)` becomes a method on `SyntaxPalette`
  (pure, testable, same prefix semantics); `SyntaxState::refresh(...)` gains
  a `&Palette` parameter (single call site, `app.rs:972`).
- `render.rs`: the five color sites read palette fields — diag gutter badges
  (:179–186), selection bg (:236), completion menu selected row (:259),
  find-match highlight (:298, a *style* incl. UNDERLINED default), sidebar
  cursor row (:503). The status bar row gains a style application (today it
  is plain default text) — the one genuinely new-colorable surface.
- Caches: spans store resolved `Style`s; the palette is fixed before the
  first parse ever runs, so **no generation/invalidation is needed**
  (the "generation bump" floated in the design chat is unnecessary —
  revisit only if live theme switching is ever added).

### Roles (grounded in what actually paints today)

| role | today |
|---|---|
| `status_bar` | plain default text (new: stylable) |
| `selection` | bg Blue |
| `completion_sel` | bg Blue |
| `sidebar_sel` | bg Blue |
| `find_match` | UNDERLINED modifier |
| `diag_error/warn/info/hint` | Red / Yellow / DarkGray / DarkGray |

No `current_line` role: plat has no current-line highlight; inventing one is
out of scope. Tab row, prompts, and the file tree paint with default styles —
not themed in v1 (they'd be *new* styling, not a palette swap).

## Built-in palettes

`dark` = current values verbatim, incl. syntax table (LightMagenta keywords,
LightCyan functions, Yellow types, Green strings/chars, LightBlue
numbers/constants, DarkGray-italic comments, LightYellow properties/fields,
Red error) and all chrome above. **Pinned by a test** so the retrofit can't
shift it.

`light` (exact built-in; tuning later is allowed, drift is not):

- syntax: keyword/function `blue`, type `dark-cyan`, string/char `green`,
  number/constant `magenta`, comment `{ fg = "dark-gray", italic = true }`,
  property/field `dark-yellow`, error `red`.
- ui: status_bar `{ fg = "black", bg = "gray" }`, selection/completion_sel/
  sidebar_sel `bg = "light-gray"` (fg reset), find_match inherited underline,
  diag_error `red`, diag_warn `dark-yellow`, info/hint `dark-gray`.

## Errors & fallbacks

| situation | behavior |
|---|---|
| `theme = "foo"`, no such file | dark base + status note `theme "foo" not found` |
| theme file unreadable / bad TOML | dark base + note; `[theme]` overrides still apply |
| bad color value (any layer) | that key ignored (layer below survives) + note with count |
| unknown `[ui]` key | ignored + note listing the key |
| unknown `[syntax]` scope | legal, no warning (open namespace) |

Notes reuse the existing startup config-warning channel; nothing LSP- or
edit-path-blocking; all decisions at startup.

## Testing

- `parse_color` exhaustive table test (all names, hex, ints, reset, junk).
- Merge semantics: string-vs-table, field inheritance, `false` clears, layer
  precedence (base ⊕ file ⊕ config), shadowing built-in by file.
- `resolve`: unknown name → dark + warning; bad color → below-layer survives +
  counted note.
- `SyntaxPalette::style_for_scope` with overrides + prefix coverage
  (extends the existing four pinned assertions; the dark-palette-equal-to-
  today test pins the no-op upgrade).
- flow test: config `[editor] theme` + `[theme.ui]` override → rendered status
  row / selection carries the exact SGR codes for the chosen color; one
  syntax span with an overridden scope color.
- pty smoke (release build): a real themes-dir file + config override, assert
  SGR sequences for one ui + one syntax color.

No perf gates: resolution happens once at startup; the render loop swaps
inline constants for field reads. Perf baseline must stay green anyway
(`cargo test` + existing benches).

## Explicitly out

- Live theme switching (`:theme` command) — restart applies, like all config.
- Theme inheritance (`extends =`), per-language theme overrides, `[[themes]]`
  auto-detection by file type.
- Styling of tab row, prompt, file tree (no colors today → not a palette
  swap).
- `current_line` highlight feature itself.
