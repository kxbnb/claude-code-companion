#![allow(dead_code)]

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::protocol::types::{CliMessage, ContentBlock};

// ─── Input State ────────────────────────────────────────────────────────────

/// Simple single-line text input state machine.
pub struct InputState {
    pub text: String,
    pub cursor: usize, // byte offset into text
}

impl InputState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(prev);
            self.cursor = prev;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.text.len());
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn kill_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }

    pub fn kill_to_start(&mut self) {
        self.text = self.text[self.cursor..].to_string();
        self.cursor = 0;
    }

    pub fn delete_word_back(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let word_start = trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        self.text = format!("{}{}", &self.text[..word_start], &self.text[self.cursor..]);
        self.cursor = word_start;
    }

    pub fn take(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn cursor_col(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }
}

// ─── Mode ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
}

// ─── Layout ─────────────────────────────────────────────────────────────────

pub struct Layout {
    pub sidebar_visible: bool,
    pub sidebar_width: u16,
    pub task_panel_visible: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            sidebar_width: 22,
            task_panel_visible: false,
        }
    }
}

// ─── Task Item ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub subject: String,
    pub status: TaskStatus,
    pub description: String,
    pub active_form: Option<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

// ─── Environment Profile ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvProfile {
    pub name: String,
    pub description: String,
    pub vars: HashMap<String, String>,
}

// ─── Git Info ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub branch: String,
    pub is_worktree: bool,
    pub repo_root: String,
    pub ahead: i32,
    pub behind: i32,
}

// ─── Command ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Command {
    New { env: Option<String> },
    Kill,
    Rename { name: String },
    Ls,
    Env,
    Model { name: String },
    PermMode { mode: String },
    Cd { path: String },
    Worktree { branch: String },
    Archive,
    Unarchive { index: Option<usize> },
    Clear,
    Help,
    Quit,
    Unknown(String),
}

// ─── App Events ─────────────────────────────────────────────────────────────

/// All events flow through this channel to the event loop
pub enum AppEvent {
    /// A parsed NDJSON message from the CLI
    CliMessage {
        session_id: String,
        message: CliMessage,
    },
    /// CLI connected via WebSocket — provides a sender for outgoing messages
    CliConnected {
        session_id: String,
        sender: mpsc::UnboundedSender<String>,
    },
    /// CLI WebSocket disconnected
    CliDisconnected {
        session_id: String,
    },
    /// CLI process exited
    CliProcessExited {
        session_id: String,
    },
}

// ─── Chat Messages ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub content_blocks: Option<Vec<ContentBlock>>,
    pub model: Option<String>,
    pub timestamp: i64,
}

// ─── Session Status ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    WaitingForCli,
    Idle,
    Running,
    Compacting,
}

// ─── Pending Permission ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub request_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub description: Option<String>,
}

// ─── Persisted Session ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub id: String,
    pub name: String,
    pub cli_session_id: Option<String>,
    pub cwd: String,
    pub model: String,
    pub version: String,
    pub permission_mode: String,
    pub env_profile: Option<String>,
    pub total_cost_usd: f64,
    pub num_turns: u32,
    pub context_used_percent: u32,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<String>,
    pub tasks: Vec<TaskItem>,
    pub created_at: i64,
    #[serde(default)]
    pub archived: bool,
}

// ─── Session ────────────────────────────────────────────────────────────────

