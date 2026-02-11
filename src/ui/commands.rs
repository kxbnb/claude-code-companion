use crate::app::{App, Command, Mode, SessionStatus};

pub fn parse_command(input: &str) -> Command {
    let trimmed = input.trim();

    // :!<cmd> shorthand (vim-style)
    if let Some(rest) = trimmed.strip_prefix('!') {
        return Command::Exec {
            cmd: rest.trim().to_string(),
        };
    }

    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim().to_string());

    match cmd {
        "new" | "n" => Command::New { env: arg },
        "kill" | "close" => Command::Kill,
        "rename" => Command::Rename {
            name: arg.unwrap_or_default(),
        },
        "ls" | "sessions" => Command::Ls,
        "env" | "envs" => Command::Env,
        "model" => Command::Model {
            name: arg.unwrap_or_default(),
        },
        "mode" => Command::PermMode {
            mode: arg.unwrap_or_default(),
        },
        "cd" => Command::Cd {
            path: arg.unwrap_or_default(),
        },
        "wt" | "worktree" => Command::Worktree {
            branch: arg.unwrap_or_default(),
        },
        "exec" => Command::Exec {
            cmd: arg.unwrap_or_default(),
        },
        "img" | "image" => Command::Img {
            path: arg.unwrap_or_default(),
        },
        "pull" => Command::Pull,
        "reconnect" | "rc" => Command::Reconnect,
        "archive" => Command::Archive,
        "unarchive" => Command::Unarchive {
            index: arg.and_then(|s| s.parse::<usize>().ok()),
        },
        "clear" => Command::Clear,
        "go" => Command::Go {
            partial_name: arg.unwrap_or_default(),
        },
        "export" => Command::Export {
            path: arg.unwrap_or_default(),
        },
        "pin" => Command::Pin,
        "unpin" => Command::Unpin,
        "help" | "h" | "?" => Command::Help,
        "q" | "quit" | "exit" => Command::Quit,
        other => Command::Unknown(other.to_string()),
    }
}

#[allow(dead_code)]
pub enum CommandResult {
    Ok,
    SpawnSession {
        session_id: String,
    },
}

