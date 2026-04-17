use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    CodeEdit,
    CodeReview,
    Testing,
    Research,
    Shell,
    Refactor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUsePattern {
    pub pattern: String,
    pub tool: String,
    pub file_group: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatusPatterns {
    pub approval: Vec<String>,
    pub thinking: Vec<String>,
    pub tool_use: Vec<ToolUsePattern>,
    pub idle: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentApprovalActions {
    pub approve: String,
    pub deny: String,
    pub abort: String,
}

/// PTY-backed agent: a shelled-out CLI binary whose output we parse with regex
/// and whose approval prompts we answer with keystrokes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyAgentSpec {
    pub command: String,
    pub default_args: Vec<String>,
    pub status_patterns: AgentStatusPatterns,
    pub approval_actions: AgentApprovalActions,
}

/// Native agent: FlightDeck talks to an LLM provider directly and runs tools in-process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeAgentSpec {
    /// References a `ProviderConfig.id`.
    pub provider_id: String,
    pub model: String,
    pub tool_allowlist: Vec<String>,
    pub system_prompt_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentKind {
    Pty(PtyAgentSpec),
    Native(NativeAgentSpec),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    pub capabilities: Vec<AgentCapability>,
    pub icon: String,
    pub color: String,
    pub is_builtin: bool,
    pub kind: AgentKind,
}

impl AgentConfig {
    pub fn pty_spec(&self) -> Option<&PtyAgentSpec> {
        match &self.kind {
            AgentKind::Pty(spec) => Some(spec),
            AgentKind::Native(_) => None,
        }
    }

    pub fn native_spec(&self) -> Option<&NativeAgentSpec> {
        match &self.kind {
            AgentKind::Native(spec) => Some(spec),
            AgentKind::Pty(_) => None,
        }
    }

    pub fn is_native(&self) -> bool {
        matches!(self.kind, AgentKind::Native(_))
    }

    /// String shown in compact agent lists. For PTY agents this is the shell command;
    /// for native agents it's `provider_id/model`.
    pub fn display_command(&self) -> String {
        match &self.kind {
            AgentKind::Pty(spec) => spec.command.clone(),
            AgentKind::Native(spec) => format!("{}/{}", spec.provider_id, spec.model),
        }
    }

    pub fn claude_code() -> Self {
        Self {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            description: "Anthropic's CLI coding agent".into(),
            installed: false,
            capabilities: vec![
                AgentCapability::CodeEdit,
                AgentCapability::CodeReview,
                AgentCapability::Testing,
                AgentCapability::Research,
                AgentCapability::Shell,
                AgentCapability::Refactor,
            ],
            icon: "Bot".into(),
            color: "text-accent-purple".into(),
            is_builtin: true,
            kind: AgentKind::Pty(PtyAgentSpec {
                command: "claude".into(),
                default_args: vec![],
                status_patterns: AgentStatusPatterns {
                    approval: vec![
                        r"Allow\s+\w+.*\?".into(),
                        r"\(y\/n\)".into(),
                        r"Do you want to (?:proceed|continue|allow)".into(),
                        r"Press\s+y\s+to\s+(?:approve|allow|confirm)".into(),
                        r"\[Y\/n\]".into(),
                        r"\[y\/N\]".into(),
                        r"Allow once|Allow always|Deny".into(),
                    ],
                    thinking: vec![r"⏺\s*Thinking".into(), r"thinking\.\.\.".into()],
                    tool_use: vec![
                        ToolUsePattern { pattern: r"⏺\s*Read\(([^)]+)\)".into(), tool: "Read".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"⏺\s*Edit\(([^)]+)\)".into(), tool: "Edit".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"⏺\s*Write\(([^)]+)\)".into(), tool: "Write".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"⏺\s*Bash\(([^)]*)\)".into(), tool: "Bash".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"⏺\s*Glob\(([^)]*)\)".into(), tool: "Glob".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"⏺\s*Grep\(([^)]*)\)".into(), tool: "Grep".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"⏺\s*Task\(([^)]*)\)".into(), tool: "Task".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Reading\s+(.+)".into(), tool: "Read".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Editing\s+(.+)".into(), tool: "Edit".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Writing\s+(.+)".into(), tool: "Write".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Running\s+(.+)".into(), tool: "Bash".into(), file_group: Some(1) },
                    ],
                    idle: vec![r"^\s*[>❯]\s*$".into()],
                },
                approval_actions: AgentApprovalActions {
                    approve: "y\n".into(),
                    deny: "n\n".into(),
                    abort: "\u{3}".into(),
                },
            }),
        }
    }

    pub fn opencode() -> Self {
        Self {
            id: "opencode".into(),
            name: "OpenCode".into(),
            description: "Open-source AI coding agent, 75+ providers".into(),
            installed: false,
            capabilities: vec![
                AgentCapability::CodeEdit,
                AgentCapability::CodeReview,
                AgentCapability::Testing,
                AgentCapability::Research,
                AgentCapability::Shell,
                AgentCapability::Refactor,
            ],
            icon: "Terminal".into(),
            color: "text-accent-green".into(),
            is_builtin: true,
            kind: AgentKind::Pty(PtyAgentSpec {
                command: "opencode".into(),
                default_args: vec![],
                status_patterns: AgentStatusPatterns {
                    approval: vec![
                        r"\(y\/n\)".into(),
                        r"Approve|Cancel".into(),
                        r"Do you want to (?:proceed|continue|allow)".into(),
                    ],
                    thinking: vec![r"thinking".into(), r"reasoning".into(), r"Planning".into()],
                    tool_use: vec![
                        ToolUsePattern { pattern: r"Reading\s+(.+)".into(), tool: "Read".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Editing\s+(.+)".into(), tool: "Edit".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Writing\s+(.+)".into(), tool: "Write".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Running\s+(.+)".into(), tool: "Bash".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Searching\s+(.+)".into(), tool: "Search".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"tool.*running".into(), tool: "Tool".into(), file_group: None },
                    ],
                    idle: vec![r"^\s*[>❯\$]\s*$".into(), r"opencode>".into()],
                },
                approval_actions: AgentApprovalActions {
                    approve: "y\n".into(),
                    deny: "n\n".into(),
                    abort: "\u{3}".into(),
                },
            }),
        }
    }

    pub fn codex() -> Self {
        Self {
            id: "codex".into(),
            name: "Codex CLI".into(),
            description: "OpenAI's CLI coding agent".into(),
            installed: false,
            capabilities: vec![
                AgentCapability::CodeEdit,
                AgentCapability::CodeReview,
                AgentCapability::Testing,
                AgentCapability::Shell,
                AgentCapability::Refactor,
            ],
            icon: "Cpu".into(),
            color: "text-accent-blue".into(),
            is_builtin: true,
            kind: AgentKind::Pty(PtyAgentSpec {
                command: "codex".into(),
                default_args: vec![],
                status_patterns: AgentStatusPatterns {
                    approval: vec![
                        r"Allow\s+\w+.*\?".into(),
                        r"\(y\/n\)".into(),
                        r"Do you want to (?:proceed|continue|allow)".into(),
                        r"\[Y\/n\]".into(),
                        r"\[y\/N\]".into(),
                    ],
                    thinking: vec![r"thinking\.\.\.".into(), r"Thinking".into()],
                    tool_use: vec![
                        ToolUsePattern { pattern: r"Reading\s+(.+)".into(), tool: "Read".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Editing\s+(.+)".into(), tool: "Edit".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Writing\s+(.+)".into(), tool: "Write".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Running\s+(.+)".into(), tool: "Bash".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Applying\s+patch".into(), tool: "Patch".into(), file_group: None },
                    ],
                    idle: vec![r"^\s*[>❯\$]\s*$".into()],
                },
                approval_actions: AgentApprovalActions {
                    approve: "y\n".into(),
                    deny: "n\n".into(),
                    abort: "\u{3}".into(),
                },
            }),
        }
    }

    pub fn gemini() -> Self {
        Self {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            description: "Google's CLI coding agent".into(),
            installed: false,
            capabilities: vec![
                AgentCapability::CodeEdit,
                AgentCapability::CodeReview,
                AgentCapability::Testing,
                AgentCapability::Research,
                AgentCapability::Shell,
                AgentCapability::Refactor,
            ],
            icon: "Sparkles".into(),
            color: "text-accent-blue".into(),
            is_builtin: true,
            kind: AgentKind::Pty(PtyAgentSpec {
                command: "gemini".into(),
                default_args: vec![],
                status_patterns: AgentStatusPatterns {
                    approval: vec![
                        r"\(y\/n\)".into(),
                        r"\[Y\/n\]".into(),
                        r"\[y\/N\]".into(),
                        r"Do you want to (?:proceed|continue|allow)".into(),
                        r"Allow\s+\w+.*\?".into(),
                    ],
                    thinking: vec![r"Thinking".into(), r"thinking\.\.\.".into(), r"Planning".into()],
                    tool_use: vec![
                        ToolUsePattern { pattern: r"Reading\s+(.+)".into(), tool: "Read".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Editing\s+(.+)".into(), tool: "Edit".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Writing\s+(.+)".into(), tool: "Write".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Running\s+(.+)".into(), tool: "Bash".into(), file_group: Some(1) },
                        ToolUsePattern { pattern: r"Searching\s+(.+)".into(), tool: "Search".into(), file_group: Some(1) },
                    ],
                    idle: vec![r"^\s*[>❯\$]\s*$".into()],
                },
                approval_actions: AgentApprovalActions {
                    approve: "y\n".into(),
                    deny: "n\n".into(),
                    abort: "\u{3}".into(),
                },
            }),
        }
    }

    pub fn terminal() -> Self {
        Self {
            id: "terminal".into(),
            name: "Terminal".into(),
            description: "Plain terminal shell".into(),
            installed: true,
            capabilities: vec![AgentCapability::Shell],
            icon: "TerminalSquare".into(),
            color: "text-text-secondary".into(),
            is_builtin: true,
            kind: AgentKind::Pty(PtyAgentSpec {
                command: if cfg!(windows) { "powershell".into() } else { "bash".into() },
                default_args: vec![],
                status_patterns: AgentStatusPatterns {
                    approval: vec![],
                    thinking: vec![],
                    tool_use: vec![],
                    idle: vec![r"^\s*[>❯\$#%]\s*$".into()],
                },
                approval_actions: AgentApprovalActions {
                    approve: "y\n".into(),
                    deny: "n\n".into(),
                    abort: "\u{3}".into(),
                },
            }),
        }
    }

    pub fn flightdeck_native() -> Self {
        Self {
            id: "flightdeck-native".into(),
            name: "FlightDeck Native".into(),
            description: "In-process Anthropic-backed coding agent".into(),
            installed: false,
            capabilities: vec![
                AgentCapability::CodeEdit,
                AgentCapability::CodeReview,
                AgentCapability::Testing,
                AgentCapability::Research,
                AgentCapability::Shell,
                AgentCapability::Refactor,
            ],
            icon: "Cpu".into(),
            color: "text-accent-orange".into(),
            is_builtin: true,
            kind: AgentKind::Native(NativeAgentSpec {
                provider_id: "anthropic-primary".into(),
                model: "claude-sonnet-4-6".into(),
                tool_allowlist: vec![
                    "read".into(),
                    "write".into(),
                    "edit".into(),
                    "bash".into(),
                    "grep".into(),
                    "glob".into(),
                ],
                system_prompt_override: None,
            }),
        }
    }

    pub fn builtins() -> Vec<Self> {
        vec![
            Self::flightdeck_native(),
            Self::claude_code(),
            Self::opencode(),
            Self::codex(),
            Self::gemini(),
            Self::terminal(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flightdeck_native_is_a_native_kind() {
        let agent = AgentConfig::flightdeck_native();
        assert!(agent.is_native());
        assert!(agent.native_spec().is_some());
        assert!(agent.pty_spec().is_none());
        let native = agent.native_spec().unwrap();
        assert_eq!(native.provider_id, "anthropic-primary");
        assert!(!native.tool_allowlist.is_empty());
    }

    #[test]
    fn builtins_lead_with_native() {
        let builtins = AgentConfig::builtins();
        assert_eq!(builtins[0].id, "flightdeck-native");
        assert!(builtins[0].is_native());
        // And everything else is PTY.
        for pty_agent in &builtins[1..] {
            assert!(pty_agent.pty_spec().is_some(), "{} should be Pty", pty_agent.id);
        }
    }

    #[test]
    fn pty_builtins_round_trip_through_json() {
        for agent in AgentConfig::builtins() {
            let json = serde_json::to_string(&agent).unwrap();
            let back: AgentConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(agent.id, back.id);
            assert_eq!(agent.is_native(), back.is_native());
        }
    }

    #[test]
    fn display_command_differs_by_kind() {
        assert_eq!(AgentConfig::claude_code().display_command(), "claude");
        let native = AgentConfig::flightdeck_native().display_command();
        assert!(native.contains('/'), "native display should be provider/model");
    }
}