pub struct Session {
    /// Our session UUID (used in WebSocket URL)
    pub id: String,
    /// Display name for the session
    pub name: String,
    /// The CLI's internal session ID (from system/init, used for --resume)
    pub cli_session_id: Option<String>,
    /// Working directory
    pub cwd: String,
    /// Model name
    pub model: String,
    /// Claude Code version
    pub version: String,
    /// Permission mode
    pub permission_mode: String,
    /// Environment profile name
    pub env_profile: Option<String>,
    /// Cumulative cost
    pub total_cost_usd: f64,
    /// Number of conversation turns
    pub num_turns: u32,
    /// Context usage percentage
    pub context_used_percent: u32,
    /// Chat message history
    pub messages: Vec<ChatMessage>,
    /// Accumulated streaming text (from content_block_delta events)
    pub streaming_text: String,
    /// Session status
    pub status: SessionStatus,
    /// Whether CLI WebSocket is connected
    pub cli_connected: bool,
    /// Channel sender for outgoing messages to CLI
    pub cli_sender: Option<mpsc::UnboundedSender<String>>,
    /// Pending permission request
    pub pending_permission: Option<PendingPermission>,
    /// Whether an interrupt was sent (double Ctrl+C to quit)
    pub interrupt_sent: bool,
    /// Available tools (from system/init)
    pub tools: Vec<String>,
    /// Scroll offset for chat view (lines from bottom)
    pub scroll_offset: usize,
    /// Task items
    pub tasks: Vec<TaskItem>,
    /// Creation timestamp
    pub created_at: i64,
    /// CLI process join handle (for aborting)
    pub cli_process_handle: Option<tokio::task::JoinHandle<()>>,
    /// Whether session needs to be persisted
    pub dirty_persist: bool,
    /// Messages queued to send once CLI connects (for resume / pre-connect)
    pub queued_messages: Vec<String>,
    /// Last assistant message ID seen (for deduplication)
    pub last_assistant_msg_id: Option<String>,
    /// Git branch name
    pub git_branch: String,
    /// Whether this session is in a git worktree
    pub is_worktree: bool,
    /// Git repository root path
    pub repo_root: String,
    /// Commits ahead of upstream
    pub git_ahead: i32,
    /// Commits behind upstream
    pub git_behind: i32,
    /// Total lines added (from CLI result messages)
    pub total_lines_added: u32,
    /// Total lines removed (from CLI result messages)
    pub total_lines_removed: u32,
    /// Available slash commands (from system/init)
    pub slash_commands: Vec<String>,
    /// Available skills (from system/init)
    pub skills: Vec<String>,
    /// Whether session is archived (soft-hidden)
    pub archived: bool,
    /// Previous permission mode (for plan mode toggle restore)
    pub previous_permission_mode: Option<String>,
}

impl Session {
    pub fn new(id: String, name: String, cwd: String) -> Self {
        Self {
            id,
            name,
            cli_session_id: None,
            cwd,
            model: String::new(),
            version: String::new(),
            permission_mode: "default".to_string(),
            env_profile: None,
            total_cost_usd: 0.0,
            num_turns: 0,
            context_used_percent: 0,
            messages: Vec::new(),
            streaming_text: String::new(),
            status: SessionStatus::WaitingForCli,
            cli_connected: false,
            cli_sender: None,
            pending_permission: None,
            interrupt_sent: false,
            tools: Vec::new(),
            scroll_offset: 0,
            tasks: Vec::new(),
            created_at: chrono::Utc::now().timestamp(),
            cli_process_handle: None,
            dirty_persist: false,
            queued_messages: Vec::new(),
            last_assistant_msg_id: None,
            git_branch: String::new(),
            is_worktree: false,
            repo_root: String::new(),
            git_ahead: 0,
            git_behind: 0,
            total_lines_added: 0,
            total_lines_removed: 0,
            slash_commands: Vec::new(),
            skills: Vec::new(),
            archived: false,
            previous_permission_mode: None,
        }
    }

    /// Send an NDJSON message to the CLI
    pub fn send_to_cli(&self, ndjson: &str) -> bool {
        if let Some(sender) = &self.cli_sender {
            sender.send(ndjson.to_string()).is_ok()
        } else {
            false
        }
    }

    /// Add a system message to the chat
    pub fn add_system_message(&mut self, content: String) {
        self.messages.push(ChatMessage {
            role: ChatRole::System,
            content,
            content_blocks: None,
            model: None,
            timestamp: chrono::Utc::now().timestamp(),
        });
        self.scroll_offset = 0;
    }

