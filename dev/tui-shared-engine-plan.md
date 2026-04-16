# TUI and Shared Engine Plan

## Implementation Status — 2026-04-15

| Item | Status | Notes |
|------|--------|-------|
| Session transcripts | ✅ Done | `render_transcript()` in TUI sessions.rs |
| Retrospective per workspace | ✅ Done | Persisted in state, displayed in flight_detail.rs |
| Agent polling | ✅ Done | Periodic 30s polling implemented |
| Git context refresh | ✅ Done | `refresh_git_context()` called at startup per workspace |
| Shared core modules | ✅ Done | flight, git, pty, orchestrator, agent, storage, error_classifier, workspace all present |
| Phase 2: Leader key model | ✅ Done | — |
| Phase 2: Pane-focused layouts | ❌ Not started | — |
| Phase 2: Search in session output | ✅ Done | — |
| Phase 3: Structured logging | ❌ Not started | — |
| Phase 3: Event stream in shared engine | ❌ Not started | — |

Last updated: 2026-04-15

## Overview

PacketCode ships two frontends that share a single Rust backend:

- **GUI**: Tauri v2 desktop app (webview + React frontend)
- **TUI**: `packetcode-tui` standalone binary (Ratatui + Rust frontend)

Both frontends depend on `packetcode_lib::core`, which is the shared orchestration and runtime engine. Changes to `packetcode_lib::core` affect both frontends simultaneously.

This document plans how both frontends should evolve, with particular attention to what the TUI does well that the GUI could learn from, what the GUI has that the TUI lacks, and how the shared engine should be maintained.

## Current Architecture

```
┌─────────────────────────────────────┐
│           GUI (Tauri)               │
│  React + TypeScript + Tailwind CSS  │
│  xterm.js + portable-pty            │
└──────────────┬──────────────────────┘
               │ Tauri IPC invoke
               ▼
┌─────────────────────────────────────┐
│       packetcode_lib::core           │
│  flight / git / pty / orchestrator  │
│  agent / agent_config / storage      │
│  error_classifier                   │
└──────────────┬──────────────────────┘
               │
┌──────────────┼──────────────────────┐
│               │                       │
│  commands/     │                       │
│  (Rust Tauri  │                       │
│  command      │                       │
│  handlers)    │                       │
└───────────────┼───────────────────────┘
                │
┌────────────────▼────────────────────┐
│       TUI (packetcode-tui)          │
│  Ratatui + Rust                     │
│  Main loop: poll events + render     │
│  Uses packetcode_lib::core directly  │
└─────────────────────────────────────┘
```

## What Each Frontend Has Today

### GUI strengths

- React-based views with routing (`App.tsx` view model)
- xterm.js terminal with PTY backend
- Webview-based diff and file preview
- Tailwind CSS responsive layout
- Full Tauri plugin ecosystem (fs, dialog, shell, process)
- Frontend-side streaming via `@tauri-apps/api/event`
- Full React component library (views, modals, Kanban board, etc.)

### TUI strengths

- Zero-dependency standalone binary
- True terminal-native UX (keyboard-first, no mouse required)
- Ratatui-based rendering with custom widgets
- Shared `packetcode_lib::core` engine with no translation layer
- File exports, session transcripts, diff view
- Persistent state via `PersistedState`
- Per-workspace retrospectives
- Agent detection and git context refresh

### TUI views currently implemented

From `src-tauri/src/tui/views/`:

- `agents.rs`
- `dashboard.rs`
- `flight_detail.rs`
- `flight_editor.rs`
- `sessions.rs`
- `settings.rs`

### TUI widgets currently implemented

From `src-tauri/src/tui/widgets/`:

- `diff.rs`
- `help.rs`
- `markdown.rs`
- `toast.rs`

## Shared Engine Surface

Both frontends consume `packetcode_lib::core` directly:

