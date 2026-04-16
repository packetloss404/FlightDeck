# Changelog

## [0.1.0] — 2026-04-16

Initial public release. Forked from PacketCode `8f2fdff3e5732489105b053bcc86e6af66e1ce36` on 2026-04-16, extracted into its own repository.

### Included from PacketCode

- Ratatui-based TUI with Dashboard, Mission Detail, Mission Editor, Sessions, Agents, Settings views
- Command palette (`:` trigger) with fuzzy command search
- 5 built-in themes plus user-theme loading from `~/.flightdeck/themes/`
- PTY-based CLI agent orchestration (Claude Code, Codex, Gemini, OpenCode)
- Flight / Milestone / Task data model with persisted state
- Help overlay (`?`) and toast notifications
- Markdown and diff widgets

### Changed from PacketCode source

- Binary renamed `packetcode-tui` → `flightdeck`
- Package renamed accordingly
- Imports rewritten from `packetcode_lib::core::*` → `crate::core::*`
- Core modules forked (not shared via crate) — FlightDeck owns its own copy of `pty`, `storage`, `flight`, `agent`, `agent_config`, `git`, `orchestrator`, `shared`, `workspace`, `error_classifier`
