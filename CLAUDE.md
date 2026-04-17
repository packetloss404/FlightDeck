# CLAUDE.md

Orientation for Claude Code sessions working on this repository.

## What this is

FlightDeck is a Rust TUI (Ratatui + Crossterm) that acts as a native coding agent — streaming chat with an LLM against the current working directory, in-process tools, OS-keyring-backed API keys. It also carries a Flight/Milestone/Task orchestration layer as an overlay (entered via `Ctrl+F`) for structured multi-task missions run across the native agent or external CLI agents (Claude Code, Codex, Gemini, OpenCode) via PTY.

Originally forked from [PacketCode](https://github.com/packetloss404/PacketCode) at `8f2fdff`. The orchestration machinery comes from that lineage; the native-agent stack under `src/core/native/` is new in FlightDeck v0.2.

Single binary, no Node/Tauri/sidecar. `cargo build --release` → ~9 MB exe.

## Layout

```
src/
├─ main.rs                 150 lines  — event loop, terminal setup, tracing init
├─ app.rs                  3300 lines — App state, key dispatch, event pumps, orchestrator_tick
├─ command_palette.rs      360 lines  — fuzzy palette + CommandEntry registry
├─ theme.rs                160 lines  — 5 builtin themes, user-theme loading
├─ core/
│   ├─ agent_config.rs     AgentConfig with AgentKind::{Pty, Native}; builtin catalogue
│   ├─ flight.rs           Flight/Milestone/Task data model + status enums
│   ├─ orchestrator.rs     tick() → Vec<TaskDispatch>; Flight launch/complete lifecycle
│   ├─ pty.rs              PtyManager, PtyEvent (Output/Exit); also the event channel reused by native
│   ├─ provider_config.rs  ProviderConfig + keyring get/set + test_connection (blocking reqwest)
│   ├─ storage.rs          JSON persistence under ~/.packetcode/ (legacy path from fork)
│   ├─ agent.rs            detect_agent() — which external CLIs are installed
│   ├─ git.rs              branch/status/pull/push helpers
│   ├─ error_classifier.rs retryable-error taxonomy
│   ├─ workspace.rs        multi-repo workspace stub
│   ├─ shared.rs           home_dir, lock_mutex, SKIP_DIRS
│   └─ native/             ★ v0.2 — in-process coding agent
│       ├─ conversation.rs  Message, MessagePart::{Text, Reasoning, ToolCall, ToolResult}, TokenUsage
│       ├─ provider/
│       │   ├─ mod.rs       Provider trait, ProviderEvent, StopReason, ProviderError, ToolSchema
│       │   └─ anthropic.rs SSE streaming client (parses content_block_{start,delta,stop}, message_delta, etc.)
│       ├─ tool/
│       │   ├─ mod.rs       Tool trait, ToolRegistry, resolve_in_project() path sandbox
│       │   └─ {read, write, edit, bash, grep, glob}.rs
│       ├─ runner.rs        Flight-mode runner — one-shot task → emit PtyEvents → exit
│       ├─ chat.rs          Interactive chat driver — chat_turn(&mut Conversation, ...)
│       ├─ compaction.rs    Token-budget pruner (180k budget / 40k protected tail)
│       ├─ safety.rs        DoomLoopDetector (3× identical tool call → halt)
│       └─ mock_provider.rs #[cfg(test)] — scripted ProviderEvent replay + run_mock_loop helper
├─ views/
│   ├─ agent.rs             ★ default view — chat UI (header + transcript + input bar)
│   ├─ providers.rs         list/add/edit/delete providers, test API keys
│   ├─ dashboard.rs         ┐
│   ├─ flight_detail.rs     │ Flight Deck overlay (entered via Ctrl+F)
│   ├─ flight_editor.rs     │
│   ├─ sessions.rs          │ — tabbed PTY output; also renders native agent output
│   ├─ agents.rs            │ — catalogue of builtins and custom external CLI agents
│   └─ settings.rs          ┘
└─ widgets/
    ├─ diff.rs              diff parser + render
    ├─ help.rs              context-sensitive keybinding overlay (`?`)
    ├─ toast.rs             timed notification queue
    ├─ markdown.rs          minimal markdown→spans
    └─ mod.rs
```

## Architectural invariants

Preserve these unless the user explicitly asks you to redesign them — they're load-bearing.

1. **One dispatch fork.** `Orchestrator::tick()` in `src/core/orchestrator.rs` is the single place where flights become work. It returns `Vec<TaskDispatch>` where `TaskDispatch::{Pty, Native}` splits on agent kind. `app.rs::orchestrator_tick()` matches and routes to either `PtyManager::create_session` or `native::runner::spawn`. New agent backends are new variants, not new dispatch points.

2. **Event channel reuse.** Native agents emit `PtyEvent::{Output, Exit}` into the same `mpsc::Sender<PtyEvent>` the PTY code uses. `PtyManager::event_tx()` exposes a clone of the sender. The Sessions view renders text from `SessionBuffer::output` without knowing if it came from a PTY or an LLM stream. Don't introduce a parallel event type for native output — extend the existing one.

3. **Secrets are keyring-only.** API keys never appear in `providers.json`. They live in the OS keyring under `service = "flightdeck"`, `account = "provider:<id>"`. In-memory copies use `Zeroizing<String>` (from the `zeroize` crate) and `ProviderForm::drop` zeroes its ephemeral `api_key: String`. Don't add `pub api_key: String` fields to `ProviderConfig` or serialize keys anywhere.

4. **AgentConfig is additive.** The struct has `kind: AgentKind::{Pty(PtyAgentSpec), Native(NativeAgentSpec)}`. PTY-specific fields (`command`, `default_args`, `status_patterns`, `approval_actions`) live on `PtyAgentSpec`. Code that accesses PTY fields uses `agent.pty_spec() -> Option<&PtyAgentSpec>` and must handle the `None` case for native agents. Don't hoist PTY-specific fields back to the top-level AgentConfig.

5. **Approval channel.** `ChatTurnRequest.approvals: Option<mpsc::UnboundedSender<ApprovalQuery>>`. When `tool.requires_approval()` returns true and `approvals` is `Some`, the chat driver sends an `ApprovalQuery { responder: oneshot::Sender<bool> }` and awaits the oneshot with a 300s timeout. `None` is the test/headless fallback (auto-approves). `app.rs` drains the approval channel in `poll_chat_events`, shows a banner overlay via `render_approval_banner`, intercepts `y/a/n/d/Esc` in `handle_agent_view_key` before any other input handling.

6. **Tool sandbox.** `tool::mod::resolve_in_project(raw_path, project_path)` canonicalizes the parent directory and rejects anything that escapes the project root. Every file-touching tool calls it. Don't write a new tool that bypasses this — build on top.

## Native-agent concurrency model

- FlightDeck's main loop is synchronous (crossterm blocking polls, Ratatui immediate-mode render). Tokio is only used inside the native-agent runners.
- When a native task kicks off (either via `TaskDispatch::Native` in `orchestrator_tick` or via `launch_chat_turn` from the Agent view), we spawn a **dedicated OS thread** that owns a **current-thread tokio runtime** and runs the async driver with `runtime.block_on(...)`. See `runner::spawn` and `chat::spawn_chat_turn`.
- The runner emits `PtyEvent`s over a **std::sync::mpsc::Sender** (clone of the one the TUI already drains).
- `ChatEvent`s go over a **tokio::sync::mpsc::UnboundedSender**; the main loop drains via `try_recv` each tick.
- The shared `Conversation` is held behind `Arc<std::sync::Mutex<Conversation>>`. The chat driver locks briefly to snapshot it into a `ProviderRequest`, streams without the lock held, then re-locks to push the assistant/tool-result messages. The UI locks just long enough to render.

If you need long-running parallelism, add another spawned thread + runtime; don't try to bolt a multi-threaded runtime onto the TUI loop.

## Provider + tool extension points

**Adding a new provider** (OpenAI, Google, local Ollama, etc.):
1. Add a variant to `ProviderKind` in `src/core/provider_config.rs` (keep snake_case serde tag).
2. Extend `test_connection()` to dispatch on the new variant.
3. Implement the `Provider` trait (`async fn stream`) in a new module under `src/core/native/provider/`. Normalize the provider's wire format into `ProviderEvent`s.
4. Either hardcode a new `AnthropicProvider::new(...)` analog in `app.rs::launch_chat_turn` (cheap) or introduce a small factory keyed on `ProviderKind` (cleaner). The plan has factory as v0.3 work.

**Adding a new tool:**
1. New file `src/core/native/tool/<name>.rs` with a zero-sized `<Name>Tool` struct implementing `Tool`.
2. JSON schema in `input_schema()` is what the model sees in `tools=[...]` — keep it clean and well-described.
3. Return `true` from `requires_approval()` if the tool has non-idempotent side effects; the runner will gate it.
4. Register in `ToolRegistry::defaults()` in `src/core/native/tool/mod.rs`.
5. If default-allowlisted for `flightdeck_native`, add the tool id to the allowlist in `agent_config.rs::flightdeck_native()`.

## Style conventions

- **Minimal comments.** Only comment the WHY when non-obvious (hidden constraints, workarounds, design choices that would surprise a reader). Never comment the WHAT — identifiers and types do that.
- **No defensive backwards-compat shims.** Pre-v1.0 fork. If a field changes shape, JSON from the old shape can fail to parse and fall back to defaults (`storage::load_state` already does this; `provider_config::load_providers` does too).
- **No feature flags for in-progress work.** Ship it or leave it out. The exception: `#[cfg(test)]` test-only modules (mock_provider).
- **Errors are strings for UI-facing code, `Result<_, ProviderError>` for the provider layer.** Most of the codebase uses `Result<(), String>` — match that unless you're in a typed-error subsystem.
- **Secrets wear `Zeroizing<String>`** from the `zeroize` crate, especially in `AnthropicProvider::api_key` and `ProviderForm::api_key` (+ Drop impl).
- **Async only where needed.** The main loop is sync. `tokio::spawn` only happens inside the dedicated runner thread's current-thread runtime.

## Running / testing

```bash
cargo check              # fast compile check
cargo test               # 73 tests, deterministic, no network
cargo build --release    # ~9 MB exe at target/release/flightdeck(.exe)
```

Test suite is hermetic:
- `MockProvider` (in `mock_provider.rs`) replays scripted `ProviderEvent`s — use it for end-to-end agent-loop tests.
- Tools are tested against temp dirs (no tempfile crate; we use `std::env::temp_dir()` + pid + guards).
- Keyring code is not unit-tested against a real keyring; rely on integration smoke tests for that.
- SSE parsing is unit-tested directly against byte fixtures in `provider/anthropic.rs`.

For LLM-calling manual smoke tests:
1. `ANTHROPIC_API_KEY` in the OS keyring under `service=flightdeck`, `account=provider:<id>` (easiest: use the in-TUI "Add Provider" flow).
2. `./target/release/flightdeck` → Agent view opens → type a prompt.
3. To test Flight-mode, `Ctrl+F` → create a Flight assigned to the `flightdeck-native` agent → Launch.

## Common tasks

**Changing top-level views / keybindings:** `app.rs::handle_key` does the dispatch. View-specific handlers are `handle_<name>_key`. The `AppView` enum at the top controls dispatch. `render_nav_bar` shows the top strip; `command_palette::build_commands` owns the palette registry.

**Adding a builtin AgentConfig:** `src/core/agent_config.rs::impl AgentConfig` has a constructor per builtin (`claude_code()`, `opencode()`, `codex()`, `gemini()`, `terminal()`, `flightdeck_native()`). Add a new one and include it in `builtins()`. The `builtins()` order is user-facing (it's the order they appear in the Agents view); native should stay first.

**Inspecting state:** It persists as readable JSON at `~/.packetcode/state.v1.json` and `~/.packetcode/providers.json`. Log files are under `~/.packetcode/logs/packetcode-tui.log.YYYY-MM-DD` (rotating daily).

## Known follow-ups (not in v0.2)

- Data-dir rename `~/.packetcode/` → `~/.flightdeck/` (legacy path still used — see `storage::data_dir`).
- Log filename rename `packetcode-tui.log` → `flightdeck.log` (in `main.rs::init_file_tracing`).
- OpenAI and Google providers behind the `Provider` trait.
- MCP client support.
- LSP, webfetch, multiedit, apply_patch tools.
- Session fork/archive semantics (per OpenCode's model).
- Cancellation channel for in-flight chat turns (`AgentAction::InterruptTurn` is currently best-effort — it clears `running` locally but doesn't kill the spawned thread; the model finishes its current streaming call before exiting).
- Refactor `runner::run` to inject the provider (currently hardcodes `AnthropicProvider::new`) so Flight-mode tests can use `MockProvider` the same way chat-mode tests already do.
- Per-conversation system-prompt customization in the Agent view.

## When asked to work on Flight/Mission orchestration

The data model is `Flight { milestones: Vec<Milestone { tasks: Vec<Task> } }`. Task state machine: `Pending → Queued → Running → (ApprovalNeeded ↔ Running)? → Done | Failed | Cancelled`. Milestone state machine: `Pending → Active → Done | Failed`. Flight state: `Draft → Ready → Active → (Paused ↔ Review)? → Done | Failed | Cancelled`.

The orchestrator is deliberately stateless beyond `running_tasks: HashMap<task_id, RunningTask>`, `active_flight_ids: HashSet<String>`, `paused_at_milestone: HashMap<flight_id, milestone_id>`. Everything else is derived from the persisted `Flight` collection. Keep it that way.
