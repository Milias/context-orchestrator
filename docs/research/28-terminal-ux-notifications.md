# Terminal UX Notifications & Feedback

> **2026-03-14** — Research on improving terminal UX through layered notification strategies, audio cues, visual feedback, and out-of-focus alerts for the context-orchestrator's ratatui TUI. Covers desktop notifications (notify-rust, OSC sequences), audio (BEL, rodio), visual attention mechanisms (flash, urgency coloring, badge counters), terminal capability detection, accessibility considerations, and prior art from Claude Code, Cursor, lazygit, helix, and starship — with a phased approach from zero-dependency in-TUI improvements to desktop notifications to rich audio.

---

## 1. Executive Summary

The TUI currently has three feedback channels: status bar text, animated counters/spinners, and a question modal — all confined to the alternate screen buffer. If the user switches away from the terminal, they have zero visibility into events. Critical events like `QuestionRoutedToUser` (which blocks agent progress) and `ErrorOccurred` have the same visual weight as informational events like `GitFilesRefreshed`. After surveying desktop notification crates (notify-rust), terminal escape sequences (OSC 0/2/8/9/99/777), audio libraries (rodio, BEL), prior art from developer tools (Claude Code hooks, Cursor sound notifications, GitHub Copilot CLI context warnings, lazygit progress indicators, helix statusline spinners, starship error symbols), and accessibility guidelines (WCAG color independence, reduced motion, multi-channel feedback), we recommend a layered notification system organized by urgency tier (Critical / Attention / Informational), integrated through the existing `apply_event()` pipeline, with graceful degradation based on detected terminal capabilities. Phase 1 requires zero new dependencies; Phase 2 adds desktop notifications via notify-rust; Phase 3 adds feature-gated audio cues via rodio.

---

## 2. Current Architecture & Gap Analysis

### What Exists

The EventBus (`src/graph/event.rs:15-120`) uses `tokio::sync::broadcast` with a 256-item buffer. All cross-component communication flows through this bus — 20+ `GraphEvent` variants.

The TUI event handler (`src/tui/event_handler.rs:16-108`) is the sole mutator of `TuiState` from events. The `apply_event()` function receives each `GraphEvent` and updates display state.

| Component | Location | Detail |
|-----------|----------|--------|
| EventBus | `src/graph/event.rs:98-120` | `tokio::broadcast`, 256-item buffer, `emit()` + `subscribe()` |
| TUI event handler | `src/tui/event_handler.rs:16-108` | `apply_event()` — single `match` on `GraphEvent`, mutates `TuiState` |
| Status bar | `src/tui/ui.rs:124-156` | Left: context-aware shortcuts. Right: error text (red) |
| Tab status bar | `src/tui/ui.rs:73-121` | Branch name, animated token counter |
| Question modal | `src/tui/widgets/input_box.rs:9-50` | Cyan border when `pending_question_text` is `Some`, "Answer: ..." title |
| Spinner animation | `src/tui/mod.rs:42-43` | `SPINNER_FRAMES` (braille dots), `CURSOR_FRAMES` (block fading) |
| Animated counters | `src/tui/mod.rs:143-168` | `AnimatedCounter` — ease-out, 25% of gap per tick |
| Tool status icons | `src/tui/widgets/tool_status.rs:35-43` | `○` `◉` `✓` `✗` `⊘` per `ToolCallStatus` |
| Service status | `src/tui/widgets/stats_panel.rs:55-77` | `✓` `⟳` `✗` `○` for background task lifecycle |
| Ticker rate | `src/app/mod.rs:102-103` | 80ms interval driving spinner, reveal, scroll, counter animation |
| Keyboard detection | `src/tui/mod.rs:296-301` | Kitty protocol via `supports_keyboard_enhancement()` |
| Event drain | `src/app/mod.rs:346-354` | `drain_pending_events()` — processes queued events before each frame |

### Gaps

