use std::path::Path;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{resolve_in_project, Tool, ToolOutput};

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }

    fn description(&self) -> &str {
        "Read a text file from the project. Returns the file contents with 1-based line numbers."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Project-relative or absolute path inside the project." },
                "offset": { "type": "integer", "description": "1-based starting line (optional)." },
                "limit":  { "type": "integer", "description": "Max lines to return (optional, default 2000)." }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput {
        let Some(raw_path) = input.get("path").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: path");
        };
        let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;

        let resolved = match resolve_in_project(raw_path, project_path) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(format!("read failed: {}", e)),
        };

        let mut out = String::new();
        for (i, line) in content.lines().enumerate().skip(offset - 1).take(limit) {
            out.push_str(&format!("{:>6}→{}\n", i + 1, line));
        }
        ToolOutput::ok(out)
    }
}
