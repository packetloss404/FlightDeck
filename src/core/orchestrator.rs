use std::collections::{HashMap, HashSet};
use tracing::info;

use super::flight::*;
use super::agent_config::AgentConfig;

/// A running task tracked by the orchestrator
#[derive(Debug, Clone)]
pub struct RunningTask {
    pub task_id: String,
    pub milestone_id: String,
    pub flight_id: String,
    pub session_id: String,
    pub agent_config_id: String,
    pub started_at: u64,
}

/// Settings for the orchestrator
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrchestratorSettings {
    pub max_parallel_sessions: usize,
    pub milestone_gating: bool,
    pub project_path: String,
}

impl Default for OrchestratorSettings {
    fn default() -> Self {
        Self {
            max_parallel_sessions: 3,
            milestone_gating: true,
            project_path: std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| ".".to_string()),
        }
    }
}

/// Core orchestration engine.
/// Manages flight execution, task scheduling, and agent session lifecycle.
pub struct Orchestrator {
    pub settings: OrchestratorSettings,
    pub running_tasks: HashMap<String, RunningTask>,
    pub active_flight_ids: HashSet<String>,
    pub paused_at_milestone: HashMap<String, String>, // flight_id -> milestone_id
}

impl Orchestrator {
    pub fn new(settings: OrchestratorSettings) -> Self {
        Self {
            settings,
            running_tasks: HashMap::new(),
            active_flight_ids: HashSet::new(),
            paused_at_milestone: HashMap::new(),
        }
    }

    /// Check if a task's dependencies are all done.
    fn deps_resolved(task: &Task, all_tasks: &[Task]) -> bool {
        task.depends_on.iter().all(|dep_id| {
            all_tasks.iter().any(|t| t.id == *dep_id && t.status == TaskStatus::Done)
        })
    }

    fn queue_ready_tasks(ms: &mut Milestone, include_paused: bool) {
        let tasks_snapshot: Vec<Task> = ms.tasks.clone();
        for task in &mut ms.tasks {
            let can_queue = match task.status {
                TaskStatus::Pending => true,
                TaskStatus::Paused => include_paused,
                _ => false,
            };

            if can_queue && Self::deps_resolved(task, &tasks_snapshot) {
                task.status = TaskStatus::Queued;
            }
        }
    }

    fn first_open_milestone_index(flight: &Flight) -> Option<usize> {
        flight.milestones.iter().position(|ms| {
            ms.status != MilestoneStatus::Done && ms.status != MilestoneStatus::Failed
        })
    }

    fn activate_milestone(flight: &mut Flight, milestone_idx: usize, include_paused: bool) {
        for (idx, ms) in flight.milestones.iter_mut().enumerate() {
            if idx == milestone_idx {
                ms.status = MilestoneStatus::Active;
                Self::queue_ready_tasks(ms, include_paused);
            } else if ms.status == MilestoneStatus::Active {
                ms.status = MilestoneStatus::Pending;
            }
        }
    }

    fn task_position(flight: &Flight, task_id: &str) -> Option<(usize, usize)> {
        for (milestone_idx, milestone) in flight.milestones.iter().enumerate() {
            if let Some(task_idx) = milestone.tasks.iter().position(|task| task.id == task_id) {
                return Some((milestone_idx, task_idx));
            }
        }

        None
    }

    /// Launch a flight: set status to active, queue eligible tasks in first milestone.
    pub fn launch_flight(&mut self, flight: &mut Flight) {
        flight.status = FlightStatus::Active;
        flight.updated_at = now();

        if let Some(ms_idx) = Self::first_open_milestone_index(flight) {
            Self::activate_milestone(flight, ms_idx, false);
        }

        self.active_flight_ids.insert(flight.id.clone());
        info!(flight_id = %flight.id, "Launched flight");
    }

    /// Pause a flight: set running tasks to paused.
    pub fn pause_flight(&mut self, flight: &mut Flight) {
        flight.status = FlightStatus::Paused;
        flight.updated_at = now();

        for ms in &mut flight.milestones {
            for task in &mut ms.tasks {
                if task.status == TaskStatus::Running || task.status == TaskStatus::Queued {
                    task.status = TaskStatus::Paused;
                }
            }
        }

        self.active_flight_ids.remove(&flight.id);
        self.running_tasks.retain(|_, rt| rt.flight_id != flight.id);
    }