| Gap | Impact |
|-----|--------|
| No out-of-focus notification | User must watch terminal to notice events; `QuestionRoutedToUser` blocks agent silently |
| No audio cues | No terminal bell, no sound effects |
| No urgency differentiation | `ErrorOccurred` is red text on status bar — same visual weight as a question prompt in cyan |
| No window title updates | OSC 0/2 not used; title stays at terminal default when user switches tabs |
| No desktop notification | No notify-rust, no OSC 9/777/99 |
| No attention flash | No momentary visual change for critical events |
| No tmux awareness | OSC sequences would be swallowed inside tmux without DCS passthrough |
| No user configuration | Notification channels are hardcoded |
| Limited accessibility | Color alone distinguishes some status (red = error, cyan = question); tool_status.rs correctly pairs symbols with colors, but status bar does not |

---

## 3. Requirements

Derived from VISION.md ("developer control" in §1, tachyonfx in §5.3, background processing indicators in §4.3), CLAUDE.md (event-driven architecture, no dead code), and established patterns (EventBus for all cross-component communication, `apply_event()` as sole TuiState mutator):

1. **Event-driven** — Notification decisions driven by `GraphEvent` variants through `apply_event()`, not ad-hoc insertion.
2. **Layered by urgency** — Three tiers (Critical / Attention / Informational) with distinct treatment per tier.
3. **Graceful degradation** — Best-effort based on detected terminal capabilities. Works in bare `xterm`; richer in Kitty/iTerm2.
4. **User-configurable** — Which channels are active per tier. Respects user terminal bell settings.
5. **Non-blocking** — No notification mechanism blocks the 80ms render tick or the EventBus.
6. **Accessible** — Never rely on color alone. Pair with symbols, text labels, and optional audio.
7. **tmux-aware** — OSC sequences wrapped in DCS passthrough when `$TMUX` is set.
8. **Cross-platform** — Linux (XDG/D-Bus), macOS (notification center), basic Windows (WinRT toast).

---

## 4. Notification Urgency Model

### Event-to-Tier Mapping

| Tier | Events | Rationale |
|------|--------|-----------|
| **Critical** (user action required) | `QuestionRoutedToUser`, `ErrorOccurred` (fatal class) | User must respond; ignoring blocks agent progress |
| **Attention** (user should know) | `AgentFinished`, `WorkItemStatusChanged(Completed)`, `CompletionProposed`, `ErrorOccurred` (non-fatal) | Work completed or something went wrong; user inspects when convenient |
| **Informational** (passive awareness) | `AgentPhaseChanged`, `ToolCallCompleted`, `StreamDelta`, `GitFilesRefreshed`, `TokenTotalsUpdated`, everything else | Background activity; visible in TUI when user is watching |

### Channels per Tier

| Channel | Critical | Attention | Informational |
|---------|----------|-----------|---------------|
| In-TUI visual (status bar/modal) | Always | Always | Always |
| Attention flash (momentary color) | Yes | No | No |
| Terminal bell (BEL `\x07`) | Yes (configurable) | Optional | No |
| Window title (OSC 0/2) | Yes | Yes | No |
| Desktop notification (notify-rust/OSC) | Yes | Optional | No |
| Audio cue (rodio) | Optional (future) | Optional (future) | No |

The classification function should be pure (no side effects) — it takes a `&GraphEvent` and returns a tier enum. This keeps urgency logic testable and separate from the notification dispatch.

---

## 5. Visual Notification Strategies

All strategies in this section require zero new dependencies — they use existing ratatui primitives and crossterm capabilities.

### Attention Flash

A momentary full-screen background color shift for critical events. When `QuestionRoutedToUser` fires, the entire frame area gets a low-opacity cyan tint for 2 ticks (160ms at 80ms/tick), then fades back. Implementation concept: a `flash_remaining: u8` field in `TuiState` that `apply_event()` sets to 2 on critical events; the rendering pipeline checks this and adjusts the root area background color. Each tick decrements it. This is the terminal equivalent of vim's `visualbell`.

tachyonfx (mentioned in VISION.md §5.3) could implement this as a composable effect, but a manual countdown is simpler and avoids an additional dependency in Phase 1.

