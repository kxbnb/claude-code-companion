use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, InputState, Mode, SessionStatus};
use crate::protocol::types::{OutgoingControlRequest, OutgoingControlResponse, OutgoingSetPermissionMode};

/// Handle permission overlay keys. Returns true if a permission key was handled.
pub fn handle_permission_keys(key: KeyEvent, app: &mut App) -> bool {
    let session = match app.active_session_mut() {
        Some(s) => s,
        None => return false,
    };

    if let Some(perm) = session.pending_permission.take() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let resp = OutgoingControlResponse::allow(perm.request_id, perm.input);
                session.send_to_cli(&resp.to_ndjson());
                session.add_system_message(format!("[approved] {}", perm.tool_name));
                app.dirty = true;
                return true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let resp = OutgoingControlResponse::deny(perm.request_id, "Denied by user");
                session.send_to_cli(&resp.to_ndjson());
                session.add_system_message(format!("[denied] {}", perm.tool_name));
                app.dirty = true;
                return true;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let resp = OutgoingControlResponse::allow(perm.request_id, perm.input);
                session.send_to_cli(&resp.to_ndjson());
                session.add_system_message(format!("[always-allow] {}", perm.tool_name));
                app.dirty = true;
                return true;
            }
            _ => {
                // Put permission back — unhandled key
                session.pending_permission = Some(perm);
            }
        }
    }
    false
}

/// Handle AskUserQuestion overlay keys. Returns true if a question key was handled.
pub fn handle_question_keys(key: KeyEvent, app: &mut App) -> bool {
    let session = match app.active_session_mut() {
        Some(s) => s,
        None => return false,
    };

    if let Some(mut question) = session.pending_question.take() {
        let q = &mut question.questions[question.selected];
        let num_options = q.options.len();

        match key.code {
            // Number keys select option directly (1-9)
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < num_options {
                    q.selected_option = Some(idx);
                    // Send the response
                    let selected = &q.options[idx];
                    let response_text = selected.label.clone();
                    let tool_use_id = question.tool_use_id.clone();

                    // Send as a tool_result via control response
                    let resp_json = serde_json::json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": tool_use_id,
                            "response": {
                                "result": response_text,
                            }
                        }
                    });
                    session.send_to_cli(&serde_json::to_string(&resp_json).unwrap());
                    session.add_system_message(format!("[answered] {}", response_text));
                    app.dirty = true;
                    return true;
                }
                session.pending_question = Some(question);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                q.selected_option = Some(
                    q.selected_option
                        .unwrap_or(0)
                        .saturating_sub(1),
                );
                session.pending_question = Some(question);
                app.dirty = true;
                return true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let current = q.selected_option.unwrap_or(0);
                q.selected_option = Some((current + 1).min(num_options.saturating_sub(1)));
                session.pending_question = Some(question);
                app.dirty = true;
                return true;
            }
            KeyCode::Enter => {
                if let Some(idx) = q.selected_option {
                    let selected = &q.options[idx];
                    let response_text = selected.label.clone();
                    let tool_use_id = question.tool_use_id.clone();

                    let resp_json = serde_json::json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": tool_use_id,
                            "response": {
                                "result": response_text,
                            }
                        }
                    });
                    session.send_to_cli(&serde_json::to_string(&resp_json).unwrap());
                    session.add_system_message(format!("[answered] {}", response_text));
                    app.dirty = true;
                    return true;
                }
                session.pending_question = Some(question);
            }
            KeyCode::Esc => {
                // Dismiss question
                session.add_system_message("[question dismissed]".to_string());
                app.dirty = true;
                return true;
            }
            _ => {
                session.pending_question = Some(question);
            }
        }
    }
    false
}