    /// Resume a paused flight.
    pub fn resume_flight(&mut self, flight: &mut Flight) {
        flight.status = FlightStatus::Active;
        flight.updated_at = now();

        if let Some(next_ms_id) = self.paused_at_milestone.remove(&flight.id) {
            if let Some(ms_idx) = flight.milestones.iter().position(|ms| ms.id == next_ms_id) {
                Self::activate_milestone(flight, ms_idx, false);
            }
        } else if let Some(ms_idx) = flight.milestones.iter().position(|ms| ms.status == MilestoneStatus::Active) {
            Self::activate_milestone(flight, ms_idx, true);
        } else if let Some(ms_idx) = Self::first_open_milestone_index(flight) {
            Self::activate_milestone(flight, ms_idx, true);
        }

        self.active_flight_ids.insert(flight.id.clone());
    }

    /// Cancel a flight: cancel all non-terminal tasks.
    pub fn cancel_flight(&mut self, flight: &mut Flight) {
        flight.status = FlightStatus::Cancelled;
        flight.updated_at = now();

        for ms in &mut flight.milestones {
            for task in &mut ms.tasks {
                if !task.status.is_terminal() {
                    task.status = TaskStatus::Cancelled;
                }
            }
        }

        self.active_flight_ids.remove(&flight.id);
        self.running_tasks.retain(|_, rt| rt.flight_id != flight.id);
    }

    /// Handle task completion. Updates statuses and queues next tasks.
    pub fn on_task_complete(&mut self, task_id: &str, success: bool, flights: &mut [Flight]) {
        let rt = match self.running_tasks.remove(task_id) {
            Some(rt) => rt,
            None => return,
        };

        let flight = match flights.iter_mut().find(|f| f.id == rt.flight_id) {
            Some(f) => f,
            None => return,
        };

        // Find milestone and task indices
        let mut found_ms_idx = None;
        for (mi, ms) in flight.milestones.iter().enumerate() {
            if ms.tasks.iter().any(|t| t.id == *task_id) {
                found_ms_idx = Some(mi);
                break;
            }
        }

        let ms_idx = match found_ms_idx {
            Some(i) => i,
            None => return,
        };

        // Update the task status
        if let Some(task) = flight.milestones[ms_idx].tasks.iter_mut().find(|t| t.id == *task_id) {
            task.status = if success { TaskStatus::Done } else { TaskStatus::Failed };
            task.completed_at = Some(now());
        }

        // Re-resolve dependencies in same milestone
        Self::queue_ready_tasks(&mut flight.milestones[ms_idx], false);

        // Check milestone completion
        let all_done = flight.milestones[ms_idx].tasks.iter().all(|t| t.status.is_terminal());
        let any_failed = flight.milestones[ms_idx].tasks.iter().any(|t| t.status == TaskStatus::Failed);

        if all_done {
            flight.milestones[ms_idx].status = if any_failed { MilestoneStatus::Failed } else { MilestoneStatus::Done };

            let has_next = ms_idx + 1 < flight.milestones.len();

            if has_next && !any_failed {
                if self.settings.milestone_gating {
                    let next_ms_id = flight.milestones[ms_idx + 1].id.clone();
                    self.paused_at_milestone.insert(flight.id.clone(), next_ms_id);
                    flight.status = FlightStatus::Review;
                    self.active_flight_ids.remove(&flight.id);
                } else {
                    // Auto-advance to next milestone
                    Self::activate_milestone(flight, ms_idx + 1, false);
                }
            } else if any_failed {
                flight.status = FlightStatus::Failed;
                flight.completed_at = Some(now());
                self.active_flight_ids.remove(&flight.id);
            } else if !has_next {
                // All milestones complete
                flight.status = FlightStatus::Done;
                flight.completed_at = Some(now());
                self.active_flight_ids.remove(&flight.id);
            }
        }

        flight.updated_at = now();
    }

    pub fn on_task_approval_needed(&mut self, task_id: &str, flights: &mut [Flight]) {
        let Some(rt) = self.running_tasks.get(task_id) else {
            return;
        };

        let Some(flight) = flights.iter_mut().find(|flight| flight.id == rt.flight_id) else {
            return;
        };

        let Some((milestone_idx, task_idx)) = Self::task_position(flight, task_id) else {
            return;
        };

        flight.milestones[milestone_idx].tasks[task_idx].status = TaskStatus::ApprovalNeeded;
        flight.updated_at = now();
    }

