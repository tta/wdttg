# WDTTG Architecture

A Rust CLI time tracker built with the **Service Layer pattern** (Model-Service-UI architecture).

## Overview

```
┌─────────────────────────────────────────────────────────┐
│                     main.rs (CLI)                       │
│              Command handlers, orchestration            │
└───────────┬─────────────────────────────┬───────────────┘
            │                             │
            │ creates/uses                │ uses
            ▼                             ▼
┌───────────────────────┐     ┌───────────────────────────┐
│       UI Layer        │     │     Services Layer        │
│  ┌─────────────────┐  │     │  ┌─────────────────────┐  │
│  │   TaskPicker    │──┼────▶│  │    TaskService      │  │
│  │   TimelineView  │  │     │  └─────────────────────┘  │
│  │   HistoryView   │  │     │  ┌─────────────────────┐  │
│  │   TagPicker     │  │     │  │   SessionService    │  │
│  │   EditForm      │  │     │  └─────────────────────┘  │
│  └─────────────────┘  │     └───────────────────────────┘
│      (ratatui TUI)    │                 │
└───────────────────────┘                 │
            │                             ▼
            │ returns              ┌──────────────┐
            ▼                      │    Models    │
     ┌─────────────┐               │  Task, etc.  │
     │ Result Enums│               └──────────────┘
     │ (Selected,  │                      │
     │  Cancelled) │                      ▼
     └─────────────┘               ┌──────────────┐
                                   │   Database   │
                                   │   (SQLite)   │
                                   └──────────────┘
```

## Directory Structure

```
src/
├── main.rs              # CLI entry point, command handlers
├── cli.rs               # Clap CLI definitions
├── db.rs                # SQLite database layer, migrations
├── config.rs            # Config file handling
├── time_parser.rs       # Flexible time parsing
├── clipboard.rs         # Clipboard utilities
├── lib.rs               # Library exports
│
├── models/              # Data structures (no DB logic)
│   ├── mod.rs
│   ├── task.rs          # Task, TaskStatus, EmQuadrant
│   └── session.rs       # Session, SessionWithTask, aggregations
│
├── services/            # Business logic, DB operations
│   ├── mod.rs
│   ├── task_service.rs  # Task CRUD, tag extraction
│   └── session_service.rs # Time tracking, reports, overlaps
│
└── ui/                  # TUI components (ratatui)
    ├── mod.rs
    ├── picker.rs        # Task picker with fuzzy search
    ├── timeline.rs      # Day timeline view
    ├── history_view.rs  # Session history browser
    ├── tag_picker.rs    # Tag selection
    ├── edit_form.rs     # Task editing
    ├── session_edit_form.rs # Session editing
    └── em_selector.rs   # Eisenhower Matrix quadrant picker
```

## Models Layer

Pure data structures with helper methods. No database access.

### Core Entities

```rust
// Task entity
struct Task {
    id: i64,
    name: String,
    notes: Option<String>,
    em_urgent: bool,
    em_important: bool,
    status: TaskStatus,
    recurring: bool,
    tags: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

// Session entity
struct Session {
    id: i64,
    task_id: i64,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    notes: Option<String>,
    link: Option<String>,
    is_background: bool,
}
```

### Value Types

```rust
enum TaskStatus { Active, Completed, Archived }
enum EmQuadrant { DoFirst, Schedule, Delegate, Eliminate }
```

### Composite Types

```rust
struct SessionWithTask {
    session: Session,
    task: Task,
}
```

### Aggregation Types (for reports)

```rust
struct QuadrantTime {
    quadrant: EmQuadrant,
    total_seconds: i64,
}

struct TagTime {
    tag: String,
    total_seconds: i64,
}

struct TaskTime {
    task_name: String,
    total_seconds: i64,
    session_count: i64,
    em_quadrant: EmQuadrant,
}
```

## Services Layer

Business logic and database operations.

### TaskService

```rust
pub struct TaskService {
    db: Arc<Database>,
}

impl TaskService {
    // CRUD
    fn get_or_create(&self, name: &str) -> Result<Task>;
    fn get_by_id(&self, id: i64) -> Result<Option<Task>>;
    fn get_by_name(&self, name: &str) -> Result<Option<Task>>;
    fn update(&self, task: &Task) -> Result<()>;

    // Queries
    fn get_pickable_tasks(&self) -> Result<Vec<Task>>;
    fn search_completed(&self, query: &str) -> Result<Vec<Task>>;

    // Modifications
    fn set_status(&self, id: i64, status: TaskStatus) -> Result<()>;
    fn set_em_quadrant(&self, id: i64, urgent: bool, important: bool) -> Result<()>;
}
```

### SessionService

```rust
pub struct SessionService {
    db: Arc<Database>,
}

impl SessionService {
    // Time tracking
    fn start(&self, task_id: i64, link: Option<&str>) -> Result<Session>;
    fn stop(&self) -> Result<Option<Session>>;
    fn start_background(&self, task_id: i64, link: Option<&str>) -> Result<Session>;
    fn stop_background(&self) -> Result<Option<Session>>;
    fn log_session(&self, task_id: i64, start: DateTime<Utc>, end: DateTime<Utc>, link: Option<&str>) -> Result<Session>;

    // Queries
    fn get_active_foreground(&self) -> Result<Option<SessionWithTask>>;
    fn get_active_background(&self) -> Result<Option<SessionWithTask>>;
    fn get_sessions_by_tag(&self, tag: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<SessionWithTask>>;

    // Reports
    fn get_quadrant_time(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<QuadrantTime>>;
    fn get_task_time(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<TaskTime>>;
    fn get_tag_time(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<TagTime>>;

    // Timeline
    fn get_day_with_gaps(&self, date: NaiveDate) -> Result<Vec<TimeBlock>>;
}
```

