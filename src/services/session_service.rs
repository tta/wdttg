use crate::db::Database;
use crate::models::{Session, SessionWithTask, Task, TaskStatus};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionServiceError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Session not found: {0}")]
    NotFound(i64),
    #[error("Task not found: {0}")]
    TaskNotFound(i64),
}

pub type Result<T> = std::result::Result<T, SessionServiceError>;

pub struct SessionService {
    db: Arc<Database>,
}

impl SessionService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn start(&self, task_id: i64) -> Result<Session> {
        let conn = self.db.connection();

        let tx = conn.unchecked_transaction()?;

        conn.execute(
            "UPDATE sessions SET end_time = datetime('now') WHERE end_time IS NULL",
            [],
        )?;

        conn.execute(
            "INSERT INTO sessions (task_id, start_time) VALUES (?1, datetime('now'))",
            params![task_id],
        )?;

        let id = conn.last_insert_rowid();

        tx.commit()?;

        drop(conn);

        self.get_by_id(id)?.ok_or(SessionServiceError::NotFound(id))
    }

    pub fn stop(&self) -> Result<Option<Session>> {
        let conn = self.db.connection();

        let active_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM sessions WHERE end_time IS NULL ORDER BY start_time DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;

        let Some(id) = active_id else {
            return Ok(None);
        };

        conn.execute(
            "UPDATE sessions SET end_time = datetime('now') WHERE id = ?1",
            params![id],
        )?;

        drop(conn);

        self.get_by_id(id)
    }

    pub fn get_active(&self) -> Result<Option<SessionWithTask>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.end_time IS NULL
             ORDER BY s.start_time DESC
             LIMIT 1",
            [],
            |row| self.row_to_session_with_task(row),
        );

        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn update(
        &self,
        id: i64,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let conn = self.db.connection();

        let start_str = start.map(|dt| format_sqlite_datetime(&dt));
        let end_str = end.map(|dt| format_sqlite_datetime(&dt));

        let rows_affected = conn.execute(
            "UPDATE sessions SET start_time = COALESCE(?1, start_time), end_time = COALESCE(?2, end_time) WHERE id = ?3",
            params![start_str, end_str, id],
        )?;

        if rows_affected == 0 {
            return Err(SessionServiceError::NotFound(id));
        }

        Ok(())
    }

    pub fn get_by_id(&self, id: i64) -> Result<Option<Session>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT id, task_id, start_time, end_time, notes, created_at FROM sessions WHERE id = ?1",
            params![id],
            |row| self.row_to_session(row),
        );

        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_day(&self, date: NaiveDate) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let start_of_day = format!("{} 00:00:00", date.format("%Y-%m-%d"));
        let end_of_day = format!("{} 23:59:59", date.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.start_time >= ?1 AND s.start_time <= ?2
             ORDER BY s.start_time ASC",
        )?;

        let sessions = stmt
            .query_map(params![start_of_day, end_of_day], |row| {
                self.row_to_session_with_task(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    pub fn get_recent(&self, limit: i64) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             ORDER BY s.start_time DESC
             LIMIT ?1",
        )?;

        let sessions = stmt
            .query_map(params![limit], |row| self.row_to_session_with_task(row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    pub fn get_last_for_task(&self, task_id: i64) -> Result<Option<Session>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT id, task_id, start_time, end_time, notes, created_at 
             FROM sessions 
             WHERE task_id = ?1 
             ORDER BY start_time DESC 
             LIMIT 1",
            params![task_id],
            |row| self.row_to_session(row),
        );

        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_last_session_task(&self) -> Result<Option<SessionWithTask>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             ORDER BY s.start_time DESC
             LIMIT 1",
            [],
            |row| self.row_to_session_with_task(row),
        );

        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn row_to_session(&self, row: &rusqlite::Row) -> std::result::Result<Session, rusqlite::Error> {
        let id: i64 = row.get(0)?;
        let task_id: i64 = row.get(1)?;
        let start_time_str: String = row.get(2)?;
        let end_time_str: Option<String> = row.get(3)?;
        let notes: Option<String> = row.get(4)?;
        let created_at_str: String = row.get(5)?;

        let start_time = parse_sqlite_datetime(&start_time_str)?;
        let end_time = end_time_str
            .map(|s| parse_sqlite_datetime(&s))
            .transpose()?;
        let created_at = parse_sqlite_datetime(&created_at_str)?;

        Ok(Session {
            id,
            task_id,
            start_time,
            end_time,
            notes,
            created_at,
        })
    }

    fn row_to_session_with_task(
        &self,
        row: &rusqlite::Row,
    ) -> std::result::Result<SessionWithTask, rusqlite::Error> {
        let session = self.row_to_session(row)?;

        let task_id: i64 = row.get(6)?;
        let task_name: String = row.get(7)?;
        let task_notes: Option<String> = row.get(8)?;
        let em_urgent: i32 = row.get(9)?;
        let em_important: i32 = row.get(10)?;
        let status_str: String = row.get(11)?;
        let recurring: i32 = row.get(12)?;
        let task_created_at_str: String = row.get(13)?;
        let task_updated_at_str: String = row.get(14)?;

        let status = TaskStatus::from_str(&status_str).map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                11,
                "status".to_string(),
                rusqlite::types::Type::Text,
            )
        })?;

        let task_created_at = parse_sqlite_datetime(&task_created_at_str)?;
        let task_updated_at = parse_sqlite_datetime(&task_updated_at_str)?;

        let task = Task {
            id: task_id,
            name: task_name,
            notes: task_notes,
            em_urgent: em_urgent != 0,
            em_important: em_important != 0,
            status,
            recurring: recurring != 0,
            tags: vec![],
            created_at: task_created_at,
            updated_at: task_updated_at,
        };

        Ok(SessionWithTask { session, task })
    }
}