**Photosensitivity consideration:** The flash is a single subtle background shift (not a strobe). Duration is capped at 160ms. Disabled when `reduced_motion` is configured.

### Status Bar Urgency Coloring

Currently the status bar (`src/tui/ui.rs:124-156`) has a fixed dark background (`Rgb(20, 20, 50)`) with red error text right-aligned. Extend to color the entire bar background based on the highest active urgency:

| Urgency | Background | Symbol prefix |
|---------|------------|---------------|
| Critical | `Rgb(80, 20, 20)` (dark red) | `[!]` |
| Attention | `Rgb(60, 50, 10)` (dark amber) | `[i]` |
| Normal | `Rgb(20, 20, 50)` (current) | (none) |

The symbol prefix ensures color-blind users can distinguish urgency tiers. This follows the pattern already established by `tool_call_status_icon()` at `src/tui/widgets/tool_status.rs:35-43`, which pairs every color with a unique Unicode symbol.

### Badge Counters on Tab Labels

The tab bar (`src/tui/ui.rs:83-93`) renders tab labels. When unacknowledged attention-tier events exist, append a count: `Overview (3)` or `Overview [!]` for critical. This follows lazygit's pattern of panel indicators and provides at-a-glance awareness without requiring the user to be in a specific tab.

### Border Urgency Coloring

The question input box (`src/tui/widgets/input_box.rs:10-27`) already changes border color to cyan when a question is pending. Extend this pattern: when a critical event is active, the conversation panel border could pulse between its normal color and an attention color on the spinner tick interval.

---

## 6. Terminal Escape Sequence Strategies

Terminal escape sequences provide out-of-band notification channels that work even when the user is not looking at the TUI.

### OSC 0/2 — Window Title

**Syntax:** `\x1b]0;Title\x07` (icon + title) or `\x1b]2;Title\x07` (title only).

**Support:** Near-universal across terminals — xterm, GNOME Terminal, Konsole, iTerm2, Kitty, WezTerm, Windows Terminal, macOS Terminal.app.

**Strategy:** Update the window title to reflect the highest active urgency. When idle: `Context Orchestrator [branch]`. When critical: `[!] Action Required — Context Orchestrator`. When attention: `[i] Agent Finished — Context Orchestrator`. This is visible in taskbar/dock tab labels even when the terminal is backgrounded.

**tmux consideration:** tmux manages its own window titles. When `$TMUX` is set, either skip title changes or use tmux's `set-option -t` via escape sequences. Alternatively, set the tmux pane title via `\x1b]2;Title\x07` which tmux forwards to its status bar if `set -g set-titles on` is configured.

### OSC 9/777/99 — Desktop Notifications

Terminal-specific notification sequences that trigger the OS notification system:

| Sequence | Terminal | Syntax |
|----------|----------|--------|
| OSC 9 | iTerm2, ConEmu | `\x1b]9;message\x07` |
| OSC 777 | rxvt-unicode | `\x1b]777;notify;title;body\x07` |
| OSC 99 | Kitty | `\x1b]99;i=id:d=0;title\x1b\\` |

These are zero-dependency (write escape bytes to stdout) but terminal-specific. The project already detects Kitty keyboard enhancement (`src/tui/mod.rs:296`); similar heuristics can detect notification support.

**Limitation:** Cannot control notification urgency, icons, or actions. No standard way to know if the terminal supports them. Best used as a secondary channel alongside notify-rust.

### OSC 8 — Terminal Hyperlinks

**Syntax:** `\x1b]8;;url\x07visible text\x1b]8;;\x07`

**Support:** Kitty, iTerm2, WezTerm, Windows Terminal, GNOME VTE, foot. Tracked at the OSC8-Adoption repository.

**Use case:** Make file paths in the activity stream and tool call results clickable. When a `ToolCallCompleted` event references a file path, the rendered path could be a terminal hyperlink opening the file in the user's editor. This is an enhancement rather than a notification, but it significantly improves UX for navigating tool results.

The `supports-hyperlinks` crate can detect OSC 8 support at runtime.

