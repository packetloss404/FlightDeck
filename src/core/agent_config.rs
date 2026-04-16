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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    pub default_args: Vec<String>,
    pub description: String,
    pub installed: bool,
    pub capabilities: Vec<AgentCapability>,
    pub icon: String,
    pub color: String,
    pub status_patterns: AgentStatusPatterns,
    pub approval_actions: AgentApprovalActions,
    pub is_builtin: bool,
}

impl AgentConfig {
    pub fn claude_code() -> Self {
        Self {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            command: "claude".into(),
            default_args: vec![],
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
            is_builtin: true,
        }
    }

    pub fn opencode() -> Self {
        Self {
            id: "opencode".into(),
            name: "OpenCode".into(),
            command: "opencode".into(),
            default_args: vec![],
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
            is_builtin: true,
        }
    }

    pub fn codex() -> Self {
        Self {
            id: "codex".into(),
            name: "Codex CLI".into(),
            command: "codex".into(),
            default_args: vec![],
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
            is_builtin: true,
        }
    }

    pub fn gemini() -> Self {
        Self {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            command: "gemini".into(),
            default_args: vec![],
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
            is_builtin: true,
        }
    }

    pub fn terminal() -> Self {
        Self {
            id: "terminal".into(),
            name: "Terminal".into(),
            command: if cfg!(windows) { "powershell".into() } else { "bash".into() },
            default_args: vec![],
            description: "Plain terminal shell".into(),
            installed: true,
            capabilities: vec![AgentCapability::Shell],
            icon: "TerminalSquare".into(),
            color: "text-text-secondary".into(),
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
            is_builtin: true,
        }
    }

    pub fn builtins() -> Vec<Self> {
        vec![Self::claude_code(), Self::opencode(), Self::codex(), Self::gemini(), Self::terminal()]
    }
}
