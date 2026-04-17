use std::path::{Path, PathBuf};

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use super::{Tool, ToolOutput};

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }

    fn description(&self) -> &str {
        "Regex search across files in the project. Returns matching `path:line:text` entries."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern":       { "type": "string", "description": "Rust regex pattern." },
                "path":          { "type": "string", "description": "Project-relative dir to search (default: project root)." },
                "case_insensitive": { "type": "boolean", "default": false },
                "max_results":   { "type": "integer", "default": 200 }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput {
        let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: pattern");
        };
        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(200) as usize;
        let subdir = input.get("path").and_then(|v| v.as_str()).unwrap_or("");

        let search_root = if subdir.is_empty() {
            project_path.to_path_buf()
        } else {
            project_path.join(subdir)
        };

        let regex = match regex::RegexBuilder::new(pattern)
            .case_insensitive(case_insensitive)
            .build()
        {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(format!("invalid regex: {}", e)),
        };

        let mut matches = Vec::new();
        if let Err(e) = walk_and_match(&search_root, project_path, &regex, &mut matches, max_results).await {
            return ToolOutput::err(e);
        }

        if matches.is_empty() {
            return ToolOutput::ok("(no matches)");
        }
        ToolOutput::ok(matches.join("\n"))
    }
}

async fn walk_and_match(
    root: &Path,
    project_root: &Path,
    regex: &Regex,
    matches: &mut Vec<String>,
    max_results: usize,
) -> Result<(), String> {
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Skip common junk directories and hidden files.
            if SKIP_DIRS.contains(&file_name.as_str()) {
                continue;
            }
            let meta = match entry.file_type().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                match_file(&path, project_root, regex, matches, max_results).await;
                if matches.len() >= max_results {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

async fn match_file(
    path: &Path,
    project_root: &Path,
    regex: &Regex,
    matches: &mut Vec<String>,
    max_results: usize,
) {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return;
    };
    let rel = path
        .strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    for (lineno, line) in content.lines().enumerate() {
        if regex.is_match(line) {
            matches.push(format!("{}:{}:{}", rel, lineno + 1, line));
            if matches.len() >= max_results {
                return;
            }
        }
    }
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".venv",
    "__pycache__",
];