### BEL (`\x07`) — Terminal Bell

The simplest cross-platform audio cue. Writing `\x07` to stdout triggers the terminal's bell behavior — which the user controls in their terminal settings (audible beep, visual flash, or silence).

**Strategy:** Emit BEL on critical events (default: enabled, configurable). This is the only audio mechanism that respects the user's existing terminal preferences without introducing new dependencies.

**tmux:** tmux intercepts BEL and can forward it to the client terminal or trigger visual activity indicators. The `visual-bell` tmux option controls this behavior — no special handling needed from the application.

### tmux DCS Passthrough

When running inside tmux, plain OSC sequences are intercepted and may not reach the outer terminal. Wrapping in DCS passthrough ensures delivery:

**Syntax:** `\x1bPtmux;\x1b` + (OSC sequence with escaped `\x1b`) + `\x1b\\`

**Detection:** Check `$TMUX` environment variable (set by tmux) or `$TERM` starting with "screen" or "tmux".

**Strategy:** A `wrap_for_tmux(sequence: &[u8]) -> Vec<u8>` utility that conditionally wraps OSC sequences when tmux is detected. Called by all OSC-emitting functions.

---

## 7. Desktop Notification Strategies

### notify-rust

The primary Rust crate for cross-platform desktop notifications. Supports:

- **Linux/BSD:** XDG `org.freedesktop.Notifications` D-Bus interface. Works with KDE, GNOME, XFCE, LXDE, Mate, Sway.
- **macOS:** Objective-C `UNUserNotificationCenter` (via mac-notification-sys).
- **Windows:** WinRT toast notifications (via winrt-notification).
- **Features:** Urgency levels (low/normal/critical), custom icons, action buttons, timeout control.

**Trade-off:** Adds a dependency with D-Bus linkage on Linux. Should be feature-gated (`desktop-notifications` Cargo feature) so users who don't want it pay zero compile or runtime cost.

### OSC Sequences (zero-dependency alternative)

For terminals that support them, OSC 9/99/777 provide desktop notifications without any crate dependency. Less reliable (terminal-specific) and less featureful (no urgency/icons) but zero-cost.

### Recommendation

Use notify-rust as the primary mechanism behind a Cargo feature flag. Use OSC sequences as a fallback for users who enable terminal-native notifications without the feature flag. Both channels should be user-configurable.

---

## 8. Audio Cue Strategies

Three approaches, ordered by complexity:

| Approach | Dependency | Cross-platform | Distinct sounds | User control |
|----------|-----------|----------------|----------------|--------------|
| **BEL** (`\x07`) | None | All terminals | No (single tone) | Terminal settings |
| **rodio** | ~5 crates (cpal, hound, etc.) | Linux/macOS/Windows | Yes (WAV/OGG/MP3 per event type) | App config |
| **TTS** (espeak-rs, tts) | Heavy (speech engine) | Linux (eSpeak NG), macOS (NSSpeechSynthesizer) | Unlimited (spoken text) | App config |

**BEL** is sufficient for Phase 1 — it signals "something happened" and the user's terminal controls the actual behavior (audible beep, visual flash, or nothing). This is the only approach that respects existing user preferences without introducing new dependencies.

**rodio** enables distinct sounds per event type (success chime, error buzz, question ding) and is the recommended Phase 3 approach. It should be feature-gated (`audio-notifications` Cargo feature). The crate spawns a background audio thread — non-blocking by design.

**TTS** is an accessibility consideration for the future. Reading "Question from agent: what database should we use?" aloud is powerful for hands-free workflows. Out of scope for initial phases but worth noting as a direction.

---

## 9. Terminal Capability Detection

A `TerminalCapabilities` struct should be computed once at startup (alongside the existing `setup_terminal()` at `src/tui/mod.rs:290-305`) and stored for the session.

### Detection Methods

