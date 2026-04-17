use std::path::Path;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{resolve_in_project, Tool, ToolOutput};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str { "write" }

    fn description(&self) -> &str {
        "Create or overwrite a text file in the project. Overwrites without confirmation — use `edit` for in-place changes."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path":    { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    fn requires_approval(&self, input: &Value, project_path: &Path) -> bool {
        // Writes inside the project dir are routine; writes elsewhere need a gate.
        let Some(raw) = input.get("path").and_then(|v| v.as_str()) else {
            return true;
        };
        resolve_in_project(raw, project_path).is_err()
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput {
        let Some(raw_path) = input.get("path").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: path");
        };
        let Some(content) = input.get("content").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: content");
        };

        let resolved = match resolve_in_project(raw_path, project_path) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };

        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolOutput::err(format!("mkdir failed: {}", e));
            }
        }
        if let Err(e) = tokio::fs::write(&resolved, content).await {
            return ToolOutput::err(format!("write failed: {}", e));
        }
        ToolOutput::ok(format!("wrote {} bytes to {}", content.len(), raw_path))
    }
}