```
packetcode_lib::core::flight           # flight/task/milestone types
packetcode_lib::core::git             # git operations
packetcode_lib::core::pty::PtyEvent   # PTY event types
packetcode_lib::core::pty::PtyManager # PTY session management
packetcode_lib::core::orchestrator    # orchestration loop and settings
packetcode_lib::core::agent           # agent detection
packetcode_lib::core::agent_config    # agent configuration
packetcode_lib::core::storage         # PersistedState, data_dir, log_dir
packetcode_lib::core::error_classifier # CLI error classification
```

The GUI additionally wraps these via Tauri command handlers in `commands/`:

- `commands/pty.rs`
- `commands/git.rs`
- `commands/orchestration.rs`
- `commands/memory.rs`
- `commands/insights.rs`
- `commands/flight_chat.rs`
- `commands/ideation.rs`
- `commands/github.rs`
- `commands/mcp.rs`
- `commands/scaffold.rs`
- `commands/deploy.rs`
- `commands/state.rs`
- `commands/agent.rs`
- `commands/code_quality.rs`
- `commands/crashes.rs`
- `commands/fs.rs`
- `commands/statusline/`
- `commands/spec.rs`
- `commands/history.rs`
- `commands/analytics.rs`

The TUI bypasses all of these command handlers and calls `packetcode_lib::core` directly.

## Critical Implication

**The TUI has direct access to the shared engine. The GUI goes through Tauri IPC.**

This means:

1. The TUI can be more responsive — no IPC round-trip for engine operations
2. The TUI has access to internals (like `PtyManager`) that the GUI cannot reach through invoke
3. New features should be added to `packetcode_lib::core` first, then optionally exposed via Tauri commands for the GUI
4. Any bug in `packetcode_lib::core` affects both frontends simultaneously

## Known Gaps

### TUI gaps relative to GUI

| Gap                   | Severity | Notes                                                                                                              |
| --------------------- | -------- | ------------------------------------------------------------------------------------------------------------------ |
| No xterm.js terminal  | High     | TUI has its own PTY session management but no terminal emulator UI — agents just stream output to a Ratatui widget |
| No webview-based diff | Medium   | TUI has a basic diff widget but no Monaco                                                                          |
| No file explorer UI   | Medium   | TUI has session and flight views but no tree-style explorer                                                        |
| No MCP management UI  | Low      | TUI settings view exists but MCP Hub is GUI-only                                                                   |
| No Kanban board       | Medium   | TUI has flight detail but no drag-and-drop issue board                                                             |
| No Insights chat view | Medium   | TUI has session view but no AI chat UI                                                                             |
| No broadcast mode     | Low      | GUI doesn't have it either yet                                                                                     |

### GUI gaps relative to TUI

| Gap                                     | Severity | Notes                                                                              |
| --------------------------------------- | -------- | ---------------------------------------------------------------------------------- |
| No retrospective per workspace          | Medium   | TUI stores retrospectives per workspace; GUI has no equivalent                     |
| No session transcript export            | Low      | TUI can export session transcripts to file; GUI relies on PTY transcript read      |
| Agent detection is polling-based in TUI | Low      | TUI `detect_agents()` runs once at startup; GUI has a statusline polling mechanism |

## Shared Engine Concerns

### What should stay in `packetcode_lib::core`

The following should always remain shared, as both frontends need the same behavior:

- Flight/task/milestone domain model (`flight.rs`)
- Git operations (`git.rs`)
- PTY session lifecycle (`pty.rs`)
- Orchestration loop (`orchestrator.rs`)
- Agent detection and config (`agent.rs`, `agent_config.rs`)
- State persistence (`storage.rs`)
- Error classification (`error_classifier.rs`)

### What is appropriately frontend-specific

These are correctly split because the UX paradigm is fundamentally different:

