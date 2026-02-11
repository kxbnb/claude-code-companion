use std::collections::HashMap;
use std::time::Duration;

use crossterm::{
    cursor, execute,
    event::{Event, EventStream, KeyEvent},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::app::{
    App, AppEvent, ChatMessage, ChatRole, Mode, PendingPermission, SessionStatus, TaskItem,
    TaskStatus,
};
use crate::process::launcher::CliLauncher;
use crate::protocol::types::{
    self, CliMessage, ControlRequestPayload, OutgoingUserMessage,
};
use crate::ui::commands;
use crate::ui::keybindings;
use crate::ui::renderer;

/// Guard that restores the terminal on drop (including panics)
struct TermGuard;

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            cursor::Show,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// Run the TUI event loop. Takes ownership of the terminal.
pub async fn run(
    mut app: App,
    mut event_rx: mpsc::UnboundedReceiver<AppEvent>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout();

    // Enter alternate screen + raw mode
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;

    // Guard ensures terminal is restored even on panic
    let _guard = TermGuard;

    let result = run_inner(&mut app, &mut stdout, &mut event_rx, &event_tx).await;

    // Persist all sessions before exiting
    app.persist_all_sessions();

    // Explicit cleanup (guard will also run on drop, but it's idempotent)
    drop(_guard);
    result
}

async fn run_inner(
    app: &mut App,
    stdout: &mut std::io::Stdout,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let mut term_reader = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Initial render
    renderer::render(app, stdout)?;
    app.dirty = false;

    loop {
        tokio::select! {
            // Terminal events (keyboard, resize)
            maybe_event = term_reader.next() => {
                if let Some(Ok(event)) = maybe_event {
                    handle_terminal_event(event, app, event_tx);
                }
            }
            // App events (WebSocket messages, connection changes)
            maybe_event = event_rx.recv() => {
                if let Some(event) = maybe_event {
                    handle_app_event(event, app);
                } else {
                    // Channel closed — all senders dropped
                    break;
                }
            }
            // Tick timer for periodic redraws
            _ = tick.tick() => {
                app.tick = app.tick.wrapping_add(1);
                // Check flash message expiry (3 seconds)
                if let Some((_, instant)) = &app.flash_message {
                    if instant.elapsed() > Duration::from_secs(3) {
                        app.flash_message = None;
                        app.dirty = true;
                    }
                }
                // Force redraw when any session is running (for spinner)
                if app.sessions.values().any(|s| s.status == SessionStatus::Running || s.status == SessionStatus::Compacting) {
                    app.dirty = true;
                }
            }
        }

        // Process pending CLI spawns
        process_pending_spawns(app, event_tx);

        // Redraw if dirty
        if app.dirty {
            renderer::render(app, stdout)?;
            app.dirty = false;
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

// ─── CLI Spawning ──────────────────────────────────────────────────────────

fn process_pending_spawns(app: &mut App, event_tx: &mpsc::UnboundedSender<AppEvent>) {
    let spawns: Vec<String> = app.pending_spawns.drain(..).collect();
    for id in spawns {
        spawn_cli_for_session(app, &id, event_tx);
    }
}

fn spawn_cli_for_session(
    app: &mut App,
    session_id: &str,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    // Gather data (immutable borrow)
    let (port, cwd, model, resume_id, env_vars) = {
        let session = match app.sessions.get(session_id) {
            Some(s) => s,
            None => return,
        };
        let env_vars: HashMap<String, String> = session
            .env_profile
            .as_ref()
            .map(|name| {
                app.env_profiles
                    .iter()
                    .find(|p| &p.name == name)
                    .map(|p| p.vars.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let model = if session.model.is_empty() {
            app.default_model.clone()
        } else {
            Some(session.model.clone())
        };
        (
            app.ws_port,
            session.cwd.clone(),
            model,
            session.cli_session_id.clone(),
            env_vars,
        )
    };

    let sid = session_id.to_string();
    let etx = event_tx.clone();

    let launcher = CliLauncher::new(port, sid.clone(), cwd, model)
        .with_env_vars(env_vars)
        .with_resume_session_id(resume_id);

    let handle = tokio::spawn(async move {
        match launcher.spawn().await {
            Ok(status) => {
                tracing::info!("CLI for session {} exited: {:?}", sid, status);
            }
            Err(e) => {
                tracing::error!("Failed to spawn CLI for session {}: {}", sid, e);
            }
        }
        let _ = etx.send(AppEvent::CliProcessExited {
            session_id: sid,
        });
    });

    // Store the handle (mutable borrow)
    if let Some(session) = app.sessions.get_mut(session_id) {
        session.cli_process_handle = Some(handle);
    }
}

// ─── Terminal Event Handling ────────────────────────────────────────────────

fn handle_terminal_event(
    event: Event,
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match event {
        Event::Key(key) => handle_key_event(key, app, event_tx),
        Event::Resize(_, _) => {
            app.dirty = true;
        }
        _ => {}
    }
}

fn handle_key_event(
    key: KeyEvent,
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    // Permission overlay intercepts all keys when pending
    if app
        .active_session()
        .map(|s| s.pending_permission.is_some())
        .unwrap_or(false)
    {
        if keybindings::handle_permission_keys(key, app) {
            return;
        }
    }

    // Question overlay intercepts all keys when pending
    if app
        .active_session()
        .map(|s| s.pending_question.is_some())
        .unwrap_or(false)
    {
        if keybindings::handle_question_keys(key, app) {
            return;
        }
    }

    match app.mode {
        Mode::Normal => {
            keybindings::handle_key_normal(key, app);
        }
        Mode::Insert => {
            let should_send = keybindings::handle_key_insert(key, app);
            if should_send {
                send_user_message(app, event_tx);
            }
        }
        Mode::Command => {
            if let Some(cmd_text) = keybindings::handle_key_command(key, app) {
                let cmd = commands::parse_command(&cmd_text);
                commands::execute_command(cmd, app);
                // Note: SpawnSession is already handled by pending_spawns in create_session
            }
        }
    }
}

// ─── Send User Message ──────────────────────────────────────────────────────

fn send_user_message(app: &mut App, _event_tx: &mpsc::UnboundedSender<AppEvent>) {
    if app.composer.is_empty() {
        return;
    }

    let text = app.composer.take();

    // Check the CLI connection state
    let (cli_connected, has_cli_session, has_sender) = app
        .active_session()
        .map(|s| (s.cli_connected, s.cli_session_id.is_some(), s.cli_sender.is_some()))
        .unwrap_or((false, false, false));

    tracing::info!(
        "send_user_message: text={:?}, cli_connected={}, has_cli_session={}, has_sender={}",
        &text,
        cli_connected,
        has_cli_session,
        has_sender
    );

    // Add to chat history first (always)
    if let Some(session) = app.active_session_mut() {
        session.messages.push(ChatMessage {
            role: ChatRole::User,
            content: text.clone(),
            content_blocks: None,
            model: None,
            timestamp: chrono::Utc::now().timestamp(),
        });
        session.scroll_offset = 0;
    }

    if !has_sender {
        // CLI is not connected — queue message for delivery after init
        if let Some(session) = app.active_session_mut() {
            session.queued_messages.push(text);
            session.status = SessionStatus::Running; // show "thinking" state
        }

        if !cli_connected && has_cli_session {
            // Persisted/disconnected session — spawn CLI with --resume
            if let Some(sid) = app.active_session_id.clone() {
                if let Some(session) = app.active_session_mut() {
                    session.add_system_message("Resuming session...".to_string());
                }
                app.pending_spawns.push(sid);
            }
        }
        // else: CLI is still starting up, message will be sent when init arrives

        app.dirty = true;
        return;
    }

    // Normal send — CLI is connected
    if let Some(session) = app.active_session_mut() {
        let session_id = session
            .cli_session_id
            .clone()
            .unwrap_or_else(|| session.id.clone());
        let msg = OutgoingUserMessage::new(text, session_id.clone());
        let ndjson = msg.to_ndjson();
        tracing::info!("Sending to CLI (session_id={}): {}", session_id, &ndjson);
        let sent = session.send_to_cli(&ndjson);
        tracing::info!("send_to_cli returned: {}", sent);

        session.status = SessionStatus::Running;
        session.streaming_text.clear();
        session.scroll_offset = 0;
    }
    app.dirty = true;
}

// ─── App Event Handling ─────────────────────────────────────────────────────

fn handle_app_event(event: AppEvent, app: &mut App) {
    match event {
        AppEvent::CliConnected {
            session_id,
            sender,
        } => {
            tracing::info!("CLI connected for session {}", session_id);
            if let Some(session) = app.sessions.get_mut(&session_id) {
                session.cli_connected = true;
                session.cli_sender = Some(sender);
                session.status = SessionStatus::Idle;
                app.dirty = true;
            }
        }
        AppEvent::CliDisconnected { session_id } => {
            if let Some(session) = app.sessions.get_mut(&session_id) {
                if session.cli_connected {
                    tracing::info!("CLI disconnected for session {}", session_id);
                    session.cli_connected = false;
                    session.cli_sender = None;
                    session.status = SessionStatus::WaitingForCli;
                    session.add_system_message("Claude CLI disconnected".to_string());
                    // Persist on disconnect
                    let _ = session.persist();
                    app.dirty = true;
                }
            }
        }
        AppEvent::CliMessage {
            session_id,
            message,
        } => {
            handle_cli_message(message, &session_id, app);
        }
        AppEvent::CliProcessExited { session_id } => {
            tracing::info!("CLI process exited for session {}", session_id);
            if let Some(session) = app.sessions.get_mut(&session_id) {
                session.cli_process_handle = None;
                // CliDisconnected will handle the rest
            }
        }
    }
}

fn handle_cli_message(msg: CliMessage, session_id: &str, app: &mut App) {
    tracing::debug!("CLI message for {}: {:?}", session_id, std::mem::discriminant(&msg));
    match msg {
        CliMessage::System(sys) => handle_system_message(sys, session_id, app),
        CliMessage::Assistant(asst) => handle_assistant_message(asst, session_id, app),
        CliMessage::Result(result) => handle_result_message(result, session_id, app),
        CliMessage::StreamEvent(stream) => handle_stream_event(stream, session_id, app),
        CliMessage::ControlRequest(ctrl) => handle_control_request(ctrl, session_id, app),
        CliMessage::ToolProgress(prog) => {
            if let Some(session) = app.sessions.get_mut(session_id) {
                session.current_tool = Some((
                    prog.tool_name.clone(),
                    prog.elapsed_time_seconds.unwrap_or(0.0),
                ));
            }
        }
        CliMessage::ToolUseSummary(summary) => {
            if let Some(session) = app.sessions.get_mut(session_id) {
                session.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: summary.summary.clone(),
                    content_blocks: None,
                    model: None,
                    timestamp: chrono::Utc::now().timestamp(),
                });
            }
        }
        CliMessage::AuthStatus(auth) => {
            if let Some(error) = auth.error {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    session.add_system_message(format!("Auth error: {}", error));
                }
            }
        }
        CliMessage::MessageHistory(history) => {
            if let Some(session) = app.sessions.get_mut(session_id) {
                for entry in &history.messages {
                    let role_str = entry.role.as_deref().unwrap_or("assistant");
                    let role = match role_str {
                        "user" => crate::app::ChatRole::User,
                        "assistant" => crate::app::ChatRole::Assistant,
                        _ => crate::app::ChatRole::System,
                    };
                    // Extract text from content (can be string or array of blocks)
                    let text = if let Some(s) = entry.content.as_str() {
                        s.to_string()
                    } else if let Some(arr) = entry.content.as_array() {
                        arr.iter()
                            .filter_map(|b| {
                                if b.get("type")?.as_str()? == "text" {
                                    b.get("text")?.as_str().map(String::from)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        String::new()
                    };
                    if !text.is_empty() {
                        session.messages.push(crate::app::ChatMessage {
                            role,
                            content: text,
                            content_blocks: None,
                            model: entry.model.clone(),
                            timestamp: chrono::Utc::now().timestamp(),
                        });
                    }
                }
                session.scroll_offset = 0;
            }
        }
        CliMessage::KeepAlive => {}
        CliMessage::Unknown => {
            tracing::debug!("Unknown CLI message type");
        }
    }
    app.dirty = true;
}

fn handle_system_message(msg: types::SystemMessage, session_id: &str, app: &mut App) {
    let session = match app.sessions.get_mut(session_id) {
        Some(s) => s,
        None => return,
    };

    match msg.subtype.as_str() {
        "init" => {
            tracing::info!("Received system/init for session {}", session_id);
            let first_init = session.version.is_empty();
            if let Some(sid) = &msg.session_id {
                session.cli_session_id = Some(sid.clone());
            }
            if let Some(model) = &msg.model {
                session.model = model.clone();
            }
            if let Some(cwd) = &msg.cwd {
                session.cwd = cwd.clone();
            }
            if let Some(mode) = &msg.permission_mode {
                session.permission_mode = mode.clone();
            }
            if let Some(version) = &msg.claude_code_version {
                session.version = version.clone();
            }
            if let Some(tools) = msg.tools {
                session.tools = tools;
            }
            if let Some(cmds) = msg.slash_commands {
                session.slash_commands = cmds;
            }
            if let Some(sk) = msg.skills {
                session.skills = sk;
            }
            session.status = SessionStatus::Idle;

            // Gather git info from the cwd (blocking but fast)
            let cwd = session.cwd.clone();
            let git = crate::app::gather_git_info(&cwd);
            session.git_branch = git.branch;
            session.is_worktree = git.is_worktree;
            session.repo_root = git.repo_root;
            session.git_ahead = git.ahead;
            session.git_behind = git.behind;
            // Only show the "Connected" message on the first init
            if first_init {
                session.add_system_message(format!(
                    "Connected to Claude Code {} (model: {})",
                    session.version, session.model
                ));
            }

            // Send queued messages if any (pre-connect or resume)
            if !session.queued_messages.is_empty() {
                let queued: Vec<String> = session.queued_messages.drain(..).collect();
                let cli_session_id = session
                    .cli_session_id
                    .clone()
                    .unwrap_or_else(|| session.id.clone());
                // Send only the last queued message (earlier ones are already in chat
                // history for display but shouldn't be sent as separate API turns)
                if let Some(last_msg) = queued.last() {
                    let msg = OutgoingUserMessage::new(last_msg.clone(), cli_session_id);
                    session.send_to_cli(&msg.to_ndjson());
                    session.status = SessionStatus::Running;
                    session.streaming_text.clear();
                    session.scroll_offset = 0;
                }
            }
        }
        "status" => {
            if let Some(status) = &msg.status {
                if status.as_str() == Some("compacting") {
                    session.status = SessionStatus::Compacting;
                } else if status.is_null() {
                    session.status = SessionStatus::Idle;
                }
            }
        }
        "compact_boundary" => {
            session.add_system_message("Context compacted".to_string());
        }
        other => {
            tracing::debug!("Unhandled system subtype: {}", other);
        }
    }
}

fn handle_assistant_message(msg: types::AssistantMessage, session_id: &str, app: &mut App) {
    let session = match app.sessions.get_mut(session_id) {
        Some(s) => s,
        None => return,
    };

    // Deduplicate: the CLI often sends the same assistant message twice
    let msg_id = &msg.message.id;
    if session.last_assistant_msg_id.as_deref() == Some(msg_id) {
        tracing::debug!("Skipping duplicate assistant message: {}", msg_id);
        session.streaming_text.clear();
        return;
    }
    session.last_assistant_msg_id = Some(msg_id.clone());

    // Extract tasks from tool_use blocks
    extract_tasks_from_blocks(&msg.message.content, session);

    // Extract AskUserQuestion if present
    extract_question_from_blocks(&msg.message.content, session);

    let text = types::extract_text_from_blocks(&msg.message.content);

    session.messages.push(ChatMessage {
        role: ChatRole::Assistant,
        content: text,
        content_blocks: Some(msg.message.content),
        model: msg.message.model,
        timestamp: chrono::Utc::now().timestamp(),
    });

    session.streaming_text.clear();
    session.scroll_offset = 0;
    session.status = SessionStatus::Running;
}

fn handle_result_message(msg: types::ResultMessage, session_id: &str, app: &mut App) {
    let session = match app.sessions.get_mut(session_id) {
        Some(s) => s,
        None => return,
    };

    if let Some(cost) = msg.total_cost_usd {
        session.total_cost_usd = cost;
    }
    if let Some(turns) = msg.num_turns {
        session.num_turns = turns;
    }
    if let Some(added) = msg.total_lines_added {
        session.total_lines_added = added;
    }
    if let Some(removed) = msg.total_lines_removed {
        session.total_lines_removed = removed;
    }

    if let Some(model_usage) = &msg.model_usage {
        for usage in model_usage.values() {
            if let (Some(input), Some(output), Some(ctx)) = (
                usage.input_tokens,
                usage.output_tokens,
                usage.context_window,
            ) {
                if ctx > 0 {
                    session.context_used_percent =
                        ((input + output) as f64 / ctx as f64 * 100.0) as u32;
                }
            }
        }
    }

    if msg.is_error {
        if let Some(errors) = &msg.errors {
            let error_text = errors.join(", ");
            session.add_system_message(format!("Error: {}", error_text));
        }
    }

    session.streaming_text.clear();
    session.current_tool = None;
    session.stream_start = None;
    session.status = SessionStatus::Idle;
    session.interrupt_sent = false;
    session.dirty_persist = true;

    // Persist after each turn
    let _ = session.persist();
}

fn handle_stream_event(msg: types::StreamEventMessage, session_id: &str, app: &mut App) {
    let session = match app.sessions.get_mut(session_id) {
        Some(s) => s,
        None => return,
    };

    let event = &msg.event;

    if event.get("type").and_then(|v| v.as_str()) == Some("message_start") {
        session.status = SessionStatus::Running;
        session.stream_start = Some(std::time::Instant::now());
        session.stream_output_tokens = 0;
        return;
    }

    if event.get("type").and_then(|v| v.as_str()) == Some("content_block_delta") {
        if let Some(delta) = event.get("delta") {
            if delta.get("type").and_then(|v| v.as_str()) == Some("text_delta") {
                if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                    session.streaming_text.push_str(text);
                    // Approximate token count (rough: ~4 chars per token)
                    session.stream_output_tokens += (text.len() as u64 + 3) / 4;
                    app.dirty = true;
                }
            }
        }
    }
}

fn handle_control_request(
    msg: types::ControlRequestMessage,
    session_id: &str,
    app: &mut App,
) {
    let session = match app.sessions.get_mut(session_id) {
        Some(s) => s,
        None => return,
    };

    match &msg.request {
        ControlRequestPayload::CanUseTool {
            tool_name,
            input,
            description,
            permission_suggestions,
            ..
        } => {
            tracing::info!("Permission request: {} - {:?}", tool_name, description);

            let summary = types::format_tool_summary(tool_name, input);

            // Store as pending permission — user must approve via Y/N/A
            session.pending_permission = Some(PendingPermission {
                request_id: msg.request_id,
                tool_name: tool_name.clone(),
                input: input.clone(),
                description: Some(format!("{} {}", tool_name, summary)),
                permission_suggestions: permission_suggestions.clone(),
            });
            app.dirty = true;
        }
        ControlRequestPayload::HookCallback { .. } => {
            tracing::debug!("Hook callback (not implemented)");
        }
        ControlRequestPayload::Unknown => {
            tracing::debug!("Unknown control request subtype");
        }
    }
}

// ─── Task Extraction ────────────────────────────────────────────────────────

fn extract_tasks_from_blocks(
    blocks: &[types::ContentBlock],
    session: &mut crate::app::Session,
) {
    use types::ContentBlock;

    for block in blocks {
        if let ContentBlock::ToolUse { name, input, id } = block {
            match name.as_str() {
                "TaskCreate" => {
                    let subject = input
                        .get("subject")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Untitled")
                        .to_string();
                    let description = input
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let active_form = input
                        .get("activeForm")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    session.tasks.push(TaskItem {
                        id: id.clone(),
                        subject,
                        status: TaskStatus::Pending,
                        description,
                        active_form,
                        blocked_by: Vec::new(),
                    });
                }
                "TaskUpdate" => {
                    let task_id = input
                        .get("taskId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(task) = session.tasks.iter_mut().find(|t| t.id == task_id) {
                        if let Some(status_str) = input.get("status").and_then(|v| v.as_str()) {
                            task.status = match status_str {
                                "pending" => TaskStatus::Pending,
                                "in_progress" => TaskStatus::InProgress,
                                "completed" => TaskStatus::Completed,
                                "deleted" => TaskStatus::Deleted,
                                _ => task.status.clone(),
                            };
                        }
                        if let Some(subject) = input.get("subject").and_then(|v| v.as_str()) {
                            task.subject = subject.to_string();
                        }
                        if let Some(desc) = input.get("description").and_then(|v| v.as_str()) {
                            task.description = desc.to_string();
                        }
                        if let Some(af) = input.get("activeForm").and_then(|v| v.as_str()) {
                            task.active_form = Some(af.to_string());
                        }
                    }
                }
                "TodoWrite" => {
                    // TodoWrite replaces the entire task list
                    if let Some(todos) = input.get("todos").and_then(|v| v.as_array()) {
                        session.tasks.clear();
                        for todo in todos {
                            let task_id = todo
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let content = todo
                                .get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let status_str = todo
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("pending");
                            let status = match status_str {
                                "in_progress" => TaskStatus::InProgress,
                                "completed" => TaskStatus::Completed,
                                "deleted" => TaskStatus::Deleted,
                                _ => TaskStatus::Pending,
                            };
                            session.tasks.push(TaskItem {
                                id: task_id,
                                subject: content,
                                status,
                                description: String::new(),
                                active_form: None,
                                blocked_by: Vec::new(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn extract_question_from_blocks(
    blocks: &[types::ContentBlock],
    session: &mut crate::app::Session,
) {
    use types::ContentBlock;

    for block in blocks {
        if let ContentBlock::ToolUse { name, input, id } = block {
            if name == "AskUserQuestion" {
                if let Some(questions_arr) = input.get("questions").and_then(|v| v.as_array()) {
                    let mut questions = Vec::new();
                    for q in questions_arr {
                        let question_text = q
                            .get("question")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mut options = Vec::new();
                        if let Some(opts) = q.get("options").and_then(|v| v.as_array()) {
                            for opt in opts {
                                options.push(crate::app::QuestionOption {
                                    label: opt
                                        .get("label")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    description: opt
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                });
                            }
                        }
                        // Add "Other..." option
                        options.push(crate::app::QuestionOption {
                            label: "Other...".to_string(),
                            description: "Type a custom response".to_string(),
                        });
                        questions.push(crate::app::QuestionItem {
                            question: question_text,
                            options,
                            selected_option: None,
                        });
                    }
                    if !questions.is_empty() {
                        session.pending_question = Some(crate::app::PendingQuestion {
                            tool_use_id: id.clone(),
                            questions,
                            selected: 0,
                        });
                    }
                }
            }
        }
    }
}