    pub fn on_task_approval_resolved(&mut self, task_id: &str, flights: &mut [Flight]) {
        let Some(rt) = self.running_tasks.get(task_id) else {
            return;
        };

        let Some(flight) = flights.iter_mut().find(|flight| flight.id == rt.flight_id) else {
            return;
        };

        let Some((milestone_idx, task_idx)) = Self::task_position(flight, task_id) else {
            return;
        };

        let task = &mut flight.milestones[milestone_idx].tasks[task_idx];
        if task.status == TaskStatus::ApprovalNeeded {
            task.status = TaskStatus::Running;
            flight.updated_at = now();
        }
    }

    /// Normalize persisted flights into a safe post-restart state.
    pub fn recover_from_flights(&mut self, flights: &mut [Flight]) {
        self.running_tasks.clear();
        self.active_flight_ids.clear();
        self.paused_at_milestone.clear();

        for flight in flights {
            flight.linked_session_ids.clear();

            let mut interrupted = false;

            for ms in &mut flight.milestones {
                if ms.status == MilestoneStatus::Active {
                    ms.status = MilestoneStatus::Pending;
                }

                for task in &mut ms.tasks {
                    if matches!(task.status, TaskStatus::Queued | TaskStatus::Running | TaskStatus::ApprovalNeeded) {
                        task.status = TaskStatus::Paused;
                        interrupted = true;
                    }

                    if task.session_id.is_some() {
                        task.session_id = None;
                    }
                }
            }

            if interrupted && matches!(flight.status, FlightStatus::Active | FlightStatus::Review) {
                flight.status = FlightStatus::Paused;
                flight.updated_at = now();
            }
        }
    }

    /// Scheduling tick: spawn agent sessions for queued tasks up to max parallel.
    /// Returns list of (flight_id, milestone_id, task_id, command, args, prompt) to spawn.
    pub fn tick(
        &mut self,
        flights: &[Flight],
        agents: &[AgentConfig],
    ) -> Vec<TaskSpawnRequest> {
        let available = self.settings.max_parallel_sessions.saturating_sub(self.running_tasks.len());
        if available == 0 {
            return vec![];
        }

        let mut requests = Vec::new();
        let mut slots_used = 0;

        for flight in flights {
            if slots_used >= available { break; }
            if !self.active_flight_ids.contains(&flight.id) { continue; }
            if self.paused_at_milestone.contains_key(&flight.id) { continue; }

            for ms in &flight.milestones {
                if ms.status == MilestoneStatus::Done || ms.status == MilestoneStatus::Failed {
                    continue;
                }

                for task in &ms.tasks {
                    if slots_used >= available { break; }
                    if task.status != TaskStatus::Queued { continue; }
                    if self.running_tasks.contains_key(&task.id) { continue; }

                    let agent = agents.iter().find(|a| a.id == task.agent_config_id);
                    if agent.is_none() { continue; }
                    let agent = agent.unwrap();

                    let mut args = agent.default_args.clone();
                    if let Some(ref agent_args) = task.agent_args {
                        args.extend(agent_args.clone());
                    }
                    if let Some(ref model) = task.model {
                        args.push("--model".to_string());
                        args.push(model.clone());
                    }

                    let prompt = format!(
                        "Flight: {}\nObjective: {}\nMilestone: {}\n\nTask: {}\n{}",
                        flight.title, flight.objective, ms.title, task.title, task.description
                    );

                    requests.push(TaskSpawnRequest {
                        flight_id: flight.id.clone(),
                        milestone_id: ms.id.clone(),
                        task_id: task.id.clone(),
                        agent_config_id: agent.id.clone(),
                        command: agent.command.clone(),
                        args,
                        prompt,
                        project_path: flight.project_path.clone(),
                    });

                    slots_used += 1;
                }

                // Only work on one milestone at a time per flight
                if ms.status == MilestoneStatus::Active || ms.status == MilestoneStatus::Pending {
                    break;
                }
            }
        }

        requests
    }

