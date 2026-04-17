use std::path::Path;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolOutput};

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str {
        "Run a shell command inside the project directory. Requires user approval."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command":     { "type": "string", "description": "The command to run." },
                "timeout_ms":  { "type": "integer", "description": "Optional timeout in ms (default 120000)." }
            },
            "required": ["command"]
        })
    }

    fn requires_approval(&self, _input: &Value, _project_path: &Path) -> bool {
        true
    }

    async fn execute(&self, input: Value, project_path: &Path) -> ToolOutput {
        let Some(cmd_str) = input.get("command").and_then(|v| v.as_str()) else {
            return ToolOutput::err("missing required field: command");
        };
        let timeout_ms = input
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000);

        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(cmd_str);
            c
        } else {
            let mut c = Command::new("bash");
            c.arg("-lc").arg(cmd_str);
            c
        };
        cmd.current_dir(project_path);

        let run = cmd.output();
        let output = match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            run,
        )
        .await
        {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return ToolOutput::err(format!("spawn failed: {}", e)),
            Err(_) => return ToolOutput::err(format!("timed out after {}ms", timeout_ms)),
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let code = output.status.code().unwrap_or(-1);

        let mut body = String::new();
        if !stdout.is_empty() {
            body.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !body.is_empty() {
                body.push_str("\n--- stderr ---\n");
            }
            body.push_str(&stderr);
        }
        if body.is_empty() {
            body = format!("(no output, exit {})", code);
        } else {
            body.push_str(&format!("\n(exit {})", code));
        }

        if output.status.success() {
            ToolOutput::ok(body)
        } else {
            ToolOutput::err(body)
        }
    }
}
