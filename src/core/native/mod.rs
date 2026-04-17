//! Native coding-agent runtime: in-process LLM client + tool-use loop.
//!
//! Entry point is `runner::spawn`, which takes a `NativeRunRequest`, an API
//! key + registry (via `RunnerConfig`), and an `mpsc::Sender<PtyEvent>` that
//! receives output events rendered into the existing Sessions view.

pub mod chat;
pub mod compaction;
pub mod conversation;
pub mod provider;
pub mod runner;
pub mod safety;
pub mod tool;

#[cfg(test)]
pub mod mock_provider;

use serde::{Deserialize, Serialize};

/// Request handed to the native runner when the orchestrator wants to start a
/// task against a `Native` agent. Analog of `TaskSpawnRequest` for the PTY path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeRunRequest {
    pub flight_id: String,
    pub milestone_id: String,
    pub task_id: String,
    pub agent_config_id: String,
    pub provider_id: String,
    pub model: String,
    pub tool_allowlist: Vec<String>,
    pub system_prompt_override: Option<String>,
    pub prompt: String,
    pub project_path: String,
}
