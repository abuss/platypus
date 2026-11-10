# PUNCHLIST — keymap/ergonomics irritations from daily use

Log format: date · what hurt · suggested change. Week one feeds Phase 0.1.

- Paste introduces a newline at the beginning of the text that is going to be pasted
  → FIXED (c213347): bracketed paste was never enabled, so the terminal *typed* pasted text one key at a time
    (newlines swallowed, tabs→indent, one undo per char). The leading newline was your clipboard selection's
    own newline, typed literally. plat now receives the whole text as one Event::Paste, inserts it verbatim
    (CRLF→LF), single undo.
- Find is not working
  → FIXED (61204d2 + 8442eb3): two causes. (1) search was forward-from-cursor, no wrap, and said nothing on
    zero matches — anything above/left of the cursor looked broken. Now wraps at edges, shows "no matches for 'x'".
    (2) uppercase letters never made it into the find prompt *or* the buffer on real terminals (crossterm tags
    uppercase with SHIFT; Edit guard demanded zero modifiers).
- `:` not work is it used more than once
  → NOT REPRODUCED headless: `:w` twice saves twice, byte-identical keystrokes. Prime suspects: your terminal's
    key encoding (modifier flags on `:`) or mode state — after `:e path`/Ctrl-P plat is in **Edit** mode, so the
    next `:` *types* a colon (by design: edit mode is home; press Esc first). Evidence step: run
    `PLAT_KEYDEBUG=1 plat <file>`, press `:` when it misbehaves, and report the status-bar readout.

## Investigated — false alarms (keep the install fresh!)

- PgUp/PgDn "moves the editor, not the terminal" (issue #29)
  → NOT A BUG: the `~/.local/bin/plat` on PATH predated the scrollback feature (PR #25); the old binary
    deliberately routed PgUp/PgDn to the editor's page motion. The current build passes the scrollback
    smoke and a screen-reconstruction row check (`term_scrollback_smoke.py`, `scr_repro.py`) — installed
    `~/.local/bin/plat` was refreshed after this investigation. **After every merged PR, reinstall the
    release binary** or you will debug ghosts.
  → The follow-up "completely scrambled display" was ALSO not the feature: it was `eprintln!` debug
    writes landing on the alternate screen, which corrupt ratatui's diff repaint. For TUI debugging use
    a file (e.g. a `PLAT_DRAWDBG=/path` env var), never stderr.
 