    pub fn to_persisted(&self) -> PersistedSession {
        PersistedSession {
            id: self.id.clone(),
            name: self.name.clone(),
            cli_session_id: self.cli_session_id.clone(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            version: self.version.clone(),
            permission_mode: self.permission_mode.clone(),
            env_profile: self.env_profile.clone(),
            total_cost_usd: self.total_cost_usd,
            num_turns: self.num_turns,
            context_used_percent: self.context_used_percent,
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            tasks: self.tasks.clone(),
            created_at: self.created_at,
            archived: self.archived,
        }
    }

    pub fn from_persisted(p: PersistedSession) -> Self {
        Self {
            id: p.id,
            name: p.name,
            cli_session_id: p.cli_session_id,
            cwd: p.cwd,
            model: p.model,
            version: p.version,
            permission_mode: p.permission_mode,
            env_profile: p.env_profile,
            total_cost_usd: p.total_cost_usd,
            num_turns: p.num_turns,
            context_used_percent: p.context_used_percent,
            messages: p.messages,
            streaming_text: String::new(),
            status: SessionStatus::WaitingForCli,
            cli_connected: false,
            cli_sender: None,
            pending_permission: None,
            interrupt_sent: false,
            tools: p.tools,
            scroll_offset: 0,
            tasks: p.tasks,
            created_at: p.created_at,
            cli_process_handle: None,
            dirty_persist: false,
            queued_messages: Vec::new(),
            last_assistant_msg_id: None,
            git_branch: String::new(),
            is_worktree: false,
            repo_root: String::new(),
            git_ahead: 0,
            git_behind: 0,
            total_lines_added: 0,
            total_lines_removed: 0,
            slash_commands: Vec::new(),
            skills: Vec::new(),
            archived: p.archived,
            previous_permission_mode: None,
        }
    }

    pub fn persist(&self) -> anyhow::Result<()> {
        let dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".companion")
            .join("sessions");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", self.id));
        let data = serde_json::to_string_pretty(&self.to_persisted())?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn delete_persisted(&self) {
        let path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".companion")
            .join("sessions")
            .join(format!("{}.json", self.id));
        let _ = std::fs::remove_file(path);
    }
}

// ─── App (Global State) ─────────────────────────────────────────────────────

pub struct App {
    pub sessions: HashMap<String, Session>,
    pub active_session_id: Option<String>,
    pub session_order: Vec<String>,
    pub mode: Mode,
    pub composer: InputState,
    pub command_input: InputState,
    pub layout: Layout,
    pub should_quit: bool,
    pub dirty: bool,
    pub env_profiles: Vec<EnvProfile>,
    pub ws_port: u16,
    pub default_cwd: String,
    pub default_model: Option<String>,
    pub flash_message: Option<(String, Instant)>,
    /// For 'gg' double-key scroll to top in Normal mode
    pub gg_pending: bool,
    /// Session IDs that need a CLI process spawned
    pub pending_spawns: Vec<String>,
}

impl App {
    pub fn new(ws_port: u16, cwd: String, model: Option<String>) -> Self {
        Self {
            sessions: HashMap::new(),
            active_session_id: None,
            session_order: Vec::new(),
            mode: Mode::Normal,
            composer: InputState::new(),
            command_input: InputState::new(),
            layout: Layout::default(),
            should_quit: false,
            dirty: true,
            env_profiles: Vec::new(),
            ws_port,
            default_cwd: cwd,
            default_model: model,
            flash_message: None,
            gg_pending: false,
            pending_spawns: Vec::new(),
        }
    }

