# 18 — Terminal Text Editing for the Input Box

> **Date:** 2026-03-14
> **Summary:** The current input box has minimal editing support (character movement, backspace, newlines). Two viable paths exist: (1) adopt `ratatui-textarea` v0.8.0 (the official ratatui org fork, ratatui 0.30 compatible) for full emacs-style keybindings, or (2) add ~115 lines of custom readline keybindings to the existing handler. Both paths are analyzed with trade-offs. The original `tui-textarea` v0.7.0 is **abandoned and incompatible** with our ratatui 0.30.

---

## 1. Executive Summary

The input box in `src/tui/input.rs` is a hand-rolled editor with only character-level movement and single-character deletion. Users expect readline-style editing: Ctrl-W to delete words, Alt-B/F to jump by word, Ctrl-K/U to kill lines, Ctrl-Y to yank, and undo/redo. Nine Rust crates were evaluated.

**Two recommended paths:**

1. **`ratatui-textarea` v0.8.0** (the official ratatui org fork by Orhun Parmaksiz, released Feb 2026) — ratatui 0.30 compatible, complete emacs keybindings, undo/redo, yank buffer. The original `tui-textarea` v0.7.0 by rhysd is abandoned (17 months without release, maintainer unresponsive, incompatible with ratatui 0.30).

2. **Custom implementation (~115 lines)** — add word boundary helpers, Ctrl+W/K/U/Y, Alt+B/F to the existing handler. Less feature-rich but avoids all dependency/compatibility risks. The existing multiline helpers provide a solid foundation.

The choice depends on whether the ratatui-textarea fork inherits the original's open issues (panic on undo after clear, no word wrap, no system clipboard) or has addressed them.

---

## 2. Current Architecture & Gap Analysis

### 2.1 What exists today

The input system spans three files:

| File | Purpose | Lines |
|------|---------|-------|
| `src/tui/mod.rs:158-215` | `TuiState` with `input_text: String` and `input_cursor: usize` | 57 |
| `src/tui/input.rs:88-181` | `handle_input_key` — the core input handler | 93 |
| `src/tui/widgets/input_box.rs:1-99` | Renders `Paragraph` from `input_text`, sets cursor position | 99 |

**Current keybindings** (from `src/tui/input.rs:93-170`):

| Key | Action |
|-----|--------|
| Left/Right | Character movement |
| Up/Down | Line movement (multiline) or scroll |
| Backspace | Delete one character before cursor |
| Shift+Enter, Alt+Enter | Insert newline |
| Enter | Send message |
| Any char | Insert at cursor |

**What's missing:**

| Category | Missing keybinding | Standard binding |
|----------|--------------------|-----------------|
| Word movement | Jump forward by word | Alt+F, Ctrl+Right |
| Word movement | Jump backward by word | Alt+B, Ctrl+Left |
| Line movement | Jump to beginning of line | Ctrl+A, Home |
| Line movement | Jump to end of line | Ctrl+E, End |
| Word deletion | Delete word before cursor | Ctrl+W |
| Word deletion | Delete word after cursor | Alt+D |
| Line deletion | Delete to end of line | Ctrl+K |
| Line deletion | Delete to beginning of line | Ctrl+U |
| Yank/Paste | Paste last killed text | Ctrl+Y |
| Undo | Undo last edit | Ctrl+Z or Ctrl+U |
| Redo | Redo last undone edit | Ctrl+R |
| Selection | Select text | Shift+arrows |

### 2.2 Implementation details

Text is stored as a plain `String` with a `usize` cursor tracking the character index (`src/tui/mod.rs:159-160`). Every character insertion and deletion does byte-offset lookup via `char_indices().nth()` (`src/tui/input.rs:98-102`, `118-123`, `130-134`). This is O(n) per keystroke. Note: for chat messages (typically <1KB), this is not a real-world bottleneck. Both tui-textarea and the custom approach use `Vec<String>`/`String` internally — neither uses a rope. Performance is not a factor in this decision.

Multiline cursor movement exists (`src/tui/input.rs:284-349`) using custom `cursor_line_col`, `line_start_and_len`, `move_cursor_up`, and `move_cursor_down` helpers.

An autocomplete system (`src/tui/input.rs:184-281`) scans backward from cursor for `/` triggers. This would need to be preserved or adapted in any replacement.

### 2.3 Crossterm keyboard enhancement

