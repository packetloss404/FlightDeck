use std::process::Command;

use crate::core::shared::hide_window;

/// Check if a CLI agent command is installed and available on PATH.
/// On Windows, also checks for .cmd wrapper (e.g., claude.cmd, codex.cmd),
/// and uses CREATE_NO_WINDOW so the `where` probes don't flash a console window.
pub fn detect_agent(command: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Try the base command
        let mut cmd = Command::new("where");
        cmd.arg(command);
        hide_window(&mut cmd);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                return true;
            }
        }
        // Try .cmd extension
        let cmd_name = format!("{}.cmd", command);
        let mut cmd = Command::new("where");
        cmd.arg(&cmd_name);
        hide_window(&mut cmd);
        if let Ok(output) = cmd.output() {
            return output.status.success();
        }
        false
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hide_window; // suppress unused-import warning on non-Windows
        Command::new("which")
            .arg(command)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