    pub fn active_session(&self) -> Option<&Session> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    pub fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get_mut(id.as_str()))
    }

    /// Create a new session. Returns the session ID. Adds to pending_spawns.
    pub fn create_session(
        &mut self,
        name: String,
        cwd: String,
        env_profile: Option<String>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut session = Session::new(id.clone(), name, cwd);
        session.env_profile = env_profile;
        self.sessions.insert(id.clone(), session);
        self.session_order.push(id.clone());
        self.active_session_id = Some(id.clone());
        self.pending_spawns.push(id.clone());
        self.dirty = true;
        id
    }

    pub fn switch_to_session(&mut self, id: &str) -> bool {
        if self.sessions.contains_key(id) {
            self.active_session_id = Some(id.to_string());
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn switch_to_index(&mut self, index: usize) -> bool {
        if let Some(id) = self.session_order.get(index).cloned() {
            self.switch_to_session(&id)
        } else {
            false
        }
    }

    pub fn next_session(&mut self) {
        let visible = self.visible_session_order();
        if visible.len() <= 1 {
            return;
        }
        if let Some(ref active_id) = self.active_session_id {
            if let Some(pos) = visible.iter().position(|id| id == active_id) {
                let next = (pos + 1) % visible.len();
                let id = visible[next].clone();
                self.active_session_id = Some(id);
                self.dirty = true;
            }
        }
    }

    pub fn prev_session(&mut self) {
        let visible = self.visible_session_order();
        if visible.len() <= 1 {
            return;
        }
        if let Some(ref active_id) = self.active_session_id {
            if let Some(pos) = visible.iter().position(|id| id == active_id) {
                let prev = if pos == 0 {
                    visible.len() - 1
                } else {
                    pos - 1
                };
                let id = visible[prev].clone();
                self.active_session_id = Some(id);
                self.dirty = true;
            }
        }
    }

    pub fn kill_active_session(&mut self) {
        if let Some(id) = self.active_session_id.take() {
            if let Some(session) = self.sessions.remove(&id) {
                if let Some(ref handle) = session.cli_process_handle {
                    handle.abort();
                }
                session.delete_persisted();
            }
            self.session_order.retain(|s| s != &id);
            // Switch to first non-archived session, or first session as fallback
            let visible = self.visible_session_order();
            if let Some(next_id) = visible.first() {
                self.active_session_id = Some(next_id.clone());
            } else if let Some(first) = self.session_order.first() {
                self.active_session_id = Some(first.clone());
            }
            self.dirty = true;
        }
    }

    pub fn active_session_index(&self) -> Option<usize> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.session_order.iter().position(|s| s == id))
    }

    pub fn flash(&mut self, message: String) {
        self.flash_message = Some((message, Instant::now()));
        self.dirty = true;
    }

    pub fn load_env_profiles(&mut self) {
        let dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".companion")
            .join("envs");
        if !dir.exists() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(data) = std::fs::read_to_string(&path) {
                        match serde_json::from_str::<EnvProfileFile>(&data) {
                            Ok(file) => {
                                let name = path
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string();
                                self.env_profiles.push(EnvProfile {
                                    name,
                                    description: file.description,
                                    vars: file.vars,
                                });
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse env profile {:?}: {}", path, e);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn load_persisted_sessions(&mut self) {
        let dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".companion")
            .join("sessions");
        if !dir.exists() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry
                    .path()
                    .extension()
                    .map(|e| e == "json")
                    .unwrap_or(false)
                {
                    if let Ok(data) = std::fs::read_to_string(entry.path()) {
                        match serde_json::from_str::<PersistedSession>(&data) {
                            Ok(persisted) => {
                                let id = persisted.id.clone();
                                let session = Session::from_persisted(persisted);
                                self.sessions.insert(id.clone(), session);
                                self.session_order.push(id);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to load persisted session {:?}: {}",
                                    entry.path(),
                                    e
                                );
                            }
                        }
                    }
                }
            }
        }
        // Set active to first loaded session
        if self.active_session_id.is_none() && !self.session_order.is_empty() {
            self.active_session_id = Some(self.session_order[0].clone());
        }
    }

    pub fn persist_all_sessions(&self) {
        for session in self.sessions.values() {
            if let Err(e) = session.persist() {
                tracing::error!("Failed to persist session {}: {}", session.id, e);
            }
        }
    }

    pub fn archive_active_session(&mut self) {
        if let Some(id) = &self.active_session_id {
            if let Some(session) = self.sessions.get_mut(id.as_str()) {
                session.archived = true;
                session.dirty_persist = true;
                let _ = session.persist();
            }
            // Switch to next non-archived session
            let current_id = id.clone();
            let next = self
                .session_order
                .iter()
                .find(|s| *s != &current_id && !self.sessions.get(s.as_str()).map(|s| s.archived).unwrap_or(true))
                .cloned();
            if let Some(next_id) = next {
                self.active_session_id = Some(next_id);
            }
            // If no non-archived session, stay on current (it's archived but still accessible)
        }
        self.dirty = true;
    }

    pub fn unarchive_session(&mut self, index: usize) {
        if let Some(id) = self.session_order.get(index).cloned() {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.archived = false;
                session.dirty_persist = true;
                let _ = session.persist();
            }
        }
        self.dirty = true;
    }

    /// Get the visible (non-archived) session order
    pub fn visible_session_order(&self) -> Vec<String> {
        self.session_order
            .iter()
            .filter(|id| {
                !self
                    .sessions
                    .get(id.as_str())
                    .map(|s| s.archived)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn get_env_vars(&self, profile_name: &str) -> HashMap<String, String> {
        self.env_profiles
            .iter()
            .find(|p| p.name == profile_name)
            .map(|p| p.vars.clone())
            .unwrap_or_default()
    }
}

/// Disk format for env profile files (name comes from filename)
#[derive(Debug, Deserialize)]
struct EnvProfileFile {
    #[serde(default)]
    description: String,
    #[serde(default)]
    vars: HashMap<String, String>,
}

// ─── Session Name Generator ────────────────────────────────────────────────

const ADJECTIVES: &[&str] = &[
    "crimson", "azure", "golden", "silver", "emerald", "coral", "violet", "amber",
    "scarlet", "cobalt", "jade", "ivory", "onyx", "ruby", "sapphire", "topaz",
    "bronze", "copper", "indigo", "teal", "slate", "pearl", "rustic", "misty",
];

const NOUNS: &[&str] = &[
    "falcon", "phoenix", "dragon", "raven", "tiger", "wolf", "hawk", "eagle",
    "panther", "cobra", "viper", "sphinx", "griffin", "lynx", "orca", "puma",
    "condor", "mantis", "jaguar", "osprey", "badger", "otter", "heron", "bison",
];

pub fn generate_session_name() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    let adj = ADJECTIVES[seed % ADJECTIVES.len()];
    let noun = NOUNS[(seed / ADJECTIVES.len()) % NOUNS.len()];
    format!("{}-{}", adj, noun)
}

// ─── Git Info Gathering ────────────────────────────────────────────────────

pub fn gather_git_info(cwd: &str) -> GitInfo {
    use std::process::Command as ProcessCommand;

    let mut info = GitInfo::default();

    let run = |args: &[&str]| -> Option<String> {
        ProcessCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };

    // Branch name
    if let Some(branch) = run(&["rev-parse", "--abbrev-ref", "HEAD"]) {
        info.branch = branch;
    } else {
        return info; // Not a git repo
    }

    // Check if worktree
    if let Some(git_dir) = run(&["rev-parse", "--git-dir"]) {
        info.is_worktree = git_dir.contains("/worktrees/");
    }

    // Repo root
    if let Some(root) = run(&["rev-parse", "--show-toplevel"]) {
        info.repo_root = root;
    }

    // Ahead/behind counts
    if let Some(counts) = run(&["rev-list", "--left-right", "--count", "@{upstream}...HEAD"]) {
        let parts: Vec<&str> = counts.split_whitespace().collect();
        if parts.len() == 2 {
            info.behind = parts[0].parse().unwrap_or(0);
            info.ahead = parts[1].parse().unwrap_or(0);
        }
    }

    info
}