    /// Record that a task has been spawned.
    pub fn record_spawn(&mut self, session_id: &str, req: &TaskSpawnRequest, flights: &mut [Flight]) {
        if let Some(flight) = flights.iter_mut().find(|flight| flight.id == req.flight_id) {
            if let Some((milestone_idx, task_idx)) = Self::task_position(flight, &req.task_id) {
                if flight.milestones[milestone_idx].status == MilestoneStatus::Pending {
                    flight.milestones[milestone_idx].status = MilestoneStatus::Active;
                }

                let task = &mut flight.milestones[milestone_idx].tasks[task_idx];
                task.status = TaskStatus::Running;
                task.session_id = Some(session_id.to_string());
                task.started_at = Some(now());

                if !flight.linked_session_ids.iter().any(|id| id == session_id) {
                    flight.linked_session_ids.push(session_id.to_string());
                }

                flight.updated_at = now();
            }
        }

        self.running_tasks.insert(req.task_id.to_string(), RunningTask {
            task_id: req.task_id.to_string(),
            milestone_id: req.milestone_id.clone(),
            flight_id: req.flight_id.clone(),
            session_id: session_id.to_string(),
            agent_config_id: req.agent_config_id.clone(),
            started_at: now(),
        });
    }

    /// Get running tasks for a specific flight.
    pub fn running_tasks_for_flight(&self, flight_id: &str) -> Vec<&RunningTask> {
        self.running_tasks.values().filter(|rt| rt.flight_id == flight_id).collect()
    }
}