`src/tui/mod.rs:223-228` already pushes `DISAMBIGUATE_ESCAPE_CODES` when the terminal supports keyboard enhancement. This means modifier key detection (Alt, Ctrl+arrow, etc.) should work reliably on modern terminals.

---

## 3. Requirements

Derived from user request, VISION.md §4.5 (cell model / developer interface), and current architecture:

1. **Emacs-style keybindings** — Ctrl+A/E/K/U/W/Y, Alt+B/D/F at minimum
2. **Word-level movement and deletion** — the primary user request
3. **Undo/redo** — expected by any non-trivial text editor
4. **Kill ring / yank buffer** — Ctrl+K killed text should be pasteable with Ctrl+Y
5. **Multi-line editing** — already supported; must not regress
6. **Autocomplete integration** — `/` trigger system must continue working
7. **ratatui widget compatibility** — must render as a ratatui widget, not take over the terminal
8. **Minimal new dependencies** — avoid pulling in large dependency trees
9. **Cursor position access** — the autocomplete system and rendering need cursor line/col
10. **Customizable key handling** — some keys (Enter, Ctrl+Q, Ctrl+B, Tab) are used by the application and must not be consumed by the editor

---

## 4. Options Analysis

### 4.1 tui-textarea

> A multi-line text editor widget for ratatui with emacs-style keybindings built in.