fn format_sqlite_datetime(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn parse_sqlite_datetime(s: &str) -> std::result::Result<DateTime<Utc>, rusqlite::Error> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc())
        .map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                0,
                "datetime".to_string(),
                rusqlite::types::Type::Text,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::TaskService;

    fn create_test_services() -> (SessionService, TaskService) {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let db = Arc::new(db);
        (SessionService::new(Arc::clone(&db)), TaskService::new(db))
    }

    #[test]
    fn test_start_creates_active_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id).unwrap();

        assert!(session.end_time.is_none());
        assert_eq!(session.task_id, task.id);
    }

    #[test]
    fn test_start_stops_previous_session() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();

        let session1 = session_service.start(task1.id).unwrap();
        assert!(session1.end_time.is_none());

        let _session2 = session_service.start(task2.id).unwrap();

        let session1_after = session_service.get_by_id(session1.id).unwrap().unwrap();
        assert!(session1_after.end_time.is_some());
    }

    #[test]
    fn test_stop_ends_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        session_service.start(task.id).unwrap();

        let stopped = session_service.stop().unwrap();
        assert!(stopped.is_some());
        assert!(stopped.unwrap().end_time.is_some());
    }

    #[test]
    fn test_stop_no_active_session() {
        let (session_service, _) = create_test_services();

        let result = session_service.stop().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_active_returns_active_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        session_service.start(task.id).unwrap();

        let active = session_service.get_active().unwrap();
        assert!(active.is_some());

        let session_with_task = active.unwrap();
        assert!(session_with_task.session.end_time.is_none());
        assert_eq!(session_with_task.task.name, "test-task");
    }

    #[test]
    fn test_get_active_none_after_stop() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        session_service.start(task.id).unwrap();
        session_service.stop().unwrap();

        let active = session_service.get_active().unwrap();
        assert!(active.is_none());
    }

    #[test]
    fn test_multiple_starts_only_one_active() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();
        let task3 = task_service.get_or_create("task-3").unwrap();

        session_service.start(task1.id).unwrap();
        session_service.start(task2.id).unwrap();
        session_service.start(task3.id).unwrap();

        let active = session_service.get_active().unwrap().unwrap();
        assert_eq!(active.task.id, task3.id);
    }

    #[test]
    fn test_update_session_times() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id).unwrap();

        let new_start = Utc::now() - chrono::Duration::hours(2);
        let new_end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .update(session.id, Some(new_start), Some(new_end))
            .unwrap();

        let updated = session_service.get_by_id(session.id).unwrap().unwrap();
        assert!(updated.end_time.is_some());
    }

    #[test]
    fn test_get_day_returns_sessions() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        session_service.start(task.id).unwrap();
        session_service.stop().unwrap();

        let today = Utc::now().date_naive();
        let sessions = session_service.get_day(today).unwrap();

        assert!(!sessions.is_empty());
        assert_eq!(sessions[0].task.name, "test-task");
    }

    #[test]
    fn test_get_recent() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();

        session_service.start(task1.id).unwrap();
        session_service.stop().unwrap();

        session_service.start(task2.id).unwrap();
        session_service.stop().unwrap();

        let recent = session_service.get_recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].task.name, "task-2");
    }

    #[test]
    fn test_get_last_for_task() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();

        session_service.start(task.id).unwrap();
        session_service.stop().unwrap();

        session_service.start(task.id).unwrap();
        session_service.stop().unwrap();

        let last = session_service.get_last_for_task(task.id).unwrap();
        assert!(last.is_some());
    }

    #[test]
    fn test_get_last_session_task() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();

        session_service.start(task1.id).unwrap();
        session_service.stop().unwrap();

        session_service.start(task2.id).unwrap();
        session_service.stop().unwrap();

        let last = session_service.get_last_session_task().unwrap();
        assert!(last.is_some());
        let last = last.unwrap();
        assert_eq!(last.task.name, "task-2");
    }

    #[test]
    fn test_get_last_session_task_no_sessions() {
        let (session_service, _) = create_test_services();

        let last = session_service.get_last_session_task().unwrap();
        assert!(last.is_none());
    }
}