/// Request to spawn a task as a PTY session
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskSpawnRequest {
    pub flight_id: String,
    pub milestone_id: String,
    pub task_id: String,
    pub agent_config_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub prompt: String,
    pub project_path: String,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(id: &str) -> Task {
        Task {
            id: id.to_string(),
            milestone_id: "ms-1".to_string(),
            flight_id: "flight-1".to_string(),
            title: format!("Task {id}"),
            description: "Run the task".to_string(),
            order: 0,
            status: TaskStatus::Queued,
            task_type: TaskType::Implementation,
            agent_config_id: "claude-code".to_string(),
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

    fn sample_flight() -> Flight {
        Flight {
            id: "flight-1".to_string(),
            title: "Flight".to_string(),
            objective: "Objective".to_string(),
            status: FlightStatus::Active,
            priority: FlightPriority::High,
            project_path: "D:/projects/FlightDeck".to_string(),
            git_branch: Some("feature/test".to_string()),
            milestones: vec![Milestone {
                id: "ms-1".to_string(),
                flight_id: "flight-1".to_string(),
                title: "Milestone".to_string(),
                description: "Desc".to_string(),
                order: 0,
                status: MilestoneStatus::Pending,
                tasks: vec![sample_task("task-1")],
                validation_criteria: Vec::new(),
            }],
            linked_session_ids: Vec::new(),
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            total_cost: 0.0,
            total_tokens: 0,
        }
    }

    fn sample_request() -> TaskSpawnRequest {
        TaskSpawnRequest {
            flight_id: "flight-1".to_string(),
            milestone_id: "ms-1".to_string(),
            task_id: "task-1".to_string(),
            agent_config_id: "claude-code".to_string(),
            command: "claude".to_string(),
            args: vec!["-p".to_string(), "Run the task".to_string()],
            prompt: "Run the task".to_string(),
            project_path: "D:/projects/FlightDeck".to_string(),
        }
    }

    #[test]
    fn record_spawn_updates_flight_and_task_runtime_state() {
        let mut orchestrator = Orchestrator::new(OrchestratorSettings::default());
        let mut flights = vec![sample_flight()];
        let request = sample_request();

        orchestrator.record_spawn("session-1", &request, &mut flights);

        let flight = &flights[0];
        let task = &flight.milestones[0].tasks[0];

        assert_eq!(flight.milestones[0].status, MilestoneStatus::Active);
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.session_id.as_deref(), Some("session-1"));
        assert!(flight.linked_session_ids.iter().any(|id| id == "session-1"));
        assert!(orchestrator.running_tasks.contains_key("task-1"));
    }

    #[test]
    fn approval_transition_helpers_update_task_state() {
        let mut orchestrator = Orchestrator::new(OrchestratorSettings::default());
        let mut flights = vec![sample_flight()];
        let request = sample_request();

        orchestrator.record_spawn("session-1", &request, &mut flights);
        orchestrator.on_task_approval_needed("task-1", &mut flights);
        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::ApprovalNeeded);

        orchestrator.on_task_approval_resolved("task-1", &mut flights);
        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::Running);
    }

    fn sample_agent() -> AgentConfig {
        AgentConfig {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            command: "claude".into(),
            default_args: vec!["-p".to_string()],
            description: "Test agent".into(),
            installed: true,
            capabilities: vec![],
            icon: "Bot".into(),
            color: "text-accent-purple".into(),
            status_patterns: super::super::agent_config::AgentStatusPatterns {
                approval: vec![],
                thinking: vec![],
                tool_use: vec![],
                idle: vec![],
            },
            approval_actions: super::super::agent_config::AgentApprovalActions {
                approve: "y\n".into(),
                deny: "n\n".into(),
                abort: "\u{3}".into(),
            },
            is_builtin: true,
        }
    }

    fn two_task_flight() -> Flight {
        let mut task1 = sample_task("task-1");
        task1.status = TaskStatus::Queued;

        let mut task2 = sample_task("task-2");
        task2.id = "task-2".to_string();
        task2.status = TaskStatus::Pending;
        task2.depends_on = vec!["task-1".to_string()];

        Flight {
            id: "flight-1".to_string(),
            title: "Flight".to_string(),
            objective: "Objective".to_string(),
            status: FlightStatus::Draft,
            priority: FlightPriority::High,
            project_path: "D:/projects/FlightDeck".to_string(),
            git_branch: Some("feature/test".to_string()),
            milestones: vec![Milestone {
                id: "ms-1".to_string(),
                flight_id: "flight-1".to_string(),
                title: "Milestone".to_string(),
                description: "Desc".to_string(),
                order: 0,
                status: MilestoneStatus::Pending,
                tasks: vec![task1, task2],
                validation_criteria: Vec::new(),
            }],
            linked_session_ids: Vec::new(),
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            total_cost: 0.0,
            total_tokens: 0,
        }
    }

    #[test]
    fn test_launch_tick_spawn_complete_lifecycle() {
        let mut orchestrator = Orchestrator::new(OrchestratorSettings {
            milestone_gating: false,
            ..OrchestratorSettings::default()
        });
        let agents = vec![sample_agent()];
        let mut flight = two_task_flight();

        // Launch flight
        orchestrator.launch_flight(&mut flight);
        assert_eq!(flight.status, FlightStatus::Active);
        assert_eq!(flight.milestones[0].status, MilestoneStatus::Active);
        assert_eq!(flight.milestones[0].tasks[0].status, TaskStatus::Queued);
        // task-2 should still be Pending (deps not met)
        assert_eq!(flight.milestones[0].tasks[1].status, TaskStatus::Pending);

        let mut flights = vec![flight];

        // Tick should return 1 spawn request for task-1
        let requests = orchestrator.tick(&flights, &agents);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].task_id, "task-1");

        // Record spawn
        orchestrator.record_spawn("sess-1", &requests[0], &mut flights);
        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::Running);

        // Complete task-1 successfully
        orchestrator.on_task_complete("task-1", true, &mut flights);
        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::Done);
        // task-2 deps resolved → Queued
        assert_eq!(flights[0].milestones[0].tasks[1].status, TaskStatus::Queued);

        // Tick again → spawn request for task-2
        let requests2 = orchestrator.tick(&flights, &agents);
        assert_eq!(requests2.len(), 1);
        assert_eq!(requests2[0].task_id, "task-2");

        // Record spawn and complete task-2
        orchestrator.record_spawn("sess-2", &requests2[0], &mut flights);
        orchestrator.on_task_complete("task-2", true, &mut flights);

        assert_eq!(flights[0].milestones[0].status, MilestoneStatus::Done);
        assert_eq!(flights[0].status, FlightStatus::Done);
    }

    #[test]
    fn test_milestone_gating_pauses_flight() {
        let mut orchestrator = Orchestrator::new(OrchestratorSettings {
            milestone_gating: true,
            ..OrchestratorSettings::default()
        });
        let agents = vec![sample_agent()];

        let mut task1 = sample_task("task-1");
        task1.milestone_id = "ms-1".to_string();

        let mut task2 = sample_task("task-2");
        task2.id = "task-2".to_string();
        task2.milestone_id = "ms-2".to_string();

        let mut flight = Flight {
            id: "flight-1".to_string(),
            title: "Flight".to_string(),
            objective: "Objective".to_string(),
            status: FlightStatus::Draft,
            priority: FlightPriority::High,
            project_path: "D:/projects/FlightDeck".to_string(),
            git_branch: Some("feature/test".to_string()),
            milestones: vec![
                Milestone {
                    id: "ms-1".to_string(),
                    flight_id: "flight-1".to_string(),
                    title: "Milestone 1".to_string(),
                    description: "Desc".to_string(),
                    order: 0,
                    status: MilestoneStatus::Pending,
                    tasks: vec![task1],
                    validation_criteria: Vec::new(),
                },
                Milestone {
                    id: "ms-2".to_string(),
                    flight_id: "flight-1".to_string(),
                    title: "Milestone 2".to_string(),
                    description: "Desc".to_string(),
                    order: 1,
                    status: MilestoneStatus::Pending,
                    tasks: vec![task2],
                    validation_criteria: Vec::new(),
                },
            ],
            linked_session_ids: Vec::new(),
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            total_cost: 0.0,
            total_tokens: 0,
        };

        // Launch, tick, spawn, complete task-1
        orchestrator.launch_flight(&mut flight);
        let mut flights = vec![flight];

        let requests = orchestrator.tick(&flights, &agents);
        assert_eq!(requests.len(), 1);
        orchestrator.record_spawn("sess-1", &requests[0], &mut flights);
        orchestrator.on_task_complete("task-1", true, &mut flights);

        // ms-1 done, flight should be in Review due to milestone gating
        assert_eq!(flights[0].milestones[0].status, MilestoneStatus::Done);
        assert_eq!(flights[0].status, FlightStatus::Review);
        assert!(orchestrator.paused_at_milestone.contains_key("flight-1"));
        assert_eq!(orchestrator.paused_at_milestone.get("flight-1").unwrap(), "ms-2");

        // Resume flight
        orchestrator.resume_flight(&mut flights[0]);
        assert_eq!(flights[0].status, FlightStatus::Active);
        assert_eq!(flights[0].milestones[1].status, MilestoneStatus::Active);
        assert_eq!(flights[0].milestones[1].tasks[0].status, TaskStatus::Queued);

        // Tick, spawn, complete task-2
        let requests2 = orchestrator.tick(&flights, &agents);
        assert_eq!(requests2.len(), 1);
        assert_eq!(requests2[0].task_id, "task-2");
        orchestrator.record_spawn("sess-2", &requests2[0], &mut flights);
        orchestrator.on_task_complete("task-2", true, &mut flights);

        assert_eq!(flights[0].status, FlightStatus::Done);
    }

    #[test]
    fn test_task_failure_fails_milestone_and_flight() {
        let mut orchestrator = Orchestrator::new(OrchestratorSettings::default());
        let agents = vec![sample_agent()];
        let mut flight = sample_flight();
        flight.status = FlightStatus::Draft;

        orchestrator.launch_flight(&mut flight);
        let mut flights = vec![flight];

        let requests = orchestrator.tick(&flights, &agents);
        assert_eq!(requests.len(), 1);
        orchestrator.record_spawn("sess-1", &requests[0], &mut flights);

        // Complete task-1 with failure
        orchestrator.on_task_complete("task-1", false, &mut flights);

        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::Failed);
        assert_eq!(flights[0].milestones[0].status, MilestoneStatus::Failed);
        assert_eq!(flights[0].status, FlightStatus::Failed);
    }

    #[test]
    fn test_cancel_flight_cancels_all_tasks() {
        let mut orchestrator = Orchestrator::new(OrchestratorSettings::default());
        let agents = vec![sample_agent()];
        let mut flight = sample_flight();
        flight.status = FlightStatus::Draft;

        orchestrator.launch_flight(&mut flight);
        let mut flights = vec![flight];

        let requests = orchestrator.tick(&flights, &agents);
        assert_eq!(requests.len(), 1);
        orchestrator.record_spawn("sess-1", &requests[0], &mut flights);
        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::Running);

        // Cancel flight
        orchestrator.cancel_flight(&mut flights[0]);

        assert_eq!(flights[0].status, FlightStatus::Cancelled);
        assert_eq!(flights[0].milestones[0].tasks[0].status, TaskStatus::Cancelled);
        assert!(orchestrator.running_tasks.is_empty());
    }
}
