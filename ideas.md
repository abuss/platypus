# Platypus - ew Terminal Text Editor: Feature Summary

A personal, daily-driver terminal text editor. Typing always goes straight
into the buffer; **Esc** enters a slim *select (action) mode* for moving, selecting,
searching and commands — no vim grammar, no movement with hjkl.
The idea is to have an editor that is a mix of two worlds, normal editing
(Fresh, VS Code) with a modal editor (Helix, Neovim), implementing elements of
each that make the interaction more streamlined: the simplicity of normal
editors with the quick interaction of modal editors, without too many features.

## Core Identity

**Name:** platypus
**Short name:** ptp 
**Target:** Terminal/TUI-based text editor combining **modal flexibility** with **non-modal convenience**  
**Design Philosophy:** Helix's power + Fresh's ergonomics + minimal dependencies  
**Base Language:** **Rust** (recommended) | Go (good alternative) | Nim (middle ground)  
**Plugin Language:** **Lua** (with mlua/rlua embedding)

---

## Core Editor Features

| Feature | Details |
|---------|---------|
| **Editing Mode Toggle** | Start in **non-modal (edit mode permanent)** or switch to **modal** on demand (Escape key) — user choice, no hjkl conflicts (arrow keys for movement) |
| **Two Modes** | Normal **edit** (insert) mode and **action** mode |
| **Text Buffers** | Efficient rope-based text structure (Ropey library) for large files, with support for multiple buffers |
| **Undo/Redo** | Full history with branching support |
| **Line/Column Numbers** | Configurable display; relative line numbers optional |
| **Word Wrap** | Toggle on/off; soft wrap by default |
| **Search & Replace** | Regex support; live preview of replacements |
| **Bookmarks** | Mark lines for quick navigation |

---

## File Navigation & Management

| Feature | Details |
|---------|---------|
| **Tree Browser** | **Fresh-style sidebar** showing directory structure; collapsible nodes; mouse & keyboard navigation |
| **File Picker** | **Helix/fzf-style** fuzzy finder for fast file access; Ctrl+P to open a sidebar with the file picker |
| **Breadcrumb Trail** | Show current file path in header; click to navigate parent directories |
| **Recent Files** | Quick access to previously opened files |
| **Sidebar Toggle** | Hide/show tree browser to maximize editing space |
| **Smart Scrolling** | Cursor stays centered; smooth scrolling on large files |

---

## Syntax & Language Support

| Feature | Details |
|---------|---------|
| **Syntax Highlighting** | Tree-sitter integration for 100+ languages; accurate parsing |
| **Language Server Protocol (LSP)** | Real-time diagnostics, autocompletion, goto-definition, rename |
| **Built-in Languages** | Rust, Python, JavaScript, TypeScript, Go, C/C++, Lua, Nim, TOML, JSON, YAML, Markdown |
| **Theme System** | Light/dark themes; customizable colors via config |
| **Bracket Matching** | Highlight pairs; smart indentation |
| **Folding** | Code folding by syntax regions; manual markers |
| **Comment Toggle** | Ctrl+/ to comment/uncomment selection |

---

## Git Integration

| Feature | Details |
|---------|---------|
| **Diff View** | Show file changes; side-by-side or unified diff |
| **Stage/Unstage** | UI for git add/remove from sidebar |
| **Status Bar Info** | Current branch, file status (modified, untracked, staged) |
| **Blame View** | Show commit info per line |
| **Commit UI** | Create commits with message editor built-in |
| **Merge Conflict Highlighting** | Visual markers for conflict markers; conflict resolution UI |

---

## Advanced Editing Features

| Feature | Details |
|---------|---------|
| **Multi-Selection** | Multiple cursors for simultaneous edits across lines/columns |
| **Multi-Cursor Editing** | Select multiple regions; edit all simultaneously |
| **Macro Recording** | Record and replay key sequences (modal mode) |
| **Column Selection** | Block select with Alt+Click or Shift+Alt+Arrows |
| **Auto-Indent** | Smart indentation based on syntax |
| **Format on Save** | Optional integration with prettier, rustfmt, etc. (LSP-based) |
| **Snippet Support** | Snippet engine for code templates; Tab to expand, Shift+Tab for placeholders |
| **Join/Split Lines** | Merge/split lines with Ctrl+J / Ctrl+L |

---

## UI & UX

| Feature | Details |
|---------|---------|
| **Status Bar** | Bottom bar showing mode, file, cursor position, git info, lsp status/info |
| **Command Palette** | Ctrl+Shift+P to search all commands (Overlay) |
| **Picker UI** | Fuzzy search for files, commands, symbols, recent files |
| **Notifications** | Inline messages for errors, warnings, info |
| **Mouse Support** | Click to position cursor; scroll; click sidebar items; context menus |
| **Resizable Panes** | Drag dividers to resize tree browser, editor, output |
| **Full Keyboard Navigation** | All features accessible via keyboard; no mouse required |
| **Configurable Keybindings** | User-defined shortcuts via config file |
| **Terminal Panel** | Embedded terminal for running commands/builds |

---

## Plugin System

