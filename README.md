# claude-code-companion

A terminal companion for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Provides a vim-like TUI for managing conversations, sessions, permissions, and tasks on top of the Claude CLI.

## Install

```bash
cargo install claude-code-companion
```

## Usage

```bash
claude-code-companion                          # start with defaults
claude-code-companion --port 9000              # custom WebSocket port
claude-code-companion --cwd ~/projects/myapp   # set working directory
claude-code-companion --model claude-opus-4-6  # specify model
claude-code-companion --connect                # attach to existing CLI
```

## How it works

The TUI spawns a Claude Code CLI subprocess in SDK mode and communicates over a local WebSocket (NDJSON protocol). Messages, tool approvals, and interrupts flow bidirectionally between the TUI and CLI.

```
┌─────────────┐  WebSocket (NDJSON)  ┌───────────┐
│  Companion   │◄───────────────────►│ Claude CLI │
│     TUI      │  ws://127.0.0.1:P   │  (SDK mode)│
└─────────────┘                      └───────────┘
```

## Modes

| Mode | Enter | Purpose |
|------|-------|---------|
| **Normal** | `Esc` | Navigation, scrolling, session switching |
| **Insert** | `i` / `a` | Compose and send messages |
| **Command** | `:` | Execute TUI commands |

## Keybindings

### Normal mode

| Key | Action |
|-----|--------|
| `i` / `a` | Enter Insert mode |
| `:` | Enter Command mode |
| `j` / `k` | Scroll down / up |
| `G` / `gg` | Jump to bottom / top |
| `Ctrl+D` / `Ctrl+U` | Half-page down / up |
| `PageUp` / `PageDown` | Scroll by 10 lines |
| `1`-`9` | Switch to session N |
| `[` / `]` | Previous / next session |
| `/` | Search chat (`n`/`N` navigate, `Esc` clear) |
| `y` | Yank last assistant response to clipboard |
| `z` | Toggle tool result collapse |
| `Tab` | Toggle sidebar |
| `t` | Toggle task panel |
| `T` | Toggle thinking block visibility |
| `p` | Toggle plan mode |
| `Ctrl+N` | New session |
| `Ctrl+C` | Interrupt (2x to quit) |

### Insert mode

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Esc` | Back to Normal mode |
| `Ctrl+J` | Insert newline (multi-line input) |
| `Up` / `Down` | Navigate lines or cycle input history |
| `Ctrl+A` / `Ctrl+E` | Home / End |
| `Ctrl+K` / `Ctrl+U` | Kill to end / start of line |
| `Ctrl+W` | Delete word backward |

### Command mode

| Key | Action |
|-----|--------|
| `Enter` | Execute command |
| `Esc` | Cancel |
| `Up` / `Down` | Cycle command history |

## Commands

| Command | Description |
|---------|-------------|
| `:new [env]` | Create new session (with optional env profile) |
| `:kill` | Delete current session |
| `:rename <name>` | Rename session |
| `:archive` | Archive session |
| `:unarchive <n>` | Unarchive session by number |
| `:go <name>` | Fuzzy switch to session by name |
| `:pin` / `:unpin` | Pin/unpin session to top of sidebar |
| `:ls` | List all sessions |
| `:model <name>` | Change model |
| `:mode <mode>` | Change permission mode |
| `:cd <path>` | Change working directory |
| `:wt <branch>` | Open git worktree as new session |
| `:!<cmd>` | Execute shell command |
| `:img <path>` | Attach image to send |
| `:pull` | Git pull in current directory |
| `:reconnect` | Respawn CLI for current session |
| `:export <path>` | Export conversation as markdown |
| `:clear` | Clear chat history |
| `:help` | Show help |
| `:quit` | Exit |

## Features

- **Multiple sessions** with independent chat history, model, and working directory
- **Session persistence** across restarts (`~/.companion/sessions/`)
- **Pinned sessions** — pin frequently used sessions to the top of the sidebar
- **Fuzzy session switch** — `:go` for quick name-based session switching
- **Git integration** — branch display, ahead/behind tracking, worktree support
- **Permission management** — approve/deny/always-allow tool use, plan mode toggle
- **Environment profiles** — preconfigured env vars in `~/.companion/envs/`
- **Task tracking** — view task progress from Claude's TodoWrite tool
- **Streaming responses** with animated spinner and tool progress indicators
- **Markdown rendering** — code blocks, headers, inline code, bold, and bullet lists
- **Search in chat** — `/` to search, `n`/`N` to navigate matches
- **Multi-line input** — `Ctrl+J` to insert newlines, input area grows up to 5 lines
- **Input & command history** — `Up`/`Down` to cycle through previous messages and commands
- **Clipboard yank** — `y` copies last assistant response to system clipboard
- **Collapsible tool results** — `z` to toggle tool output visibility
- **Auto-scroll lock** — scrolling up locks position; `G` unlocks
- **Export** — `:export` saves conversation as markdown
- **Desktop notifications** — terminal bell + macOS notification on task completion
- **Shell execution** — run commands without leaving the TUI

## Configuration

| Path | Purpose |
|------|---------|
| `~/.companion/sessions/` | Persisted session data |
| `~/.companion/envs/` | Environment profile JSON files |

### Environment profiles

Create JSON files in `~/.companion/envs/`:

```json
{
  "description": "Development",
  "vars": {
    "DEBUG": "1",
    "API_URL": "http://localhost:3000"
  }
}
```

Then use with `:new dev` (filename without `.json`).

## License

MIT