| Capability | Detection | Env var / method |
|------------|-----------|------------------|
| Kitty extensions | Existing crossterm detection | `supports_keyboard_enhancement()` |
| Kitty OSC 99 | Heuristic: if Kitty keyboard works | (correlated with keyboard detection) |
| iTerm2 | Environment variable | `$TERM_PROGRAM == "iTerm.app"` or `$LC_TERMINAL == "iTerm2"` |
| WezTerm | Environment variable | `$TERM_PROGRAM == "WezTerm"` |
| Windows Terminal | Environment variable | `$WT_SESSION` is set |
| tmux | Environment variable | `$TMUX` is set, or `$TERM` starts with "screen"/"tmux" |
| OSC 0/2 (title) | Assume supported | Near-universal; skip only for `$TERM == "dumb"` |
| BEL | Assume supported | Universal |
| OSC 8 (hyperlinks) | Runtime probe or heuristic | `supports-hyperlinks` crate, or infer from terminal identity |
| Color depth | Existing crossterm detection | `crossterm::style::available_color_count()` |

### Design Principle

Every notification channel checks capabilities before emitting. Unsupported sequences are silently skipped. The user never sees garbled escape codes from an unsupported feature — they simply don't get that notification channel.

---

## 10. Prior Art from Developer Tools

| Tool | Notification approach | Strengths | Weaknesses |
|------|----------------------|-----------|------------|
| **Claude Code** | Custom hooks for notifications; Kitty/Ghostty native desktop notifications; iTerm2 filter alerts | Extensible via hooks; zero-config for supporting terminals | No universal solution; macOS Terminal.app unsupported |
| **Cursor** | Play sound when agent finishes (Settings → Features → Chat); macOS system notifications | Simple, effective dual-channel (audio + visual) | IDE-only; no terminal equivalent |
| **GitHub Copilot CLI** | Context truncation warning above input (≤20% remaining); background task timeline; usage counter (`/usage`) | Progressive disclosure; non-intrusive | No out-of-focus notifications |
| **lazygit** | Bottom-line progress loader; contextual keybinding help per tab; toast messages | Discoverable; context-aware shortcuts | No audio or desktop notifications |
| **helix** | Progress spinner in statusline for LSP operations; diagnostic counters | Unobtrusive; informational without interruption | No attention-level differentiation |
| **starship** | Error indicator (`✖` + exit code); command duration (>2s); background job counter; battery alert | Modular; rich symbols via Nerd Fonts | Prompt-only; no event notifications |
| **Zellij** | Built-in status bar with keybindings, mode, pane/tab info; activity indicators per pane | Always-visible context; zero-config | No desktop notifications or audio |
| **bottom (btm)** | Color-coded resource graphs; threshold-based highlighting | Rich visual data; effective for monitoring | No notification channels beyond visual |

**Key patterns across tools:**
- Status bar / bottom bar is the universal notification surface
- Audio notifications are rare (only Cursor) but valued when present
- Desktop notifications are emerging but inconsistent
- Contextual keybinding hints reduce cognitive load (lazygit, Zellij)
- Symbols paired with colors are the accessibility standard (starship, helix)

---

## 11. Comparison Matrix

| Channel | Dependency | Cross-platform | Out-of-focus | Urgency levels | Configurable | Accessible |
|---------|-----------|----------------|-------------|----------------|-------------|-----------|
| Status bar text | None | Yes | No | Via color + symbol | Always on | Needs symbols |
| Attention flash | None | Yes | No | Single (critical) | Yes | Reduced motion opt-out |
| BEL (`\x07`) | None | Yes | Yes (terminal-dependent) | Single | Terminal settings | Audible |
| Window title (OSC 0/2) | None | Most terminals | Visible in taskbar | Via prefix text | Yes | Text-based |
| OSC 9/99/777 | None | Terminal-specific | Yes | No | Yes | Text-based |
| notify-rust | Crate (D-Bus on Linux) | Yes | Yes | Low/Normal/Critical | Yes | Text + icons |
| rodio | Crate (~5 transitive) | Yes | Yes | Per-sound-file | Yes | Audible |