pub fn execute_command(cmd: Command, app: &mut App) -> CommandResult {
    match cmd {
        Command::New { env } => {
            let name = crate::app::generate_session_name();
            let cwd = app.default_cwd.clone();
            let id = app.create_session(name, cwd, env);
            app.mode = Mode::Insert;
            CommandResult::SpawnSession { session_id: id }
        }
        Command::Kill => {
            if app.visible_session_order().len() <= 1 {
                app.flash("Cannot kill the last session".to_string());
                CommandResult::Ok
            } else {
                app.kill_active_session();
                CommandResult::Ok
            }
        }
        Command::Rename { name } => {
            if name.is_empty() {
                app.flash("Usage: :rename <name>".to_string());
            } else if let Some(session) = app.active_session_mut() {
                session.name = name;
                session.dirty_persist = true;
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Ls => {
            let mut lines = Vec::new();
            for (i, id) in app.session_order.iter().enumerate() {
                if let Some(session) = app.sessions.get(id) {
                    let active = if app.active_session_id.as_deref() == Some(id.as_str()) {
                        ">"
                    } else {
                        " "
                    };
                    let conn = if session.cli_connected {
                        "\u{25cf}"
                    } else {
                        "\u{25cb}"
                    };
                    let status = match session.status {
                        SessionStatus::Idle => "idle",
                        SessionStatus::Running => "running",
                        SessionStatus::WaitingForCli => "waiting",
                        SessionStatus::Compacting => "compacting",
                    };
                    let archived_tag = if session.archived { " [archived]" } else { "" };
                    lines.push(format!(
                        "{}{} {}. {} {} ${:.4}{}",
                        active,
                        conn,
                        i + 1,
                        session.name,
                        status,
                        session.total_cost_usd,
                        archived_tag,
                    ));
                }
            }
            if let Some(session) = app.active_session_mut() {
                session.add_system_message(lines.join("\n"));
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Env => {
            if app.env_profiles.is_empty() {
                if let Some(session) = app.active_session_mut() {
                    session.add_system_message(
                        "No environment profiles found. Add profiles to ~/.companion/envs/"
                            .to_string(),
                    );
                }
            } else {
                let lines: Vec<String> = app
                    .env_profiles
                    .iter()
                    .map(|p| format!("  {} - {} ({} vars)", p.name, p.description, p.vars.len()))
                    .collect();
                if let Some(session) = app.active_session_mut() {
                    session.add_system_message(format!(
                        "Environment profiles:\n{}",
                        lines.join("\n")
                    ));
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Model { name } => {
            if name.is_empty() {
                app.flash("Usage: :model <name>".to_string());
            } else if let Some(session) = app.active_session_mut() {
                session.add_system_message(format!("Model changed to: {}", name));
                session.model = name;
                session.dirty_persist = true;
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::PermMode { mode } => {
            if mode.is_empty() {
                app.flash("Usage: :mode <permission_mode>".to_string());
            } else if let Some(session) = app.active_session_mut() {
                session.add_system_message(format!("Permission mode: {}", mode));
                session.permission_mode = mode;
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Cd { path } => {
            if path.is_empty() {
                app.flash("Usage: :cd <path>".to_string());
            } else {
                let expanded = if path.starts_with('~') {
                    dirs::home_dir()
                        .map(|h| path.replacen('~', &h.to_string_lossy(), 1))
                        .unwrap_or(path.clone())
                } else if path.starts_with('/') {
                    path.clone()
                } else {
                    // Relative to current cwd
                    let cwd = app
                        .active_session()
                        .map(|s| s.cwd.clone())
                        .unwrap_or_else(|| app.default_cwd.clone());
                    format!("{}/{}", cwd, path)
                };
                let resolved = std::path::PathBuf::from(&expanded);
                if resolved.is_dir() {
                    let canonical = resolved
                        .canonicalize()
                        .unwrap_or(resolved)
                        .to_string_lossy()
                        .to_string();
                    if let Some(session) = app.active_session_mut() {
                        session.cwd = canonical.clone();
                        session.add_system_message(format!("cwd: {}", canonical));
                        session.dirty_persist = true;
                        // Re-gather git info
                        let git = crate::app::gather_git_info(&canonical);
                        session.git_branch = git.branch;
                        session.is_worktree = git.is_worktree;
                        session.repo_root = git.repo_root;
                        session.git_ahead = git.ahead;
                        session.git_behind = git.behind;
                    }
                } else {
                    app.flash(format!("Not a directory: {}", expanded));
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Img { path } => {
            if path.is_empty() {
                app.flash("Usage: :img <path>".to_string());
                return CommandResult::Ok;
            }
            let expanded = if path.starts_with('~') {
                dirs::home_dir()
                    .map(|h| path.replacen('~', &h.to_string_lossy(), 1))
                    .unwrap_or(path.clone())
            } else if path.starts_with('/') {
                path.clone()
            } else {
                let cwd = app
                    .active_session()
                    .map(|s| s.cwd.clone())
                    .unwrap_or_else(|| app.default_cwd.clone());
                format!("{}/{}", cwd, path)
            };

            let file_path = std::path::Path::new(&expanded);
            if !file_path.exists() {
                app.flash(format!("File not found: {}", expanded));
                return CommandResult::Ok;
            }

            // Determine media type from extension
            let media_type = match file_path.extension().and_then(|e| e.to_str()) {
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("webp") => "image/webp",
                _ => {
                    app.flash("Unsupported image format (use png/jpg/gif/webp)".to_string());
                    return CommandResult::Ok;
                }
            };

            // Read and base64 encode
            match std::fs::read(&expanded) {
                Ok(bytes) => {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    if let Some(session) = app.active_session_mut() {
                        let session_id = session
                            .cli_session_id
                            .clone()
                            .unwrap_or_else(|| session.id.clone());
                        let msg = crate::protocol::types::OutgoingImageMessage::new(
                            None,
                            b64,
                            media_type.to_string(),
                            session_id,
                        );
                        session.send_to_cli(&msg.to_ndjson());
                        session.add_system_message(format!("[attached image: {}]", path));
                        session.status = crate::app::SessionStatus::Running;
                    }
                }
                Err(e) => {
                    app.flash(format!("Failed to read image: {}", e));
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Reconnect => {
            if let Some(id) = app.active_session_id.clone() {
                if let Some(session) = app.sessions.get_mut(&id) {
                    // Kill existing process handle if any
                    if let Some(handle) = session.cli_process_handle.take() {
                        handle.abort();
                    }
                    session.cli_connected = false;
                    session.cli_sender = None;
                    session.status = SessionStatus::WaitingForCli;
                    session.add_system_message("Reconnecting...".to_string());
                }
                app.pending_spawns.push(id);
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Pull => {
            let cwd = app
                .active_session()
                .map(|s| s.cwd.clone())
                .unwrap_or_else(|| app.default_cwd.clone());

            use std::process::Command as ProcessCommand;
            let result = ProcessCommand::new("git")
                .args(["pull"])
                .current_dir(&cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();

            match result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let mut text = String::from("$ git pull");
                    if !stdout.is_empty() {
                        text.push('\n');
                        text.push_str(stdout.trim_end());
                    }
                    if !stderr.is_empty() {
                        text.push('\n');
                        text.push_str(stderr.trim_end());
                    }
                    if let Some(session) = app.active_session_mut() {
                        session.add_system_message(text);
                        // Refresh git info
                        let git = crate::app::gather_git_info(&cwd);
                        session.git_branch = git.branch;
                        session.is_worktree = git.is_worktree;
                        session.repo_root = git.repo_root;
                        session.git_ahead = git.ahead;
                        session.git_behind = git.behind;
                    }
                }
                Err(e) => {
                    app.flash(format!("git pull failed: {}", e));
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Exec { cmd: shell_cmd } => {
            if shell_cmd.is_empty() {
                app.flash("Usage: :exec <command> or :!<command>".to_string());
                return CommandResult::Ok;
            }
            let cwd = app
                .active_session()
                .map(|s| s.cwd.clone())
                .unwrap_or_else(|| app.default_cwd.clone());

            use std::process::Command as ProcessCommand;
            let result = ProcessCommand::new("sh")
                .args(["-c", &shell_cmd])
                .current_dir(&cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();

            match result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let mut text = format!("$ {}", shell_cmd);
                    if !stdout.is_empty() {
                        text.push('\n');
                        text.push_str(stdout.trim_end());
                    }
                    if !stderr.is_empty() {
                        text.push('\n');
                        text.push_str(stderr.trim_end());
                    }
                    if !output.status.success() {
                        text.push_str(&format!("\n[exit {}]", output.status.code().unwrap_or(-1)));
                    }
                    if let Some(session) = app.active_session_mut() {
                        session.add_system_message(text);
                    }
                }
                Err(e) => {
                    app.flash(format!("exec failed: {}", e));
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Worktree { branch } => {
            if branch.is_empty() {
                app.flash("Usage: :wt <branch>".to_string());
                return CommandResult::Ok;
            }

            // Find the repo root from the active session's cwd (or default)
            let cwd = app
                .active_session()
                .map(|s| s.cwd.clone())
                .unwrap_or_else(|| app.default_cwd.clone());

            let git_info = crate::app::gather_git_info(&cwd);
            if git_info.repo_root.is_empty() {
                app.flash("Not in a git repository".to_string());
                return CommandResult::Ok;
            }

            // Worktree path: sibling directory to repo root
            let repo_root = std::path::PathBuf::from(&git_info.repo_root);
            let parent = repo_root.parent().unwrap_or(&repo_root);
            let wt_path = parent.join(&branch);

            if wt_path.exists() {
                // Directory already exists — just start a session there
                let wt_dir = wt_path.to_string_lossy().to_string();
                let name = crate::app::generate_session_name();
                let id = app.create_session(name, wt_dir, None);
                app.mode = Mode::Insert;
                return CommandResult::SpawnSession { session_id: id };
            }

            // Try to create the worktree
            use std::process::Command as ProcessCommand;
            // First try checking out existing branch
            let result = ProcessCommand::new("git")
                .args(["worktree", "add", &wt_path.to_string_lossy(), &branch])
                .current_dir(&cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();

            match result {
                Ok(output) if output.status.success() => {
                    let wt_dir = wt_path.to_string_lossy().to_string();
                    let name = crate::app::generate_session_name();
                    let id = app.create_session(name, wt_dir, None);
                    app.mode = Mode::Insert;
                    app.flash(format!("Worktree created: {}", branch));
                    CommandResult::SpawnSession { session_id: id }
                }
                Ok(_) => {
                    // Existing branch not found — create new branch
                    let result2 = ProcessCommand::new("git")
                        .args([
                            "worktree",
                            "add",
                            "-b",
                            &branch,
                            &wt_path.to_string_lossy(),
                        ])
                        .current_dir(&cwd)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .output();

                    match result2 {
                        Ok(output2) if output2.status.success() => {
                            let wt_dir = wt_path.to_string_lossy().to_string();
                            let name = crate::app::generate_session_name();
                            let id = app.create_session(name, wt_dir, None);
                            app.mode = Mode::Insert;
                            app.flash(format!("Worktree created: {}", branch));
                            CommandResult::SpawnSession { session_id: id }
                        }
                        Ok(output2) => {
                            let err =
                                String::from_utf8_lossy(&output2.stderr).trim().to_string();
                            app.flash(format!("Worktree failed: {}", err));
                            CommandResult::Ok
                        }
                        Err(e) => {
                            app.flash(format!("Failed to run git: {}", e));
                            CommandResult::Ok
                        }
                    }
                }
                Err(e) => {
                    app.flash(format!("Failed to run git: {}", e));
                    CommandResult::Ok
                }
            }
        }
        Command::Archive => {
            let visible = app.visible_session_order();
            if visible.len() <= 1 {
                app.flash("Cannot archive the last visible session".to_string());
            } else {
                app.archive_active_session();
                app.flash("Session archived".to_string());
            }
            CommandResult::Ok
        }
        Command::Unarchive { index } => {
            if let Some(idx) = index {
                let real_idx = idx.saturating_sub(1); // 1-indexed for user
                if real_idx < app.session_order.len() {
                    let is_archived = app
                        .session_order
                        .get(real_idx)
                        .and_then(|id| app.sessions.get(id))
                        .map(|s| s.archived)
                        .unwrap_or(false);
                    if is_archived {
                        app.unarchive_session(real_idx);
                        app.flash("Session unarchived".to_string());
                    } else {
                        app.flash("Session is not archived".to_string());
                    }
                } else {
                    app.flash("Invalid session index".to_string());
                }
            } else {
                app.flash("Usage: :unarchive <n>".to_string());
            }
            CommandResult::Ok
        }
        Command::Clear => {
            if let Some(session) = app.active_session_mut() {
                session.messages.clear();
                session.streaming_text.clear();
                session.scroll_offset = 0;
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Go { partial_name } => {
            if partial_name.is_empty() {
                app.flash("Usage: :go <name>".to_string());
            } else {
                let query = partial_name.to_lowercase();
                let visible = app.visible_session_order();
                let found = visible.iter().find(|id| {
                    app.sessions
                        .get(id.as_str())
                        .map(|s| s.name.to_lowercase().contains(&query))
                        .unwrap_or(false)
                });
                if let Some(id) = found {
                    let id = id.clone();
                    app.switch_to_session(&id);
                    app.flash(format!("Switched to session"));
                } else {
                    app.flash(format!("No session matching '{}'", partial_name));
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Export { path } => {
            if path.is_empty() {
                app.flash("Usage: :export <path>".to_string());
            } else {
                let expanded = if path.starts_with('~') {
                    dirs::home_dir()
                        .map(|h| path.replacen('~', &h.to_string_lossy(), 1))
                        .unwrap_or(path.clone())
                } else {
                    path.clone()
                };
                if let Some(session) = app.active_session() {
                    let mut md = String::new();
                    for msg in &session.messages {
                        match msg.role {
                            crate::app::ChatRole::User => {
                                md.push_str("## You\n\n");
                                md.push_str(&msg.content);
                                md.push_str("\n\n");
                            }
                            crate::app::ChatRole::Assistant => {
                                md.push_str("## Claude\n\n");
                                md.push_str(&msg.content);
                                md.push_str("\n\n");
                            }
                            crate::app::ChatRole::System => {
                                md.push_str("> ");
                                md.push_str(&msg.content.replace('\n', "\n> "));
                                md.push_str("\n\n");
                            }
                        }
                    }
                    match std::fs::write(&expanded, &md) {
                        Ok(()) => {
                            app.flash(format!("Exported to {}", expanded));
                        }
                        Err(e) => {
                            app.flash(format!("Export failed: {}", e));
                        }
                    }
                }
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Pin => {
            if let Some(session) = app.active_session_mut() {
                session.pinned = true;
                session.dirty_persist = true;
                let _ = session.persist();
                app.flash("Session pinned".to_string());
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Unpin => {
            if let Some(session) = app.active_session_mut() {
                session.pinned = false;
                session.dirty_persist = true;
                let _ = session.persist();
                app.flash("Session unpinned".to_string());
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Help => {
            let help = [
                "Commands:",
                "  :new [env]       New session (optional env profile)",
                "  :kill            Close current session (hard delete)",
                "  :archive         Archive current session (soft hide)",
                "  :unarchive <n>   Unarchive session by index",
                "  :rename <name>   Rename current session",
                "  :cd <path>       Change working directory",
                "  :wt <branch>     New session in git worktree",
                "  :!<cmd>          Execute shell command",
                "  :img <path>      Attach image to send",
                "  :pull             Git pull in current dir",
                "  :reconnect/:rc    Respawn CLI for session",
                "  :ls              List all sessions",
                "  :model <name>    Change model",
                "  :mode <mode>     Change permission mode",
                "  :env             List environment profiles",
                "  :clear           Clear chat history",
                "  :go <name>       Switch to session by name (fuzzy)",
                "  :export <path>   Export conversation as markdown",
                "  :pin             Pin current session to top",
                "  :unpin           Unpin current session",
                "  :q               Quit",
                "",
                "Keys (Normal mode):",
                "  i/a      Insert mode     :        Command mode",
                "  j/k      Scroll down/up  G/gg     Bottom/top",
                "  Ctrl+D/U Half-page dn/up PageUp/Dn  Scroll by 10",
                "  1-9      Switch session  ]/[      Next/prev session",
                "  /        Search chat     n/N      Next/prev match",
                "  y        Yank response   z        Collapse tools",
                "  p        Toggle plan     T        Toggle thinking",
                "  t        Toggle tasks    Tab      Toggle sidebar",
                "  Ctrl+N   New session     Ctrl+C   Interrupt/quit",
                "",
                "Keys (Insert mode):",
                "  Enter    Send message    Esc      Normal mode",
                "  Ctrl+J   Insert newline  Up/Down  History / line nav",
                "  Ctrl+A/E Home/End        Ctrl+K/U Kill to end/start",
                "  Ctrl+W   Delete word     Ctrl+C   Interrupt",
                "",
                "Keys (Command mode):",
                "  Enter    Execute         Esc      Cancel",
                "  Up/Down  Command history",
            ];
            if let Some(session) = app.active_session_mut() {
                session.add_system_message(help.join("\n"));
            }
            app.dirty = true;
            CommandResult::Ok
        }
        Command::Quit => {
            app.should_quit = true;
            CommandResult::Ok
        }
        Command::Unknown(cmd) => {
            app.flash(format!("Unknown command: {}", cmd));
            CommandResult::Ok
        }
    }
}
