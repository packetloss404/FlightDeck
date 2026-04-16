# FlightDeck

Terminal-native mission control for AI coding agents. A standalone Ratatui TUI that orchestrates PTY-based CLI agents (Claude Code, Codex, Gemini, OpenCode) through the same Flight / Milestone / Task model as its sibling project [PacketCode](https://github.com/packetloss404/PacketCode).

## Status

Forked from PacketCode @ `8f2fdff`. This is the v0.1 extraction — the TUI now has its own repo and can evolve on its own cadence.

## Why a separate product

PacketCode is a desktop IDE with API-native agents, a cost dashboard, vision inputs, and many surfaces. FlightDeck is focused on one thing: making mission-based orchestration of CLI agents feel great in a terminal. Keeping it separate lets the TUI breathe and the GUI move fast without either blocking the other.

## Build & run

```bash
cargo build --release
./target/release/flightdeck
```

Rust 1.70+ recommended. No Node, no Tauri, no system deps beyond what `portable-pty` needs (on Windows the ConPTY subsystem, on Unix a POSIX pty).

## Features

- **Dashboard** — at-a-glance view of all active missions, their status, and which agents are currently busy.
- **Mission detail** — milestones, tasks, approval state, coordination feed.
- **Mission editor** — inline form to build a mission's structure.
- **Sessions** — tabbed PTY sessions for live agent output.
- **Themes** — 5 built-in (`default_dark`, `catppuccin_mocha`, `gruvbox_dark`, `nord`, `tokyonight`); user themes via `~/.flightdeck/themes/*.json`.
- **Command palette** — vim-style `:command` search and execution.
- **Keyboard-driven** — everything is reachable without a mouse.

## Keybindings

Press `?` at any time for the context-sensitive help overlay. Baseline:

- `Tab` / `Shift+Tab` — cycle focus
- `:` — command palette
- `Esc` — back / close overlays
- `1`–`5` — switch top-level views
- `Ctrl+C` — quit

## Data location

State lives under the OS data dir (via the `dirs` crate). Flights, agents, settings, and UI state are persisted as JSON. FlightDeck reads and writes its own namespace; it does not currently share state with PacketCode.

## License

Apache-2.0 — see [LICENSE](./LICENSE).