**Read across rows:** notify-rust has the best overall coverage (cross-platform, out-of-focus, urgency, accessible) but at the cost of a dependency. BEL is the best zero-dependency option for out-of-focus notification.

**Read down columns:** "Out-of-focus" is the critical gap — only BEL, window title, OSC notifications, notify-rust, and rodio work when the user is not looking at the TUI.

---

## 12. User Configurability Model

Notification preferences should be part of the existing configuration system, loaded at startup. Default philosophy: **critical ON for all available channels; attention visual-only; informational always-on, visual-only.**

### Configuration Surface

```toml
[notifications]
# Critical tier
critical_bell = true           # BEL for QuestionRoutedToUser, fatal errors
critical_desktop = true        # Desktop notification (requires feature flag)
critical_flash = true          # Visual attention flash

# Attention tier
attention_bell = false          # BEL for AgentFinished, completion (default off)
attention_desktop = false       # Desktop notification for attention events

# General
window_title = true            # Dynamic window title reflecting urgency
reduced_motion = false          # Disable flash, snap animations instantly
```

**Rationale for defaults:** Following clig.dev's "less is more" principle — fewer, more relevant notifications improve user satisfaction. Critical events (blocking agent progress) justify interruption. Attention events should not interrupt flow by default but can be opted in by users who want them.

---

## 13. Accessibility Considerations

### Color Independence

The TUI currently uses color for status differentiation in some areas. The tool status icons (`src/tui/widgets/tool_status.rs:35-43`) correctly pair every color with a unique Unicode symbol — this pattern should be extended everywhere:

| Status | Current | Accessible version |
|--------|---------|-------------------|
| Error (status bar) | Red text only | `[!]` prefix + red background |
| Question pending | Cyan border only | `[?]` prefix + cyan border |
| Agent preparing | Status text only | `⟳` prefix + status text |
| Agent streaming | Status text only | `▸` prefix + status text |

### Reduced Motion

The 80ms ticker drives spinners, scroll easing, counter animation, and text reveal. A `reduced_motion` configuration should:
- Snap `AnimatedCounter`, `AnimatedScroll` to targets instantly (skip easing)
- Disable the attention flash
- Keep spinners (they indicate activity, not decoration) but slow the tick rate

### Screen Reader Limitations

Raw-mode TUI is inherently inaccessible to screen readers — alternate screen buffer contents are not exposed to assistive technology. Mitigations:
- Desktop notifications via notify-rust include readable text (screen readers announce these)
- BEL triggers the terminal's accessibility announcement
- A future `--accessible` mode could output to stdout in non-raw mode — scope for a separate research doc

### Multi-Channel Principle

Critical notifications should always activate at least two channels (visual + auditory). This ensures users who cannot perceive one channel still receive the notification:
- Visual: status bar urgency coloring + symbol prefix
- Auditory: BEL character (user controls whether this is sound or visual bell)
- Text: desktop notification with readable content

---

## 14. Recommended Architecture (Phased)

### Phase 1: Zero-Dependency In-TUI Improvements

- **Urgency classification function**: Pure function mapping `&GraphEvent` → urgency tier enum
- **Status bar urgency coloring**: Background color + symbol prefix based on highest active tier
- **BEL for critical events**: Write `\x07` to stdout on `QuestionRoutedToUser` and fatal errors
- **Window title updates**: OSC 0/2 with urgency prefix, tmux-aware (check `$TMUX`)
- **`TerminalCapabilities` struct**: Computed at startup, stored alongside `TuiState`
- **Unicode urgency prefixes**: `[!]` for critical, `[i]` for attention on status messages

No new dependencies. No new Cargo features. Integrates through the existing `apply_event()` → rendering pipeline.

### Phase 2: Desktop Notifications & Configuration

- **notify-rust integration**: Behind `desktop-notifications` Cargo feature flag. D-Bus on Linux, notification center on macOS, WinRT on Windows
- **OSC 9/99/777 fallback**: Terminal-specific desktop notifications for users without the feature flag
- **Notification configuration**: `[notifications]` section in config file
- **Attention flash**: 2-tick background color shift on critical events (tachyonfx or manual)
- **Tab badge counters**: Unacknowledged event counts on tab labels

