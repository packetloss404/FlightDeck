//! Tool abstraction: in-process capabilities the native agent can invoke.
//!
//! Each tool declares a JSON Schema for its input, a permission predicate
//! (does this invocation need human approval?), and an async `execute` that
//! produces a string result (optionally flagged as an error).

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
pub mod write;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::provider::ToolSchema;

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: false }
    }
    pub fn err(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: true }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;

    /// Return `true` if this invocation requires user approval before running.
    /// Safe, local, read-only operations should return false so the loop runs
    /// smoothly; dangerous side effects (bash, writes outside the project dir)
    /// return true so the runner can park the task on an approval gate.
    fn requires_approval(&self, _input: &Value, _project_path: &Path) -> bool {
        false
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput;

    fn to_schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    /// Build a registry pre-populated with the v0.2 default tools:
    /// read, write, edit, bash, grep, glob.
    pub fn defaults() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(read::ReadTool));
        reg.register(Arc::new(write::WriteTool));
        reg.register(Arc::new(edit::EditTool));
        reg.register(Arc::new(bash::BashTool));
        reg.register(Arc::new(grep::GrepTool));
        reg.register(Arc::new(glob::GlobTool));
        reg
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Return the schema for each tool whose name is in `allowlist`. An empty
    /// allowlist means "all registered tools".
    pub fn schemas_for(&self, allowlist: &[String]) -> Vec<ToolSchema> {
        if allowlist.is_empty() {
            return self.tools.values().map(|t| t.to_schema()).collect();
        }
        allowlist
            .iter()
            .filter_map(|name| self.tools.get(name).map(|t| t.to_schema()))
            .collect()
    }
}

/// Resolve a caller-supplied path against the project root. Rejects absolute
/// traversal outside the project. Returns the canonicalized absolute path
/// when inside the project, or `Err(message)` otherwise.
pub fn resolve_in_project(raw: &str, project_path: &Path) -> Result<std::path::PathBuf, String> {
    let candidate = if std::path::Path::new(raw).is_absolute() {
        std::path::PathBuf::from(raw)
    } else {
        project_path.join(raw)
    };
    // Don't require existence — write() creates new files. Canonicalize the
    // parent dir instead of the target itself, then append the filename.
    let (parent, filename) = match candidate.parent().zip(candidate.file_name()) {
        Some(pair) => pair,
        None => return Err(format!("invalid path: {}", raw)),
    };
    let parent_canon = parent
        .canonicalize()
        .map_err(|e| format!("resolve {:?}: {}", parent, e))?;
    let project_canon = project_path
        .canonicalize()
        .map_err(|e| format!("resolve project {:?}: {}", project_path, e))?;
    if !parent_canon.starts_with(&project_canon) {
        return Err(format!("path escapes project dir: {}", raw));
    }
    Ok(parent_canon.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_contains_all_six_tools() {
        let reg = ToolRegistry::defaults();
        for name in ["read", "write", "edit", "bash", "grep", "glob"] {
            assert!(reg.get(name).is_some(), "missing tool: {}", name);
        }
    }

    #[test]
    fn schemas_for_empty_returns_all() {
        let reg = ToolRegistry::defaults();
        assert_eq!(reg.schemas_for(&[]).len(), 6);
    }

    #[test]
    fn schemas_for_filters_by_allowlist() {
        let reg = ToolRegistry::defaults();
        let schemas = reg.schemas_for(&["read".to_string(), "write".to_string()]);
        assert_eq!(schemas.len(), 2);
    }
}
