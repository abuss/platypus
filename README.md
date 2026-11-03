# plat — platypus editor (Phase 0)

A terminal text editor where **edit mode is home**: typing always goes into
the buffer; **Esc** enters a slim action mode for moving, selecting, acting
on ranges, and searching. Selection-first — no vim grammar, arrow movement.

Spec: `docs/superpowers/specs/2026-11-01-plat-phase0-design.md`
Plan: `docs/superpowers/plans/2026-11-01-plat-phase0.md`

## Build & run

```sh
cargo build --release
./target/release/plat [files...]
```

Dev switches: `PLAT_PANIC=1 plat file` panics right after terminal init —
used to verify the panic hook restores the terminal (it prints the panic
with `isig/icanon/echo` re-enabled; checked under a pty).
`PLAT_KEYDEBUG=1 plat file` echoes each raw key event into the status bar —
for triaging what your terminal actually sends when a binding "doesn't work".
`PLAT_LINENO=0 plat file` disables line numbers (on by default).

Terminal paste (bracketed paste) inserts the pasted text at the cursor
verbatim, newlines and tabs intact, as a single undo step.

## Keymap (Phase 0)

### Global Ctrl (every mode)

| Key | Action |
|---|---|
| Ctrl-S | save |
| Ctrl-Z / Ctrl-Y | undo / redo |
| Ctrl-F | find |
| Ctrl-B / Ctrl-P | file sidebar: directory tree / fuzzy picker |
| Ctrl-Q | **quit** (dirty → confirm) |
| Ctrl-C / Ctrl-X / Ctrl-V | copy / cut / paste — CUA, works in every mode (copy needs a selection) |
| Ctrl-W | close buffer (dirty → confirm) |
| Ctrl-O | **suspend to shell** (run lazygit etc., then `fg`) |
| Ctrl-\ | reserved — not bound |

### Edit mode

typing · Enter · Tab (2 spaces) · Backspace/Delete · arrows/Home/End/PgUp/PgDn ·
Ctrl-A/Ctrl-E (line start/end). Everything else types.

### Action mode (Esc to enter; Esc cancels selection, Esc again → Edit)

| Keys | Action |
|---|---|
| Arrows, `w`/`b` | move; word jumps |
| `f`/`F` then char | jump to char on line |
| `Space` | toggle char under cursor |
| `Shift+arrows` | extend selection |
| `x` / `X` | select line / expand to full lines |
| `Alt+-` then `"` `'` `(` `[` `{` | select inside quotes/brackets |
| `y` / `p` / `P` | yank · paste after · paste before |
| `d` | delete selection (also copies) |
| `c` | change: delete selection, enter Edit |
| `/` | find (**regex**, live preview while typing); `n`/`N` cycle, wrapping at buffer edges; reports "no matches" |
| `u` / `U` | undo / redo |
| `:` | commands: `w [path]`, `q`, `wq`, `e path` |

Prompts own the keyboard: Enter commit · Esc cancel · Up/Down history ·
Tab cycles confirm choices (save/discard/cancel), Tab in the find prompt
toggles pattern ⇄ replacement.

## Search & replace

`/` opens the find prompt and matches underline **as you type** — the pattern
is a regex (`\d+`, `(foo|bar)`, `.`), and an incomplete regex never blanks the
highlights (it reports the error and keeps the last good ones). `n`/`N` cycle
through matches.

Press **Tab** to switch to the replacement field, type it, press **Enter**:
plat walks the matches one at a time, scrolling each into view:

| Key | Action |
|---|---|
| `y` | replace this match, move to the next |
| `n` | skip this match |
| `a` | replace this one and all the rest (one `Ctrl-Z` reverts the whole batch) |
| `e` | replace this one and stop |
| Esc | end (already-applied replaces stay, each was its own undo step) |

In the replacement, `$1` / `${name}` expand capture groups and `$$` is a
literal `$`; a reference to a group the pattern doesn't have is rejected with
an error instead of silently deleting text. Enter on the pattern field (or an
empty replacement) keeps the plain find behavior.

## File sidebar (Phase 1)

**Ctrl-B** opens the directory tree, **Ctrl-P** the fuzzy picker; both start in the
current directory. While the sidebar is open it owns the keyboard:

| Key | Action |
|---|---|
| ↑ / ↓, PgUp/PgDn, Home/End | move through the rows |
| ← / → | collapse / expand the selected directory |
| Enter | open the selected file (sidebar closes); on a dir, toggle it |
| type | filter the picker by name, then path (live, ranked) |
| Backspace | undo one filter character |
| Esc | close the sidebar without opening anything |
| Alt-← / Alt-→ | shrink / grow the sidebar (10–50 cols, kept for the session) |

Ctrl-S, Ctrl-Q, Ctrl-W, Ctrl-O, Ctrl-Z/Y still work with the sidebar open.
`:e path` still opens a file by name.

The scan skips `.git`, `node_modules`, `target`, `__pycache__` and `.egg-info`,
plus the rules in a root `.gitignore` (`*.ext`, `dir/`, plain names; no `!`
negation yet). Unreadable directories are skipped and reported in the status bar.

## Acceptance (Phase 0)

- 54 tests green (`cargo test`).
- 100k-line file: opens and renders in well under a second (pty smoke,
  release build, `total 0.698s` including pty overhead) — no wrap (Phase 1).
- Panic hook pty-verified: terminal state restored before panic output.
- Two-week daily-driver protocol with `PUNCHLIST.md` starts now; week-one
  irritations become the Phase 0.1 keymap tuning pass.

## Acceptance (Phase 1: file sidebar)

- 116 unit + 7 flow tests green (`cargo test`), no Phase 0 regressions.
- Sidebar pty smoke (release build, real keys): Ctrl-B lists the tree, `→`
  expands a dir, arrows walk into a child, Alt-← shrinks the sidebar and the
  editor follows, Esc closes without opening, Ctrl-P + typing filters live,
  Enter opens the hit, Ctrl-Q quits; `node_modules` never appears; the status
  bar names the open mode. Alt-arrows arrive from a real terminal as
  `Left/ALT` (keydebug-verified).
- Scan cost: 5,100-file tree builds in ~10 ms, one fuzzy filter ~5 ms
  (spec budget 100 ms). 100k-line file still renders in ~6 ms (sidebar closed).
- Skipped/deferred to Phase 1.1: tree refresh, config-file persistence of
  sidebar width, mouse, lazy/ threaded scan, `ignore` crate (real gitignore
  semantics incl. `!` negation), icons/colors.

## Acceptance (Phase 1: search & replace)

- 136 unit + 8 flow tests green (`cargo test`), no Phase 0 regressions.
- pty smoke (release build, real keys): `/` + regex underlines matches while
  typing, Tab moves to the replacement field, Enter starts the walkthrough,
  `y` then `a` produced the expected bytes on disk (`2026-\d\d` → `YYYY`,
  capture-free bulk rewrite), one Ctrl-Z reverted the `a` batch only, then a
  literal find still reported "1 match(es)" and Ctrl-Q confirmed the dirty
  buffer.
- Perf gate (100k lines, release, `cargo test --release -- --ignored`):
  `set_search` 9.7 ms, `replace_all` 54 ms, batch `undo` 31 ms — so live
  search keeps every keystroke inside one frame budget.
- Bug found by the smoke and fixed on the way: save-then-undo left `dirty`
  false, so Ctrl-Q exited silently discarding the undo. Undo/redo now mark the
  buffer dirty.