### Phase 3: Rich Audio & Extended Accessibility

- **rodio audio cues**: Behind `audio-notifications` Cargo feature flag. Distinct sounds per event type
- **Expanded accessibility audit**: Verify every color-coded element has a symbol pair
- **OSC 8 hyperlinks**: Clickable file paths in activity stream and tool results
- **Reduced motion mode**: Full implementation with snap-to-target animations

---

## 15. VISION.md Alignment

| Vision element | Notification relevance |
|---------------|----------------------|
| "Developer control" (§1) | Configurable notification channels per urgency tier — user decides what interrupts them |
| TUI framework (§5.3) | All Phase 1 improvements use ratatui primitives; tachyonfx mentioned for animation polish |
| Background processing indicators (§4.3) | "Use a clear visual indicator for 'processing' status" — urgency model extends this to all event types |
| "Observable context construction" (§2.2) | Window title and badge counters make system state observable even when backgrounded |
| "Developer pinning" (§4.7) | "Token budget visibility is a simple but powerful UX innovation" — same principle applies to notification visibility |

---

## 16. Red/Green Team Audit

### Green Team — Validations

1. **Architecture fit confirmed**: The urgency classification → `apply_event()` → rendering pipeline preserves the existing pattern. No new communication channels needed. Out-of-band effects (BEL, OSC, desktop notifications) execute in `drain_pending_events()` or after `apply_event()` returns — consistent with how the main loop in `src/app/mod.rs:180-181` already processes events before drawing.

2. **Crate ecosystem verified**: notify-rust (crates.io, actively maintained, 3M+ downloads), rodio (crates.io, built on cpal, cross-platform audio), supports-hyperlinks (crates.io, detects OSC 8 support). All are production-grade with active maintenance.

3. **OSC sequence syntax verified**: OSC 0 (`\x1b]0;text\x07`) and OSC 2 (`\x1b]2;text\x07`) are standardized in ECMA-48 / xterm control sequences. OSC 9 (iTerm2) and OSC 99 (Kitty) are documented in their respective terminal documentation. DCS passthrough (`\x1bPtmux;\x1b...sequence...\x1b\\`) is documented in tmux(1) manpage.

4. **Environment variable detection verified**: `$TERM_PROGRAM` is set by iTerm2, WezTerm, Apple Terminal, and others. `$TMUX` is set by tmux. `$WT_SESSION` is set by Windows Terminal. These are stable, documented interfaces.

5. **Existing pattern reuse**: Tool status icons (`src/tui/widgets/tool_status.rs:35-43`) already pair symbols with colors — extending this to the status bar is consistent, not novel.

### Red Team — Challenges

1. **"BEL is annoying and users will turn it off immediately."** Valid concern. Mitigation: BEL is user-configurable (`critical_bell = false`). More importantly, the terminal itself controls BEL behavior — many users have it set to visual bell or silent. BEL is not our only out-of-focus channel; desktop notifications (Phase 2) are the more effective mechanism.

2. **"notify-rust adds D-Bus linkage on Linux, complicating builds and cross-compilation."** Valid. Mitigation: Feature-gated behind `desktop-notifications`. Users who don't enable it have zero impact. For CI/cross-compilation, the feature is simply disabled.

3. **"Attention flash could trigger photosensitivity."** Valid. Mitigation: Flash is a single subtle background color shift (dark blue → dark red), not a strobe. Duration capped at 160ms (2 ticks). Disabled entirely when `reduced_motion = true`. Well within WCAG 2.3.1 guidelines (no more than 3 flashes per second).

4. **"Window title changes confuse terminal multiplexer users."** Partially valid. Mitigation: tmux detection prevents raw OSC 0 writes inside tmux sessions. tmux has its own title management (`set-titles`). For non-tmux users, title changes are standard and expected (many CLI tools do this).

