use serde::{Deserialize, Serialize};

// === Flight Status & Priority ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightStatus {
    Draft,
    Planning,
    Ready,
    Active,
    Paused,
    Review,
    Done,
    Failed,
    Cancelled,
}

impl FlightStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Planning => "Planning",
            Self::Ready => "Ready",
            Self::Active => "Active",
            Self::Paused => "Paused",
            Self::Review => "Review",
            Self::Done => "Done",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl FlightPriority {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Critical => "Critical",
        }
    }
}

// === Milestone ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneStatus {
    Pending,
    Active,
    Done,
    Failed,
}

impl MilestoneStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Active => "Active",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub flight_id: String,
    pub title: String,
    pub description: String,
    pub order: usize,
    pub status: MilestoneStatus,
    pub tasks: Vec<Task>,
    pub validation_criteria: Vec<String>,
}

// === Task ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Blocked,
    Queued,
    Running,
    ApprovalNeeded,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Blocked => "Blocked",
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::ApprovalNeeded => "Needs Approval",
            Self::Paused => "Paused",
            Self::Done => "Done",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Implementation,
    Testing,
    Review,
    Validation,
    Research,
    Refactor,
    Documentation,
}

impl TaskType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Implementation => "Impl",
            Self::Testing => "Test",
            Self::Review => "Review",
            Self::Validation => "Validate",
            Self::Research => "Research",
            Self::Refactor => "Refactor",
            Self::Documentation => "Docs",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHandoff {
    pub summary: String,
    pub files_changed: Vec<String>,
    pub tests_needed: Vec<String>,
    pub follow_ups: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationVerdict {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskValidationAssertion {
    pub label: String,
    pub status: ValidationVerdict,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskValidationReport {
    pub verdict: ValidationVerdict,
    pub summary: String,
    pub assertions: Vec<TaskValidationAssertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub exit_code: Option<i32>,
    pub summary: String,
    pub files_changed: Vec<String>,
    pub errors: Vec<String>,
    pub duration_ms: u64,
    pub handoff: Option<TaskHandoff>,
    pub validation: Option<TaskValidationReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewType {
    ToolCall,
    FileWrite,
    Command,
    MilestoneGate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPacket {
    pub id: String,
    pub task_id: String,
    pub flight_id: String,
    pub milestone_id: String,
    pub requested_at: u64,
    pub review_type: ReviewType,
    pub summary: String,
    pub diff: Option<String>,
    pub command: Option<String>,
    pub file_paths: Vec<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub milestone_id: String,
    pub flight_id: String,
    pub title: String,
    pub description: String,
    pub order: usize,
    pub status: TaskStatus,
    pub task_type: TaskType,
    pub agent_config_id: String,
    pub agent_args: Option<Vec<String>>,
    pub model: Option<String>,
    pub depends_on: Vec<String>,
    pub session_id: Option<String>,
    pub result: Option<TaskResult>,
    pub review_packet: Option<ReviewPacket>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub cost: f64,
    pub tokens: u64,
}

// === Approval Decision ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionType {
    Approved,
    Denied,
    ForceOverridden,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    pub id: String,
    pub review_packet_id: String,
    pub task_id: String,
    pub flight_id: String,
    pub decision: ApprovalDecisionType,
    pub decided_at: u64,
    pub reason: Option<String>,
}

// === Flight ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flight {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub status: FlightStatus,
    pub priority: FlightPriority,
    pub project_path: String,
    pub git_branch: Option<String>,
    pub milestones: Vec<Milestone>,
    pub linked_session_ids: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub completed_at: Option<u64>,
    pub total_cost: f64,
    pub total_tokens: u64,
}

impl Flight {
    /// Get all tasks across all milestones.
    pub fn all_tasks(&self) -> Vec<&Task> {
        self.milestones.iter().flat_map(|m| m.tasks.iter()).collect()
    }

    /// Count done / total tasks.
    pub fn progress(&self) -> (usize, usize) {
        let tasks = self.all_tasks();
        let done = tasks.iter().filter(|t| t.status == TaskStatus::Done).count();
        (done, tasks.len())
    }

    /// Check if any tasks need attention (approval or failed).
    pub fn needs_attention(&self) -> bool {
        self.all_tasks().iter().any(|t| {
            t.status == TaskStatus::ApprovalNeeded || t.status == TaskStatus::Failed
        })
    }
}

// === Issue ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub ticket_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub labels: Vec<String>,
    pub epic: Option<String>,
    pub session_id: Option<String>,
    pub flight_id: Option<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub blocked_by: Vec<String>,
    pub blocks: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(status: TaskStatus) -> Task {
        Task {
            id: uuid::Uuid::new_v4().to_string(),
            milestone_id: "m1".to_string(),
            flight_id: "f1".to_string(),
            title: "Test task".to_string(),
            description: String::new(),
            order: 0,
            status,
            task_type: TaskType::Implementation,
            agent_config_id: String::new(),
            agent_args: None,
            model: None,
            depends_on: Vec::new(),
            session_id: None,
            result: None,
            review_packet: None,
            created_at: 0,
            started_at: None,
            completed_at: None,
            cost: 0.0,
            tokens: 0,
        }
    }

    fn make_milestone(tasks: Vec<Task>) -> Milestone {
        Milestone {
            id: "m1".to_string(),
            flight_id: "f1".to_string(),
            title: "Milestone 1".to_string(),
            description: String::new(),
            order: 0,
            status: MilestoneStatus::Active,
            tasks,
            validation_criteria: Vec::new(),
        }
    }

    fn make_flight(milestones: Vec<Milestone>) -> Flight {
        Flight {
            id: "f1".to_string(),
            title: "Test flight".to_string(),
            objective: String::new(),
            status: FlightStatus::Active,
            priority: FlightPriority::Medium,
            project_path: "/tmp/test".to_string(),
            git_branch: None,
            milestones,
            linked_session_ids: Vec::new(),
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            total_cost: 0.0,
            total_tokens: 0,
        }
    }

    #[test]
    fn all_tasks_returns_tasks_across_milestones() {
        let m1 = make_milestone(vec![make_task(TaskStatus::Done)]);
        let m2 = make_milestone(vec![make_task(TaskStatus::Pending), make_task(TaskStatus::Running)]);
        let flight = make_flight(vec![m1, m2]);
        assert_eq!(flight.all_tasks().len(), 3);
    }

    #[test]
    fn progress_counts_done_tasks() {
        let t1 = make_task(TaskStatus::Done);
        let t2 = make_task(TaskStatus::Running);
        let flight = make_flight(vec![make_milestone(vec![t1, t2])]);
        let (done, total) = flight.progress();
        assert_eq!(done, 1);
        assert_eq!(total, 2);
    }

    #[test]
    fn needs_attention_detects_approval_needed() {
        let flight = make_flight(vec![make_milestone(vec![make_task(TaskStatus::ApprovalNeeded)])]);
        assert!(flight.needs_attention());
    }

    #[test]
    fn needs_attention_detects_failed() {
        let flight = make_flight(vec![make_milestone(vec![make_task(TaskStatus::Failed)])]);
        assert!(flight.needs_attention());
    }

    #[test]
    fn needs_attention_false_when_all_ok() {
        let flight = make_flight(vec![make_milestone(vec![make_task(TaskStatus::Running)])]);
        assert!(!flight.needs_attention());
    }

    #[test]
    fn task_status_is_terminal() {
        assert!(TaskStatus::Done.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::ApprovalNeeded.is_terminal());
    }
}
