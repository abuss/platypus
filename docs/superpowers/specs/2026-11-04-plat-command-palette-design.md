# Command palette (#10) — design

## Problem

plat's actions are discoverable only through the README keymap table or
`key_debug`. Helix's screenshot menu (which-key popup) inspired this, but
plat has no leader-key namespace — the fitting equivalent is a searchable
**command palette**: one overlay that lists the user-facing commands, their
descriptions, and their current key bindings, and runs the picked command
exactly as if its key had been pressed.

## Success criteria

- `C-t` (Action mode, default; remappable) opens a centered overlay with a
  filter line and the command list; typing filters live; Enter runs; Esc
  closes.
- Every executed command goes through the existing keypress command path —
  no parallel action implementations, no behavioral forks.
- Descriptions and key hints are always current with the live (remapped)
  keymap.
- Default dark rendering is additive only: nothing outside the popup and
  its role changes.

## Interaction

- **Open:** `Command::CommandPalette` (new variant), built-in bind `C-t` in
  Action mode (0x14 arrives cleanly; `C-Space` stays `select_line`).
  Remappable via `[keys]` under name `command_palette`; `"none"` unbinds it.
  Opens only from the normal key-routing path: not while a prompt, a pending
  capture, the sidebar in picker-edit, or the palette itself is active.
  Opening closes any open completion menu (`completion = None`).
- **Filter:** printable chars append to a filter string; Backspace deletes
  one char; the list re-filters on every change using the existing fuzzy
  scorer (`subsequence_score` from `picker.rs`, promoted to `pub(crate)` —
  same word-boundary bonuses and gap penalty as the file picker).
  Empty filter shows all items in table order.
- **Select:** Up/Down move the selection (wrap-around, like the completion
  menu). Scrolled view when list exceeds popup height.
- **Run:** Enter closes the palette **first**, then dispatches the selected
  `Command` through the same executor a keypress uses — including
  palette-adjacent meta-commands (`command_mode` opens the `:>` prompt,
  `quit` still honors the unsaved-changes confirm).
  Enter with an empty match list does nothing (no close, no run).
- **Cancel:** Esc closes and restores the editor exactly (no status note).

## Command registry

`src/keymap.rs` gains:

```rust
pub struct PaletteItem {
    pub cmd: Command,
    pub name: &'static str,   // filterable id, matches the [keys] command name
    pub desc: &'static str,   // one-line human description
}
pub const PALETTE_ITEMS: &[PaletteItem] = &[ ... ];
```

V1 curated list (18 entries; editing primitives, motions, and selection ops
are deliberately absent):

| name | Command | desc |
|---|---|---|
| save | Save | save current buffer |
| save_as | SaveAsPrompt (new variant) | save with a new file name |
| close_buffer | CloseBuffer | close active buffer |
| next_buffer | NextBuffer | switch to next buffer |
| prev_buffer | PrevBuffer | switch to previous buffer |
| undo | Undo | undo last edit |
| redo | Redo | redo last undone edit |
| find | Find | search (regex, live preview) |
| open_file | OpenSidebarPicker | fuzzy file picker |
| file_tree | OpenSidebarTree | browse project files |
| goto_definition | GotoDef | jump to definition (LSP) |
| jump_back | JumpBack | jump back in nav stack |
| next_diagnostic | NextDiagnostic | next LSP diagnostic |
| prev_diagnostic | PrevDiagnostic | previous LSP diagnostic |
| yank | Yank | copy selection (or line) |
| paste | Paste | paste at cursor |
| command_mode | CommandPrefix | colon commands (:q :w :wq :w …) |
| quit | Quit | save prompt if needed, then exit |

(`suspend` excluded from the list — leaving plat is via `C-o` or `quit`;
a palette that hides too much vs the README table would be worse.)

`SaveAsPrompt`: new `Command` variant that opens the existing
`PromptKind::SaveAs`; also registered in `command_by_name` as `save_as` so
it is remappable.

`command_by_name` must resolve `command_palette` → `CommandPalette`.

## Key hints

Each row shows the key(s) currently bound to that exact `Command`:
reverse-scan the built-in default table **and** the user `[keys]` overlay,
rendered via a new `spec_label(KeySpec) -> String` (inverse of
`parse_key_spec`: `C-t`, `A-h`, `S-Tab`, …). A command bound by nothing
shows no hint. All curated palette commands are payload-free variants, so
`Command` equality is the join — no ordering/payload complications.

## Rendering & theme

- Centered boxed popup (box-drawing border), sized to the longest row and
  capped at 80 % of screen width/height with min sane clamps; the editor
  stays visible behind it (like the completion menu).
- Rows: `name` (padded), `desc` (truncated at the width cap), right-aligned
  key hint. Filter line on top: `> term`.
- New theme role **`palette_sel`** (role count 14 → 15): selected row style;
  built-ins dark AND light = `bg Blue` (same base as `completion_sel`, since
  each real paint site gets its own role). Overridable via
  `[theme.ui] palette_sel = …`; unknown-role warning machinery covers it.
- Border/text: `default`-role base (fg/bg) inside the popup, matching how
  the popup text should read under custom themes.

## Error policy & edges

- Palette open while the window is tiny (e.g. 5×5): popup clamps to screen,
  never panics, never indexes out of bounds (pattern: completion-menu clamps).
- Filter with no matches: list area empty; Enter is a no-op; Esc still closes.
- Unicode in descriptions only — all names/descs are ASCII in V1.
- Unbinding `command_palette` (`"none"`) leaves C-t inert: no fallback to a
  different command (overlay-`none` semantics, existing rule).

## Testing

- keymap: `C-t` built-in lookup returns `CommandPalette` (Action mode);
  `[keys] command_palette = "C-p"` moves it, `"none"` unbinds (Nop);
  `command_by_name("command_palette")` and `("save_as")` resolve;
  `spec_label` round-trips every `parse_key_spec` test vector.
- app/flow: open palette → type `sav` → first row is `save`; Enter saves the
  buffer (dirty clears); Esc restores mode state; a remapped `:>` via
  `command_mode` works; key-hint column reflects a user remap.
- render: popup paints with border; selected row carries `palette_sel`
  (built-in Blue in dark; `[theme.ui] palette_sel = { bg = "#ff0000" }`
  overrides); narrow terminal does not panic.
- theme: `palette_sel` known role, no notes, dark/light built-in pinned.
- Suite stays green + warning-free; existing keystroke perf gates unaffected
  (one more built-in lookup arm; the modal path costs keystrokes nothing).

## Acceptance (to add to README at ship)

- Flow test name + pty smoke: real binary, `C-t` opens the popup (grep the
  stream for the popup title row + a command name + the palette_sel SGR),
  typing `save` filters, Enter saves. Counts + warning-free claim.

## Out of scope (deliberate)

- History of palette picks, fuzzy score pinning, mouse clicks on rows
  (→ mouse #11), which-key prefix popups (no leader key in plat),
  searchable settings, icons/emoji in rows, aliasing (`:w`-style abbrevs).
