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

/// Handle key events in Normal mode. Returns true if a user message should be sent.
pub fn handle_key_normal(key: KeyEvent, app: &mut App) -> bool {
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
                }
                app.dirty = true;
                return false;
            }
            KeyCode::Char('u') => {
                // Half-page up
                if let Some(session) = app.active_session_mut() {
                    session.scroll_offset = session.scroll_offset.saturating_add(15);
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
            }
            app.dirty = true;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_add(1);
            }
            app.dirty = true;
        }
        KeyCode::Char('G') => {
            // Scroll to bottom
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = 0;
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
        // Toggle plan mode
        KeyCode::Char('p') => {
            if let Some(session) = app.active_session_mut() {
                if session.permission_mode == "plan" {
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
        // Esc
        KeyCode::Esc => {
            app.gg_pending = false;
        }
        // Page scroll
        KeyCode::PageUp => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_add(10);
            }
            app.dirty = true;
        }
        KeyCode::PageDown => {
            if let Some(session) = app.active_session_mut() {
                session.scroll_offset = session.scroll_offset.saturating_sub(10);
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
        _ => None,
    }
}
