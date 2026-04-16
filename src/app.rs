use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use regex::Regex;
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::core::agent;
use crate::core::agent_config::AgentConfig;
use crate::core::flight::*;
use crate::core::git;
use crate::core::orchestrator::{Orchestrator, OrchestratorSettings};
use crate::core::pty::{PtyEvent, PtyManager};
use crate::core::storage::{self, PersistedState, PersistedUiState};

use super::command_palette::{self, CommandPalette};
use super::theme::Theme;
use super::views;
use super::widgets::diff::{self, DiffFile, DiffViewState};
use super::widgets::help::HelpOverlay;
use super::widgets::toast::{ToastManager, ToastLevel};

const SESSION_BUFFER_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum AppView {
    Dashboard,
    FlightDetail(String),
    FlightEditor,
    Sessions,
    Agents,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorFocus {
    Meta,
    Milestones,
    Tasks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardFocus {
    Flights,
    Attention,
}

#[derive(Debug, Clone)]
pub enum EditorInputTarget {
    FlightTitle,
    FlightObjective,
    FlightGitBranch,
    FlightProjectPath,
    MilestoneTitle(usize),
    MilestoneDescription(usize),
    MilestoneValidation(usize),
    TaskTitle(usize, usize),
    TaskDescription(usize, usize),
    TaskModel(usize, usize),
    TaskDependencies(usize, usize),
}

#[derive(Debug, Clone)]
pub struct EditorInput {
    pub target: EditorInputTarget,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct FlightEditorState {
    pub original_flight_id: Option<String>,
    pub draft: Flight,
    pub focus: EditorFocus,
    pub selected_meta_idx: usize,
    pub selected_milestone_idx: usize,
    pub selected_task_idx: usize,
    pub input: Option<EditorInput>,
}

#[derive(Debug, Clone)]
pub struct SessionBuffer {
    pub session_id: String,
    pub flight_id: String,
    pub task_id: String,
    pub agent_config_id: String,
    pub title: String,
    pub project_path: String,
    pub started_at: u64,
    pub output: String,
    pub unread: bool,
    pub exited: bool,
    pub exit_code: Option<i32>,
    pub success: Option<bool>,
    pub killed: bool,
    pub scroll: u16,
    pub agent_state: AgentRuntimeState,
    pub current_tool: Option<String>,
    pub current_file: Option<String>,
    pub needs_approval: bool,
    pub auto_scroll: bool,
    pub unread_count: u32,
    pub detected_diffs: Vec<DiffFile>,
    pub recent_tools: Vec<String>,
    pub doom_loop_detected: bool,
    runtime_buffer: String,
}

#[derive(Debug, Clone)]
pub struct AttentionItem {
    pub flight_id: String,
    pub milestone_title: String,
    pub task_title: Option<String>,
    pub kind: AttentionKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionKind {
    Approval,
    FailedTask,
    FlightReview,
    FlightPaused,
}

impl AttentionKind {
    pub fn priority(self) -> u8 {
        match self {
            AttentionKind::FailedTask => 0,
            AttentionKind::Approval => 1,
            AttentionKind::FlightReview => 2,
            AttentionKind::FlightPaused => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AttentionKind::Approval => "approval",
            AttentionKind::FailedTask => "failed",
            AttentionKind::FlightReview => "review",
            AttentionKind::FlightPaused => "paused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRuntimeState {
    Idle,
    Thinking,
    ToolUse,
    Responding,
    ApprovalNeeded,
}

#[derive(Debug, Clone, Copy)]
enum ApprovalAction {
    Approve,
    Deny,
    Abort,
}

#[derive(Debug, Clone)]
enum SettingsInputTarget {
    ProjectPath,
    CommitMessage,
}

#[derive(Debug, Clone)]
pub struct SettingsInput {
    target: SettingsInputTarget,
    pub value: String,
}

impl SessionBuffer {
    fn append_output(&mut self, data: &str, focused: bool) {
        self.output.push_str(data);
        if self.output.len() > SESSION_BUFFER_LIMIT {
            let overflow = self.output.len() - SESSION_BUFFER_LIMIT;
            self.output.drain(..overflow);
        }
        if !focused {
            self.unread = true;
            self.unread_count += 1;
        }
        if self.auto_scroll {
            self.scroll = u16::MAX;
        }
    }

    pub fn status_label(&self) -> &'static str {
        if !self.exited {
            "running"
        } else if self.killed {
            "killed"
        } else if self.success.unwrap_or(false) {
            "done"
        } else {
            "failed"
        }
    }

    pub fn runtime_label(&self) -> &'static str {
        match self.agent_state {
            AgentRuntimeState::Idle => "idle",
            AgentRuntimeState::Thinking => "thinking",
            AgentRuntimeState::ToolUse => "tool",
            AgentRuntimeState::Responding => "responding",
            AgentRuntimeState::ApprovalNeeded => "approval",
        }
    }
}

pub struct App {
    pub view: AppView,
    pub flights: Vec<Flight>,
    pub agents: Vec<AgentConfig>,
    pub settings: OrchestratorSettings,
    pub orchestrator: Orchestrator,
    pub pty_manager: Arc<Mutex<PtyManager>>,
    pty_rx: mpsc::Receiver<PtyEvent>,
    pub selected_flight_idx: usize,
    pub selected_attention_idx: usize,
    pub selected_agent_idx: usize,
    pub selected_session_idx: usize,
    pub dashboard_focus: DashboardFocus,
    pub flight_editor: Option<FlightEditorState>,
    pub session_buffers: HashMap<String, SessionBuffer>,
    pub session_order: Vec<String>,
    pub git_branch: Option<String>,
    pub git_status_summary: Option<String>,
    pub git_last_message: Option<String>,
    pub settings_input: Option<SettingsInput>,
    pub theme: Theme,
    pub leader_pending: Option<Instant>,
    pub command_palette: CommandPalette,
    pub toasts: ToastManager,
    pub diff_view: DiffViewState,
    pub session_filter: Option<String>,
    pub session_filter_input: bool,
    pub session_search: Option<String>,
    pub session_search_input: bool,
    pub session_search_matches: Vec<usize>,
    pub session_search_index: usize,
    pub help_overlay: HelpOverlay,
    pub retrospectives: Vec<storage::FlightRetrospective>,
    suppressed_exit_sessions: HashSet<String>,
}

impl App {
    pub fn new(
        mut flights: Vec<Flight>,
        agents: Vec<AgentConfig>,
        settings: OrchestratorSettings,
        persisted_ui: PersistedUiState,
        pty_tx: mpsc::Sender<PtyEvent>,
        pty_rx: mpsc::Receiver<PtyEvent>,
    ) -> Self {
        let mut orchestrator = Orchestrator::new(settings.clone());
        orchestrator.recover_from_flights(&mut flights);

        let mut all_agents = AgentConfig::builtins();
        for custom in agents.into_iter().filter(|a| !a.is_builtin) {
            if !all_agents.iter().any(|a| a.id == custom.id) {
                all_agents.push(custom);
            }
        }

        Self {
            view: AppView::Dashboard,
            flights,
            agents: all_agents,
            settings,
            orchestrator,
            pty_manager: Arc::new(Mutex::new(PtyManager::new(pty_tx))),
            pty_rx,
            selected_flight_idx: 0,
            selected_attention_idx: 0,
            selected_agent_idx: 0,
            selected_session_idx: 0,
            dashboard_focus: DashboardFocus::Flights,
            flight_editor: None,
            session_buffers: HashMap::new(),
            session_order: Vec::new(),
            git_branch: None,
            git_status_summary: None,
            git_last_message: None,
            settings_input: None,
            theme: super::theme::load_theme(persisted_ui.theme.as_deref()),
            leader_pending: None,
            command_palette: CommandPalette::new(),
            toasts: ToastManager::new(),
            diff_view: DiffViewState::default(),
            session_filter: None,
            session_filter_input: false,
            session_search: None,
            session_search_input: false,
            session_search_matches: Vec::new(),
            session_search_index: 0,
            help_overlay: HelpOverlay::new(),
            retrospectives: Vec::new(),
            suppressed_exit_sessions: HashSet::new(),
        }
    }

    pub fn detect_agents(&mut self) {
        for a in &mut self.agents {
            a.installed = agent::detect_agent(&a.command);
        }
        let _ = self.persist_state();
    }

    pub fn persist_state(&self) -> Result<(), String> {
        storage::save_state(&PersistedState {
            version: 1,
            flights: self.flights.clone(),
            agents: self.agents.clone(),
            settings: self.settings.clone(),
            ui: PersistedUiState {
                selected_flight_id: self.flights.get(self.selected_flight_idx).map(|flight| flight.id.clone()),
                selected_view: Some(match &self.view {
                    AppView::Dashboard => "dashboard",
                    AppView::FlightDetail(_) => "flight_detail",
                    AppView::FlightEditor => "flight_editor",
                    AppView::Sessions => "sessions",
                    AppView::Agents => "agents",
                    AppView::Settings => "settings",
                }.to_string()),
                theme: Some(self.theme.name.clone()),
            },
            issues: Vec::new(),
            approval_log: Vec::new(),
            workspaces: Vec::new(),
            retrospectives: self.retrospectives.clone(),
            memory_events: Vec::new(),
            memory_patterns: Vec::new(),
            servers: Vec::new(),
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Help overlay takes highest priority when visible
        if self.help_overlay.visible {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc => self.help_overlay.toggle(),
                KeyCode::PageUp | KeyCode::Up => {
                    self.help_overlay.scroll = self.help_overlay.scroll.saturating_sub(3);
                }
                KeyCode::PageDown | KeyCode::Down => {
                    self.help_overlay.scroll = self.help_overlay.scroll.saturating_add(3);
                }
                _ => {}
            }
            return false;
        }

        // ? opens help overlay from any view (except text input modes)
        if key.code == KeyCode::Char('?') && !self.is_text_input_active() {
            self.help_overlay.toggle();
            return false;
        }

        // Command palette takes priority when visible
        if self.command_palette.visible {
            return self.handle_palette_key(key);
        }

        // Ctrl+P opens command palette from any view
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open_command_palette();
            return false;
        }

        // Leader key handling — Space or Ctrl+X starts, next key within 1s triggers combo
        if let Some(started) = self.leader_pending {
            self.leader_pending = None;
            if started.elapsed() < Duration::from_secs(1) {
                return self.handle_leader_combo(key);
            }
            // Timed out, fall through to normal handling
        }

        if key.code == KeyCode::Char('x') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.leader_pending = Some(Instant::now());
            return false;
        }

        // Space activates leader overlay (when not typing in an input field)
        if key.code == KeyCode::Char(' ') && !self.is_text_input_active() {
            self.leader_pending = Some(Instant::now());
            return false;
        }

        match &self.view {
            AppView::Dashboard => self.handle_dashboard_key(key),
            AppView::FlightDetail(_) => self.handle_detail_key(key),
            AppView::FlightEditor => self.handle_flight_editor_key(key),
            AppView::Sessions => self.handle_sessions_key(key),
            AppView::Agents => self.handle_agents_key(key),
            AppView::Settings => self.handle_settings_key(key),
        }
    }

    fn handle_leader_combo(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('f') => self.view = AppView::Dashboard,
            KeyCode::Char('d') => self.view = AppView::Dashboard,
            KeyCode::Char('s') => self.view = AppView::Sessions,
            KeyCode::Char('a') => self.view = AppView::Agents,
            KeyCode::Char('n') => self.start_create_flight(),
            KeyCode::Char('4') => self.view = AppView::Settings,
            KeyCode::Char('t') => {
                let next = super::theme::next_theme_name(&self.theme.name);
                self.theme = super::theme::load_theme(Some(next));
                let _ = self.persist_state();
            }
            KeyCode::Char('q') => return true, // quit
            KeyCode::Char('?') => self.help_overlay.toggle(),
            KeyCode::Char('p') => self.open_command_palette(),
            KeyCode::Char('g') => self.refresh_git_context(),
            _ => {} // unknown combo, ignore
        }
        false
    }

    fn is_text_input_active(&self) -> bool {
        self.session_filter_input
            || self.session_search_input
            || self.settings_input.is_some()
            || self
                .flight_editor
                .as_ref()
                .map(|e| e.input.is_some())
                .unwrap_or(false)
    }

    fn view_name(&self) -> &'static str {
        match &self.view {
            AppView::Dashboard => "dashboard",
            AppView::FlightDetail(_) => "flight_detail",
            AppView::FlightEditor => "flight_editor",
            AppView::Sessions => "sessions",
            AppView::Agents => "agents",
            AppView::Settings => "settings",
        }
    }

    fn open_command_palette(&mut self) {
        let names = super::theme::theme_names();
        let cmds = command_palette::build_commands(
            &names,
            &self.theme.name,
            !self.flights.is_empty(),
        );
        self.command_palette.open(cmds);
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.command_palette.close(),
            KeyCode::Enter => {
                if let Some(cmd_id) = self.command_palette.selected_command_id() {
                    self.execute_palette_command(cmd_id);
                }
                self.command_palette.close();
            }
            KeyCode::Up => self.command_palette.move_up(),
            KeyCode::Down => self.command_palette.move_down(),
            KeyCode::Backspace => self.command_palette.backspace(),
            KeyCode::Char(ch) => self.command_palette.type_char(ch),
            _ => {}
        }
        false
    }

    fn execute_palette_command(&mut self, cmd_id: &str) {
        match cmd_id {
            "nav.dashboard" => self.view = AppView::Dashboard,
            "nav.sessions" => self.view = AppView::Sessions,
            "nav.agents" => self.view = AppView::Agents,
            "nav.settings" => self.view = AppView::Settings,
            "flight.create" => self.start_create_flight(),
            "flight.launch" => {
                if let Some(f) = self.flights.get_mut(self.selected_flight_idx) {
                    if matches!(f.status, FlightStatus::Draft | FlightStatus::Ready) {
                        self.orchestrator.launch_flight(f);
                        let _ = self.persist_state();
                    }
                }
            }
            "flight.pause" => {
                if let Some(flight) = self.flights.get(self.selected_flight_idx) {
                    let fid = flight.id.clone();
                    self.pause_flight(&fid);
                }
            }
            "git.refresh" => self.refresh_git_context(),
            "git.pull" => self.run_git_pull(),
            "git.push" => self.run_git_push(),
            id if id.starts_with("theme.") => {
                let theme_name = &id[6..];
                self.theme = super::theme::load_theme(Some(theme_name));
                let _ = self.persist_state();
            }
            _ => {}
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('1') => self.view = AppView::Dashboard,
            KeyCode::Char('2') => self.view = AppView::Sessions,
            KeyCode::Char('3') => self.view = AppView::Agents,
            KeyCode::Char('4') => self.view = AppView::Settings,
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.dashboard_focus = match self.dashboard_focus {
                    DashboardFocus::Flights => DashboardFocus::Attention,
                    DashboardFocus::Attention => DashboardFocus::Flights,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match self.dashboard_focus {
                    DashboardFocus::Flights => {
                        if self.selected_flight_idx > 0 {
                            self.selected_flight_idx -= 1;
                        }
                    }
                    DashboardFocus::Attention => {
                        if self.selected_attention_idx > 0 {
                            self.selected_attention_idx -= 1;
                        }
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match self.dashboard_focus {
                    DashboardFocus::Flights => {
                        if self.selected_flight_idx + 1 < self.flights.len() {
                            self.selected_flight_idx += 1;
                        }
                    }
                    DashboardFocus::Attention => {
                        let len = self.attention_items().len();
                        if self.selected_attention_idx + 1 < len {
                            self.selected_attention_idx += 1;
                        }
                    }
                }
            }
            KeyCode::Enter => {
                match self.dashboard_focus {
                    DashboardFocus::Flights => {
                        if let Some(f) = self.flights.get(self.selected_flight_idx) {
                            self.view = AppView::FlightDetail(f.id.clone());
                        }
                    }
                    DashboardFocus::Attention => {
                        if let Some(item) = self.attention_items().get(self.selected_attention_idx) {
                            self.view = AppView::FlightDetail(item.flight_id.clone());
                        }
                    }
                }
            }
            KeyCode::Char('l') => {
                match self.dashboard_focus {
                    DashboardFocus::Flights => {
                        if let Some(f) = self.flights.get_mut(self.selected_flight_idx) {
                            if matches!(f.status, FlightStatus::Draft | FlightStatus::Ready) {
                                self.orchestrator.launch_flight(f);
                                let _ = self.persist_state();
                            }
                        }
                    }
                    DashboardFocus::Attention => {
                        if let Some(item) = self.attention_items().get(self.selected_attention_idx) {
                            self.view = AppView::FlightDetail(item.flight_id.clone());
                        }
                    }
                }
            }
            KeyCode::Char('s') => {
                self.view = AppView::Sessions;
            }
            KeyCode::Char('c') => self.start_create_flight(),
            KeyCode::Char('e') => {
                if let Some(f) = self.flights.get(self.selected_flight_idx) {
                    let flight_id = f.id.clone();
                    self.start_edit_flight(&flight_id);
                }
            }
            _ => {}
        }
        false
    }

    fn handle_detail_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.view = AppView::Dashboard,
            KeyCode::Char('1') => self.view = AppView::Dashboard,
            KeyCode::Char('2') => self.view = AppView::Sessions,
            KeyCode::Char('3') => self.view = AppView::Agents,
            KeyCode::Char('4') => self.view = AppView::Settings,
            KeyCode::Char('l') => {
                if let AppView::FlightDetail(id) = &self.view {
                    if let Some(f) = self.flights.iter_mut().find(|f| f.id == *id) {
                        match f.status {
                            FlightStatus::Draft | FlightStatus::Ready => self.orchestrator.launch_flight(f),
                            FlightStatus::Paused | FlightStatus::Review => self.orchestrator.resume_flight(f),
                            _ => {}
                        }
                        let _ = self.persist_state();
                    }
                }
            }
            KeyCode::Char('p') => {
                if let AppView::FlightDetail(id) = &self.view {
                    let flight_id = id.clone();
                    self.pause_flight(&flight_id);
                }
            }
            KeyCode::Char('c') => {
                if let AppView::FlightDetail(id) = &self.view {
                    let flight_id = id.clone();
                    self.cancel_flight(&flight_id);
                }
            }
            KeyCode::Char('y') => self.send_flight_approval_action(ApprovalAction::Approve),
            KeyCode::Char('n') => self.send_flight_approval_action(ApprovalAction::Deny),
            KeyCode::Char('a') => self.send_flight_approval_action(ApprovalAction::Abort),
            KeyCode::Char('s') => self.view = AppView::Sessions,
            KeyCode::Char('e') => {
                if let AppView::FlightDetail(id) = &self.view {
                    let flight_id = id.clone();
                    self.start_edit_flight(&flight_id);
                }
            }
            _ => {}
        }
        false
    }

    fn handle_flight_editor_key(&mut self, key: KeyEvent) -> bool {
        if self.flight_editor.is_none() {
            self.view = AppView::Dashboard;
            return false;
        }

        if self.handle_flight_editor_input_key(key) {
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc => self.exit_flight_editor(false),
            KeyCode::Tab => self.cycle_editor_focus(),
            KeyCode::BackTab => self.cycle_editor_focus_reverse(),
            KeyCode::Up | KeyCode::Char('k') => self.move_editor_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_editor_selection(1),
            KeyCode::Enter => self.activate_editor_selection(),
            KeyCode::Char('a') => self.add_editor_item(),
            KeyCode::Char('d') => self.delete_editor_item(),
            KeyCode::Char('e') => self.edit_current_editor_item(false),
            KeyCode::Char('o') => self.edit_current_editor_item(true),
            KeyCode::Char('v') => self.edit_selected_milestone_validation(),
            KeyCode::Char('p') => self.cycle_editor_priority(),
            KeyCode::Char('t') => self.cycle_selected_task_type(),
            KeyCode::Char('g') => self.cycle_selected_task_agent(),
            KeyCode::Char('m') => self.edit_selected_task_model(),
            KeyCode::Char('r') => self.edit_selected_task_dependencies(),
            KeyCode::Char('[') => self.move_editor_item(-1),
            KeyCode::Char(']') => self.move_editor_item(1),
            KeyCode::Char('S') => self.save_flight_editor(),
            KeyCode::Char('1') => self.view = AppView::Dashboard,
            KeyCode::Char('2') => self.view = AppView::Sessions,
            KeyCode::Char('3') => self.view = AppView::Agents,
            KeyCode::Char('4') => self.view = AppView::Settings,
            _ => {}
        }

        false
    }

    fn handle_sessions_key(&mut self, key: KeyEvent) -> bool {
        // Diff popup mode
        if self.diff_view.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('d') => self.diff_view.visible = false,
                KeyCode::PageUp | KeyCode::Up => {
                    self.diff_view.scroll = self.diff_view.scroll.saturating_sub(5);
                }
                KeyCode::PageDown | KeyCode::Down => {
                    self.diff_view.scroll = self.diff_view.scroll.saturating_add(5);
                }
                _ => {}
            }
            return false;
        }

        // Session search input mode (transcript search)
        if self.session_search_input {
            match key.code {
                KeyCode::Esc => {
                    self.session_search = None;
                    self.session_search_input = false;
                    self.session_search_matches.clear();
                    self.session_search_index = 0;
                }
                KeyCode::Enter => {
                    self.session_search_input = false;
                    // Jump to first match
                    if !self.session_search_matches.is_empty() {
                        self.session_search_index = 0;
                        self.jump_to_search_match();
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ref mut search) = self.session_search {
                        search.pop();
                        if search.is_empty() {
                            self.session_search = None;
                            self.session_search_input = false;
                            self.session_search_matches.clear();
                            self.session_search_index = 0;
                        } else {
                            self.update_search_matches();
                        }
                    }
                }
                KeyCode::Char(ch) => {
                    self.session_search
                        .get_or_insert_with(String::new)
                        .push(ch);
                    self.update_search_matches();
                }
                _ => {}
            }
            return false;
        }

        // Session filter input mode
        if self.session_filter_input {
            match key.code {
                KeyCode::Esc => {
                    self.session_filter = None;
                    self.session_filter_input = false;
                }
                KeyCode::Enter => {
                    self.session_filter_input = false;
                }
                KeyCode::Backspace => {
                    if let Some(ref mut filter) = self.session_filter {
                        filter.pop();
                        if filter.is_empty() {
                            self.session_filter = None;
                            self.session_filter_input = false;
                        }
                    }
                }
                KeyCode::Char(ch) => {
                    self.session_filter
                        .get_or_insert_with(String::new)
                        .push(ch);
                }
                _ => {}
            }
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('1') => self.view = AppView::Dashboard,
            KeyCode::Char('2') => self.view = AppView::Sessions,
            KeyCode::Char('3') => self.view = AppView::Agents,
            KeyCode::Char('4') => self.view = AppView::Settings,
            KeyCode::Esc => {
                if self.session_search.is_some() {
                    self.session_search = None;
                    self.session_search_matches.clear();
                    self.session_search_index = 0;
                } else {
                    self.view = AppView::Dashboard;
                }
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => self.navigate_session_hierarchy_up(),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => self.navigate_session_hierarchy_down(),
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_session_idx > 0 {
                    self.selected_session_idx -= 1;
                }
                self.clear_selected_session_unread();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_session_idx + 1 < self.session_order.len() {
                    self.selected_session_idx += 1;
                }
                self.clear_selected_session_unread();
            }
            KeyCode::Char('x') => {
                if let Some(session_id) = self.selected_session_id().map(str::to_string) {
                    self.kill_session(&session_id, false);
                }
            }
            KeyCode::Char('y') => self.send_selected_approval_action(ApprovalAction::Approve),
            KeyCode::Char('n') if self.session_search.is_some() => {
                // Next search match
                if !self.session_search_matches.is_empty() {
                    self.session_search_index = (self.session_search_index + 1) % self.session_search_matches.len();
                    self.jump_to_search_match();
                }
            }
            KeyCode::Char('N') if self.session_search.is_some() => {
                // Previous search match
                if !self.session_search_matches.is_empty() {
                    if self.session_search_index == 0 {
                        self.session_search_index = self.session_search_matches.len() - 1;
                    } else {
                        self.session_search_index -= 1;
                    }
                    self.jump_to_search_match();
                }
            }
            KeyCode::Char('n') => self.send_selected_approval_action(ApprovalAction::Deny),
            KeyCode::Char('a') => self.send_selected_approval_action(ApprovalAction::Abort),
            KeyCode::Char('g') => {
                if let Some(session_id) = self.selected_session_id().map(str::to_string) {
                    if let Some(session) = self.session_buffers.get_mut(&session_id) {
                        session.scroll = 0;
                        session.auto_scroll = false;
                    }
                }
            }
            KeyCode::Char('G') => {
                if let Some(session_id) = self.selected_session_id().map(str::to_string) {
                    if let Some(session) = self.session_buffers.get_mut(&session_id) {
                        session.scroll = u16::MAX;
                        session.auto_scroll = true;
                    }
                }
            }
            KeyCode::Char('/') => {
                self.session_filter = Some(String::new());
                self.session_filter_input = true;
            }
            KeyCode::Char('?') => {
                self.session_search = Some(String::new());
                self.session_search_input = true;
                self.session_search_matches.clear();
                self.session_search_index = 0;
            }
            KeyCode::Char('D') => {
                // Open diff viewer for current session
                if let Some(session_id) = self.selected_session_id() {
                    if let Some(session) = self.session_buffers.get(session_id) {
                        let diffs = diff::extract_diffs_from_output(&session.output);
                        if !diffs.is_empty() {
                            // Store diffs on the session buffer
                            let session_id = session_id.to_string();
                            if let Some(buf) = self.session_buffers.get_mut(&session_id) {
                                buf.detected_diffs = diffs;
                            }
                            self.diff_view.visible = true;
                            self.diff_view.scroll = 0;
                        }
                    }
                }
            }
            KeyCode::Char('E') => self.export_selected_session(),
            KeyCode::PageUp => self.adjust_session_scroll(-10),
            KeyCode::PageDown => self.adjust_session_scroll(10),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.adjust_session_scroll(-10),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.adjust_session_scroll(10),
            _ => {}
        }
        false
    }

    fn handle_agents_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('1') => self.view = AppView::Dashboard,
            KeyCode::Char('2') => self.view = AppView::Sessions,
            KeyCode::Char('4') => self.view = AppView::Settings,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_agent_idx > 0 {
                    self.selected_agent_idx -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_agent_idx + 1 < self.agents.len() {
                    self.selected_agent_idx += 1;
                }
            }
            KeyCode::Char('r') => self.detect_agents(),
            _ => {}
        }
        false
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> bool {
        if self.handle_settings_input_key(key) {
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('1') => self.view = AppView::Dashboard,
            KeyCode::Char('2') => self.view = AppView::Sessions,
            KeyCode::Char('3') => self.view = AppView::Agents,
            KeyCode::Left => {
                if self.settings.max_parallel_sessions > 1 {
                    self.settings.max_parallel_sessions -= 1;
                    self.orchestrator.settings.max_parallel_sessions = self.settings.max_parallel_sessions;
                    let _ = self.persist_state();
                }
            }
            KeyCode::Right => {
                if self.settings.max_parallel_sessions < 8 {
                    self.settings.max_parallel_sessions += 1;
                    self.orchestrator.settings.max_parallel_sessions = self.settings.max_parallel_sessions;
                    let _ = self.persist_state();
                }
            }
            KeyCode::Char('m') => {
                self.settings.milestone_gating = !self.settings.milestone_gating;
                self.orchestrator.settings.milestone_gating = self.settings.milestone_gating;
                let _ = self.persist_state();
            }
            KeyCode::Char('e') => {
                self.settings_input = Some(SettingsInput {
                    target: SettingsInputTarget::ProjectPath,
                    value: self.settings.project_path.clone(),
                });
            }
            KeyCode::Char('c') => {
                self.settings.project_path = default_project_path();
                self.orchestrator.settings.project_path = self.settings.project_path.clone();
                let _ = self.persist_state();
                self.refresh_git_context();
            }
            KeyCode::Char('g') => self.refresh_git_context(),
            KeyCode::Char('P') => self.run_git_push(),
            KeyCode::Char('L') => self.run_git_pull(),
            KeyCode::Char('C') => {
                self.settings_input = Some(SettingsInput {
                    target: SettingsInputTarget::CommitMessage,
                    value: String::new(),
                });
            }
            KeyCode::Char('t') => {
                let next = super::theme::next_theme_name(&self.theme.name);
                self.theme = super::theme::load_theme(Some(next));
                let _ = self.persist_state();
            }
            _ => {}
        }
        false
    }

    fn start_create_flight(&mut self) {
        self.flight_editor = Some(FlightEditorState {
            original_flight_id: None,
            draft: blank_flight(self.settings.project_path.clone()),
            focus: EditorFocus::Meta,
            selected_meta_idx: 0,
            selected_milestone_idx: 0,
            selected_task_idx: 0,
            input: None,
        });
        self.view = AppView::FlightEditor;
    }

    fn start_edit_flight(&mut self, flight_id: &str) {
        let Some(flight) = self.flights.iter().find(|f| f.id == flight_id).cloned() else {
            return;
        };

        self.flight_editor = Some(FlightEditorState {
            original_flight_id: Some(flight_id.to_string()),
            draft: flight,
            focus: EditorFocus::Meta,
            selected_meta_idx: 0,
            selected_milestone_idx: 0,
            selected_task_idx: 0,
            input: None,
        });
        self.view = AppView::FlightEditor;
    }

    fn exit_flight_editor(&mut self, saved: bool) {
        let original_id = self
            .flight_editor
            .as_ref()
            .and_then(|editor| editor.original_flight_id.clone());

        self.flight_editor = None;
        self.view = if saved {
            match original_id {
                Some(id) => AppView::FlightDetail(id),
                None => {
                    if let Some(flight) = self.flights.get(self.selected_flight_idx) {
                        AppView::FlightDetail(flight.id.clone())
                    } else {
                        AppView::Dashboard
                    }
                }
            }
        } else {
            match original_id {
                Some(id) => AppView::FlightDetail(id),
                None => AppView::Dashboard,
            }
        };
    }

    fn handle_flight_editor_input_key(&mut self, key: KeyEvent) -> bool {
        let Some(editor) = self.flight_editor.as_mut() else {
            return false;
        };
        let Some(input) = editor.input.as_mut() else {
            return false;
        };

        match key.code {
            KeyCode::Esc => {
                editor.input = None;
            }
            KeyCode::Enter => {
                let target = input.target.clone();
                let value = input.value.trim().to_string();
                editor.input = None;
                self.apply_editor_input(target, value);
            }
            KeyCode::Backspace => {
                input.value.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.value.push(c);
            }
            _ => {}
        }

        true
    }

    fn handle_settings_input_key(&mut self, key: KeyEvent) -> bool {
        let Some(input) = self.settings_input.as_mut() else {
            return false;
        };

        match key.code {
            KeyCode::Esc => self.settings_input = None,
            KeyCode::Enter => {
                let target = input.target.clone();
                let value = input.value.trim().to_string();
                self.settings_input = None;
                match target {
                    SettingsInputTarget::ProjectPath => {
                        if !value.is_empty() {
                            self.settings.project_path = value.clone();
                            self.orchestrator.settings.project_path = value;
                            let _ = self.persist_state();
                            self.refresh_git_context();
                        }
                    }
                    SettingsInputTarget::CommitMessage => {
                        if !value.is_empty() {
                            self.run_git_commit(&value);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                input.value.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.value.push(c);
            }
            _ => {}
        }

        true
    }

    fn apply_editor_input(&mut self, target: EditorInputTarget, value: String) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        match target {
            EditorInputTarget::FlightTitle => editor.draft.title = value,
            EditorInputTarget::FlightObjective => editor.draft.objective = value,
            EditorInputTarget::FlightGitBranch => {
                editor.draft.git_branch = if value.is_empty() { None } else { Some(value) }
            }
            EditorInputTarget::FlightProjectPath => editor.draft.project_path = value,
            EditorInputTarget::MilestoneTitle(ms_idx) => {
                if let Some(ms) = editor.draft.milestones.get_mut(ms_idx) {
                    ms.title = value;
                }
            }
            EditorInputTarget::MilestoneDescription(ms_idx) => {
                if let Some(ms) = editor.draft.milestones.get_mut(ms_idx) {
                    ms.description = value;
                }
            }
            EditorInputTarget::MilestoneValidation(ms_idx) => {
                if let Some(ms) = editor.draft.milestones.get_mut(ms_idx) {
                    ms.validation_criteria = value
                        .split('|')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToString::to_string)
                        .collect();
                }
            }
            EditorInputTarget::TaskTitle(ms_idx, task_idx) => {
                if let Some(task) = editor
                    .draft
                    .milestones
                    .get_mut(ms_idx)
                    .and_then(|ms| ms.tasks.get_mut(task_idx))
                {
                    task.title = value;
                }
            }
            EditorInputTarget::TaskDescription(ms_idx, task_idx) => {
                if let Some(task) = editor
                    .draft
                    .milestones
                    .get_mut(ms_idx)
                    .and_then(|ms| ms.tasks.get_mut(task_idx))
                {
                    task.description = value;
                }
            }
            EditorInputTarget::TaskModel(ms_idx, task_idx) => {
                if let Some(task) = editor
                    .draft
                    .milestones
                    .get_mut(ms_idx)
                    .and_then(|ms| ms.tasks.get_mut(task_idx))
                {
                    task.model = if value.is_empty() { None } else { Some(value) };
                }
            }
            EditorInputTarget::TaskDependencies(ms_idx, task_idx) => {
                let dependency_ids = editor
                    .draft
                    .milestones
                    .get(ms_idx)
                    .map(|ms| parse_dependency_input(ms, task_idx, &value))
                    .unwrap_or_default();

                if let Some(task) = editor
                    .draft
                    .milestones
                    .get_mut(ms_idx)
                    .and_then(|ms| ms.tasks.get_mut(task_idx))
                {
                    task.depends_on = dependency_ids;
                }
            }
        }

        editor.draft.updated_at = now();
    }

    fn cycle_editor_focus(&mut self) {
        if let Some(editor) = self.flight_editor.as_mut() {
            editor.focus = match editor.focus {
                EditorFocus::Meta => EditorFocus::Milestones,
                EditorFocus::Milestones => EditorFocus::Tasks,
                EditorFocus::Tasks => EditorFocus::Meta,
            };
        }
    }

    fn cycle_editor_focus_reverse(&mut self) {
        if let Some(editor) = self.flight_editor.as_mut() {
            editor.focus = match editor.focus {
                EditorFocus::Meta => EditorFocus::Tasks,
                EditorFocus::Milestones => EditorFocus::Meta,
                EditorFocus::Tasks => EditorFocus::Milestones,
            };
        }
    }

    fn move_editor_selection(&mut self, delta: i32) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        match editor.focus {
            EditorFocus::Meta => {
                let max = 4_i32;
                editor.selected_meta_idx = (editor.selected_meta_idx as i32 + delta).clamp(0, max) as usize;
            }
            EditorFocus::Milestones => {
                if editor.draft.milestones.is_empty() {
                    editor.selected_milestone_idx = 0;
                    editor.selected_task_idx = 0;
                } else {
                    let max = editor.draft.milestones.len().saturating_sub(1) as i32;
                    editor.selected_milestone_idx = (editor.selected_milestone_idx as i32 + delta).clamp(0, max) as usize;
                    let task_max = editor
                        .draft
                        .milestones
                        .get(editor.selected_milestone_idx)
                        .map(|ms| ms.tasks.len().saturating_sub(1))
                        .unwrap_or(0);
                    editor.selected_task_idx = editor.selected_task_idx.min(task_max);
                }
            }
            EditorFocus::Tasks => {
                let task_count = editor
                    .draft
                    .milestones
                    .get(editor.selected_milestone_idx)
                    .map(|ms| ms.tasks.len())
                    .unwrap_or(0);
                if task_count == 0 {
                    editor.selected_task_idx = 0;
                } else {
                    let max = task_count.saturating_sub(1) as i32;
                    editor.selected_task_idx = (editor.selected_task_idx as i32 + delta).clamp(0, max) as usize;
                }
            }
        }
    }

    fn activate_editor_selection(&mut self) {
        let focus = self.flight_editor.as_ref().map(|editor| editor.focus);
        match focus {
            Some(EditorFocus::Meta) => self.edit_current_editor_item(false),
            Some(EditorFocus::Milestones) => self.edit_current_editor_item(false),
            Some(EditorFocus::Tasks) => self.edit_current_editor_item(false),
            None => {}
        }
    }

    fn edit_current_editor_item(&mut self, description: bool) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        let mut cycle_priority = false;

        let input = match editor.focus {
            EditorFocus::Meta => match editor.selected_meta_idx {
                0 => Some(EditorInput {
                    target: EditorInputTarget::FlightTitle,
                    value: editor.draft.title.clone(),
                }),
                1 => Some(EditorInput {
                    target: EditorInputTarget::FlightObjective,
                    value: editor.draft.objective.clone(),
                }),
                2 => {
                    cycle_priority = true;
                    None
                }
                3 => Some(EditorInput {
                    target: EditorInputTarget::FlightGitBranch,
                    value: editor.draft.git_branch.clone().unwrap_or_default(),
                }),
                4 => Some(EditorInput {
                    target: EditorInputTarget::FlightProjectPath,
                    value: editor.draft.project_path.clone(),
                }),
                _ => None,
            },
            EditorFocus::Milestones => {
                if editor.draft.milestones.is_empty() {
                    None
                } else {
                    let ms_idx = editor.selected_milestone_idx;
                    let milestone = &editor.draft.milestones[ms_idx];
                    Some(EditorInput {
                        target: if description {
                            EditorInputTarget::MilestoneDescription(ms_idx)
                        } else {
                            EditorInputTarget::MilestoneTitle(ms_idx)
                        },
                        value: if description {
                            milestone.description.clone()
                        } else {
                            milestone.title.clone()
                        },
                    })
                }
            }
            EditorFocus::Tasks => {
                let ms_idx = editor.selected_milestone_idx;
                let task_idx = editor.selected_task_idx;
                let Some(task) = editor
                    .draft
                    .milestones
                    .get(ms_idx)
                    .and_then(|ms| ms.tasks.get(task_idx))
                else {
                    return;
                };
                Some(EditorInput {
                    target: if description {
                        EditorInputTarget::TaskDescription(ms_idx, task_idx)
                    } else {
                        EditorInputTarget::TaskTitle(ms_idx, task_idx)
                    },
                    value: if description {
                        task.description.clone()
                    } else {
                        task.title.clone()
                    },
                })
            }
        };

        editor.input = input;
        if cycle_priority {
            self.cycle_editor_priority();
        }
    }

    fn cycle_editor_priority(&mut self) {
        if let Some(editor) = self.flight_editor.as_mut() {
            editor.draft.priority = match editor.draft.priority {
                FlightPriority::Low => FlightPriority::Medium,
                FlightPriority::Medium => FlightPriority::High,
                FlightPriority::High => FlightPriority::Critical,
                FlightPriority::Critical => FlightPriority::Low,
            };
            editor.draft.updated_at = now();
        }
    }

    fn cycle_selected_task_type(&mut self) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };
        if editor.focus != EditorFocus::Tasks {
            return;
        }

        if let Some(task) = editor
            .draft
            .milestones
            .get_mut(editor.selected_milestone_idx)
            .and_then(|ms| ms.tasks.get_mut(editor.selected_task_idx))
        {
            task.task_type = next_task_type(task.task_type);
            editor.draft.updated_at = now();
        }
    }

    fn edit_selected_milestone_validation(&mut self) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };
        if editor.focus != EditorFocus::Milestones {
            return;
        }

        let ms_idx = editor.selected_milestone_idx;
        let Some(ms) = editor.draft.milestones.get(ms_idx) else {
            return;
        };

        editor.input = Some(EditorInput {
            target: EditorInputTarget::MilestoneValidation(ms_idx),
            value: ms.validation_criteria.join(" | "),
        });
    }

    fn cycle_selected_task_agent(&mut self) {
        let next_agent_id = {
            let Some(editor) = self.flight_editor.as_ref() else {
                return;
            };
            if editor.focus != EditorFocus::Tasks {
                return;
            }

            let Some(task) = editor
                .draft
                .milestones
                .get(editor.selected_milestone_idx)
                .and_then(|ms| ms.tasks.get(editor.selected_task_idx))
            else {
                return;
            };

            next_agent_id(&self.agents, &task.agent_config_id)
        };

        if let Some(editor) = self.flight_editor.as_mut() {
            if let Some(task) = editor
                .draft
                .milestones
                .get_mut(editor.selected_milestone_idx)
                .and_then(|ms| ms.tasks.get_mut(editor.selected_task_idx))
            {
                task.agent_config_id = next_agent_id;
                editor.draft.updated_at = now();
            }
        }
    }

    fn edit_selected_task_model(&mut self) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };
        if editor.focus != EditorFocus::Tasks {
            return;
        }

        let ms_idx = editor.selected_milestone_idx;
        let task_idx = editor.selected_task_idx;
        let Some(task) = editor
            .draft
            .milestones
            .get(ms_idx)
            .and_then(|ms| ms.tasks.get(task_idx))
        else {
            return;
        };

        editor.input = Some(EditorInput {
            target: EditorInputTarget::TaskModel(ms_idx, task_idx),
            value: task.model.clone().unwrap_or_default(),
        });
    }

    fn edit_selected_task_dependencies(&mut self) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };
        if editor.focus != EditorFocus::Tasks {
            return;
        }

        let ms_idx = editor.selected_milestone_idx;
        let task_idx = editor.selected_task_idx;
        let Some(ms) = editor.draft.milestones.get(ms_idx) else {
            return;
        };
        let Some(task) = ms.tasks.get(task_idx) else {
            return;
        };

        editor.input = Some(EditorInput {
            target: EditorInputTarget::TaskDependencies(ms_idx, task_idx),
            value: format_dependency_input(ms, task),
        });
    }

    fn move_editor_item(&mut self, delta: i32) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        match editor.focus {
            EditorFocus::Milestones => {
                let idx = editor.selected_milestone_idx;
                let len = editor.draft.milestones.len();
                if len < 2 {
                    return;
                }
                let new_idx = (idx as i32 + delta).clamp(0, (len - 1) as i32) as usize;
                if new_idx != idx {
                    editor.draft.milestones.swap(idx, new_idx);
                    editor.selected_milestone_idx = new_idx;
                    reindex_editor_draft(&mut editor.draft);
                    editor.draft.updated_at = now();
                }
            }
            EditorFocus::Tasks => {
                let ms_idx = editor.selected_milestone_idx;
                let Some(ms) = editor.draft.milestones.get_mut(ms_idx) else {
                    return;
                };
                let idx = editor.selected_task_idx;
                let len = ms.tasks.len();
                if len < 2 {
                    return;
                }
                let new_idx = (idx as i32 + delta).clamp(0, (len - 1) as i32) as usize;
                if new_idx != idx {
                    ms.tasks.swap(idx, new_idx);
                    editor.selected_task_idx = new_idx;
                    reindex_editor_draft(&mut editor.draft);
                    editor.draft.updated_at = now();
                }
            }
            EditorFocus::Meta => {}
        }
    }

    fn add_editor_item(&mut self) {
        let default_agent_id = self.default_agent_id();
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        match editor.focus {
            EditorFocus::Meta | EditorFocus::Milestones => {
                let milestone = blank_milestone(&editor.draft.id, editor.draft.milestones.len());
                editor.draft.milestones.push(milestone);
                editor.selected_milestone_idx = editor.draft.milestones.len().saturating_sub(1);
                editor.selected_task_idx = 0;
                editor.focus = EditorFocus::Milestones;
            }
            EditorFocus::Tasks => {
                if editor.draft.milestones.is_empty() {
                    let milestone = blank_milestone(&editor.draft.id, 0);
                    editor.draft.milestones.push(milestone);
                    editor.selected_milestone_idx = 0;
                }
                if let Some(ms) = editor.draft.milestones.get_mut(editor.selected_milestone_idx) {
                    let task = blank_task(&editor.draft.id, &ms.id, ms.tasks.len(), default_agent_id.clone());
                    ms.tasks.push(task);
                    editor.selected_task_idx = ms.tasks.len().saturating_sub(1);
                }
            }
        }

        editor.draft.updated_at = now();
    }

    fn delete_editor_item(&mut self) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        match editor.focus {
            EditorFocus::Milestones => {
                if editor.selected_milestone_idx < editor.draft.milestones.len() {
                    editor.draft.milestones.remove(editor.selected_milestone_idx);
                    editor.selected_milestone_idx = editor.selected_milestone_idx.saturating_sub(1);
                    editor.selected_task_idx = 0;
                }
            }
            EditorFocus::Tasks => {
                if let Some(ms) = editor.draft.milestones.get_mut(editor.selected_milestone_idx) {
                    if editor.selected_task_idx < ms.tasks.len() {
                        ms.tasks.remove(editor.selected_task_idx);
                        editor.selected_task_idx = editor.selected_task_idx.saturating_sub(1);
                    }
                }
            }
            EditorFocus::Meta => {}
        }

        reindex_editor_draft(&mut editor.draft);
        editor.draft.updated_at = now();
    }

    fn default_agent_id(&self) -> String {
        self.agents
            .iter()
            .find(|agent| agent.installed)
            .or_else(|| self.agents.first())
            .map(|agent| agent.id.clone())
            .unwrap_or_else(|| "opencode".to_string())
    }

    fn save_flight_editor(&mut self) {
        let Some(editor) = self.flight_editor.as_mut() else {
            return;
        };

        if editor.draft.title.trim().is_empty() {
            editor.draft.title = "Untitled Flight".to_string();
        }
        if editor.draft.objective.trim().is_empty() {
            editor.draft.objective = "No objective provided".to_string();
        }

        reindex_editor_draft(&mut editor.draft);
        editor.draft.updated_at = now();

        let original_id = editor.original_flight_id.clone();
        let saved_id = editor.draft.id.clone();
        let saved_flight = editor.draft.clone();

        match original_id.clone() {
            Some(flight_id) => {
                if let Some(existing) = self.flights.iter_mut().find(|flight| flight.id == flight_id) {
                    *existing = saved_flight;
                }
                if let Some(idx) = self.flights.iter().position(|flight| flight.id == flight_id) {
                    self.selected_flight_idx = idx;
                }
            }
            None => {
                self.flights.push(saved_flight);
                self.selected_flight_idx = self.flights.len().saturating_sub(1);
            }
        }

        let _ = self.persist_state();
        self.flight_editor = None;
        self.view = AppView::FlightDetail(saved_id);
    }

    pub fn poll_pty_events(&mut self) {
        while let Ok(event) = self.pty_rx.try_recv() {
            match event {
                PtyEvent::Output { session_id, data } => {
                    let focused = self.selected_session_id() == Some(session_id.as_str());
                    if let Some(buffer) = self.session_buffers.get_mut(&session_id) {
                        buffer.append_output(&data, focused);
                    }
                    self.update_session_runtime(&session_id, &data);
                }
                PtyEvent::Exit {
                    session_id,
                    exit_code,
                    success,
                    killed,
                } => {
                    let session_title = self
                        .session_buffers
                        .get(&session_id)
                        .map(|b| b.title.clone())
                        .unwrap_or_default();

                    if let Some(buffer) = self.session_buffers.get_mut(&session_id) {
                        buffer.exited = true;
                        buffer.exit_code = exit_code;
                        buffer.success = Some(success);
                        buffer.killed = killed;
                    }

                    // Toast notification for session exit with error classification
                    if !killed {
                        if success {
                            self.toasts.push(
                                format!("Session completed: {}", session_title),
                                ToastLevel::Success,
                            );
                        } else {
                            // Classify the error from session output tail
                            let stderr_hint = self.session_buffers
                                .get(&session_id)
                                .map(|b| {
                                    // Use the last 500 chars of output as error context
                                    let out = &b.output;
                                    if out.len() > 500 { &out[out.len()-500..] } else { out }
                                })
                                .unwrap_or(&"");
                            let classified = crate::core::error_classifier::classify_cli_error(stderr_hint);
                            self.toasts.push(
                                format!("{}: {} — {}", session_title, classified.message, classified.suggestion),
                                if classified.is_transient { ToastLevel::Warning } else { ToastLevel::Error },
                            );
                        }
                    }

                    let suppressed = self.suppressed_exit_sessions.remove(&session_id);

                    let task_id = self
                        .orchestrator
                        .running_tasks
                        .iter()
                        .find(|(_, rt)| rt.session_id == session_id)
                        .map(|(id, _)| id.clone());

                    if let Some(task_id) = task_id {
                        if suppressed {
                            self.orchestrator.running_tasks.remove(&task_id);
                        } else {
                            self.orchestrator.on_task_complete(&task_id, success, &mut self.flights);
                        }
                        let _ = self.persist_state();
                    }

                    if let Ok(mut mgr) = self.pty_manager.lock() {
                        mgr.remove_session(&session_id);
                    }
                }
            }
        }
    }

    pub fn orchestrator_tick(&mut self) {
        let requests = self.orchestrator.tick(&self.flights, &self.agents);

        for req in &requests {
            let session_id = {
                let mut mgr = match self.pty_manager.lock() {
                    Ok(mgr) => mgr,
                    Err(_) => continue,
                };
                let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));
                match mgr.create_session(&req.project_path, cols, rows, &req.command, &req.args) {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!(task_id = %req.task_id, error = %e, "Failed to spawn agent");
                        continue;
                    }
                }
            };

            self.ensure_session_buffer(req, &session_id);

            self.orchestrator.record_spawn(&session_id, req, &mut self.flights);

            if let Ok(mut mgr) = self.pty_manager.lock() {
                let _ = mgr.write(&session_id, &format!("{}\n", req.prompt));
            }
        }

        if !requests.is_empty() {
            let _ = self.persist_state();
        }
    }

    fn ensure_session_buffer(&mut self, req: &crate::core::orchestrator::TaskSpawnRequest, session_id: &str) {
        let title = self
            .flights
            .iter()
            .find(|f| f.id == req.flight_id)
            .and_then(|f| {
                f.milestones
                    .iter()
                    .find(|ms| ms.id == req.milestone_id)
                    .and_then(|ms| ms.tasks.iter().find(|task| task.id == req.task_id))
            })
            .map(|task| task.title.clone())
            .unwrap_or_else(|| req.task_id.clone());

        self.session_buffers.insert(
            session_id.to_string(),
            SessionBuffer {
                session_id: session_id.to_string(),
                flight_id: req.flight_id.clone(),
                task_id: req.task_id.clone(),
                agent_config_id: req.agent_config_id.clone(),
                title,
                project_path: req.project_path.clone(),
                started_at: now(),
                output: String::new(),
                unread: false,
                exited: false,
                exit_code: None,
                success: None,
                killed: false,
                scroll: 0,
                agent_state: AgentRuntimeState::Idle,
                current_tool: None,
                current_file: None,
                needs_approval: false,
                auto_scroll: true,
                unread_count: 0,
                detected_diffs: Vec::new(),
                recent_tools: Vec::new(),
                doom_loop_detected: false,
                runtime_buffer: String::new(),
            },
        );

        self.session_order.retain(|existing| existing != session_id);
        self.session_order.insert(0, session_id.to_string());
        self.selected_session_idx = 0;
    }

    fn pause_flight(&mut self, flight_id: &str) {
        self.kill_sessions_for_flight(flight_id);
        if let Some(flight) = self.flights.iter_mut().find(|f| f.id == flight_id) {
            if flight.status == FlightStatus::Active {
                self.orchestrator.pause_flight(flight);
                let _ = self.persist_state();
            }
        }
    }

    fn cancel_flight(&mut self, flight_id: &str) {
        self.kill_sessions_for_flight(flight_id);
        if let Some(flight) = self.flights.iter_mut().find(|f| f.id == flight_id) {
            if !matches!(flight.status, FlightStatus::Done | FlightStatus::Cancelled) {
                self.orchestrator.cancel_flight(flight);
                let _ = self.persist_state();
            }
        }
    }

    fn kill_sessions_for_flight(&mut self, flight_id: &str) {
        let session_ids: Vec<String> = self
            .orchestrator
            .running_tasks_for_flight(flight_id)
            .into_iter()
            .map(|rt| rt.session_id.clone())
            .collect();

        for session_id in session_ids {
            self.kill_session(&session_id, true);
        }
    }

    fn kill_session(&mut self, session_id: &str, suppress_completion: bool) {
        if suppress_completion {
            self.suppressed_exit_sessions.insert(session_id.to_string());
        }

        if let Ok(mut mgr) = self.pty_manager.lock() {
            let _ = mgr.kill(session_id);
        }
    }

    fn selected_session_id(&self) -> Option<&str> {
        self.session_order.get(self.selected_session_idx).map(String::as_str)
    }

    pub fn attention_items(&self) -> Vec<AttentionItem> {
        let mut items = Vec::new();

        for flight in &self.flights {
            for milestone in &flight.milestones {
                for task in &milestone.tasks {
                    match task.status {
                        TaskStatus::ApprovalNeeded => items.push(AttentionItem {
                            flight_id: flight.id.clone(),
                            milestone_title: milestone.title.clone(),
                            task_title: Some(task.title.clone()),
                            kind: AttentionKind::Approval,
                            detail: "Agent is waiting for approval".to_string(),
                        }),
                        TaskStatus::Failed => items.push(AttentionItem {
                            flight_id: flight.id.clone(),
                            milestone_title: milestone.title.clone(),
                            task_title: Some(task.title.clone()),
                            kind: AttentionKind::FailedTask,
                            detail: task
                                .result
                                .clone()
                                .map(|result| result.summary)
                                .unwrap_or_else(|| "Task failed during execution".to_string()),
                        }),
                        _ => {}
                    }
                }
            }

            match flight.status {
                FlightStatus::Review => items.push(AttentionItem {
                    flight_id: flight.id.clone(),
                    milestone_title: flight
                        .milestones
                        .iter()
                        .find(|ms| ms.status == MilestoneStatus::Done)
                        .map(|ms| ms.title.clone())
                        .unwrap_or_else(|| flight.title.clone()),
                    task_title: None,
                    kind: AttentionKind::FlightReview,
                    detail: "Milestone review is waiting for operator confirmation".to_string(),
                }),
                FlightStatus::Paused => items.push(AttentionItem {
                    flight_id: flight.id.clone(),
                    milestone_title: flight.title.clone(),
                    task_title: None,
                    kind: AttentionKind::FlightPaused,
                    detail: "Flight is paused and not progressing".to_string(),
                }),
                _ => {}
            }
        }

        // Add doom loop detection items
        for buffer in self.session_buffers.values() {
            if buffer.doom_loop_detected && !buffer.exited {
                items.push(AttentionItem {
                    flight_id: buffer.flight_id.clone(),
                    milestone_title: buffer.title.clone(),
                    task_title: Some("Doom loop".to_string()),
                    kind: AttentionKind::FailedTask,
                    detail: format!(
                        "Agent stuck repeating: {}",
                        buffer.recent_tools.last().cloned().unwrap_or_default()
                    ),
                });
            }
        }

        items.sort_by_key(|item| item.kind.priority());
        items
    }

    fn update_session_runtime(&mut self, session_id: &str, data: &str) {
        let Some(agent_config_id) = self
            .session_buffers
            .get(session_id)
            .map(|buffer| buffer.agent_config_id.clone())
        else {
            return;
        };

        let Some(agent_config) = self.agents.iter().find(|agent| agent.id == agent_config_id) else {
            return;
        };

        let parsed = if let Some(buffer) = self.session_buffers.get_mut(session_id) {
            parse_agent_runtime(buffer, data, agent_config)
        } else {
            None
        };

        let Some(parsed) = parsed else {
            return;
        };

        if let Some(task_id) = self
            .session_buffers
            .get(session_id)
            .map(|buffer| buffer.task_id.clone())
        {
            if parsed.needs_approval {
                self.orchestrator.on_task_approval_needed(&task_id, &mut self.flights);
            } else {
                self.orchestrator.on_task_approval_resolved(&task_id, &mut self.flights);
            }
        }
    }

    fn send_selected_approval_action(&mut self, action: ApprovalAction) {
        if let Some(session_id) = self.selected_session_id().map(str::to_string) {
            self.send_approval_action(&session_id, action);
        }
    }

    fn send_flight_approval_action(&mut self, action: ApprovalAction) {
        let Some(flight_id) = (match &self.view {
            AppView::FlightDetail(id) => Some(id.clone()),
            _ => None,
        }) else {
            return;
        };

        let session_id = self
            .session_buffers
            .values()
            .find(|buffer| buffer.flight_id == flight_id && buffer.needs_approval && !buffer.exited)
            .map(|buffer| buffer.session_id.clone());

        if let Some(session_id) = session_id {
            self.send_approval_action(&session_id, action);
        }
    }

    fn send_approval_action(&mut self, session_id: &str, action: ApprovalAction) {
        let Some(buffer) = self.session_buffers.get(session_id) else {
            return;
        };
        if !buffer.needs_approval {
            return;
        }

        let Some(agent_config) = self.agents.iter().find(|agent| agent.id == buffer.agent_config_id) else {
            return;
        };

        let payload = match action {
            ApprovalAction::Approve => &agent_config.approval_actions.approve,
            ApprovalAction::Deny => &agent_config.approval_actions.deny,
            ApprovalAction::Abort => &agent_config.approval_actions.abort,
        }
        .clone();

        if let Ok(mut mgr) = self.pty_manager.lock() {
            let _ = mgr.write(session_id, &payload);
        }

        if let Some(buffer) = self.session_buffers.get_mut(session_id) {
            buffer.needs_approval = false;
            buffer.agent_state = AgentRuntimeState::Responding;
        }

        if let Some(task_id) = self
            .session_buffers
            .get(session_id)
            .map(|buffer| buffer.task_id.clone())
        {
            self.orchestrator
                .on_task_approval_resolved(&task_id, &mut self.flights);
        }
    }

    pub fn refresh_git_context(&mut self) {
        self.git_branch = git::get_branch(&self.settings.project_path).ok();
        self.git_status_summary = git::get_status(&self.settings.project_path)
            .ok()
            .map(|status| summarize_git_status(&status));

        if self.git_branch.is_none() {
            self.git_last_message = Some("Project path is not a git repository or git is unavailable".to_string());
        }
    }

    fn run_git_pull(&mut self) {
        self.git_last_message = Some(match git::pull(&self.settings.project_path) {
            Ok(message) => format!("git pull: {}", summarize_git_output(&message)),
            Err(error) => format!("git pull failed: {error}"),
        });
        self.refresh_git_context();
    }

    fn run_git_push(&mut self) {
        self.git_last_message = Some(match git::push(&self.settings.project_path) {
            Ok(message) => format!("git push: {}", summarize_git_output(&message)),
            Err(error) => format!("git push failed: {error}"),
        });
        self.refresh_git_context();
    }

    fn run_git_commit(&mut self, message: &str) {
        self.git_last_message = Some(match git::commit(&self.settings.project_path, message, true) {
            Ok(output) => format!("git commit: {}", summarize_git_output(&output)),
            Err(error) => format!("git commit failed: {error}"),
        });
        self.refresh_git_context();
    }

    fn clear_selected_session_unread(&mut self) {
        if let Some(session_id) = self.selected_session_id().map(str::to_string) {
            if let Some(buffer) = self.session_buffers.get_mut(&session_id) {
                buffer.unread = false;
                buffer.unread_count = 0;
            }
        }
    }

    fn adjust_session_scroll(&mut self, delta: i32) {
        if let Some(session_id) = self.selected_session_id().map(str::to_string) {
            if let Some(buffer) = self.session_buffers.get_mut(&session_id) {
                let next = (buffer.scroll as i32 + delta).max(0).min(u16::MAX as i32);
                buffer.scroll = next as u16;
                buffer.auto_scroll = false;
            }
        }
    }

    fn update_search_matches(&mut self) {
        self.session_search_matches.clear();
        self.session_search_index = 0;
        let Some(ref query) = self.session_search else { return };
        if query.is_empty() { return; }
        let query_lower = query.to_lowercase();
        let Some(session_id) = self.selected_session_id().map(str::to_string) else { return };
        let Some(session) = self.session_buffers.get(&session_id) else { return };
        for (i, line) in session.output.lines().enumerate() {
            if line.to_lowercase().contains(&query_lower) {
                self.session_search_matches.push(i);
            }
        }
    }

    fn jump_to_search_match(&mut self) {
        if self.session_search_matches.is_empty() { return; }
        let line_idx = self.session_search_matches[self.session_search_index];
        let Some(session_id) = self.selected_session_id().map(str::to_string) else { return };
        if let Some(session) = self.session_buffers.get_mut(&session_id) {
            session.scroll = (line_idx as u16).saturating_sub(3);
            session.auto_scroll = false;
        }
    }

    fn export_selected_session(&mut self) {
        let Some(session_id) = self.selected_session_id().map(str::to_string) else {
            return;
        };
        let Some(session) = self.session_buffers.get(&session_id) else {
            return;
        };

        let agent_name = self
            .agents
            .iter()
            .find(|a| a.id == session.agent_config_id)
            .map(|a| a.name.as_str())
            .unwrap_or("unknown");

        let flight_title = self
            .flights
            .iter()
            .find(|f| f.id == session.flight_id)
            .map(|f| f.title.as_str())
            .unwrap_or("Unknown flight");

        let export_dir = crate::core::storage::data_dir().join("exports");
        let _ = std::fs::create_dir_all(&export_dir);

        let filename = format!("session-{}.md", &session.session_id[..8]);
        let path = export_dir.join(&filename);

        let elapsed = format_elapsed(session.started_at);
        let content = format!(
            "# Session: {}\n\n\
            - **Flight**: {}\n\
            - **Agent**: {}\n\
            - **Task**: {}\n\
            - **Status**: {}\n\
            - **Duration**: {}\n\
            - **Session ID**: {}\n\n\
            ---\n\n\
            ```\n{}\n```\n",
            session.title,
            flight_title,
            agent_name,
            &session.task_id[..8],
            session.status_label(),
            elapsed,
            session.session_id,
            session.output,
        );

        match std::fs::write(&path, &content) {
            Ok(_) => {
                self.toasts.push(
                    format!("Exported to {}", path.display()),
                    ToastLevel::Success,
                );
            }
            Err(e) => {
                self.toasts.push(
                    format!("Export failed: {}", e),
                    ToastLevel::Error,
                );
            }
        }
    }

    fn navigate_session_hierarchy_up(&mut self) {
        // Move from current session to its parent flight's first session
        let Some(session_id) = self.selected_session_id().map(str::to_string) else {
            return;
        };
        let Some(session) = self.session_buffers.get(&session_id) else {
            return;
        };

        // Find the flight for this session, then go to the flight detail view
        let flight_id = session.flight_id.clone();
        self.view = AppView::FlightDetail(flight_id);
    }

    fn navigate_session_hierarchy_down(&mut self) {
        // From flight detail, navigate into the first running session for that flight
        if let AppView::FlightDetail(ref flight_id) = self.view.clone() {
            // Find first session for this flight
            if let Some(pos) = self.session_order.iter().position(|sid| {
                self.session_buffers
                    .get(sid)
                    .map(|b| b.flight_id == *flight_id)
                    .unwrap_or(false)
            }) {
                self.selected_session_idx = pos;
                self.view = AppView::Sessions;
                self.clear_selected_session_unread();
            }
        } else if let AppView::Dashboard = self.view {
            // From dashboard, go into the selected flight's sessions
            if let Some(flight) = self.flights.get(self.selected_flight_idx) {
                let flight_id = flight.id.clone();
                if let Some(pos) = self.session_order.iter().position(|sid| {
                    self.session_buffers
                        .get(sid)
                        .map(|b| b.flight_id == flight_id)
                        .unwrap_or(false)
                }) {
                    self.selected_session_idx = pos;
                    self.view = AppView::Sessions;
                    self.clear_selected_session_unread();
                }
            }
        }
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_nav_bar(frame, layout[0]);

        let theme = &self.theme;
        match &self.view {
            AppView::Dashboard => views::dashboard::render(frame, layout[1], self, theme),
            AppView::FlightDetail(id) => views::flight_detail::render(frame, layout[1], self, id, theme),
            AppView::FlightEditor => views::flight_editor::render(frame, layout[1], self, theme),
            AppView::Sessions => views::sessions::render(frame, layout[1], self, theme),
            AppView::Agents => views::agents::render(frame, layout[1], self, theme),
            AppView::Settings => views::settings::render(frame, layout[1], self, theme),
        }

        self.render_status_bar(frame, layout[2]);

        // Overlays (rendered last, on top of main content)
        if self.diff_view.visible {
            if let Some(session_id) = self.selected_session_id() {
                if let Some(session) = self.session_buffers.get(session_id) {
                    diff::render_diff_overlay(
                        frame,
                        area,
                        &session.detected_diffs,
                        self.diff_view.scroll,
                        &self.theme,
                    );
                }
            }
        }

        self.render_leader_overlay(frame, area);
        self.help_overlay.render(frame, area, &self.theme, self.view_name());
        self.command_palette.render(frame, area, &self.theme);
        self.toasts.render(frame, area, &self.theme);
    }

    fn render_leader_overlay(&self, frame: &mut Frame, area: Rect) {
        if self.leader_pending.is_none() {
            return;
        }

        let t = &self.theme;

        let width = 52u16.min(area.width.saturating_sub(4));
        let height = 7u16.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2 + area.x;
        let y = (area.height.saturating_sub(height)) / 2 + area.y;
        let overlay = Rect::new(x, y, width, height);

        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .title(Span::styled(
                " Leader ",
                Style::default().fg(t.brand).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.brand))
            .style(Style::default().bg(t.bg));

        let inner = block.inner(overlay);
        frame.render_widget(block, overlay);

        if inner.height < 3 {
            return;
        }

        let key_style = Style::default().fg(t.brand).add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(t.fg);
        let dim_style = Style::default().fg(t.fg_dim);

        let lines = vec![
            Line::from(vec![
                Span::styled("[f]", key_style),
                Span::styled(" Flights  ", label_style),
                Span::styled("[s]", key_style),
                Span::styled(" Sessions  ", label_style),
                Span::styled("[a]", key_style),
                Span::styled(" Agents", label_style),
            ]),
            Line::from(vec![
                Span::styled("[g]", key_style),
                Span::styled(" Git      ", label_style),
                Span::styled("[d]", key_style),
                Span::styled(" Dashboard ", label_style),
                Span::styled("[t]", key_style),
                Span::styled(" Theme", label_style),
            ]),
            Line::from(vec![
                Span::styled("[p]", key_style),
                Span::styled(" Palette  ", label_style),
                Span::styled("[?]", key_style),
                Span::styled(" Help       ", label_style),
                Span::styled("[q]", key_style),
                Span::styled(" Quit", label_style),
            ]),
            Line::from(vec![]),
            Line::from(vec![
                Span::styled("Press a key or Esc to dismiss", dim_style),
            ]),
        ];

        let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
        frame.render_widget(paragraph, inner);
    }

    fn render_nav_bar(&self, frame: &mut Frame, area: Rect) {
        let t = &self.theme;
        let tabs = vec!["1:Dashboard", "2:Sessions", "3:Agents", "4:Settings"];
        let active_idx = match &self.view {
            AppView::Dashboard | AppView::FlightDetail(_) | AppView::FlightEditor => 0,
            AppView::Sessions => 1,
            AppView::Agents => 2,
            AppView::Settings => 3,
        };

        let spans: Vec<Span> = tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                if i == active_idx {
                    Span::styled(
                        format!(" {} ", tab),
                        Style::default().fg(t.brand).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(format!(" {} ", tab), Style::default().fg(t.fg_dim))
                }
            })
            .collect();

        let mut line_spans = vec![
            Span::styled(
                " PacketCode ",
                Style::default().fg(t.brand).add_modifier(Modifier::BOLD),
            ),
            Span::raw("│"),
        ];
        line_spans.extend(spans);

        let bar = Paragraph::new(Line::from(line_spans)).style(Style::default().bg(t.bg));
        frame.render_widget(bar, area);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let t = &self.theme;
        let running = self.orchestrator.running_tasks.len();
        let max = self.settings.max_parallel_sessions;
        let active_flights = self.orchestrator.active_flight_ids.len();
        let attention = self.attention_items().len();
        let branch = self.git_branch.as_deref().unwrap_or("no-git");
        let total_cost: f64 = self.flights.iter().map(|f| f.total_cost).sum();
        let total_tokens: u64 = self.flights.iter().map(|f| f.total_tokens).sum();

        let mut spans: Vec<Span> = Vec::new();

        // Left: metrics
        spans.push(Span::styled(
            format!(" {}F ", active_flights),
            if active_flights > 0 { Style::default().fg(t.status_active) } else { Style::default().fg(t.fg_dim) },
        ));
        spans.push(Span::styled(
            format!("{}/{} agents ", running, max),
            if running > 0 { Style::default().fg(t.status_active) } else { Style::default().fg(t.fg_dim) },
        ));

        // Running agent details (elapsed per agent)
        if running > 0 {
            spans.push(Span::styled("│ ", Style::default().fg(t.fg_muted)));
            let running_sessions: Vec<_> = self
                .session_buffers
                .values()
                .filter(|b| !b.exited)
                .collect();
            for (i, session) in running_sessions.iter().take(3).enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" ", Style::default()));
                }
                let agent_name = self
                    .agents
                    .iter()
                    .find(|a| a.id == session.agent_config_id)
                    .map(|a| a.name.as_str())
                    .unwrap_or("?");
                let short_name = if agent_name.len() > 8 { &agent_name[..8] } else { agent_name };
                let elapsed = format_elapsed(session.started_at);
                let state_color = match session.agent_state {
                    AgentRuntimeState::ToolUse => t.status_active,
                    AgentRuntimeState::Thinking => t.status_info,
                    AgentRuntimeState::ApprovalNeeded => t.status_warning,
                    _ => t.fg_dim,
                };
                spans.push(Span::styled(
                    format!("{}:{}", short_name, elapsed),
                    Style::default().fg(state_color),
                ));
            }
            if running_sessions.len() > 3 {
                spans.push(Span::styled(
                    format!(" +{}", running_sessions.len() - 3),
                    Style::default().fg(t.fg_dim),
                ));
            }
            spans.push(Span::styled(" ", Style::default()));
        }

        // Cost
        if total_cost > 0.0 {
            spans.push(Span::styled("│ ", Style::default().fg(t.fg_muted)));
            spans.push(Span::styled(
                format!("{} ", super::theme::format_cost(total_cost)),
                Style::default().fg(t.status_info),
            ));
            spans.push(Span::styled(
                format!("{} tok ", super::theme::format_tokens(total_tokens)),
                Style::default().fg(t.fg_dim),
            ));
        }

        // Attention
        if attention > 0 {
            spans.push(Span::styled("│ ", Style::default().fg(t.fg_muted)));
            spans.push(Span::styled(
                format!("⚠{} ", attention),
                Style::default().fg(t.status_warning),
            ));
        }

        // Git
        spans.push(Span::styled("│ ", Style::default().fg(t.fg_muted)));
        spans.push(Span::styled(
            format!("{} ", branch),
            Style::default().fg(t.fg_dim),
        ));

        // Leader indicator
        if self.leader_pending.is_some() {
            spans.push(Span::styled("│ ", Style::default().fg(t.fg_muted)));
            spans.push(Span::styled(
                "LEADER... ",
                Style::default().fg(t.brand).add_modifier(Modifier::BOLD),
            ));
        }

        // Help hint
        spans.push(Span::styled("│ ", Style::default().fg(t.fg_muted)));
        spans.push(Span::styled("SPC:leader ?:help", Style::default().fg(t.fg_muted)));

        let bar = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(t.bg));
        frame.render_widget(bar, area);
    }
}