/// Handle key events in Normal mode. Returns true if a user message should be sent.
pub fn handle_key_normal(key: KeyEvent, app: &mut App) -> bool {
    // Handle search mode keys
    if let Some(ref mut search) = app.search {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Still typing the search query (before Enter)
                if search.matches.is_empty() && search.current_match == 0 {
                    search.input.insert_char(c);
                    app.dirty = true;
                    return false;
                }
                // After search executed, n/N navigate
                match c {
                    'n' => {
                        if !search.matches.is_empty() {
                            search.current_match = (search.current_match + 1) % search.matches.len();
                            // Scroll to current match
                            if let Some(session) = app.active_session_mut() {
                                // We'll set scroll_offset so the matched line is visible
                                // matches are line indices; we need to convert to scroll_offset from bottom
                                // This is approximate — we just need to get close
                                let match_line = search.matches[search.current_match];
                                // scroll_offset = total_lines - match_line - half_screen (approx)
                                // We don't know total lines here, so just set a flag for renderer
                            }
                        }
                        app.dirty = true;
                        return false;
                    }
                    'N' => {
                        if !search.matches.is_empty() {
                            search.current_match = if search.current_match == 0 {
                                search.matches.len() - 1
                            } else {
                                search.current_match - 1
                            };
                        }
                        app.dirty = true;
                        return false;
                    }
                    _ => {
                        // Any other char after search — clear search and handle normally
                        app.search = None;
                        app.dirty = true;
                        // Fall through to handle normally
                    }
                }
            }
            KeyCode::Backspace => {
                if search.matches.is_empty() && search.current_match == 0 {
                    search.input.backspace();
                    if search.input.is_empty() {
                        app.search = None;
                    }
                    app.dirty = true;
                    return false;
                }
            }
            KeyCode::Enter => {
                // Execute the search — populate matches
                // We need the chat lines to search, but we don't have them here
                // So we just mark search as "executed" by setting current_match to 0
                // The renderer will compute matches and store them
                // For now, we mark the search as "ready to execute"
                if search.matches.is_empty() {
                    // Signal that search should be executed (renderer will populate matches)
                    search.current_match = 0;
                    // We need to do the actual matching here since we have access to session
                    if let Some(session) = app.active_session() {
                        let query = search.input.text.to_lowercase();
                        if !query.is_empty() {
                            // We can't call build_chat_lines from keybindings without renderer dependency
                            // Instead, search directly in session.messages
                            let mut match_indices = Vec::new();
                            let mut line_idx = 0usize;
                            for msg in &session.messages {
                                let text = match msg.role {
                                    crate::app::ChatRole::User => format!("You: {}", msg.content),
                                    crate::app::ChatRole::Assistant => format!("Claude: {}", msg.content),
                                    crate::app::ChatRole::System => msg.content.clone(),
                                };
                                let lines: Vec<&str> = text.split('\n').collect();
                                for (j, line) in lines.iter().enumerate() {
                                    if line.to_lowercase().contains(&query) {
                                        match_indices.push(line_idx + j);
                                    }
                                }
                                line_idx += lines.len() + 1; // +1 for blank separator line
                            }
                            // Also search streaming text
                            if !session.streaming_text.is_empty() {
                                let text = format!("Claude: {}", session.streaming_text);
                                for (j, line) in text.split('\n').enumerate() {
                                    if line.to_lowercase().contains(&query) {
                                        match_indices.push(line_idx + j);
                                    }
                                }
                            }
                            search.matches = match_indices;
                            search.current_match = 0;
                        }
                    }
                }
                app.dirty = true;
                return false;
            }
            KeyCode::Esc => {
                app.search = None;
                app.dirty = true;
                return false;
            }
            _ => {}
        }
        // Don't fall through for unhandled keys during search input
        if app.search.is_some() {
            return false;
        }
    }

    // Handle 'gg' for scroll to top
    if app.gg_pending {
        app.gg_pending = false;
        if key.code == KeyCode::Char('g') {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = usize::MAX; // clamped during render
            }
            app.dirty = true;
            return false;
        }
        // Not 'g' after 'g' — fall through to handle this key normally
    }

    // Ctrl combos first
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if let Some(session) = app.active_session_mut() {
                    if session.status == SessionStatus::Running && !session.interrupt_sent {
                        let interrupt = OutgoingControlRequest::interrupt();
                        session.send_to_cli(&interrupt.to_ndjson());
                        session.add_system_message(
                            "Interrupt sent (Ctrl+C again to quit)".to_string(),
                        );
                        session.interrupt_sent = true;
                        app.dirty = true;
                        return false;
                    }
                }
                app.should_quit = true;
                app.dirty = true;
                return false;
            }
            KeyCode::Char('q') => {
                app.should_quit = true;
                app.dirty = true;
                return false;
            }
            KeyCode::Char('n') => {
                let name = crate::app::generate_session_name();
                let cwd = app.default_cwd.clone();
                app.create_session(name, cwd, None);
                app.mode = Mode::Insert;
                return false;
            }
            KeyCode::Char('d') => {
                // Half-page down
                if let Some(session) = app.active_session_mut() {
                    session.scroll_offset = session.scroll_offset.saturating_sub(15);
                    if session.scroll_offset == 0 { session.scroll_locked = false; }
                }
                app.dirty = true;
                return false;
            }
            KeyCode::Char('u') => {
                // Half-page up
                if let Some(session) = app.active_session_mut() {
                    session.scroll_offset = session.scroll_offset.saturating_add(15);
                    session.scroll_locked = true;
                }
                app.dirty = true;
                return false;
            }
            _ => return false,
        }
    }

    match key.code {
        // Enter Insert mode
        KeyCode::Char('i') | KeyCode::Char('a') => {
            app.mode = Mode::Insert;
            app.dirty = true;
        }
        KeyCode::Char('A') => {
            app.mode = Mode::Insert;
            app.composer.end();
            app.dirty = true;
        }
        // Enter Command mode
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.command_input = InputState::new();
            app.dirty = true;
        }
        // Scroll
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_sub(1);
                if session.scroll_offset == 0 { session.scroll_locked = false; }
            }
            app.dirty = true;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_add(1);
                session.scroll_locked = true;
            }
            app.dirty = true;
        }
        KeyCode::Char('G') => {
            // Scroll to bottom
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = 0;
                session.scroll_locked = false;
            }
            app.dirty = true;
        }
        KeyCode::Char('g') => {
            app.gg_pending = true;
        }
        // Session switching: 1-9 (maps to visible sessions)
        KeyCode::Char(c @ '1'..='9') => {
            let idx = (c as usize) - ('1' as usize);
            let visible = app.visible_session_order();
            if let Some(id) = visible.get(idx) {
                app.switch_to_session(id);
            }
        }
        // Next/prev session
        KeyCode::Char(']') => {
            app.next_session();
        }
        KeyCode::Char('[') => {
            app.prev_session();
        }
        // Toggle sidebar
        KeyCode::Tab => {
            app.layout.sidebar_visible = !app.layout.sidebar_visible;
            app.dirty = true;
        }
        // Toggle task panel
        KeyCode::Char('t') => {
            app.layout.task_panel_visible = !app.layout.task_panel_visible;
            app.dirty = true;
        }
        // Toggle thinking block visibility
        KeyCode::Char('T') => {
            app.show_thinking = !app.show_thinking;
            app.flash(if app.show_thinking { "Thinking: shown".to_string() } else { "Thinking: hidden".to_string() });
            app.dirty = true;
        }
        // Toggle plan mode
        KeyCode::Char('p') => {
            if let Some(session) = app.active_session_mut() {
                if !session.cli_connected {
                    app.flash("CLI not connected".to_string());
                } else if session.permission_mode == "plan" {
                    // Restore previous mode
                    let prev = session
                        .previous_permission_mode
                        .take()
                        .unwrap_or_else(|| "default".to_string());
                    let cli_session_id = session
                        .cli_session_id
                        .clone()
                        .unwrap_or_else(|| session.id.clone());
                    let msg = OutgoingSetPermissionMode::new(prev.clone(), cli_session_id);
                    session.send_to_cli(&msg.to_ndjson());
                    session.permission_mode = prev;
                } else {
                    // Save current mode and switch to plan
                    session.previous_permission_mode = Some(session.permission_mode.clone());
                    let cli_session_id = session
                        .cli_session_id
                        .clone()
                        .unwrap_or_else(|| session.id.clone());
                    let msg =
                        OutgoingSetPermissionMode::new("plan".to_string(), cli_session_id);
                    session.send_to_cli(&msg.to_ndjson());
                    session.permission_mode = "plan".to_string();
                }
            }
            app.dirty = true;
        }
        // Search
        KeyCode::Char('/') => {
            app.search = Some(crate::app::SearchState {
                input: crate::app::InputState::new(),
                matches: Vec::new(),
                current_match: 0,
            });
            app.dirty = true;
        }
        // Yank last assistant message to clipboard
        KeyCode::Char('y') => {
            if let Some(session) = app.active_session() {
                if let Some(msg) = session.messages.iter().rev().find(|m| matches!(m.role, crate::app::ChatRole::Assistant)) {
                    let content = msg.content.clone();
                    match std::process::Command::new("pbcopy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(mut child) => {
                            if let Some(stdin) = child.stdin.as_mut() {
                                use std::io::Write;
                                let _ = stdin.write_all(content.as_bytes());
                            }
                            let _ = child.wait();
                            app.flash("Yanked to clipboard".to_string());
                        }
                        Err(_) => {
                            app.flash("Failed to copy (pbcopy not found)".to_string());
                        }
                    }
                } else {
                    app.flash("No assistant message to yank".to_string());
                }
            }
            app.dirty = true;
        }
        // Toggle tool results collapsed
        KeyCode::Char('z') => {
            if let Some(session) = app.active_session_mut() {
                session.tool_results_collapsed = !session.tool_results_collapsed;
                app.flash(if session.tool_results_collapsed {
                    "Tool results: collapsed".to_string()
                } else {
                    "Tool results: expanded".to_string()
                });
            }
            app.dirty = true;
        }
        // Esc
        KeyCode::Esc => {
            app.gg_pending = false;
        }
        // Page scroll
        KeyCode::PageUp => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_add(10);
                session.scroll_locked = true;
            }
            app.dirty = true;
        }
        KeyCode::PageDown => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_sub(10);
                if session.scroll_offset == 0 { session.scroll_locked = false; }
            }
            app.dirty = true;
        }
        _ => {}
    }
    false
}

