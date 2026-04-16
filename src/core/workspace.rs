use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridPosition {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePane {
    pub id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub grid_position: GridPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub agents: Vec<String>,
    pub panes: Vec<WorkspacePane>,
    pub project_path: String,
    pub prompt: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub status: String,
    pub bypass_permissions: Option<bool>,
    pub model_overrides: Option<std::collections::HashMap<String, Option<String>>>,
    pub effort_overrides: Option<std::collections::HashMap<String, Option<String>>>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub remote_project_path: Option<String>,
}