pub fn format_elapsed(started_at: u64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let elapsed_secs = now_ms.saturating_sub(started_at) / 1000;
    if elapsed_secs < 60 {
        format!("{}s", elapsed_secs)
    } else if elapsed_secs < 3600 {
        format!("{}m{}s", elapsed_secs / 60, elapsed_secs % 60)
    } else {
        format!("{}h{}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn default_project_path() -> String {
    std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string())
}

fn blank_flight(project_path: String) -> Flight {
    let timestamp = now();
    Flight {
        id: uuid::Uuid::new_v4().to_string(),
        title: String::new(),
        objective: String::new(),
        status: FlightStatus::Draft,
        priority: FlightPriority::Medium,
        project_path,
        git_branch: None,
        milestones: Vec::new(),
        linked_session_ids: Vec::new(),
        created_at: timestamp,
        updated_at: timestamp,
        completed_at: None,
        total_cost: 0.0,
        total_tokens: 0,
    }
}

fn blank_milestone(flight_id: &str, order: usize) -> Milestone {
    Milestone {
        id: uuid::Uuid::new_v4().to_string(),
        flight_id: flight_id.to_string(),
        title: format!("Milestone {}", order + 1),
        description: String::new(),
        order,
        status: MilestoneStatus::Pending,
        tasks: Vec::new(),
        validation_criteria: Vec::new(),
    }
}

fn blank_task(flight_id: &str, milestone_id: &str, order: usize, agent_config_id: String) -> Task {
    Task {
        id: uuid::Uuid::new_v4().to_string(),
        milestone_id: milestone_id.to_string(),
        flight_id: flight_id.to_string(),
        title: format!("Task {}", order + 1),
        description: String::new(),
        order,
        status: TaskStatus::Pending,
        task_type: TaskType::Implementation,
        agent_config_id,
        agent_args: None,
        model: None,
        depends_on: Vec::new(),
        session_id: None,
        result: None,
        review_packet: None,
        created_at: now(),
        started_at: None,
        completed_at: None,
        cost: 0.0,
        tokens: 0,
    }
}

fn reindex_editor_draft(flight: &mut Flight) {
    for (ms_idx, milestone) in flight.milestones.iter_mut().enumerate() {
        milestone.order = ms_idx;
        milestone.flight_id = flight.id.clone();
        for (task_idx, task) in milestone.tasks.iter_mut().enumerate() {
            task.order = task_idx;
            task.flight_id = flight.id.clone();
            task.milestone_id = milestone.id.clone();
        }
    }
}

fn next_task_type(current: TaskType) -> TaskType {
    match current {
        TaskType::Implementation => TaskType::Testing,
        TaskType::Testing => TaskType::Review,
        TaskType::Review => TaskType::Validation,
        TaskType::Validation => TaskType::Research,
        TaskType::Research => TaskType::Refactor,
        TaskType::Refactor => TaskType::Documentation,
        TaskType::Documentation => TaskType::Implementation,
    }
}

fn next_agent_id(agents: &[AgentConfig], current_agent_id: &str) -> String {
    if agents.is_empty() {
        return current_agent_id.to_string();
    }

    let current_idx = agents
        .iter()
        .position(|agent| agent.id == current_agent_id)
        .unwrap_or(0);
    let next_idx = (current_idx + 1) % agents.len();
    agents[next_idx].id.clone()
}

fn format_dependency_input(ms: &Milestone, task: &Task) -> String {
    let mut indices = Vec::new();
    for dep_id in &task.depends_on {
        if let Some(idx) = ms.tasks.iter().position(|candidate| candidate.id == *dep_id) {
            indices.push((idx + 1).to_string());
        }
    }
    indices.join(",")
}

fn parse_dependency_input(ms: &Milestone, task_idx: usize, value: &str) -> Vec<String> {
    let mut dependency_ids = Vec::new();

    for token in value.split(',').map(str::trim).filter(|token| !token.is_empty()) {
        let Ok(position) = token.parse::<usize>() else {
            continue;
        };
        if position == 0 {
            continue;
        }

        let dep_idx = position - 1;
        if dep_idx >= task_idx {
            continue;
        }

        if let Some(task) = ms.tasks.get(dep_idx) {
            if !dependency_ids.iter().any(|existing| existing == &task.id) {
                dependency_ids.push(task.id.clone());
            }
        }
    }

    dependency_ids
}

#[derive(Debug, Clone)]
struct ParsedAgentRuntime {
    needs_approval: bool,
}

fn parse_agent_runtime(
    buffer: &mut SessionBuffer,
    data: &str,
    agent: &AgentConfig,
) -> Option<ParsedAgentRuntime> {
    let stripped = strip_ansi(data);
    buffer.runtime_buffer.push_str(&stripped);
    if buffer.runtime_buffer.len() > 4096 {
        let overflow = buffer.runtime_buffer.len() - 4096;
        buffer.runtime_buffer.drain(..overflow);
    }

    let recent = buffer.runtime_buffer.chars().rev().take(1024).collect::<String>();
    let recent = recent.chars().rev().collect::<String>();
    let lines: Vec<&str> = recent.lines().collect();
    let start = lines.len().saturating_sub(8);
    let last_lines = &lines[start..];
    let last_chunk = last_lines.join("\n");

    let needs_approval = agent
        .status_patterns
        .approval
        .iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .any(|regex| regex.is_match(&last_chunk));

    let mut current_tool = None;
    let mut current_file = None;
    for line in last_lines.iter().rev() {
        for pattern in &agent.status_patterns.tool_use {
            let Ok(regex) = Regex::new(&pattern.pattern) else {
                continue;
            };
            if let Some(captures) = regex.captures(line) {
                current_tool = Some(pattern.tool.clone());
                current_file = pattern
                    .file_group
                    .and_then(|idx| captures.get(idx).map(|m| m.as_str().trim().to_string()));
                break;
            }
        }
        if current_tool.is_some() {
            break;
        }
    }

    let mut agent_state = AgentRuntimeState::Responding;
    if needs_approval {
        agent_state = AgentRuntimeState::ApprovalNeeded;
    } else if current_tool.is_some() {
        agent_state = AgentRuntimeState::ToolUse;
    } else if agent
        .status_patterns
        .thinking
        .iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .any(|regex| regex.is_match(&last_chunk))
    {
        agent_state = AgentRuntimeState::Thinking;
    } else if agent
        .status_patterns
        .idle
        .iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .any(|regex| regex.is_match(&last_chunk))
    {
        agent_state = AgentRuntimeState::Idle;
    }

    buffer.needs_approval = needs_approval;
    buffer.agent_state = agent_state;

    // Doom loop detection: track recent tool calls
    if let Some(ref tool) = current_tool {
        let tool_key = format!(
            "{}:{}",
            tool,
            current_file.as_deref().unwrap_or("-")
        );
        buffer.recent_tools.push(tool_key);
        if buffer.recent_tools.len() > 10 {
            buffer.recent_tools.drain(..buffer.recent_tools.len() - 10);
        }
        // Check if last 3 tool calls are identical
        if buffer.recent_tools.len() >= 3 {
            let len = buffer.recent_tools.len();
            let last_three = &buffer.recent_tools[len - 3..];
            if last_three[0] == last_three[1] && last_three[1] == last_three[2] {
                buffer.doom_loop_detected = true;
            }
        }
    }

    buffer.current_tool = current_tool;
    buffer.current_file = current_file;

    Some(ParsedAgentRuntime { needs_approval })
}

fn strip_ansi(value: &str) -> String {
    let ansi_re = Regex::new(r"\x1B\[[0-9;]*[A-Za-z]|\x1B\].*?\x07|\x1B[()][A-Z0-9]|\x1B[>=<]|\x0F|\x0E")
        .expect("valid ansi regex");
    ansi_re.replace_all(value, "").to_string()
}

fn summarize_git_status(status: &str) -> String {
    let lines: Vec<&str> = status.lines().filter(|line| !line.trim().is_empty()).collect();
    if lines.is_empty() {
        "clean".to_string()
    } else {
        format!("{} change(s)", lines.len())
    }
}

fn summarize_git_output(output: &str) -> String {
    output
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("ok")
        .trim()
        .to_string()
}