/// Handle key events in Insert mode. Returns true if a user message should be sent.
pub fn handle_key_insert(key: KeyEvent, app: &mut App) -> bool {
    // Ctrl combos
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if let Some(session) = app.active_session_mut() {
                    if session.status == SessionStatus::Running && !session.interrupt_sent {
                        let interrupt = OutgoingControlRequest::interrupt();
                        session.send_to_cli(&interrupt.to_ndjson());
                        session.add_system_message("Interrupt sent".to_string());
                        session.interrupt_sent = true;
                        app.dirty = true;
                        return false;
                    }
                }
                app.mode = Mode::Normal;
                app.dirty = true;
                return false;
            }
            KeyCode::Char('q') => {
                app.should_quit = true;
                app.dirty = true;
                return false;
            }
            KeyCode::Char('j') => {
                app.composer.insert_newline();
                app.dirty = true;
                return false;
            }
            _ => {}
        }
    }

    // Slash command menu handling
    if app.slash_menu.visible {
        match key.code {
            KeyCode::Esc => {
                app.slash_menu.visible = false;
                app.dirty = true;
                return false;
            }
            KeyCode::Up => {
                if app.slash_menu.selected > 0 {
                    app.slash_menu.selected -= 1;
                }
                app.dirty = true;
                return false;
            }
            KeyCode::Down => {
                let count = app.slash_menu.filtered_items().len();
                if app.slash_menu.selected + 1 < count {
                    app.slash_menu.selected += 1;
                }
                app.dirty = true;
                return false;
            }
            KeyCode::Tab | KeyCode::Enter => {
                let filtered = app.slash_menu.filtered_items();
                if let Some(item) = filtered.get(app.slash_menu.selected) {
                    let name = item.name.clone();
                    // Replace the /filter text with the selected command
                    app.composer.text = format!("/{}", name);
                    app.composer.cursor = app.composer.text.len();
                }
                app.slash_menu.visible = false;
                app.dirty = true;
                // If Enter, also send the message
                if key.code == KeyCode::Enter {
                    return !app.composer.is_empty();
                }
                return false;
            }
            KeyCode::Backspace => {
                app.composer.backspace();
                // Update filter from composer text
                if app.composer.text.starts_with('/') {
                    app.slash_menu.filter = app.composer.text[1..].to_string();
                    app.slash_menu.selected = 0;
                } else {
                    app.slash_menu.visible = false;
                }
                app.dirty = true;
                return false;
            }
            KeyCode::Char(c) => {
                app.composer.insert_char(c);
                if app.composer.text.starts_with('/') {
                    app.slash_menu.filter = app.composer.text[1..].to_string();
                    app.slash_menu.selected = 0;
                }
                app.dirty = true;
                return false;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.dirty = true;
        }
        KeyCode::Enter => {
            if !app.composer.is_empty() {
                return true; // signal to event_loop to send the message
            }
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'a' => app.composer.home(),
                    'e' => app.composer.end(),
                    'k' => app.composer.kill_to_end(),
                    'u' => app.composer.kill_to_start(),
                    'w' => app.composer.delete_word_back(),
                    _ => {}
                }
            } else {
                app.composer.insert_char(c);
                // Activate slash menu when / is typed as first char
                if c == '/' && app.composer.text == "/" {
                    if let Some(session) = app.active_session() {
                        if !session.slash_commands.is_empty() || !session.skills.is_empty() {
                            let mut items: Vec<crate::app::SlashMenuItem> = Vec::new();
                            for cmd in &session.slash_commands {
                                items.push(crate::app::SlashMenuItem {
                                    name: cmd.clone(),
                                    is_skill: false,
                                });
                            }
                            for skill in &session.skills {
                                items.push(crate::app::SlashMenuItem {
                                    name: skill.clone(),
                                    is_skill: true,
                                });
                            }
                            app.slash_menu = crate::app::SlashMenu {
                                visible: true,
                                filter: String::new(),
                                items,
                                selected: 0,
                            };
                        }
                    }
                }
            }
            app.dirty = true;
        }
        KeyCode::Backspace => {
            app.composer.backspace();
            app.dirty = true;
        }
        KeyCode::Delete => {
            app.composer.delete();
            app.dirty = true;
        }
        KeyCode::Left => {
            app.composer.move_left();
            app.dirty = true;
        }
        KeyCode::Right => {
            app.composer.move_right();
            app.dirty = true;
        }
        KeyCode::Home => {
            app.composer.home();
            app.dirty = true;
        }
        KeyCode::End => {
            app.composer.end();
            app.dirty = true;
        }
        KeyCode::Up => {
            // If multi-line and not on first line, move cursor up
            if app.composer.cursor_line() > 0 {
                app.composer.move_up();
            } else {
                // Input history: go backward
                if !app.input_history.is_empty() {
                    if app.input_history_idx.is_none() {
                        app.input_history_draft = app.composer.text.clone();
                        app.input_history_idx = Some(app.input_history.len() - 1);
                    } else if let Some(idx) = app.input_history_idx {
                        if idx > 0 {
                            app.input_history_idx = Some(idx - 1);
                        }
                    }
                    if let Some(idx) = app.input_history_idx {
                        app.composer.text = app.input_history[idx].clone();
                        app.composer.cursor = app.composer.text.len();
                    }
                }
            }
            app.dirty = true;
        }
        KeyCode::Down => {
            // If multi-line and not on last line, move cursor down
            if app.composer.cursor_line() < app.composer.line_count() - 1 {
                app.composer.move_down();
            } else {
                // Input history: go forward
                if let Some(idx) = app.input_history_idx {
                    if idx + 1 < app.input_history.len() {
                        app.input_history_idx = Some(idx + 1);
                        app.composer.text = app.input_history[idx + 1].clone();
                        app.composer.cursor = app.composer.text.len();
                    } else {
                        // Restore draft
                        app.input_history_idx = None;
                        app.composer.text = app.input_history_draft.clone();
                        app.composer.cursor = app.composer.text.len();
                    }
                }
            }
            app.dirty = true;
        }
        _ => {}
    }
    false
}