- **Crate:** [tui-textarea](https://crates.io/crates/tui-textarea) / [GitHub](https://github.com/rhysd/tui-textarea)
- **Version:** 0.7.0 (released 2024-10-22)
- **Stars:** ~489 | **License:** MIT | **Downloads:** 1.2M+ all-time, 356K recent
- **Maintenance:** ABANDONED. Last release Oct 2024 (17 months ago). 36 open issues, 16 open PRs. Maintainer unresponsive to "is this repo active?" query (issue #124, Feb 2026).
- **Compatibility:** v0.7.0 depends on ratatui ^0.29.0 — **will not compile with our ratatui 0.30**. Use the official fork `ratatui-textarea` instead (see §4.1b).

**Built-in keybindings:**

| Key | Action |
|-----|--------|
| Ctrl+F, Right | Move forward by character |
| Ctrl+B, Left | Move backward by character |
| Ctrl+P, Up | Move up by line |
| Ctrl+N, Down | Move down by line |
| Alt+F, Ctrl+Right | Move forward by word |
| Alt+B, Ctrl+Left | Move backward by word |
| Ctrl+A, Home | Move to head of line |
| Ctrl+E, End | Move to end of line |
| Alt+<, Ctrl+Alt+Up | Move to top of buffer |
| Alt+>, Ctrl+Alt+Down | Move to bottom of buffer |
| Alt+], Ctrl+Up | Move up by paragraph |
| Alt+[, Ctrl+Down | Move down by paragraph |
| Ctrl+H, Backspace | Delete character before cursor |
| Ctrl+D, Delete | Delete character after cursor |
| Ctrl+W, Alt+Backspace | Delete word before cursor |
| Alt+D, Alt+Delete | Delete word after cursor |
| Ctrl+K | Delete to end of line |
| Ctrl+J | Delete to head of line |
| Ctrl+U | Undo |
| Ctrl+R | Redo |
| Ctrl+C, Copy | Copy selection |
| Ctrl+X, Cut | Cut selection |
| Ctrl+Y, Paste | Paste yanked text |
| Ctrl+V, PageDown | Scroll down |
| Alt+V, PageUp | Scroll up |

**Strengths:**
- Native ratatui `Widget` — drop-in replacement for `Paragraph`
- Complete emacs keybindings covering every requirement
- Yank buffer (text killed with Ctrl+K/J/W is pasteable with Ctrl+Y)
- Undo/redo with configurable history depth (default 50)
- Regex search (opt-in feature flag)
- `input_without_shortcuts()` for custom key handling — lets us intercept Enter, Ctrl+Q, etc.
- Backend-agnostic (crossterm, termion, termwiz)
- Single-line mode available

**Weaknesses:**
- Ctrl+U is undo (not "delete to line start" as in GNU readline) — diverges from readline convention
- Ctrl+J is "delete to line start" instead of "newline" — also non-standard
- No vim mode
- No system clipboard integration (yank buffer is internal only)
- Internally uses `Vec<String>` — same O(n) characteristics as current `String`, not a rope
- No `clear()` method — must use `select_all()` + `cut()` workaround (issue #96)
- No word wrap (issue #5, open since 2022)
- Ctrl+C mapped to "copy" may conflict with terminal SIGINT
- Ctrl+V mapped to "scroll down" conflicts with user expectation of "paste"

**Integration effort:** Moderate. Replace `input_text: String` + `input_cursor: usize` with `TextArea<'static>`. Adapt autocomplete to read from `TextArea::lines()` and `TextArea::cursor()`. Use `input_without_shortcuts()` to intercept application keys before passing to the textarea.

---

### 4.1b ratatui-textarea (the official fork — RECOMMENDED over 4.1)

> The ratatui organization's maintained fork of tui-textarea.

- **Crate:** [ratatui-textarea](https://crates.io/crates/ratatui-textarea) / [GitHub](https://github.com/ratatui/ratatui-textarea)
- **Version:** 0.8.0 (released 2026-02-21)
- **Maintainer:** Orhun Parmaksiz (core ratatui contributor)
- **Compatibility:** Uses `ratatui-core ^0.1.0` + `ratatui-widgets ^0.3.0` — designed for ratatui 0.30+

Same API and keybindings as tui-textarea (§4.1) since it's a direct fork. The key difference: **this one actually compiles with our dependencies.** Import path changes from `tui_textarea` to `ratatui_textarea`.

**Open questions:** Does it inherit the same open bugs (panic #121, no word wrap #5, no clear() #96)? The fork is relatively new — needs verification before adoption.

---

### 4.2 edtui

> A vim-inspired editor widget for ratatui with emacs mode added in v0.10.1.

- **Crate:** [edtui](https://crates.io/crates/edtui) / [GitHub](https://github.com/preiter93/edtui)
- **Version:** 0.11.2 (updated 2026-03-08)
- **Stars:** ~127 | **License:** MIT | **Downloads:** 83K all-time
- **Maintenance:** Very active — 49 versions, last release March 2026

**Features:** Vim Normal/Insert/Visual modes, Emacs mode, system clipboard, syntax highlighting, line numbers (absolute + relative), mouse events, line wrapping, single-line mode.

**Emacs keybindings:** Ctrl+F/B (char), Alt+F/B (word), Ctrl+A/E (line), Ctrl+D/H (char delete), Alt+D/Backspace (word delete), Ctrl+K (kill to end), Alt+U (kill to start), Ctrl+Y (paste), Ctrl+U (undo), Ctrl+R (redo), Ctrl+S (search).

**Strengths:**
- Vim AND emacs modes — power users get their preferred style
- System clipboard integration (not just internal yank buffer)
- Syntax highlighting for code input
- Visual selection mode
- More feature-rich than tui-textarea

**Weaknesses:**
- Emacs mode is "less feature complete and less tested" per the author
- Vim modal editing (Normal/Insert/Visual) adds UX complexity for a chat input — users don't expect `i` before typing a message
- Heavier dependency (brings in clipboard crate, syntax highlighting stack)
- Smaller community (127 vs 489 stars)
- The full-editor paradigm (modes, line numbers, status line) is overkill for a message input box
- Would need to default to Insert/Emacs mode and hide the Normal mode affordances

**Integration effort:** Moderate-to-high. Similar widget replacement as tui-textarea, but the modal state machine adds complexity. Must ensure the editor starts in Insert/Emacs mode and that mode switching doesn't confuse users in a chat context.

---

### 4.3 reedline

> Nushell's readline replacement — a full-featured line editor.

- **Crate:** [reedline](https://crates.io/crates/reedline) / [GitHub](https://github.com/nushell/reedline)
- **Version:** 0.46.0 (updated 2026-02-28)
- **Stars:** ~738 | **License:** MIT
- **Downloads:** 2M+ all-time, 295K recent
- **Maintenance:** Very active — 48 versions, Nushell core dependency

**Features:** Emacs and Vi modes, kill ring, syntax highlighting, tab completion, history with search, multi-line support, fish-style autosuggestions, input validation, clipboard support (feature flag), SQLite-backed history.

**Strengths:**
- Most feature-complete option — battle-tested in Nushell
- Proper kill ring (multiple killed items, cycling with Alt+Y)
- Customizable keybindings via trait-based architecture
- History with interactive search

**Weaknesses:**
- **Not a ratatui widget.** Reedline is a standalone line editor that takes over terminal I/O. It cannot be embedded as a widget inside a ratatui `Frame`.
- Would require completely restructuring the TUI architecture — either abandoning ratatui for the input area or running reedline in a separate terminal region.
- Designed for shell-style single-prompt input, not for a widget embedded in a complex TUI layout.
- Brings in a large dependency tree.

**Integration effort:** Very high / impractical. Would require architectural redesign to split the terminal between ratatui (for the conversation/panels) and reedline (for input). Not recommended.

---

### 4.4 tui-input

> A headless input library for TUI apps.

- **Crate:** [tui-input](https://crates.io/crates/tui-input) / [GitHub](https://github.com/sayanarijit/tui-input)
- **Version:** 0.15.0 (Dec 2025)
- **Stars:** 182 | **License:** MIT

**Features:** Backend-agnostic input handling, serde support.

**Strengths:**
- Lightweight, headless design
- Easy to integrate

**Weaknesses:**
- No word movement, no word deletion, no kill ring, no undo/redo
- Not significantly better than the current custom implementation
- No multi-line support
- Does not solve the user's problem

**Integration effort:** Low, but pointless — doesn't add the desired features.

---

### 4.5 string_cmd

> Emacs/Vi keybinding library for string editing.

- **Crate:** [string_cmd](https://crates.io/crates/string_cmd) / [GitHub](https://lib.rs/crates/string_cmd)
- **Version:** 0.1.2
- **License:** MIT

**Features:** Emacs and Vi modes, basic navigation (Ctrl+B/F/A/E), basic deletion (Ctrl+H/D/K/U/W), crossterm integration.

**Strengths:**
- Lightweight — pure string editing logic without UI
- Could be used to add keybinding handling to the existing custom input

**Weaknesses:**
- No undo/redo
- No kill ring / yank buffer
- No word movement (only word deletion via Ctrl+W)
- No multi-line support
- Very early stage (0.1.2)
- No ratatui integration — would need manual rendering

**Integration effort:** Low-moderate, but provides only a subset of the desired features.

---

### 4.6 Other crates considered but not recommended

**modalkit + modalkit-ratatui** — Modal editing framework used by the iamb Matrix client. Provides vim/emacs modal editing as ratatui widgets. Too heavyweight and complex for a chat input — designed for full editor applications, not input boxes.

**ratatui-code-editor** — Tree-sitter powered code editor widget with system clipboard, undo/redo, and Unicode awareness. Only 597 downloads, 3 versions published. Too immature and oriented toward code editing rather than message composition.

**rustyline** — GNU readline implementation for Rust. Like reedline, it takes over terminal I/O and cannot be embedded as a ratatui widget. Not suitable.

**linefeed** — Another readline-like crate. Same terminal-takeover problem as rustyline/reedline.

---

### 4.7 Custom implementation (enhance current code)

> Add readline keybindings directly to `handle_input_key` in `src/tui/input.rs`.

**Approach:** Implement word boundary detection, word movement, word deletion, and a kill buffer on top of the existing multiline helpers.

**Detailed line estimate (red team validated):**

| Feature | Lines | Complexity |
|---------|-------|-----------|
| `find_word_boundary_left/right()` | ~30 | Low — iterate chars, check `is_alphanumeric()` |
| Ctrl+A / Ctrl+E (line home/end) | ~10 | Trivial — use existing `line_start_and_len()` |
| Alt+B / Alt+F (word movement) | ~10 | Trivial — call boundary helpers |
| Ctrl+W (delete word back) | ~15 | Low — boundary + `String::drain` |
| Alt+D (delete word forward) | ~15 | Low — same pattern |
| Ctrl+K (kill to end of line) | ~10 | Low — use `line_start_and_len()` |
| Ctrl+U (kill to start of line) | ~10 | Low |
| Kill buffer (single `String` field) | ~5 | One field on `TuiState` |
| Ctrl+Y (yank from kill buffer) | ~10 | Insert from kill buffer |
| **Total (core readline)** | **~115** | |
| Undo/redo stack (optional) | ~100-150 | Medium — needs edit coalescing |

**Strengths:**
- Full control over every keybinding — no conflicts
- No new dependencies, no compatibility risks
- Perfectly tailored to autocomplete integration
- Can match GNU readline conventions exactly (Ctrl+U = kill-to-start, not undo)
- No word wrap regression (current `Paragraph` wraps; tui-textarea doesn't)
- No system clipboard regression
- The existing codebase already has multiline helpers, byte-offset logic, and `char_indices()` patterns

**Weaknesses:**
- ~115 lines for core readline, ~250 with undo/redo
- Word boundary detection via `is_alphanumeric()` is simpler than UAX#29 but sufficient for chat input (consistent with existing char-level handling)
- No selection/highlighting without additional work
- No regex search

**Integration effort:** Low-moderate — extends existing code, zero integration friction.

---

## 5. Comparison Matrix

| Criterion | tui-textarea | edtui | reedline | tui-input | string_cmd | Custom |
|-----------|:-----------:|:-----:|:--------:|:---------:|:----------:|:------:|
| **Word movement** | Yes | Yes | Yes | No | No | Must build |
| **Word deletion** | Yes | Yes | Yes | No | Ctrl+W only | Must build |
| **Line kill (Ctrl+K/U)** | Yes | Yes | Yes | No | Yes | Must build |
| **Kill ring / yank** | Yank buffer | System clipboard | Full kill ring | No | No | Must build |
| **Undo/redo** | Yes (50 depth) | Yes | Yes | No | No | Must build |
| **Multi-line** | Yes | Yes | Yes | No | No | Already exists |
| **ratatui widget** | Yes | Yes | No | Headless | No | Yes (Paragraph) |
| **Emacs mode** | Default | Available | Default | N/A | Available | Must build |
| **Vim mode** | No | Default | Available | N/A | Available | No |
| **Custom key intercept** | `input_without_shortcuts()` | Event handler | Keybinding config | N/A | Manual | Full control |
| **Search** | Regex (opt-in) | Ctrl+S | History search | No | No | No |
| **Dependency weight** | Light | Medium | Heavy | Light | Light | None |
| **ratatui 0.30 compat** | No (PR pending) | Likely yes | N/A | Yes | N/A | Yes |
| **GitHub stars** | ~489 | ~127 | ~738 | 182 | ~0 | N/A |
| **Integration effort** | Moderate | Moderate-high | Impractical | Pointless | Low-moderate | High |

---

## 6. VISION.md Alignment

VISION.md §4.5 describes a "Non-Linear Developer Interface (Cell Model)" with "a Jupyter-cell-like model." The input box is the developer's primary interaction point. Per §5.3, the TUI uses ratatui with a command palette / status bar at the bottom.

**Alignment points:**
- tui-textarea is a ratatui widget, consistent with the ratatui-first architecture
- Rich editing in the input box supports the vision's emphasis on developer control
- The yank buffer and undo/redo support the "observable, debuggable" philosophy — edits aren't destructive
- Regex search in the textarea could later support searching within composed messages

**Deviation:** None. The vision doesn't prescribe a specific input library. All options that are ratatui widgets are consistent.

---

## 7. Recommended Architecture

### Phase 1: Two viable paths

**Path A: Adopt `ratatui-textarea` v0.8.0**

1. **Add dependency:** `ratatui-textarea = { version = "0.8", features = ["crossterm"] }`
2. **Replace state:** In `TuiState`, replace `input_text: String` + `input_cursor: usize` with `textarea: TextArea<'static>`
3. **Adapt input handler:** In `handle_input_key`, use `textarea.input_without_shortcuts(event)` for character-by-character control. Intercept Enter (send), Ctrl+Q (quit), Ctrl+B (toggle panel), Tab (autocomplete/focus) before passing to the textarea.
4. **Adapt autocomplete:** Read from `textarea.lines()` and `textarea.cursor()` instead of `input_text` and `input_cursor`. The `/` trigger scan logic stays the same.
5. **Adapt rendering:** Replace `Paragraph::new(input_text)` with `frame.render_widget(&textarea, area)`. Remove manual cursor positioning — textarea handles it internally.
6. **Adapt message sending:** On Enter, call `textarea.lines().join("\n").trim().to_string()` to extract text. **WARNING:** Do NOT use `select_all()` + `delete_str()` to clear — this triggers a panic if the user then undoes (issue #121). Instead, replace with `TextArea::default()` and re-apply styling.

**Path B: Custom readline implementation (~115 lines)**

1. Add `find_word_boundary_left()` / `find_word_boundary_right()` helpers to `input.rs`
2. Add a `kill_buffer: String` field to `TuiState`
3. Extend the `match key.code` in `handle_input_key` with modifier-aware branches
4. No dependency changes, no rendering changes, no autocomplete adapter needed

**Key type changes:**

```rust
// Before (src/tui/mod.rs)
pub struct TuiState {
    pub input_text: String,
    pub input_cursor: usize,
    // ...
}

// After
pub struct TuiState {
    pub textarea: TextArea<'static>,
    // ...
}
```

**Key input handling pattern:**

```rust
// Before (src/tui/input.rs)
fn handle_input_key(key: KeyEvent, state: &mut TuiState, graph: &ConversationGraph) -> Action {
    match key.code {
        KeyCode::Char(c) => { /* manual insert */ }
        KeyCode::Backspace => { /* manual delete */ }
        KeyCode::Left => { /* manual cursor move */ }
        // ...
    }
}

// After
fn handle_input_key(key: KeyEvent, state: &mut TuiState, graph: &ConversationGraph) -> Action {
    // Application keys intercepted first
    match key.code {
        KeyCode::Enter if !key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
            return send_message(state);
        }
        _ => {}
    }
    // Everything else goes to tui-textarea
    state.textarea.input(key);
    update_autocomplete(state, graph);
    Action::None
}
```

### Phase 2: Keybinding customization (later)

If users want vim mode or custom keybindings:
- Evaluate migrating to `edtui` for vim/emacs dual-mode support
- Or implement a keybinding configuration layer on top of tui-textarea using `input_without_shortcuts()` + manual command dispatch

### Phase 3: External editor escape hatch (future)

For long-form input, allow `$EDITOR` integration:
- Ctrl+X Ctrl+E opens the current input text in the user's `$EDITOR`
- On editor close, the edited text replaces the textarea content
- This pattern is used by bash, git commit, and edtui's `system-editor` feature

---

## 8. Integration Design

### 8.1 Data flow

```
KeyEvent (crossterm)
    │
    ├── Global keys (Ctrl+Q, Ctrl+B, Tab) → Action
    │
    ├── Autocomplete active? → handle_autocomplete_key → Action
    │
    ├── Application keys (Enter → send) → Action
    │
    └── TextArea::input(event) → updates internal state
            │
            └── update_autocomplete() reads TextArea::cursor() + TextArea::lines()
```

### 8.2 Files changed

| File | Change |
|------|--------|
| `Cargo.toml` | Add `tui-textarea` dependency |
| `src/tui/mod.rs` | Replace `input_text`/`input_cursor` with `TextArea<'static>` in `TuiState` |
| `src/tui/input.rs` | Rewrite `handle_input_key` to delegate to textarea; remove `cursor_line_col` and multiline helpers (textarea handles them); keep autocomplete logic reading from textarea API |
| `src/tui/widgets/input_box.rs` | Replace `Paragraph` rendering with `frame.render_widget(&state.textarea, area)`; remove manual cursor positioning |
| `src/app/mod.rs` | Update any references to `input_text`/`input_cursor` to use `textarea` API |

### 8.3 Autocomplete adapter

The autocomplete system scans backward from cursor for `/`. This needs to read from the textarea:

```rust
fn update_autocomplete(state: &mut TuiState, graph: &ConversationGraph) {
    let (row, col) = state.textarea.cursor();
    let current_line = &state.textarea.lines()[row];
    let before_cursor: Vec<char> = current_line.chars().take(col).collect();
    // ... rest of slash detection logic unchanged
}
```

### 8.4 Keybinding conflicts

| Our binding / convention | tui-textarea default | Severity | Resolution |
|--------------------------|---------------------|----------|------------|
| Ctrl+Q → Quit | Not used | Low | Intercept before textarea |
| Ctrl+B → Toggle panel | Cursor backward | HIGH | Remap panel toggle (e.g., F2) |
| Enter → Send message | Insert newline | HIGH | Intercept unmodified Enter |
| Tab → Autocomplete/focus | Not used | Low | Intercept before textarea |
| Up/Down → Scroll | Cursor up/down | Medium | Check textarea state for scroll fallback |
| Ctrl+C → (SIGINT convention) | Copy selection | HIGH | Intercept; users expect cancel/interrupt |
| Ctrl+V → (system paste) | Page down scroll | HIGH | Intercept; users expect paste from clipboard |
| Ctrl+D → (EOF convention) | Delete char forward | Medium | May surprise users who expect exit-on-empty |
| Ctrl+J → (newline/LF) | Delete to line start | Medium | Some terminals send Ctrl+J for Enter |
| PageDown → scroll conversation | Scroll textarea | Medium | App already handles PageDown at `input.rs:168` |

The Ctrl+B conflict is notable — in GNU readline, Ctrl+B moves the cursor backward. We currently use it for panel toggle. Options:
1. Keep Ctrl+B as panel toggle (breaking readline convention)
2. Remap panel toggle to another key (e.g., Ctrl+\\, F2)
3. Make it context-dependent (Ctrl+B at line start → panel toggle, otherwise → cursor back)

**Recommendation:** Remap panel toggle. Users who want readline keybindings will expect Ctrl+B to work for cursor movement.

---

## 9. Red/Green Team

### Green Team (validated by audit)

- **tui-textarea v0.7.0 confirmed as latest on crates.io.** 1.2M+ downloads, 22 versions. The keybinding table was verified against the README — all bindings are accurate.
- **`TextArea::cursor()` returns `(usize, usize)` (row, col)** — VERIFIED. `TextArea::lines()` returns `&[String]` — VERIFIED. The autocomplete adapter design is sound.
- **`input_without_shortcuts()` exists** — VERIFIED in API docs. Allows intercepting keys before textarea processes them.
- **Ctrl+U = undo, Ctrl+J = delete-to-head** — VERIFIED from keybinding table. These are genuine readline divergences, not documentation errors.
- **edtui v0.11.2 confirmed** (updated from initial claim of v0.10.1). Very actively maintained — last release 2026-03-08.
- **reedline v0.46.0 confirmed** (updated from initial claim of v0.45.0). 2M+ downloads.
- **All other crate versions verified** — tui-input 0.15.0, string_cmd 0.1.2 confirmed via crates.io API.
- **Undo/redo, yank buffer, word movement all confirmed** in tui-textarea. These features genuinely exist and work as documented.

### Red Team (challenges from audit)

- **CRITICAL: tui-textarea 0.7.0 is ABANDONED and incompatible with ratatui 0.30.** Last release Oct 2024 (17 months ago). 36 open issues, 16 open PRs. Maintainer unresponsive to "is this repo active?" (issue #124, Feb 2026, no response). The crate depends on ratatui ^0.29.0 — will not compile with our ratatui 0.30. **Mitigation:** Use `ratatui-textarea` v0.8.0, the official ratatui org fork by Orhun Parmaksiz (released Feb 2026, ratatui 0.30 compatible). edtui 0.11.2 (updated 2026-03-08) is another viable fallback.
- **PANIC BUG in the clearing pattern.** Issue #121 (open, Jan 2026): calling `select_all()` + `delete_str()` then `undo()` causes a panic. This is the exact pattern §7 Phase 1 originally recommended for clearing the textarea on message send. If a user sends a message, then presses Ctrl+U (undo), the app crashes. **Mitigation:** Replace with `TextArea::default()` instead of select+delete. Must verify if the fork has fixed this.
- **Ctrl+C conflict is serious.** tui-textarea maps Ctrl+C to "copy selection." In raw mode, SIGINT is suppressed and the keystroke reaches the app. But users may expect Ctrl+C to cancel/quit, not copy. Must intercept before textarea. Issue #106 reports further clipboard confusion with Shift+arrow selections.
- **Ctrl+V conflict is a showstopper.** tui-textarea maps Ctrl+V to "page down." Users universally expect Ctrl+V to paste from system clipboard. This is the single most surprising keybinding conflict. Must intercept Ctrl+V and handle system paste via crossterm's `EnableBracketedPaste`.
- **No `clear()` method** (issue #96). Clearing the textarea on message send requires: `select_all()` then `cut()` or creating a new `TextArea::default()`. Minor ergonomic issue but adds friction.
- **No word wrap** (issue #5, open since Dec 2022). Long lines extend beyond the widget boundary. The current `Paragraph` widget wraps. This is a UX regression for users typing long messages.
- **tui-textarea uses `Vec<String>` internally** — not a rope. The claim in §2.2 that the current O(n) `char_indices().nth()` is a bottleneck is equally true for tui-textarea. However, for chat messages (typically <1KB), this is not a real-world performance concern.
- **The custom implementation is more viable than initially presented.** Detailed red team analysis: the existing codebase has multiline cursor movement, line/col tracking, and byte-offset helpers. Core readline keybindings (word movement + deletion + kill buffer + yank) require ~115 lines, not the initially claimed 300-500. Undo/redo adds ~100-150 lines but wasn't in the user's original request. This option avoids ALL compatibility issues, ALL keybinding conflicts, the word-wrap regression, the system clipboard gap, and the panic bug. The `unicode-segmentation` crate provides UAX#29 word boundaries if needed, but `is_alphanumeric()` is sufficient for chat input and consistent with existing char-level handling.
- **`src/app/mod.rs` also references `input_text`/`input_cursor`** — not mentioned in §8.2 files changed. This file must also be updated.

### Code Accuracy Audit

- `src/tui/input.rs:93-170` — CORRECT (match block within `handle_input_key` which spans 88-181)
- `src/tui/mod.rs:158-160` — CORRECT (`input_text: String` and `input_cursor: usize`)
- `src/tui/mod.rs:223-228` — CORRECT (off by one: actually 223-227, but code is present)
- `src/tui/widgets/input_box.rs:13-20` — CORRECT (`Paragraph::new(input_text)`)
- `src/tui/widgets/input_box.rs:23-28` — CORRECT (`cursor_line_col` + `set_cursor_position`)
- `src/tui/input.rs:184-246` — CORRECT (autocomplete `/` trigger scan)
- `src/tui/input.rs:284-349` — CORRECT (all multiline cursor helpers present)
- **Missing reference:** `src/app/mod.rs` also accesses `input_text`/`input_cursor` — not listed in §8.2

---

## 10. Sources

### Crates evaluated
- [ratatui-textarea](https://github.com/ratatui/ratatui-textarea) — v0.8.0 (Feb 2026), MIT, **official ratatui org fork**, ratatui 0.30 compatible
- [tui-textarea](https://github.com/rhysd/tui-textarea) — ~489 stars, v0.7.0 (Oct 2024), MIT, ABANDONED, ratatui 0.29 only (1.2M downloads)
- [edtui](https://github.com/preiter93/edtui) — ~127 stars, v0.11.2, MIT, vim/emacs editor widget for ratatui (83K downloads)
- [reedline](https://github.com/nushell/reedline) — ~738 stars, v0.46.0, MIT, Nushell's readline replacement (2M downloads)
- [tui-input](https://github.com/sayanarijit/tui-input) — 182 stars, v0.15.0, MIT, headless TUI input library (1.1M downloads)
- [string_cmd](https://lib.rs/crates/string_cmd) — v0.1.2, MIT, emacs/vi keybinding string editor (2K downloads)
- [ropey](https://github.com/cessen/ropey) — v1.6.1 (2.0 beta), MIT, rope data structure for large text editing
- [modalkit-ratatui](https://lib.rs/crates/modalkit-ratatui) — modal vim/emacs editing for ratatui (used by iamb)
- [ratatui-code-editor](https://github.com/vipmax/ratatui-code-editor) — Tree-sitter code editor widget (597 downloads, immature)
- [unicode-segmentation](https://github.com/unicode-rs/unicode-segmentation) — UAX#29 word boundary detection (relevant for custom impl)

### Documentation
- [tui-textarea docs](https://docs.rs/tui-textarea/latest/tui_textarea/) — API reference and keybinding list
- [edtui docs](https://docs.rs/edtui/latest/edtui/) — API reference
- [reedline docs](https://docs.rs/reedline/latest/reedline/) — API reference
- [Nushell line editor guide](https://www.nushell.sh/book/line_editor.html) — reedline keybindings and config
- [GNU Readline emacs cheat sheet](https://readline.kablamo.org/emacs.html) — standard readline keybindings
- [Readline emacs editing mode (catonmat)](https://catonmat.net/ftp/readline-emacs-editing-mode-cheat-sheet.pdf) — comprehensive keybinding reference

### Architecture references
- `src/tui/input.rs` — current input handler
- `src/tui/mod.rs:158-215` — `TuiState` definition
- `src/tui/widgets/input_box.rs` — current rendering
- `docs/VISION.md` §4.5 — cell model / developer interface
- `docs/VISION.md` §5.3 — TUI framework choice
