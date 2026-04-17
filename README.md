# FlightDeck

A terminal-native coding agent in Rust.

FlightDeck opens to a chat with an in-process AI coding agent that works against your current directory — read/write/edit files, run commands, grep, glob, all inside a Ratatui TUI. When you want structured multi-task work across multiple agents, `Ctrl+F` drops you into the Flight Deck overlay: missions composed of milestones and tasks, each assigned to an agent (the in-process native agent, or external CLIs like Claude Code / Codex / Gemini / OpenCode via PTY).

**Status:** v0.2 — forked from [PacketCode](https://github.com/packetloss404/PacketCode) at `8f2fdff`. The TUI now owns its own LLM stack and can be used as a standalone coding agent; Flight orchestration is an overlay on top.

## What it does

**As a coding agent** (the default surface):
- Direct streaming chat with Anthropic models (Claude Sonnet/Opus/Haiku); OpenAI and Google slot into the Provider trait for a future release.
- Six in-process tools: `read`, `write`, `edit`, `bash`, `grep`, `glob`. All sandboxed to the project directory.
- Approval gates for sensitive operations — `bash` and writes outside the project dir prompt you before running.
- Safety rails: token-budget compaction at 180k/40k and doom-loop detection (3× identical tool call → halt and ask).

**As a mission orchestrator** (the Flight Deck overlay):
- **Flights** composed of **Milestones** of **Tasks**, each routed to a chosen agent (native or PTY-backed CLI).
- Up to N concurrent sessions (`max_parallel_sessions`); milestone gating pauses between milestones for human review.
- Tabbed PTY sessions with live output, approval-keystroke mapping for each CLI's prompt style, doom-loop detection and diff extraction across the output stream.

**Operational niceties:**
- Fuzzy command palette (`Ctrl+P`), 5 built-in themes + user themes from `~/.flightdeck/themes/`, tracing to rotating log files, persistent state across restarts.

## Quickstart

```bash
# Build
cargo build --release

# Run
./target/release/flightdeck        # Unix
.\target\release\flightdeck.exe    # Windows
```

Requires Rust 1.70+. No Node, no Tauri, no system dependencies beyond what `portable-pty` needs (ConPTY on Windows, POSIX pty elsewhere) and a working OS keyring (Windows Credential Manager / macOS Keychain / secret-service on Linux).

### First run

1. FlightDeck opens into the **Agent** view. The header shows "No provider configured".
2. Press `Ctrl+P`, type `provider`, hit Enter on "Add LLM Provider".
3. Fill in: display name ("Anthropic"), default model (`claude-sonnet-4-6`), paste your Anthropic API key.
4. Hit `Tab` to Save. The key goes to the OS keyring; a metadata record lands in `providers.json`.
5. Back in the Agent view, type a request and press Enter.

API keys are **never** stored in the JSON registry — only in the keyring, keyed `service=flightdeck, account=provider:<id>`.

## Keybindings

**Global**
- `Ctrl+C` — quit
- `Ctrl+P` — command palette (fuzzy-search everything)
- `Ctrl+F` — toggle Agent view ↔ Flight Deck overlay
- `?` — help overlay (context-sensitive)
- `Esc` — back / close overlays
- `1` `2` `3` `4` — Agent / Flight Deck / Sessions / Settings (when no text input is focused)

**Agent view**
- Type into the input bar; `Enter` sends.
- `Ctrl+L` — clear conversation
- `Ctrl+C` while a turn is running — interrupt (best-effort)
- `Tab` — move focus to transcript; `j`/`k` or arrows scroll; `i` or Tab back to input
- During an approval prompt: `y`/`a` approve, `n`/`d`/`Esc` deny

**Flight Deck overlay**
- Dashboard: `j`/`k` navigate, `Enter` open, `c` create, `e` edit, `l` launch
- Sessions: tabbed PTY output, search within session with `/`
- `Esc` on a detail/editor view returns to Dashboard

## Architecture

```
main.rs ── event loop (50ms poll)
   │
   ├─ app.rs ── App state (~3300 lines; flights, agents, providers, agent view, PTY manager, orchestrator)
   │    │
   │    ├─ views/ ── rendering
   │    │    ├─ agent.rs       (new default — chat UI)
   │    │    ├─ providers.rs   (add/edit/test API keys)
   │    │    ├─ dashboard.rs   ┐
   │    │    ├─ flight_*.rs    ├─ Flight Deck overlay
   │    │    ├─ sessions.rs    │
   │    │    ├─ agents.rs      ┘
   │    │    └─ settings.rs
   │    │
   │    ├─ widgets/ ── shared UI (diff, help, toast, markdown)
   │    └─ command_palette.rs
   │
   └─ core/
        ├─ agent_config.rs  — AgentKind::{Pty,Native}; builtin catalogue
        ├─ flight.rs        — Flight/Milestone/Task data model
        ├─ orchestrator.rs  — tick() → Vec<TaskDispatch::{Pty,Native}>
        ├─ pty.rs           — PtyManager, PtyEvent (used by PTY agents and reused by native)
        ├─ provider_config.rs — ProviderConfig + keyring helpers + Anthropic test_connection
        └─ native/
             ├─ conversation.rs  — Message with parts {Text, Reasoning, ToolCall, ToolResult}
             ├─ provider/
             │    ├─ mod.rs         — Provider trait, ProviderEvent
             │    └─ anthropic.rs   — SSE streaming client
             ├─ tool/
             │    ├─ mod.rs   — Tool trait, ToolRegistry, project-sandboxed path resolver
             │    └─ {read, write, edit, bash, grep, glob}.rs
             ├─ runner.rs     — Flight-mode runner: one-shot task → exit
             ├─ chat.rs       — Interactive chat driver + spawn_chat_turn
             ├─ compaction.rs — Token-budget pruner
             └─ safety.rs     — Doom-loop detector
```

**Key design seams:**

- Both dispatch paths (PTY and native) emit events on the same `mpsc::Sender<PtyEvent>` channel, so the Sessions view renders native-agent output identically to PTY output.
- The orchestrator has exactly one scheduling decision (`tick()`), which returns a `TaskDispatch` enum. Adding a new agent backend is one variant, one match arm.
- API keys live only in the OS keyring. In-memory copies are wrapped in `Zeroizing<String>` and cleared on drop (including the ephemeral buffer used by the provider add/edit form).
- Conversation compaction and doom-loop detection live on both the Flight-mode runner and the chat driver — the safety layer is the same in both execution modes.

## Configuration

State lives under `~/.packetcode/` (inherited data dir from the fork; will migrate to `~/.flightdeck/` in a later release):

- `state.v1.json` — flights, agents, settings, UI state, retrospectives.
- `providers.json` — provider metadata (no secrets).
- `logs/packetcode-tui.log.YYYY-MM-DD` — tracing output.

Environment:
- `RUST_LOG=debug` — verbose tracing (default `info`).

## Develop

```bash
cargo check             # fast type-check
cargo test              # full test suite (73 tests; deterministic, no network calls)
cargo build --release   # optimized binary
```

All LLM-calling tests use `MockProvider` (`src/core/native/mock_provider.rs`) — no network access needed.

## License

Apache-2.0 — see [LICENSE](./LICENSE).