/// Handle key events in Command mode. Returns Some(command_text) when Enter is pressed.
pub fn handle_key_command(key: KeyEvent, app: &mut App) -> Option<String> {
    // Ctrl combos
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('q') => {
                app.mode = Mode::Normal;
                app.dirty = true;
                return None;
            }
            KeyCode::Char('a') => {
                app.command_input.home();
                app.dirty = true;
                return None;
            }
            KeyCode::Char('e') => {
                app.command_input.end();
                app.dirty = true;
                return None;
            }
            KeyCode::Char('k') => {
                app.command_input.kill_to_end();
                app.dirty = true;
                return None;
            }
            KeyCode::Char('u') => {
                app.command_input.kill_to_start();
                app.dirty = true;
                return None;
            }
            KeyCode::Char('w') => {
                app.command_input.delete_word_back();
                app.dirty = true;
                return None;
            }
            _ => return None,
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.dirty = true;
            None
        }
        KeyCode::Enter => {
            let cmd_text = app.command_input.take();
            app.mode = Mode::Normal;
            app.dirty = true;
            if cmd_text.is_empty() {
                None
            } else {
                app.command_history.push(cmd_text.clone());
                app.command_history_idx = None;
                Some(cmd_text)
            }
        }
        KeyCode::Backspace => {
            if app.command_input.is_empty() {
                app.mode = Mode::Normal;
            } else {
                app.command_input.backspace();
            }
            app.dirty = true;
            None
        }
        KeyCode::Char(c) => {
            app.command_input.insert_char(c);
            app.dirty = true;
            None
        }
        KeyCode::Left => {
            app.command_input.move_left();
            app.dirty = true;
            None
        }
        KeyCode::Right => {
            app.command_input.move_right();
            app.dirty = true;
            None
        }
        KeyCode::Up => {
            if !app.command_history.is_empty() {
                if app.command_history_idx.is_none() {
                    app.command_history_draft = app.command_input.text.clone();
                    app.command_history_idx = Some(app.command_history.len() - 1);
                } else if let Some(idx) = app.command_history_idx {
                    if idx > 0 {
                        app.command_history_idx = Some(idx - 1);
                    }
                }
                if let Some(idx) = app.command_history_idx {
                    app.command_input.text = app.command_history[idx].clone();
                    app.command_input.cursor = app.command_input.text.len();
                }
            }
            app.dirty = true;
            None
        }
        KeyCode::Down => {
            if let Some(idx) = app.command_history_idx {
                if idx + 1 < app.command_history.len() {
                    app.command_history_idx = Some(idx + 1);
                    app.command_input.text = app.command_history[idx + 1].clone();
                    app.command_input.cursor = app.command_input.text.len();
                } else {
                    app.command_history_idx = None;
                    app.command_input.text = app.command_history_draft.clone();
                    app.command_input.cursor = app.command_input.text.len();
                }
            }
            app.dirty = true;
            None
        }
        _ => None,
    }
}
