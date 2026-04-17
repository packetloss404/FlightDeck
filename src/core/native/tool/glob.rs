use std::path::Path;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolOutput};

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }

    fn description(&self) -> &str {
        "Match files by glob pattern (e.g. `src/**/*.rs`). Runs relative to the project directory."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern":     { "type": "string" },
                "max_results": { "type": "integer", "default": 500 }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput {
        let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: pattern");
        };
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(500) as usize;

        let full_pattern = project_path.join(pattern);
        let pattern_str = match full_pattern.to_str() {
            Some(s) => s.to_string(),
            None => return ToolOutput::err("pattern contains invalid utf-8"),
        };

        let paths = match glob::glob(&pattern_str) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(format!("invalid pattern: {}", e)),
        };

        let mut results = Vec::new();
        for entry in paths {
            if results.len() >= max_results {
                break;
            }
            if let Ok(path) = entry {
                let rel = path
                    .strip_prefix(project_path)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                results.push(rel);
            }
        }

        if results.is_empty() {
            return ToolOutput::ok("(no matches)");
        }
        ToolOutput::ok(results.join("\n"))
    }
}
