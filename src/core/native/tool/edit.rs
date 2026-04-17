use std::path::Path;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{resolve_in_project, Tool, ToolOutput};

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }

    fn description(&self) -> &str {
        "Exact string replacement in a file. Fails if `old_string` is not unique unless `replace_all` is true."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path":        { "type": "string" },
                "old_string":  { "type": "string" },
                "new_string":  { "type": "string" },
                "replace_all": { "type": "boolean", "default": false }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn requires_approval(&self, input: &Value, project_path: &Path) -> bool {
        let Some(raw) = input.get("path").and_then(|v| v.as_str()) else {
            return true;
        };
        resolve_in_project(raw, project_path).is_err()
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput {
        let Some(raw_path) = input.get("path").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: path");
        };
        let Some(old_string) = input.get("old_string").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: old_string");
        };
        let Some(new_string) = input.get("new_string").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: new_string");
        };
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let resolved = match resolve_in_project(raw_path, project_path) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(format!("read failed: {}", e)),
        };

        let occurrences = content.matches(old_string).count();
        if occurrences == 0 {
            return ToolOutput::err("old_string not found in file");
        }
        if occurrences > 1 && !replace_all {
            return ToolOutput::err(format!(
                "old_string matches {} locations — pass replace_all=true or extend context to make it unique",
                occurrences
            ));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        if let Err(e) = tokio::fs::write(&resolved, &new_content).await {
            return ToolOutput::err(format!("write failed: {}", e));
        }

        ToolOutput::ok(format!(
            "edited {}: replaced {} occurrence{}",
            raw_path,
            if replace_all { occurrences } else { 1 },
            if (if replace_all { occurrences } else { 1 }) == 1 { "" } else { "s" }
        ))
    }
}
