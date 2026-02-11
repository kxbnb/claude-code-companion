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
        "archive" => Command::Archive,
        "unarchive" => Command::Unarchive {
            index: arg.and_then(|s| s.parse::<usize>().ok()),
        },
        "clear" => Command::Clear,
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
                "  :ls              List all sessions",
                "  :model <name>    Change model",
                "  :mode <mode>     Change permission mode",
                "  :env             List environment profiles",
                "  :clear           Clear chat history",
                "  :q               Quit",
                "",
                "Keys (Normal mode):",
                "  i/a      Insert mode     :        Command mode",
                "  j/k      Scroll          G/gg     Bottom/top",
                "  1-9      Switch session  ]/[      Next/prev session",
                "  p        Toggle plan     Ctrl+N   New session",
                "  t        Toggle tasks    Tab      Toggle sidebar",
                "  Ctrl+C   Interrupt/quit",
                "",
                "Keys (Insert mode):",
                "  Enter    Send message    Esc      Normal mode",
                "  Ctrl+C   Interrupt       Ctrl+Q   Quit",
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
