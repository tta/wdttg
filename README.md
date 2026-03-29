# WDTTG - Where Did The Time Go

A command-line time tracker with Eisenhower Matrix quadrant support for ROI-focused time management.

## Installation

### From Source

```bash
git clone <repo-url>
cd wdttg
cargo install --path .
```

### Requirements
- Rust 1.70+
- SQLite3 (system dependency)

## Quick Start

```bash
# Start tracking a task
wdttg start "coding/feature-x"
# Select EM quadrant when prompted (1-4)

# Check current timer
wdttg status

# Stop tracking
wdttg stop

# Mark task as done
wdttg done

# View time reports
wdttg report quadrant week
wdttg report task week
wdttg report tag week
```

## Commands

### Time Tracking

| Command | Description |
|---------|-------------|
| `wdttg start [query]` | Start tracking a task (opens picker if no query) |
| `wdttg start [query] --em 11` | Start with EM quadrant (00, 01, 10, 11) |
| `wdttg start [query] --link <url>` | Start with link to external resource |
| `wdttg start [query] --bg` | Start as background task |
| `wdttg stop` | Stop the active timer |
| `wdttg stop --all` | Stop all active timers (foreground and background) |
| `wdttg stop --task <name>` | Stop timer for a specific task |
| `wdttg status` | Show current timer status |
| `wdttg log` | Add a retrospective time entry |
| `wdttg log --task <name> --start <time> --end <time>` | Log with specific times |
| `wdttg log ... --em 11` | Log with EM quadrant |
| `wdttg log ... --link <url>` | Log with link to external resource |

### Task Management

| Command | Description |
|---------|-------------|
| `wdttg done` | Mark the last task as completed |
| `wdttg reopen [query]` | Reopen a completed task |
| `wdttg archive [query]` | Archive a task |
| `wdttg edit [query]` | Edit task properties |
| `wdttg edit [query] --name <name>` | Rename task |
| `wdttg edit [query] --notes <text>` | Set task notes |
| `wdttg edit [query] --em 11` | Set EM quadrant |
| `wdttg edit [query] --status <status>` | Set status (active, completed, archived) |
| `wdttg edit [query] --recurring <bool>` | Set recurring flag |

### Timeline

| Command | Description |
|---------|-------------|
| `wdttg timeline [date]` | Visual timeline of a day (default: today) |

### Reports

| Command | Description |
|---------|-------------|
| `wdttg report quadrant [period]` | Time by Eisenhower Matrix quadrant |
| `wdttg report task [period]` | Time by task |
| `wdttg report tag [period]` | Time by tag |
| `wdttg report history [tag]` | Browse sessions by tag (TUI) |
| `wdttg report history --period month --limit 50` | History with custom period and limit |

### Utility

| Command | Description |
|---------|-------------|
| `wdttg backup --keep N` | Create database backup |
| `wdttg completions <shell>` | Generate shell completions |

## Shell Completions

Generate shell completions for your shell:

```bash
# Bash (add to ~/.bashrc)
wdttg completions bash > ~/.local/share/bash-completion/completions/wdttg

# Zsh (add to ~/.zshrc)
wdttg completions zsh > ~/.zcompdump-wdttg
# Or add to your fpath:
wdttg completions zsh > ~/.zsh/completions/_wdttg

# Fish
wdttg completions fish > ~/.config/fish/completions/wdttg.fish
```

## Eisenhower Matrix

Tasks are categorized using the Eisenhower Matrix:

| Quadrant | Tag | Description |
|----------|-----|-------------|
| Do First | `#em11` | Urgent + Important |
| Schedule | `#em01` | Not Urgent + Important |
| Delegate | `#em10` | Urgent + Not Important |
| Eliminate | `#em00` | Not Urgent + Not Important |

**Target**: Spend 80% of time in `#em01` (Schedule) and 20% in `#em11` (Do First).

## Task Naming & Tags

Use `/` to create hierarchical tags:

```
coding/jira-123
meeting/standup
review/pr-456
```

This creates tasks with tags `coding`, `meeting`, `review` for aggregation.

## Non-Interactive Mode

All commands support `--no-prompt` for scripting:

```bash
# Start task in non-interactive mode (requires --em for new tasks)
wdttg start "coding/feature-x" --em 11 --no-prompt

# Stop timer
wdttg stop --no-prompt

# Complete task
wdttg done --no-prompt

# Create backup (useful for cron)
wdttg backup --keep 7 --no-prompt
```

## Log Command (Retrospective Entries)

Add time for past work:

```bash
# Interactive
wdttg log

# Non-interactive with relative times
wdttg log --task "code review" --start "2h ago" --end "1h ago" --no-prompt

# With specific times
wdttg log --task "meeting" --start "2024-01-15 14:00" --end "2024-01-15 15:30" --no-prompt

# With month/day format (current year)
wdttg log --task "task" --start "3/22 11:00" --end "3/22 12:30" --no-prompt
```

### Time Format Options

The `--start` and `--end` flags accept:
- Relative: `2h ago`, `30m ago`, `1d ago`
- Time only: `14:30` (assumes today, or yesterday if in future)
- Date + time: `2024-01-15 14:00`, `3/22 11:00`
- Keywords: `now`, `yesterday`, `yesterday 14:00`

## History TUI

Browse sessions by tag with an interactive TUI:

```bash
# Open tag picker
wdttg report history

# Go directly to a specific tag
wdttg report history coding

# Change time period with --period (day/week/month)
wdttg report history --period month
```

**Keybindings in TUI:**
- `↑/↓` or `j/k`: Navigate
- `Enter`: Select
- `d/w/m`: Change period (day/week/month)
- `Esc` or `q`: Back/Quit
- `Ctrl+C`: Quit

## Configuration

Config file location:
- **macOS**: `~/Library/Application Support/wdttg/wdttg.json`
- **Linux**: `~/.config/wdttg/wdttg.json`

Example `wdttg.json`:

```json
{
  "db": "~/.local/share/wdttg/wdttg.db",
  "vault": "~/notes/obsidian"
}
```

**Fields:**
- `db`: Database path (default: `~/.local/share/wdttg/wdttg.db`)
- `vault`: Obsidian vault path for wiki-link resolution

**Environment variable overrides:**
- `WDTTG_DB` overrides `db` from config
- `WDTTG_VAULT` overrides `vault` from config

### Database Location

Default: `~/.local/share/wdttg/wdttg.db`

Backups are stored in: `~/.local/share/wdttg/backups/`

## Period Formats

Reports accept these period values:
- `day` or `today`: Today only
- `week`: Last 7 days (default)
- `month`: Last 30 days
- ISO date: `2024-01-15`

## Development

```bash
# Run tests
cargo test

# Run with clippy
cargo clippy --all-targets

# Build release
cargo build --release
```

## License

MIT