5. **"Terminal capability detection is unreliable — environment variables can be spoofed or absent."** Valid. Mitigation: Graceful degradation by design. Every capability has a fallback (OSC → BEL → visual-only). Unsupported sequences are silently skipped. No capability is assumed — the default path is visual-only, which always works.

6. **"This adds notification logic to `apply_event()` — will it exceed the 400-line file limit?"** Valid concern given CLAUDE.md rules. Mitigation: The urgency classification function should live in a separate module (e.g., `src/tui/notification.rs`). `apply_event()` calls the classifier and sets TuiState fields. Out-of-band effects (BEL, OSC) are dispatched by the main loop after `apply_event()` returns, not inside it.

7. **"Why not just use Claude Code's hook system instead of building our own?"** Fair question. Claude Code hooks are external shell commands triggered by events — powerful but external. Our system is internal: urgency classification drives built-in visual treatment, not just external commands. The two are complementary, not competing. A future hook system could be layered on top of the urgency model.

8. **"By the time Phase 3 ships, context windows may be 2M+ tokens and agent-blocking questions may be rare."** Possible. But the notification system addresses more than questions — it covers agent completion, errors, and general UX polish. Even with infinite context, users still need to know when their agent finished a task.

---

## 17. Sources

### Desktop Notifications
- notify-rust: https://crates.io/crates/notify-rust — Cross-platform desktop notifications for Rust
- XDG Desktop Notifications: https://specifications.freedesktop.org/notification-spec/ — Linux notification standard
- mac-notification-sys: https://crates.io/crates/mac-notification-sys — macOS notification bindings

### Terminal Escape Sequences
- xterm Control Sequences: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html — Canonical reference for OSC/CSI/DCS
- iTerm2 Escape Codes: https://iterm2.com/documentation-escape-codes.html — OSC 9 and proprietary sequences
- Kitty Notifications Protocol: https://sw.kovidgoyal.net/kitty/desktop-notifications/ — OSC 99 specification
- OSC8-Adoption: https://github.com/Alhadis/OSC8-Adoption — Terminal hyperlink support tracking
- tmux DCS Passthrough: https://github.com/tmux/tmux/wiki/FAQ — tmux escape sequence forwarding

### Audio
- rodio: https://crates.io/crates/rodio — Audio playback for Rust (built on cpal)
- Ratatui + Rodio integration: https://dev.to/askrodney/ratatui-audio-with-rodio-sound-fx-for-rust-text-based-ui-bhd

### Terminal Detection
- supports-hyperlinks: https://crates.io/crates/supports-hyperlinks — Detect OSC 8 support
- Escape Sequences Reference: https://gist.github.com/fdncred/c649b8ab3577a0e2873a8f229730e939 — Comprehensive terminal capability reference

### Accessibility
- WCAG 2.3.1 Three Flashes: https://www.w3.org/WAI/WCAG21/Understanding/three-flashes-or-below-threshold.html
- Bloomberg Terminal Color Accessibility: https://www.bloomberg.com/ux/2021/10/14/designing-the-terminal-for-color-accessibility/
- Designing for Color Blindness: https://www.smashingmagazine.com/2024/02/designing-for-colorblindness/

### Prior Art
- Claude Code Terminal Config: https://code.claude.com/docs/en/terminal-config — Hook system for notifications
- Claude Code Tmux Notifications: https://quemy.info/2025-08-04-notification-system-tmux-claude.html
- Cursor Sound Notifications: https://www.dredyson.com/the-complete-beginners-guide-to-reliable-sound-notifications-in-cursor-ide/
- CLI UX Best Practices: https://clig.dev/ — Command Line Interface Guidelines
- CLI Progress Patterns: https://evilmartians.com/chronicles/cli-ux-best-practices-3-patterns-for-improving-progress-displays/

### Internal References
- Doc 02 — Developer UX & Workflow Design: `docs/research/02-developer-ux-and-workflow.md`
- Doc 18 — Terminal Text Editing: `docs/research/18-terminal-text-editing.md`
- Doc 27 — LLM-Defined Event Triggers: `docs/research/27-llm-defined-event-triggers.md`
