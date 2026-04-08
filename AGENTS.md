# AGENTS.md

Guidelines for AI agents working on this codebase.

## Navigation Convention

All TUI views (timeline, history, tag picker, picker) follow vim convention:

- `k` / Up arrow: Move UP visually (toward top of screen)
- `j` / Down arrow: Move DOWN visually (toward bottom of screen)

### Implementation details

Sessions are sorted chronologically (oldest first, newest last):
- Index 0 = oldest session (rendered at TOP of screen)
- Index N-1 = newest session (rendered at BOTTOM of screen)

Therefore:
- `select_prev_session()` DECREMENTS index (moves toward index 0, UP visually to EARLIER sessions)
- `select_next_session()` INCREMENTS index (moves toward higher index, DOWN visually to LATER sessions)

### Key bindings summary

| View | Normal Mode | Search Mode |
|------|-------------|-------------|
| Timeline | `j`/`k` for navigation | `Ctrl+n`/`Ctrl+p` for navigation |
| History | `j`/`k` for navigation | `Ctrl+n`/`Ctrl+p` for navigation |
| TagPicker | `j`/`k` for navigation | N/A |
| Picker | `j`/`k` for navigation | N/A |

### Common mistakes

- Do NOT swap j/k meanings - `j` always means DOWN, `k` always means UP
- Do NOT confuse "next/prev" with chronological order - it refers to VISUAL order

## Commands

### Build and test

```bash
cargo build
cargo test
cargo clippy
```

### Install

```bash
cargo install --path=. --root=${HOME}/.local/
```

### Run with test database

```bash
WDTTG_DB=/tmp/test.db wdttg <command>
```

## Configuration

Environment variables (can be set via `.env` file):

- `WDTTG_DB`: Database path (default: `~/.local/share/wdttg/wdttg.db`)
- `WDTTG_VAULT`: Obsidian vault path (for link resolution)

## Project Structure

```
wdttg/
├── src/
│   ├── main.rs              # CLI entry point, command handlers
│   ├── cli.rs               # Clap CLI definitions
│   ├── db.rs                # SQLite database layer, migrations
│   ├── config.rs            # Config file handling
│   ├── time_parser.rs       # Flexible time parsing
│   ├── clipboard.rs         # Clipboard and note path utilities
│   ├── lib.rs               # Library exports
│   ├── models/              # Data structures (no DB logic)
│   │   ├── mod.rs
│   │   ├── task.rs          # Task, TaskStatus, EmQuadrant
│   │   └── session.rs       # Session, SessionWithTask, aggregations
│   ├── services/            # Business logic, DB operations
│   │   ├── mod.rs
│   │   ├── task_service.rs  # Task CRUD, tag extraction
│   │   └── session_service.rs # Time tracking, reports, overlaps
│   └── ui/                  # TUI components (ratatui)
│       ├── mod.rs
│       ├── picker.rs        # Task picker with fuzzy search
│       ├── timeline.rs      # Day timeline view
│       ├── history_view.rs  # Session history browser
│       ├── tag_picker.rs    # Tag selection
│       ├── edit_form.rs     # Task editing
│       ├── session_edit_form.rs # Session editing
│       └── em_selector.rs   # Eisenhower Matrix quadrant picker
└── .env                     # Local configuration
```
