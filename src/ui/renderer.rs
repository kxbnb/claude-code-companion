use std::io::Write;

use crossterm::{
    cursor, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, Clear, ClearType},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ChatRole, Mode, Session, SessionStatus, TaskStatus};
use crate::protocol::types::{self, ContentBlock};

// ─── Styled Line ────────────────────────────────────────────────────────────

#[derive(Clone)]
enum LineStyle {
    Normal,
    User,
    Assistant,
    System,
    Tool,
    ToolResult,
    Streaming,
    Error,
    Dim,
}

struct StyledLine {
    text: String,
    style: LineStyle,
}

// ─── Main Render ────────────────────────────────────────────────────────────

pub fn render(app: &App, stdout: &mut impl Write) -> anyhow::Result<()> {
    let (width, height) = terminal::size()?;
    let width = width as usize;
    let height = height as usize;

    if height < 4 || width < 20 {
        queue!(stdout, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        queue!(stdout, Print("Terminal too small"))?;
        stdout.flush()?;
        return Ok(());
    }

    // Compute layout regions
    let sidebar_w = if app.layout.sidebar_visible {
        (app.layout.sidebar_width as usize).min(width / 3)
    } else {
        0
    };
    let content_x = sidebar_w;
    let content_w = width.saturating_sub(sidebar_w);

    // Task panel height (only if visible and there are tasks)
    let active_tasks: Vec<_> = app
        .active_session()
        .map(|s| {
            s.tasks
                .iter()
                .filter(|t| t.status != TaskStatus::Deleted)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let task_h = if app.layout.task_panel_visible && !active_tasks.is_empty() {
        (active_tasks.len() + 1).min(6) // header + tasks, max 6 rows
    } else {
        0
    };

    // Layout: chat area + task panel + 1 input + 1 status
    let chat_height = height.saturating_sub(2 + task_h);

    queue!(stdout, cursor::Hide, cursor::MoveTo(0, 0))?;

    // Sidebar (if visible)
    if sidebar_w > 0 {
        render_sidebar(stdout, app, sidebar_w, chat_height + task_h)?;
    }

    // Chat area
    let session = app.active_session();
    let chat_lines = if let Some(session) = session {
        build_chat_lines(session, content_w, app.show_thinking)
    } else {
        vec![StyledLine {
            text: "No active session. Press Ctrl+N to create one.".to_string(),
            style: LineStyle::Dim,
        }]
    };

    // Permission banner takes space from chat area bottom
    let perm_lines = if let Some(session) = session {
        build_permission_banner(session, content_w)
    } else {
        0
    };
    let question_lines = if let Some(session) = session {
        session.pending_question.as_ref().map(|q| {
            if q.questions.is_empty() { 0 }
            else { q.questions[q.selected].options.len() + 2 }
        }).unwrap_or(0)
    } else {
        0
    };
    let effective_chat_h = chat_height.saturating_sub(perm_lines + question_lines);

    let scroll_offset = session.map(|s| s.scroll_offset).unwrap_or(0);
    render_chat_area(
        stdout,
        &chat_lines,
        effective_chat_h,
        content_w,
        content_x,
        scroll_offset,
    )?;

    // Permission banner (rendered at bottom of chat area)
    if perm_lines > 0 {
        if let Some(session) = session {
            let banner_row = effective_chat_h as u16;
            render_permission_banner(stdout, session, banner_row, content_w, content_x)?;
        }
    }

    // Question overlay
    if question_lines > 0 {
        if let Some(session) = session {
            let question_row = (effective_chat_h + perm_lines) as u16;
            render_question_overlay(stdout, session, question_row, content_w, content_x)?;
        }
    }

    // Task panel
    if task_h > 0 {
        let task_row = chat_height as u16;
        render_task_panel(stdout, &active_tasks, task_row, task_h, content_w, content_x)?;
    }

    // Input line
    let input_row = (chat_height + task_h) as u16;
    let input_scroll_start = render_input(stdout, app, input_row, content_w, content_x)?;

    // Slash command menu (above input line)
    if app.slash_menu.visible {
        render_slash_menu(stdout, app, input_row.saturating_sub(1), content_w, content_x)?;
    }

    // Status bar (full width, last row)
    let status_row = (height - 1) as u16;
    render_status_bar(stdout, app, status_row, width)?;

    // Flash message overlay (on status bar, right-aligned)
    if let Some((msg, _)) = &app.flash_message {
        let flash = truncate_to_width(msg, width / 2);
        let flash_x = (width.saturating_sub(flash.len() + 2)) as u16;
        queue!(
            stdout,
            cursor::MoveTo(flash_x, status_row),
            SetBackgroundColor(Color::DarkYellow),
            SetForegroundColor(Color::Black),
            Print(format!(" {} ", flash)),
            ResetColor,
        )?;
    }

    // Position cursor
    match app.mode {
        Mode::Insert => {
            let prompt_len = 2; // "> "
            let cursor_x = (content_x + prompt_len + app.composer.cursor_col().saturating_sub(input_scroll_start))
                .min(width.saturating_sub(1)) as u16;
            queue!(stdout, cursor::MoveTo(cursor_x, input_row), cursor::Show)?;
        }
        Mode::Command => {
            // Cursor in command line (on status bar)
            let cursor_x = (1 + app.command_input.cursor_col()).min(width.saturating_sub(1)) as u16;
            queue!(
                stdout,
                cursor::MoveTo(cursor_x, status_row),
                cursor::Show
            )?;
        }
        Mode::Normal => {
            queue!(stdout, cursor::Hide)?;
        }
    }

    stdout.flush()?;
    Ok(())
}

// ─── Sidebar ────────────────────────────────────────────────────────────────

fn render_sidebar(
    stdout: &mut impl Write,
    app: &App,
    sidebar_w: usize,
    sidebar_h: usize,
) -> anyhow::Result<()> {
    // Build visible (non-archived) session entries
    let visible_order = app.visible_session_order();
    let active_id = app.active_session_id.as_deref();

    // Build sidebar rows: each session gets 1 or 2 lines
    struct SidebarEntry {
        line1: String,
        line2: Option<String>,
        is_active: bool,
    }

    let mut entries: Vec<SidebarEntry> = Vec::new();
    for (i, id) in visible_order.iter().enumerate() {
        if let Some(session) = app.sessions.get(id.as_str()) {
            let is_active = active_id == Some(id.as_str());
            let marker = if is_active { ">" } else { " " };
            const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let status_icon = match session.status {
                SessionStatus::Running | SessionStatus::Compacting => {
                    SPINNER[(app.tick as usize) % SPINNER.len()]
                }
                SessionStatus::WaitingForCli => "\u{25cb}", // ○
                SessionStatus::Idle => {
                    if session.cli_connected {
                        "\u{25cf}" // ●
                    } else {
                        "\u{25cb}" // ○
                    }
                }
            };

            let line1 = format!(
                "{}{} {}. {}",
                marker,
                status_icon,
                i + 1,
                truncate_to_width(&session.name, sidebar_w.saturating_sub(7))
            );

            // Git info second line
            let line2 = if !session.git_branch.is_empty() {
                let mut parts = Vec::new();
                parts.push(truncate_to_width(&session.git_branch, sidebar_w.saturating_sub(10)));
                if session.is_worktree {
                    parts.push("wt".to_string());
                }
                if session.git_ahead > 0 || session.git_behind > 0 {
                    let mut ab = String::new();
                    if session.git_ahead > 0 {
                        ab.push_str(&format!("\u{2191}{}", session.git_ahead));
                    }
                    if session.git_behind > 0 {
                        ab.push_str(&format!("\u{2193}{}", session.git_behind));
                    }
                    parts.push(ab);
                }
                if session.total_lines_added > 0 || session.total_lines_removed > 0 {
                    parts.push(format!(
                        "+{}-{}",
                        session.total_lines_added, session.total_lines_removed
                    ));
                }
                Some(format!("     {}", parts.join(" ")))
            } else {
                None
            };

            entries.push(SidebarEntry {
                line1,
                line2,
                is_active,
            });
        }
    }

    // Flatten entries into (text, is_git_line, is_active) tuples
    let mut rows: Vec<(String, bool, bool)> = Vec::new();
    for entry in &entries {
        rows.push((
            truncate_to_width(&entry.line1, sidebar_w.saturating_sub(1)),
            false,
            entry.is_active,
        ));
        if let Some(ref line2) = entry.line2 {
            rows.push((
                truncate_to_width(line2, sidebar_w.saturating_sub(1)),
                true,
                entry.is_active,
            ));
        }
    }

    for i in 0..sidebar_h {
        let row = i as u16;
        queue!(stdout, cursor::MoveTo(0, row))?;

        let (text, is_git_line, is_active) = rows
            .get(i)
            .cloned()
            .unwrap_or((String::new(), false, false));

        if is_active {
            queue!(
                stdout,
                SetBackgroundColor(Color::DarkGrey),
                SetForegroundColor(if is_git_line { Color::Grey } else { Color::White }),
                SetAttribute(Attribute::Bold),
            )?;
        } else if !text.is_empty() {
            if is_git_line {
                queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
            } else {
                queue!(stdout, SetForegroundColor(Color::Grey))?;
            }
        }

        queue!(
            stdout,
            Print(format!("{:w$}", text, w = sidebar_w.saturating_sub(1))),
            ResetColor,
            SetAttribute(Attribute::Reset),
        )?;

        // Sidebar border
        queue!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("\u{2502}"), // │
            ResetColor,
        )?;
    }

    Ok(())
}

// ─── Chat Lines ─────────────────────────────────────────────────────────────

fn build_chat_lines(session: &Session, width: usize, show_thinking: bool) -> Vec<StyledLine> {
    let mut lines = Vec::new();

    for msg in &session.messages {
        match msg.role {
            ChatRole::User => {
                let text = format!("You: {}", msg.content);
                for line in wrap_text(&text, width) {
                    lines.push(StyledLine {
                        text: line,
                        style: LineStyle::User,
                    });
                }
                lines.push(StyledLine {
                    text: String::new(),
                    style: LineStyle::Normal,
                });
            }
            ChatRole::Assistant => {
                if let Some(blocks) = &msg.content_blocks {
                    // Group consecutive tool_use blocks by name
                    let mut i = 0;
                    while i < blocks.len() {
                        match &blocks[i] {
                            ContentBlock::ToolUse { name, input, .. } => {
                                // Count consecutive tool uses with same name
                                let mut count = 1;
                                let first_summary = types::format_tool_summary(name, input);
                                while i + count < blocks.len() {
                                    if let ContentBlock::ToolUse { name: next_name, .. } = &blocks[i + count] {
                                        if next_name == name {
                                            count += 1;
                                        } else {
                                            break;
                                        }
                                    } else {
                                        break;
                                    }
                                }
                                let text = if count > 1 {
                                    format!("[{} x{}] {}", name, count, first_summary)
                                } else {
                                    format!("[{}] {}", name, first_summary)
                                };
                                for line in wrap_text(&text, width) {
                                    lines.push(StyledLine {
                                        text: line,
                                        style: LineStyle::Tool,
                                    });
                                }
                                i += count;
                            }
                            ContentBlock::ToolResult {
                                content, is_error, ..
                            } => {
                                let text = types::extract_tool_result_text(content);
                                if !text.is_empty() {
                                    let truncated = truncate_chars(&text, 500);
                                    for line in wrap_text(&truncated, width) {
                                        lines.push(StyledLine {
                                            text: line,
                                            style: if *is_error {
                                                LineStyle::Error
                                            } else {
                                                LineStyle::ToolResult
                                            },
                                        });
                                    }
                                }
                                i += 1;
                            }
                            ContentBlock::Thinking { thinking, .. } => {
                                if show_thinking && !thinking.is_empty() {
                                    let truncated = truncate_chars(thinking, 200);
                                    let text = format!("(thinking) {}", truncated);
                                    for line in wrap_text(&text, width) {
                                        lines.push(StyledLine {
                                            text: line,
                                            style: LineStyle::Dim,
                                        });
                                    }
                                }
                                i += 1;
                            }
                            _ => {
                                i += 1;
                            }
                        }
                    }
                }

                if !msg.content.is_empty() {
                    let text = format!("Claude: {}", msg.content);
                    for line in wrap_text(&text, width) {
                        lines.push(StyledLine {
                            text: line,
                            style: LineStyle::Assistant,
                        });
                    }
                }
                lines.push(StyledLine {
                    text: String::new(),
                    style: LineStyle::Normal,
                });
            }
            ChatRole::System => {
                for line in wrap_text(&msg.content, width) {
                    lines.push(StyledLine {
                        text: line,
                        style: LineStyle::System,
                    });
                }
                lines.push(StyledLine {
                    text: String::new(),
                    style: LineStyle::Normal,
                });
            }
        }
    }

    // Streaming text
    if !session.streaming_text.is_empty() {
        let text = format!("Claude: {}", session.streaming_text);
        for line in wrap_text(&text, width) {
            lines.push(StyledLine {
                text: line,
                style: LineStyle::Streaming,
            });
        }
        // Show streaming stats
        if let Some(start) = session.stream_start {
            let elapsed = start.elapsed().as_secs_f64();
            let toks = session.stream_output_tokens;
            let tps = if elapsed > 0.0 { toks as f64 / elapsed } else { 0.0 };
            lines.push(StyledLine {
                text: format!("{:.1}s \u{2502} ~{} tokens \u{2502} {:.0} tok/s", elapsed, toks, tps),
                style: LineStyle::Dim,
            });
        }
    }

    // Status indicators
    match session.status {
        SessionStatus::WaitingForCli => {
            if lines.is_empty() {
                lines.push(StyledLine {
                    text: "Waiting for Claude CLI to connect...".to_string(),
                    style: LineStyle::Dim,
                });
            }
        }
        SessionStatus::Running
            if session.streaming_text.is_empty()
                && session
                    .messages
                    .last()
                    .map(|m| matches!(m.role, ChatRole::User))
                    .unwrap_or(false) =>
        {
            lines.push(StyledLine {
                text: "Claude is thinking...".to_string(),
                style: LineStyle::Dim,
            });
        }
        SessionStatus::Compacting => {
            lines.push(StyledLine {
                text: "Compacting context...".to_string(),
                style: LineStyle::System,
            });
        }
        _ => {}
    }

    lines
}

fn render_chat_area(
    stdout: &mut impl Write,
    lines: &[StyledLine],
    chat_height: usize,
    width: usize,
    x_offset: usize,
    scroll_offset: usize,
) -> anyhow::Result<()> {
    let total = lines.len();
    // Clamp scroll_offset so we don't go past the top
    let clamped_offset = scroll_offset.min(total.saturating_sub(1));
    let end = total.saturating_sub(clamped_offset);
    let start = end.saturating_sub(chat_height);
    let visible = &lines[start..end];

    for (i, line) in visible.iter().enumerate() {
        let row = i as u16;
        queue!(stdout, cursor::MoveTo(x_offset as u16, row))?;

        match line.style {
            LineStyle::User => {
                queue!(
                    stdout,
                    SetForegroundColor(Color::Green),
                    SetAttribute(Attribute::Bold)
                )?;
            }
            LineStyle::Assistant => {
                queue!(stdout, SetForegroundColor(Color::White))?;
            }
            LineStyle::System => {
                queue!(stdout, SetForegroundColor(Color::Yellow))?;
            }
            LineStyle::Tool => {
                queue!(stdout, SetForegroundColor(Color::Cyan))?;
            }
            LineStyle::ToolResult => {
                queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
            }
            LineStyle::Streaming => {
                queue!(stdout, SetForegroundColor(Color::White))?;
            }
            LineStyle::Error => {
                queue!(stdout, SetForegroundColor(Color::Red))?;
            }
            LineStyle::Dim => {
                queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
            }
            LineStyle::Normal => {}
        }

        let display = truncate_to_width(&line.text, width);
        queue!(
            stdout,
            Print(format!("{:width$}", display, width = width)),
            ResetColor,
            SetAttribute(Attribute::Reset),
        )?;
    }

    // Clear remaining chat area lines
    let rendered = visible.len();
    for i in rendered..chat_height {
        let row = i as u16;
        queue!(
            stdout,
            cursor::MoveTo(x_offset as u16, row),
            Print(format!("{:width$}", "", width = width)),
        )?;
    }

    Ok(())
}

// ─── Permission Banner ──────────────────────────────────────────────────────

fn build_permission_banner(session: &Session, _width: usize) -> usize {
    if let Some(perm) = &session.pending_permission {
        let mut count = 2; // header + key hints
        if perm.tool_name == "Edit" {
            if perm.input.get("file_path").is_some() { count += 1; }
            if perm.input.get("old_string").is_some() { count += 1; }
            if perm.input.get("new_string").is_some() { count += 1; }
        } else if perm.tool_name == "Write" || perm.tool_name == "Bash" {
            count += 1;
        } else {
            count += 1;
        }
        count
    } else {
        0
    }
}

fn render_permission_banner(
    stdout: &mut impl Write,
    session: &Session,
    start_row: u16,
    width: usize,
    x_offset: usize,
) -> anyhow::Result<()> {
    if let Some(perm) = &session.pending_permission {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("\u{2502} Permission: {}", perm.tool_name));

        // Show edit diff for Edit tool
        if perm.tool_name == "Edit" {
            if let Some(file_path) = perm.input.get("file_path").and_then(|v| v.as_str()) {
                lines.push(format!("\u{2502} File: {}", file_path));
            }
            if let Some(old) = perm.input.get("old_string").and_then(|v| v.as_str()) {
                let preview = truncate_chars(old, width.saturating_sub(6));
                lines.push(format!("\u{2502} - {}", preview));
            }
            if let Some(new) = perm.input.get("new_string").and_then(|v| v.as_str()) {
                let preview = truncate_chars(new, width.saturating_sub(6));
                lines.push(format!("\u{2502} + {}", preview));
            }
        } else if perm.tool_name == "Write" {
            if let Some(file_path) = perm.input.get("file_path").and_then(|v| v.as_str()) {
                lines.push(format!("\u{2502} File: {}", file_path));
            }
        } else if perm.tool_name == "Bash" {
            if let Some(cmd) = perm.input.get("command").and_then(|v| v.as_str()) {
                let preview = truncate_chars(cmd, width.saturating_sub(4));
                lines.push(format!("\u{2502} {}", preview));
            }
        } else {
            let desc = perm
                .description
                .as_deref()
                .unwrap_or(&perm.tool_name);
            lines.push(format!("\u{2502} {}", truncate_chars(desc, width.saturating_sub(4))));
        }

        lines.push("\u{2502} [Y]es  [N]o  [A]lways allow".to_string());

        for (i, line) in lines.iter().enumerate() {
            let row = start_row + i as u16;
            let display = truncate_to_width(line, width);
            queue!(
                stdout,
                cursor::MoveTo(x_offset as u16, row),
                SetBackgroundColor(Color::DarkYellow),
                SetForegroundColor(Color::Black),
                Print(format!("{:width$}", display, width = width)),
                ResetColor,
            )?;
        }
    }
    Ok(())
}

// ─── Slash Command Menu ──────────────────────────────────────────────────────

fn render_slash_menu(
    stdout: &mut impl Write,
    app: &App,
    menu_row: u16,
    width: usize,
    x_offset: usize,
) -> anyhow::Result<()> {
    if !app.slash_menu.visible {
        return Ok(());
    }

    let items = app.slash_menu.filtered_items();
    let max_visible = 8.min(items.len());
    if max_visible == 0 {
        return Ok(());
    }

    // Render upward from the menu_row
    for i in 0..max_visible {
        let row = menu_row.saturating_sub((max_visible - i) as u16);
        let item = &items[i];
        let is_selected = i == app.slash_menu.selected;

        let prefix = if item.is_skill { "\u{2726} " } else { "/ " };
        let label = format!("{}{}", prefix, item.name);
        let display = truncate_to_width(&label, width.saturating_sub(2));

        queue!(stdout, cursor::MoveTo(x_offset as u16, row))?;

        if is_selected {
            queue!(
                stdout,
                SetBackgroundColor(Color::Rgb { r: 60, g: 60, b: 100 }),
                SetForegroundColor(Color::White),
                SetAttribute(Attribute::Bold),
            )?;
        } else {
            queue!(
                stdout,
                SetBackgroundColor(Color::Rgb { r: 40, g: 40, b: 40 }),
                SetForegroundColor(Color::Grey),
            )?;
        }

        queue!(
            stdout,
            Print(format!(" {:w$}", display, w = width.saturating_sub(1))),
            ResetColor,
            SetAttribute(Attribute::Reset),
        )?;
    }

    Ok(())
}

// ─── Question Overlay ────────────────────────────────────────────────────────

fn render_question_overlay(
    stdout: &mut impl Write,
    session: &Session,
    start_row: u16,
    width: usize,
    x_offset: usize,
) -> anyhow::Result<()> {
    if let Some(q) = &session.pending_question {
        if q.questions.is_empty() {
            return Ok(());
        }
        let question = &q.questions[q.selected];
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("\u{2502} {}", truncate_to_width(&question.question, width.saturating_sub(4))));
        for (i, opt) in question.options.iter().enumerate() {
            let marker = if question.selected_option == Some(i) { ">" } else { " " };
            lines.push(format!("\u{2502} {} {}. {} - {}", marker, i + 1, opt.label, truncate_to_width(&opt.description, width.saturating_sub(10))));
        }
        lines.push("\u{2502} Press 1-9 to select, Enter to confirm, Esc to dismiss".to_string());

        for (i, line) in lines.iter().enumerate() {
            let row = start_row + i as u16;
            let display = truncate_to_width(line, width);
            queue!(
                stdout,
                cursor::MoveTo(x_offset as u16, row),
                SetBackgroundColor(Color::Rgb { r: 30, g: 50, b: 80 }),
                SetForegroundColor(Color::White),
                Print(format!("{:width$}", display, width = width)),
                ResetColor,
            )?;
        }
    }
    Ok(())
}

