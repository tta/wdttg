use rusqlite::backup::Backup;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cannot determine data directory")]
    NoDataDir,
    #[error("Cannot backup in-memory database")]
    InMemoryBackup,
}

pub type Result<T> = std::result::Result<T, DbError>;

pub struct Database {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Database {
    pub fn open() -> Result<Self> {
        let path = Self::get_db_path()?;
        Self::open_path(&path)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: None,
        })
    }

    pub fn open_path(path: &PathBuf) -> Result<Self> {
        if path.to_str() != Some(":memory:") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(path)?;
        run_migrations(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            path: Some(path.clone()),
        })
    }

    fn get_db_path() -> Result<PathBuf> {
        if let Ok(path) = std::env::var("WDTTG_DB") {
            return Ok(PathBuf::from(path));
        }

        let data_dir = dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .ok_or(DbError::NoDataDir)?;

        Ok(data_dir.join("wdttg").join("wdttg.db"))
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }

    pub fn backup_path(&self) -> Result<PathBuf> {
        let db_path = self.path.as_ref().ok_or(DbError::InMemoryBackup)?;

        let backup_dir = db_path.parent().unwrap().join("backups");
        std::fs::create_dir_all(&backup_dir)?;
        Ok(backup_dir)
    }

    pub fn backup(&self, keep: usize) -> Result<(PathBuf, usize, usize)> {
        let backup_dir = self.backup_path()?;
        let timestamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
        let backup_file = backup_dir.join(format!("{}.db", timestamp));

        let conn = self.connection();
        let mut backup_conn = Connection::open(&backup_file)?;
        let backup = Backup::new(&conn, &mut backup_conn)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;

        let mut backups: Vec<PathBuf> = std::fs::read_dir(&backup_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().map(|e| e == "db").unwrap_or(false))
            .map(|entry| entry.path())
            .collect();

        backups.sort();

        let removed = if backups.len() > keep {
            let to_remove = backups.len() - keep;
            for old_backup in backups.iter().take(to_remove) {
                std::fs::remove_file(old_backup)?;
            }
            to_remove
        } else {
            0
        };

        let kept = backups.len().saturating_sub(removed);

        Ok((backup_file, kept, removed))
    }
}

fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            notes TEXT,
            em_urgent INTEGER NOT NULL CHECK (em_urgent IN (0, 1)),
            em_important INTEGER NOT NULL CHECK (em_important IN (0, 1)),
            status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'completed', 'archived')),
            recurring INTEGER NOT NULL DEFAULT 0 CHECK (recurring IN (0, 1)),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE
        );

        CREATE TABLE IF NOT EXISTS task_tags (
            task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            PRIMARY KEY (task_id, tag_id)
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            start_time TEXT NOT NULL,
            end_time TEXT,
            notes TEXT,
            note TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_task ON sessions(task_id);
        CREATE INDEX IF NOT EXISTS idx_sessions_start ON sessions(start_time);
        CREATE INDEX IF NOT EXISTS idx_sessions_end ON sessions(end_time);
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
        CREATE INDEX IF NOT EXISTS idx_task_tags_tag ON task_tags(tag_id);

        CREATE TRIGGER IF NOT EXISTS tasks_updated_at
        AFTER UPDATE ON tasks
        FOR EACH ROW
        BEGIN
            UPDATE tasks SET updated_at = datetime('now') WHERE id = OLD.id;
        END;
        "#,
    )?;

    let note_column_exists: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('sessions') WHERE name='note')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !note_column_exists {
        conn.execute("ALTER TABLE sessions ADD COLUMN note TEXT", [])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_database() {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let conn = db.connection();

        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='tasks')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "tasks table should exist");

        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "sessions table should exist");

        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='tags')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "tags table should exist");

        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_tags')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists, "task_tags table should exist");
    }

    #[test]
    fn test_migrations_idempotent() {
        let db = Database::open_memory().expect("Failed to create in-memory database");

        let result = run_migrations(&db.connection());
        assert!(result.is_ok(), "Migrations should be idempotent");
    }

    #[test]
    fn test_index_exists() {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let conn = db.connection();

        let index_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_sessions_task')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(index_exists, "idx_sessions_task should exist");

        let index_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_tasks_status')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(index_exists, "idx_tasks_status should exist");
    }

    #[test]
    fn test_trigger_exists() {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let conn = db.connection();

        let trigger_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='tasks_updated_at')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(trigger_exists, "tasks_updated_at trigger should exist");
    }

    #[test]
    fn test_insert_task() {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let conn = db.connection();

        conn.execute(
            "INSERT INTO tasks (name, em_urgent, em_important) VALUES (?1, ?2, ?3)",
            ["test task", "0", "1"],
        )
        .unwrap();

        let name: String = conn
            .query_row("SELECT name FROM tasks WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "test task");
    }

    #[test]
    fn test_insert_session() {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let conn = db.connection();

        conn.execute(
            "INSERT INTO tasks (name, em_urgent, em_important) VALUES (?1, ?2, ?3)",
            ["test task", "0", "1"],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO sessions (task_id, start_time) VALUES (?1, datetime('now'))",
            [1],
        )
        .unwrap();

        let task_id: i64 = conn
            .query_row("SELECT task_id FROM sessions WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(task_id, 1);
    }
}