- All React component code (views, modals, layouts) — GUI only
- Ratatui widgets and view code — TUI only
- Tauri command handlers — GUI only (TUI doesn't need IPC)
- xterm.js integration — GUI only
- Event streaming via `@tauri-apps/api/event` — GUI only

### Risks

1. **Divergence**: if the two frontends drift in behavior (e.g., GUI and TUI handle git conflicts differently), users will have a confusing experience switching between them
2. **Engine changes breaking TUI**: any change to `packetcode_lib::core` immediately affects the TUI without a test harness to catch breakage
3. **Missing TUI parity for new features**: new commands added to the GUI via `commands/` may never get a TUI equivalent, creating a capability gap

## Planned TUI Improvements

### Phase 1: Parity with current GUI features

Priority gaps to close in rough order:

1. **Session transcript view** — expose the PTY transcript read capability as a navigable view in the TUI
2. **Git context per workspace** — ensure the TUI refreshes and displays git state for the active workspace's project path (not just once at startup)
3. **Retrospective persistence** — verify the TUI retrospective system is correctly persisted and loaded across restarts
4. **Agent status polling in TUI** — add periodic agent status refresh to the TUI main loop rather than one-shot detection at startup

### Phase 2: TUI-specific ergonomics

1. **Leader key model** — the TUI already has a leader-key pending mechanism; expand it to support vim-style leader key sequences for all TUI navigation
2. **Pane-focused layout** — allow the TUI to display multiple views simultaneously (sessions pane + flight detail pane) using Ratatui layout primitives
3. **Search within session output** — add a search/filter widget for PTY session output

### Phase 3: Shared engine improvements that benefit both frontends

These are changes to `packetcode_lib::core` that should be designed to work for both frontends:

1. **Better error classification output** — the GUI and TUI both use `error_classifier::classify_cli_error`; improve it with more error categories
2. **Structured logging with contextual spans** — add tracing spans to the orchestration loop so both frontends can display what the orchestrator is doing
3. **Flight event stream** — if the orchestrator emits events, both frontends should be able to subscribe; consider an event bus in `packetcode_lib::core`

## GUI Lessons for TUI

The GUI does several things the TUI could adopt:

1. **Statusline polling** — the GUI polls agent status lines periodically; the TUI could surface similar information in a status bar widget
2. **Broadcast bar** — the GUI has a `BroadcastBar` component for sending one prompt to all workspace panes; the TUI could model this as a leader-key broadcast mode
3. **Activity / handoff feed** — the GUI has `MissionWorkspaceView` with activity feeds; the TUI could surface this as a scrollable event log widget
4. **Kanban drag-and-drop** — the GUI has a full Kanban issue board; the TUI can model this as a keyboard-navigable list with status transitions

## TUI Lessons for GUI

The TUI does some things the GUI could learn from:

1. **Keyboard-first modal system** — the TUI command palette is entirely keyboard-driven; the GUI command palette could improve its keyboard interaction model
2. **Session transcript export** — the TUI writes session transcripts to disk; the GUI could expose this as an explicit export action
3. **Retrospective per workspace** — TUI stores retrospectives; the GUI has no equivalent — this should be added to the GUI

## Build Order Recommendation

1. **Phase 1a**: TUI parity — git context refresh, session transcript navigation, retrospective persistence
2. **Phase 1b**: GUI parity — add retrospectives to GUI, improve command palette keyboard model
3. **Phase 2**: Shared engine — add structured event bus to `packetcode_lib::core`, add tracing spans to orchestration loop
4. **Phase 3**: TUI ergonomics — leader key expansion, pane layouts, search

## Success Criteria

- Any new `packetcode_lib::core` feature is accessible from both GUI and TUI without duplication
- TUI and GUI produce consistent results when operating on the same flights, workspaces, and agents
- New GUI features that should work in TUI are designed with TUI access in mind from the start
- The TUI is a first-class alternative frontend, not a second-class afterthought

## Non-Goals

- The TUI will never replicate the full GUI React component tree
- The TUI will not gain a webview or browser component
- The TUI will not replace the GUI as the primary frontend