// ─── Task Panel ─────────────────────────────────────────────────────────────

fn render_task_panel(
    stdout: &mut impl Write,
    tasks: &[&crate::app::TaskItem],
    start_row: u16,
    panel_h: usize,
    width: usize,
    x_offset: usize,
) -> anyhow::Result<()> {
    // Header
    let header = format!(" Tasks ({}) ", tasks.len());
    let header_display = truncate_to_width(&header, width);
    queue!(
        stdout,
        cursor::MoveTo(x_offset as u16, start_row),
        SetBackgroundColor(Color::Rgb { r: 40, g: 40, b: 60 }),
        SetForegroundColor(Color::White),
        Print(format!("{:width$}", header_display, width = width)),
        ResetColor,
    )?;

    // Task rows
    for i in 0..panel_h.saturating_sub(1) {
        let row = start_row + 1 + i as u16;
        queue!(stdout, cursor::MoveTo(x_offset as u16, row))?;

        if i < tasks.len() {
            let task = tasks[i];
            let icon = match task.status {
                TaskStatus::Pending => "[ ]",
                TaskStatus::InProgress => "[>]",
                TaskStatus::Completed => "[x]",
                TaskStatus::Deleted => "[-]",
            };
            let text = format!(" {} {}", icon, task.subject);
            let display = truncate_to_width(&text, width);

            let color = match task.status {
                TaskStatus::Pending => Color::Grey,
                TaskStatus::InProgress => Color::Yellow,
                TaskStatus::Completed => Color::Green,
                TaskStatus::Deleted => Color::DarkGrey,
            };

            queue!(
                stdout,
                SetForegroundColor(color),
                Print(format!("{:width$}", display, width = width)),
                ResetColor,
            )?;
        } else {
            queue!(
                stdout,
                Print(format!("{:width$}", "", width = width)),
            )?;
        }
    }

    Ok(())
}