| Feature | Details |
|---------|---------|
| **Language** | **Lua** (simple, proven, embedded via mlua) |
| **Plugin Discovery** | Auto-load from `~/.config/myeditor/plugins/*.lua` |
| **Plugin API** | Exposed editor operations: open/close files, get/set buffer, register commands, show pickers |
| **Command Registration** | Plugins register custom commands (accessible via Ctrl+Shift+P) |
| **Event Hooks** | on_file_open, on_save, on_selection_change, on_command, etc. |
| **Buffer Manipulation** | Full access to text buffers; trigger LSP, syntax highlighting |
| **UI Integration** | Show notifications, set status, open pickers from plugins |
| **Package Manager** | Simple: fetch plugins from GitHub repos; optional registry |
| **Hot Reload** | Reload plugins on config change (development mode) |

---

## Configuration

| Feature | Details |
|---------|---------|
| **Format** | TOML (like Helix) or Lua (like Neovim); recommend **TOML for simplicity** |
| **Config Location** | `~/.config/myeditor/config.toml` |
| **Plugin Config** | Separate `~/.config/myeditor/plugins/` directory |
| **Themes** | Bundled themes (One Dark, GitHub Light, Dracula, etc.); user themes in config |
| **Font Size** | Terminal font size (delegated to terminal emulator) |
| **Key Bindings** | Remappable via config file |
| **Default Settings** | ~50 lines of TOML for full customization (lightweight philosophy) |

---

## Terminal & Platform Support

| Feature | Details |
|---------|---------|
| **Cross-Platform** | Linux, macOS, Windows (via Windows Terminal or WSL) |
| **Terminal Emulator Agnostic** | Works in any ANSI-compliant terminal (xterm, GNOME Terminal, iTerm2, etc.) |
| **Color Support** | 256 colors + true color (24-bit RGB) if terminal supports it |
| **Unicode** | Full UTF-8 support; proper handling of multi-byte characters |
| **Mouse Events** | Click, scroll, drag in any terminal that supports it |
| **Terminal Size Handling** | Responsive to window resize; no crashes on small terminals |

---

## Optional Advanced Features (Phase 2)

| Feature | Details | Inspiration |
|---------|---------|-------------|
| **Agent Integration** | Run Claude, Copilot, or local LLM for code suggestions | Fresh |
| **Orchestrator Dock** | Manage background tasks, AI agents, build processes | Fresh |
| **Remote Editing** | SSH, Kubernetes, dev containers support | Fresh |
| **Workspace Management** | Multi-folder workspaces; project-level settings | Fresh |
| **Output Panel** | Show build/lint/test output; navigate errors | Common |
| **Debug Integration** | DAP (Debug Adapter Protocol) for stepping through code | Helix |
| **Split Windows** | Horizontal/vertical splits for side-by-side editing | Common |

---

## Non-Features (Intentional Omissions)

| Feature | Reason |
|---------|--------|
| **VimScript or Emacs Lisp** | Lua is simpler; no need for legacy scripting languages |
| **Built-in Terminal** | Out of scope; users have better terminals available |
| **Graphical UI** | TUI-focused; no Electron/Qt/GTK burden |
| **Heavy AI Integration** | Start minimal; plugins can add this later |
| **Porting Vim Plugins** | Fresh ecosystem; Lua-based plugins are simpler |

---

## Language Recommendation Summary

| Language | Best Fit | Why |
|----------|----------|-----|
| **Rust** | **Primary choice** | Performance of Helix; memory safety; Crossterm/Ratatui ecosystem proven; mlua embedding trivial |
| **Go** | **Good alternative** | Faster development; tcell/bubble tea TUI libs; acceptable performance; simpler than Rust |
| **Nim** | **Experimental option** | Python-like syntax; C backend performance; smaller ecosystem; fewer reference editors |

**Recommendation: Start with Rust.** You get:
- Proven TUI libraries (Crossterm, Ratatui, Ropey)
- Helix as a reference for architecture
- Lua embedding is zero-friction (mlua crate)
- Long-term maintainability and performance

---

## Development Roadmap (Phases)

### Phase 1: MVP (Core Editor)
- Modal/non-modal toggle
- Tree browser + file picker
- Syntax highlighting (Tree-sitter)
- Basic LSP support
- Search & replace
- Git diff view
- TOML config

### Phase 2: Polish & Plugins
- Lua plugin system
- Git integration (stage/unstage/commit)
- Multi-cursor editing
- Snippet engine
- Theme system
- Command palette
- Mouse support refinement

### Phase 3: Advanced
- Merge conflict resolution
- Blame view
- Remote editing (SSH)
- Workspace management
- Optional: agent integration

---

## Success Criteria

✓ **Fast:** Responsive to 100K+ line files; instant startup  
✓ **Hybrid:** Seamless modal/non-modal switching  
✓ **Extensible:** Lua plugins for custom workflows  
✓ **Batteries-included:** Git, LSP, syntax highlighting, multi-cursor out of the box  
✓ **Lightweight:** <50 MB binary; minimal config needed  
✓ **Proven:** Rust core with battle-tested TUI ecosystem

---

