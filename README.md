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
| `1`-`9` | Switch to session N |
| `[` / `]` | Previous / next session |
| `Tab` | Toggle sidebar |
| `t` | Toggle task panel |
| `p` | Toggle plan mode |
| `Ctrl+N` | New session |
| `Ctrl+D` / `Ctrl+U` | Half-page down / up |
| `Ctrl+C` | Interrupt (2x to quit) |

### Insert mode

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Esc` | Back to Normal mode |
| `Ctrl+A` / `Ctrl+E` | Home / End |
| `Ctrl+K` / `Ctrl+U` | Kill to end / start of line |
| `Ctrl+W` | Delete word backward |

## Commands

| Command | Description |
|---------|-------------|
| `:new [env]` | Create new session (with optional env profile) |
| `:kill` | Delete current session |
| `:rename <name>` | Rename session |
| `:archive` | Archive session |
| `:unarchive <n>` | Unarchive session by number |
| `:ls` | List all sessions |
| `:model <name>` | Change model |
| `:mode <mode>` | Change permission mode |
| `:cd <path>` | Change working directory |
| `:wt <branch>` | Open git worktree as new session |
| `:!<cmd>` | Execute shell command |
| `:clear` | Clear chat history |
| `:help` | Show help |
| `:quit` | Exit |

## Features

- **Multiple sessions** with independent chat history, model, and working directory
- **Session persistence** across restarts (`~/.companion/sessions/`)
- **Git integration** — branch display, ahead/behind tracking, worktree support
- **Permission management** — approve/deny/always-allow tool use, plan mode toggle
- **Environment profiles** — preconfigured env vars in `~/.companion/envs/`
- **Task tracking** — view task progress from Claude's TodoWrite tool
- **Streaming responses** with animated spinner and tool progress indicators
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