// ─── Input Line ─────────────────────────────────────────────────────────────

fn render_input(
    stdout: &mut impl Write,
    app: &App,
    row: u16,
    width: usize,
    x_offset: usize,
) -> anyhow::Result<usize> {
    queue!(stdout, cursor::MoveTo(x_offset as u16, row))?;

    let prompt = if app.mode == Mode::Insert {
        "> "
    } else {
        "  "
    };
    let prompt_color = if app.mode == Mode::Insert {
        Color::Cyan
    } else {
        Color::DarkGrey
    };
    let available = width.saturating_sub(prompt.len());

    let text = &app.composer.text;
    let char_count = text.chars().count();
    let cursor_char = app.composer.cursor_col();

    let scroll_start = if char_count <= available {
        0
    } else {
        let margin = available / 4;
        if cursor_char < available.saturating_sub(margin) {
            0
        } else {
            cursor_char.saturating_sub(available.saturating_sub(margin))
        }
    };

    let display_text: String = text.chars().skip(scroll_start).take(available).collect();

    queue!(
        stdout,
        SetForegroundColor(prompt_color),
        Print(prompt),
        ResetColor,
        Print(format!("{:width$}", display_text, width = available)),
    )?;

    Ok(scroll_start)
}

// ─── Status Bar ─────────────────────────────────────────────────────────────