## UI Layer

Interactive TUI components using ratatui. UI is a **pure function of data**.

### Design Principle

- **Input**: Models (read-only)
- **Output**: User's choice (enum result)
- **No side effects**: UI doesn't call services or modify data

### Components

| Component         | Input                       | Output              | Purpose                  |
| ----------------- | --------------------------- | ------------------- | ------------------------ |
| `TaskPicker`      | `Vec<Task>`                 | `PickerResult`      | Select/create task       |
| `TimelineView`    | `Vec<TimeBlock>`, date      | `TimelineResult`    | View/edit day's sessions |
| `HistoryView`     | `Vec<SessionWithTask>`, tag | `HistoryViewResult` | Browse sessions by tag   |
| `TagPicker`       | `Vec<TagWithTime>`          | `TagPickerResult`   | Select tag for filtering |
| `EditForm`        | `Task`                      | `EditFormResult`    | Edit task properties     |
| `SessionEditForm` | `Session`, task_name        | `SessionEditResult` | Edit session times/notes |
| `EmSelector`      | (none)                      | `EmSelectorResult`  | Select EM quadrant       |

### Result Enums

UI returns user intent, not performed actions:

```rust
pub enum PickerResult {
    Selected(Task),
    CreateNew(String),
    Cancelled,
}

pub enum TimelineResult {
    Closed,
    Cancelled,
    EditSession(i64),
}

pub enum EditFormResult {
    Saved(EditableTask),
    Cancelled,
}
```

### Usage Pattern

```rust
fn handle_start(db: &Arc<Database>) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));

    // 1. Get data from service
    let tasks = task_service.get_pickable_tasks()?;

    // 2. Show UI, get user choice
    let picker = TaskPicker::new(tasks);
    let result = picker.run()?;

    // 3. Act on user choice
    match result {
        PickerResult::Selected(task) => {
            // Start session...
        }
        PickerResult::CreateNew(name) => {
            let task = task_service.get_or_create(&name)?;
            // Start session...
        }
        PickerResult::Cancelled => {
            println!("Cancelled.");
        }
    }

    Ok(())
}
```

## Data Flow Examples

### Start Task

```
main.rs:handle_start()
    │
    ├── TaskService::get_pickable_tasks()
    │       └── DB query → row_to_task() → Vec<Task>
    │
    ├── TaskPicker::new(tasks).run()
    │       └── User selects task → PickerResult::Selected(Task)
    │
    ├── TaskService::set_em_quadrant(task.id, urgent, important)
    │       └── DB update
    │
    └── SessionService::start(task.id, link)
            ├── Stop any active foreground session
            └── INSERT new session → Session model
```

### Timeline Edit

```
main.rs:handle_report_timeline()
    │
    ├── SessionService::get_day_with_gaps(date)
    │       └── Vec<TimeBlock>
    │
    ├── TimelineView::new(blocks).run()
    │       └── User presses 'e' on session
    │       └── Returns TimelineResult::EditSession(session_id)
    │
    ├── SessionService::get_by_id(session_id)
    │       └── Session
    │
    ├── SessionEditForm::new(session).run()
    │       └── User edits times/notes
    │       └── Returns SessionEditResult::Saved { start, end, notes, link }
    │
    └── SessionService::update_full(session_id, start, end, notes, link)
            └── DB update
```

## Error Handling

Each layer has its own error type:

```rust
// Database layer
pub enum DbError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    NoDataDir,
}

// Service layer
pub enum TaskServiceError {
    Database(rusqlite::Error),
    NotFound(i64),
}

pub enum SessionServiceError {
    Database(rusqlite::Error),
    NotFound(i64),
    InvalidTimeRange,
    BackgroundAlreadyRunning,
    ForegroundAlreadyRunning,
}

// UI layer
pub enum TimelineError {
    Io(std::io::Error),
    Render(String),
}
```

## Why This Architecture?

| Benefit                    | Explanation                                          |
| -------------------------- | ---------------------------------------------------- |
| **Testability**            | Services can use in-memory DB; UI receives mock data |
| **Separation of concerns** | Models = data, Services = logic, UI = presentation   |
| **Decoupling**             | UI doesn't know about services or DB                 |
| **Reusability**            | Same services used by CLI commands and TUI           |
| **Type safety**            | EmQuadrant, TaskStatus are typed enums               |
| **Error propagation**      | Layer-specific errors with thiserror                 |

## Comparison to Other Patterns

This is the **Service Layer pattern**, also known as:

| Context            | Names                                            |
| ------------------ | ------------------------------------------------ |
| Enterprise Java    | Service / DAO / Entity                           |
| .NET               | Service / Repository / Model                     |
| Clean Architecture | Use Cases / Gateways / Entities                  |
| DDD                | Application Services / Repositories / Aggregates |

The key insight: business logic lives in services, not in models or handlers.