fn render_status_bar(
    stdout: &mut impl Write,
    app: &App,
    row: u16,
    width: usize,
) -> anyhow::Result<()> {
    queue!(stdout, cursor::MoveTo(0, row))?;

    if app.mode == Mode::Command {
        // Command mode: show command input
        let cmd_text = format!(":{}", app.command_input.text);
        let display = truncate_to_width(&cmd_text, width);
        queue!(
            stdout,
            SetBackgroundColor(Color::DarkGrey),
            SetForegroundColor(Color::White),
            Print(format!("{:width$}", display, width = width)),
            ResetColor,
        )?;
        return Ok(());
    }

    // Mode indicator
    let (mode_text, mode_color) = match app.mode {
        Mode::Normal => ("NORMAL", Color::Grey),
        Mode::Insert => ("INSERT", Color::Green),
        Mode::Command => ("COMMAND", Color::Yellow),
    };

    let session = app.active_session();

    let conn = session
        .map(|s| if s.cli_connected { "\u{25cf}" } else { "\u{25cb}" })
        .unwrap_or("\u{25cb}");

    let session_name = session
        .map(|s| s.name.as_str())
        .unwrap_or("no session");

    const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spin = SPINNER[(app.tick as usize) % SPINNER.len()];

    let is_running = session.map(|s| s.status == SessionStatus::Running).unwrap_or(false);

    let status = session
        .map(|s| match s.status {
            SessionStatus::WaitingForCli => "waiting".to_string(),
            SessionStatus::Idle => "idle".to_string(),
            SessionStatus::Running => {
                if let Some((ref tool, elapsed)) = s.current_tool {
                    format!("{} {} {:.0}s", spin, tool, elapsed)
                } else {
                    format!("{} thinking", spin)
                }
            }
            SessionStatus::Compacting => format!("{} compacting", spin),
        })
        .unwrap_or_else(|| "--".to_string());

    let model = session
        .map(|s| {
            if s.model.is_empty() {
                "...".to_string()
            } else {
                shorten_model(&s.model)
            }
        })
        .unwrap_or_else(|| "...".to_string());

    let visible = app.visible_session_order();
    let session_count = visible.len();
    let session_idx = app
        .active_session_id
        .as_ref()
        .and_then(|id| visible.iter().position(|s| s == id))
        .map(|i| i + 1)
        .unwrap_or(0);

    // Build right side: model | ctx | session count
    let mut right_parts: Vec<String> = Vec::new();
    right_parts.push(model);
    if let Some(s) = session {
        let ctx = s.context_used_percent;
        let bar_len = 5;
        let filled = ((ctx as usize) * bar_len / 100).min(bar_len);
        let bar: String = "\u{2588}".repeat(filled) + &"\u{2591}".repeat(bar_len - filled);
        right_parts.push(format!("{}% {}", ctx, bar));
    }
    right_parts.push(format!("{}/{}", session_idx, session_count));

    // Git branch + worktree badge for status bar
    let branch_display = session
        .filter(|s| !s.git_branch.is_empty())
        .map(|s| {
            if s.is_worktree {
                format!("{} wt", s.git_branch)
            } else {
                s.git_branch.clone()
            }
        })
        .unwrap_or_default();
    let branch = branch_display.as_str();

    // Plan mode indicator
    let plan_indicator = session
        .filter(|s| s.permission_mode == "plan")
        .map(|_| " [plan]")
        .unwrap_or("");

    // Current directory (last component, or ~ abbreviation)
    let cwd_short = session
        .map(|s| {
            let home = dirs::home_dir().unwrap_or_default();
            let home_str = home.to_string_lossy();
            if s.cwd == home_str.as_ref() {
                "~".to_string()
            } else if let Some(rel) = s.cwd.strip_prefix(home_str.as_ref()) {
                let rel = rel.strip_prefix('/').unwrap_or(rel);
                format!("~/{}", rel)
            } else {
                s.cwd.clone()
            }
        })
        .unwrap_or_default();

    let left_mode = format!(" {} ", mode_text);
    let mut left_parts = vec![
        format!("{} {}", conn, session_name),
    ];
    if !cwd_short.is_empty() {
        left_parts.push(cwd_short);
    }
    if !branch.is_empty() {
        left_parts.push(branch.to_string());
    }
    if !plan_indicator.is_empty() {
        left_parts.push(plan_indicator.trim().to_string());
    }
    let left_info = format!("{} ", left_parts.join(" "));
    let left_status = format!("{} ", status);
    let right = format!(" {} ", right_parts.join(" \u{2502} ")); // │ separator

    let left_mode_w = UnicodeWidthStr::width(left_mode.as_str());
    let left_info_w = UnicodeWidthStr::width(left_info.as_str());
    let left_status_w = UnicodeWidthStr::width(left_status.as_str());
    let right_w = UnicodeWidthStr::width(right.as_str());
    let padding = width.saturating_sub(left_mode_w + left_info_w + left_status_w + right_w);

    let status_color = if is_running {
        Color::Yellow
    } else {
        Color::White
    };

    queue!(
        stdout,
        SetBackgroundColor(Color::Rgb { r: 30, g: 30, b: 30 }),
        SetForegroundColor(mode_color),
        SetAttribute(Attribute::Bold),
        Print(&left_mode),
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Rgb { r: 30, g: 30, b: 30 }),
        SetForegroundColor(Color::White),
        Print(&left_info),
        SetForegroundColor(status_color),
        Print(&left_status),
        SetForegroundColor(Color::White),
        Print(format!("{:pad$}", "", pad = padding)),
        SetForegroundColor(Color::DarkGrey),
        Print(truncate_to_width(&right, right_w.min(width.saturating_sub(left_mode_w + left_info_w + left_status_w)))),
        ResetColor,
    )?;

    Ok(())
}

/// Shorten model names for the status bar (e.g. "claude-sonnet-4-5-20250929" → "sonnet-4.5")
fn shorten_model(model: &str) -> String {
    if let Some(rest) = model.strip_prefix("claude-") {
        // e.g. "sonnet-4-5-20250929" or "opus-4-6"
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() >= 3 {
            // name-major-minor[-date]
            return format!("{}-{}.{}", parts[0], parts[1], parts[2]);
        } else if parts.len() >= 2 {
            return format!("{}-{}", parts[0], parts[1]);
        }
    }
    // Fallback: truncate long model names
    if model.len() > 20 {
        format!("{}...", &model[..17])
    } else {
        model.to_string()
    }
}

// ─── UTF-8 Safe Text Utilities ──────────────────────────────────────────────

fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    for ch in s.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > max_width {
            break;
        }
        result.push(ch);
        current_width += ch_width;
    }
    result
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let boundary = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    if boundary < s.len() {
        format!("{}...", &s[..boundary])
    } else {
        s.to_string()
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();

    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut remaining = raw_line;
        while !remaining.is_empty() {
            let line_width = UnicodeWidthStr::width(remaining);
            if line_width <= width {
                lines.push(remaining.to_string());
                break;
            }

            let mut byte_at_width = 0;
            let mut col = 0;
            for (i, ch) in remaining.char_indices() {
                let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if col + ch_w > width {
                    break;
                }
                col += ch_w;
                byte_at_width = i + ch.len_utf8();
            }

            if byte_at_width == 0 {
                if let Some(ch) = remaining.chars().next() {
                    byte_at_width = ch.len_utf8();
                } else {
                    break;
                }
            }

            let break_at = remaining[..byte_at_width]
                .rfind(|c: char| c.is_whitespace())
                .unwrap_or(byte_at_width);

            let break_at = if break_at == 0 {
                byte_at_width
            } else {
                break_at
            };

            lines.push(remaining[..break_at].to_string());
            remaining = remaining[break_at..].trim_start();
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}
