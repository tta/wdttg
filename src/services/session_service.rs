use crate::db::Database;
use crate::models::{Session, SessionWithTask, Task, TaskStatus};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
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
    #[error("Invalid time range: end time must be after start time")]
    InvalidTimeRange,
}

pub type Result<T> = std::result::Result<T, SessionServiceError>;

#[derive(Debug, Clone)]
pub struct TimeBlock {
    pub session_id: Option<i64>,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub task: Option<Task>,
    pub quadrant: Option<crate::models::EmQuadrant>,
    pub link: Option<String>,
}

impl TimeBlock {
    pub fn duration_seconds(&self) -> Option<i64> {
        self.end.map(|end| (end - self.start).num_seconds())
    }

    pub fn is_gap(&self) -> bool {
        self.task.is_none()
    }
}

pub struct SessionService {
    db: Arc<Database>,
}

impl SessionService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn start(&self, task_id: i64, link: Option<&str>) -> Result<Session> {
        let conn = self.db.connection();

        let tx = conn.unchecked_transaction()?;

        conn.execute(
            "UPDATE sessions SET end_time = datetime('now') WHERE end_time IS NULL",
            [],
        )?;

        conn.execute(
            "INSERT INTO sessions (task_id, start_time, link) VALUES (?1, datetime('now'), ?2)",
            params![task_id, link],
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
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.created_at,
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

    pub fn update_full(
        &self,
        id: i64,
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
        notes: Option<String>,
        link: Option<String>,
    ) -> Result<Session> {
        if let Some(end_time) = end {
            if end_time <= start {
                return Err(SessionServiceError::InvalidTimeRange);
            }
        }

        let conn = self.db.connection();

        let start_str = format_sqlite_datetime(&start);
        let end_str = end.map(|dt| format_sqlite_datetime(&dt));

        let rows_affected = conn.execute(
            "UPDATE sessions SET start_time = ?1, end_time = ?2, notes = ?3, link = ?4 WHERE id = ?5",
            params![start_str, end_str, notes, link, id],
        )?;

        if rows_affected == 0 {
            return Err(SessionServiceError::NotFound(id));
        }

        drop(conn);

        self.get_by_id(id)?.ok_or(SessionServiceError::NotFound(id))
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        let conn = self.db.connection();

        let rows_affected = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;

        if rows_affected == 0 {
            return Err(SessionServiceError::NotFound(id));
        }

        Ok(())
    }

    pub fn get_by_id(&self, id: i64) -> Result<Option<Session>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT id, task_id, start_time, end_time, notes, link, is_background, created_at FROM sessions WHERE id = ?1",
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

        let start_of_day_local = date.and_hms_opt(0, 0, 0).unwrap();
        let end_of_day_local = date.and_hms_opt(23, 59, 59).unwrap();

        let local_tz = Local::now().timezone();
        let start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&start_of_day_local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap();
        let end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&end_of_day_local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap();

        let start_of_day = format_sqlite_datetime(&start_utc);
        let end_of_day = format_sqlite_datetime(&end_utc);

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.created_at,
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

    pub fn get_day_with_gaps(&self, date: NaiveDate) -> Result<Vec<TimeBlock>> {
        let sessions = self.get_day(date)?;

        if sessions.is_empty() {
            return Ok(vec![]);
        }

        let local_tz = Local::now().timezone();
        let start_of_day_local = date.and_hms_opt(0, 0, 0).unwrap();
        let end_of_day_local = date.and_hms_opt(23, 59, 59).unwrap();

        let start_of_day: DateTime<Utc> = local_tz
            .from_local_datetime(&start_of_day_local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap();
        let end_of_day: DateTime<Utc> = local_tz
            .from_local_datetime(&end_of_day_local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap();

        let mut blocks: Vec<TimeBlock> = Vec::new();
        let mut last_end: Option<DateTime<Utc>> = None;

        for swt in sessions {
            let session_end = swt.session.end_time.unwrap_or_else(|| {
                let now = Utc::now();
                if now > end_of_day {
                    end_of_day
                } else {
                    now
                }
            });

            if let Some(prev_end) = last_end {
                if swt.session.start_time > prev_end {
                    blocks.push(TimeBlock {
                        session_id: None,
                        start: prev_end,
                        end: Some(swt.session.start_time),
                        task: None,
                        quadrant: None,
                        link: None,
                    });
                }
            } else if swt.session.start_time > start_of_day {
                blocks.push(TimeBlock {
                    session_id: None,
                    start: start_of_day,
                    end: Some(swt.session.start_time),
                    task: None,
                    quadrant: None,
                    link: None,
                });
            }

            let quadrant = swt.task.em_quadrant();
            blocks.push(TimeBlock {
                session_id: Some(swt.session.id),
                start: swt.session.start_time,
                end: Some(session_end),
                task: Some(swt.task),
                quadrant: Some(quadrant),
                link: swt.session.link.clone(),
            });

            last_end = Some(session_end);
        }

        if let Some(prev_end) = last_end {
            if prev_end < end_of_day {
                blocks.push(TimeBlock {
                    session_id: None,
                    start: prev_end,
                    end: Some(end_of_day),
                    task: None,
                    quadrant: None,
                    link: None,
                });
            }
        }

        Ok(blocks)
    }

    pub fn get_recent(&self, limit: i64) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.created_at,
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
            "SELECT id, task_id, start_time, end_time, notes, link, is_background, created_at 
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
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.created_at,
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

    pub fn log_session(
        &self,
        task_id: i64,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        link: Option<&str>,
    ) -> Result<Session> {
        let conn = self.db.connection();

        let start_str = format_sqlite_datetime(&start);
        let end_str = format_sqlite_datetime(&end);

        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, link) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, start_str, end_str, link],
        )?;

        let id = conn.last_insert_rowid();

        drop(conn);

        self.get_by_id(id)?.ok_or(SessionServiceError::NotFound(id))
    }

    pub fn get_date_range(&self, start: NaiveDate, end: NaiveDate) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let start_str = format!("{} 00:00:00", start.format("%Y-%m-%d"));
        let end_str = format!("{} 23:59:59", end.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.start_time >= ?1 AND s.start_time <= ?2 AND s.end_time IS NOT NULL
             ORDER BY s.start_time ASC",
        )?;

        let sessions = stmt
            .query_map(params![start_str, end_str], |row| {
                self.row_to_session_with_task(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    pub fn get_quadrant_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::QuadrantTime>> {
        use crate::models::EmQuadrant;

        let conn = self.db.connection();

        let start_str = format!("{} 00:00:00", start.format("%Y-%m-%d"));
        let end_str = format!("{} 23:59:59", end.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT t.em_urgent, t.em_important, SUM(strftime('%s', s.end_time) - strftime('%s', s.start_time)) as total_seconds
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.start_time >= ?1 AND s.start_time <= ?2 AND s.end_time IS NOT NULL
             GROUP BY t.em_urgent, t.em_important",
        )?;

        let results = stmt
            .query_map(params![start_str, end_str], |row| {
                let em_urgent: i32 = row.get(0)?;
                let em_important: i32 = row.get(1)?;
                let total_seconds: i64 = row.get(2)?;

                let quadrant = EmQuadrant::new(em_urgent != 0, em_important != 0);
                Ok(crate::models::QuadrantTime {
                    quadrant,
                    total_seconds,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(results)
    }

    pub fn get_task_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::TaskTime>> {
        let conn = self.db.connection();

        let start_str = format!("{} 00:00:00", start.format("%Y-%m-%d"));
        let end_str = format!("{} 23:59:59", end.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT t.name, SUM(strftime('%s', s.end_time) - strftime('%s', s.start_time)) as total_seconds, COUNT(s.id) as session_count, t.em_urgent, t.em_important
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.start_time >= ?1 AND s.start_time <= ?2 AND s.end_time IS NOT NULL
             GROUP BY t.id, t.name, t.em_urgent, t.em_important
             ORDER BY total_seconds DESC",
        )?;

        let results = stmt
            .query_map(params![start_str, end_str], |row| {
                let em_urgent: i32 = row.get(3)?;
                let em_important: i32 = row.get(4)?;
                Ok(crate::models::TaskTime {
                    task_name: row.get(0)?,
                    total_seconds: row.get(1)?,
                    session_count: row.get(2)?,
                    em_quadrant: crate::models::EmQuadrant::new(em_urgent != 0, em_important != 0),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(results)
    }

    pub fn get_tag_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::TagTime>> {
        let conn = self.db.connection();

        let start_str = format!("{} 00:00:00", start.format("%Y-%m-%d"));
        let end_str = format!("{} 23:59:59", end.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT tg.name, SUM(strftime('%s', s.end_time) - strftime('%s', s.start_time)) as total_seconds
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             INNER JOIN task_tags tt ON t.id = tt.task_id
             INNER JOIN tags tg ON tt.tag_id = tg.id
             WHERE s.start_time >= ?1 AND s.start_time <= ?2 AND s.end_time IS NOT NULL
             GROUP BY tg.id, tg.name
             ORDER BY total_seconds DESC",
        )?;

        let results = stmt
            .query_map(params![start_str, end_str], |row| {
                Ok(crate::models::TagTime {
                    tag: row.get(0)?,
                    total_seconds: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(results)
    }

    pub fn get_tags_with_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::TagWithTime>> {
        let conn = self.db.connection();

        let start_str = format!("{} 00:00:00", start.format("%Y-%m-%d"));
        let end_str = format!("{} 23:59:59", end.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT tg.name, SUM(strftime('%s', s.end_time) - strftime('%s', s.start_time)) as total_seconds, COUNT(s.id) as session_count
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             INNER JOIN task_tags tt ON t.id = tt.task_id
             INNER JOIN tags tg ON tt.tag_id = tg.id
             WHERE s.start_time >= ?1 AND s.start_time <= ?2 AND s.end_time IS NOT NULL
             GROUP BY tg.id, tg.name
             ORDER BY total_seconds DESC",
        )?;

        let results = stmt
            .query_map(params![start_str, end_str], |row| {
                Ok(crate::models::TagWithTime {
                    tag: row.get(0)?,
                    total_seconds: row.get(1)?,
                    session_count: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(results)
    }

    pub fn get_sessions_by_tag(
        &self,
        tag: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let start_str = format!("{} 00:00:00", start.format("%Y-%m-%d"));
        let end_str = format!("{} 23:59:59", end.format("%Y-%m-%d"));

        let mut stmt = conn.prepare(
            "SELECT DISTINCT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             INNER JOIN task_tags tt ON t.id = tt.task_id
             INNER JOIN tags tg ON tt.tag_id = tg.id
             WHERE tg.name = ?1 AND s.start_time >= ?2 AND s.start_time <= ?3 AND s.end_time IS NOT NULL
             ORDER BY s.start_time DESC",
        )?;

        let sessions = stmt
            .query_map(params![tag, start_str, end_str], |row| {
                self.row_to_session_with_task(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    fn row_to_session(&self, row: &rusqlite::Row) -> std::result::Result<Session, rusqlite::Error> {
        let id: i64 = row.get(0)?;
        let task_id: i64 = row.get(1)?;
        let start_time_str: String = row.get(2)?;
        let end_time_str: Option<String> = row.get(3)?;
        let notes: Option<String> = row.get(4)?;
        let link: Option<String> = row.get(5)?;
        let is_background: i32 = row.get(6)?;
        let created_at_str: String = row.get(7)?;

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
            link,
            is_background: is_background != 0,
            created_at,
        })
    }

    fn row_to_session_with_task(
        &self,
        row: &rusqlite::Row,
    ) -> std::result::Result<SessionWithTask, rusqlite::Error> {
        let session = self.row_to_session(row)?;

        let task_id: i64 = row.get(8)?;
        let task_name: String = row.get(9)?;
        let task_notes: Option<String> = row.get(10)?;
        let em_urgent: i32 = row.get(11)?;
        let em_important: i32 = row.get(12)?;
        let status_str: String = row.get(13)?;
        let recurring: i32 = row.get(14)?;
        let task_created_at_str: String = row.get(15)?;
        let task_updated_at_str: String = row.get(16)?;

        let status: TaskStatus = status_str.parse().map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                13,
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
    use chrono::Timelike;

    fn create_test_services() -> (SessionService, TaskService) {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let db = Arc::new(db);
        (SessionService::new(Arc::clone(&db)), TaskService::new(db))
    }

    #[test]
    fn test_start_creates_active_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id, None).unwrap();

        assert!(session.end_time.is_none());
        assert_eq!(session.task_id, task.id);
    }

    #[test]
    fn test_start_stops_previous_session() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();

        let session1 = session_service.start(task1.id, None).unwrap();
        assert!(session1.end_time.is_none());

        let _session2 = session_service.start(task2.id, None).unwrap();

        let session1_after = session_service.get_by_id(session1.id).unwrap().unwrap();
        assert!(session1_after.end_time.is_some());
    }

    #[test]
    fn test_stop_ends_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        session_service.start(task.id, None).unwrap();

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
        session_service.start(task.id, None).unwrap();

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
        session_service.start(task.id, None).unwrap();
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

        session_service.start(task1.id, None).unwrap();
        session_service.start(task2.id, None).unwrap();
        session_service.start(task3.id, None).unwrap();

        let active = session_service.get_active().unwrap().unwrap();
        assert_eq!(active.task.id, task3.id);
    }

    #[test]
    fn test_update_session_times() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id, None).unwrap();

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
        session_service.start(task.id, None).unwrap();
        session_service.stop().unwrap();

        let today = Local::now().date_naive();
        let sessions = session_service.get_day(today).unwrap();

        assert!(!sessions.is_empty());
        assert_eq!(sessions[0].task.name, "test-task");
    }

    #[test]
    fn test_get_recent() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();

        session_service.start(task1.id, None).unwrap();
        session_service.stop().unwrap();

        session_service.start(task2.id, None).unwrap();
        session_service.stop().unwrap();

        let recent = session_service.get_recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].task.name, "task-2");
    }

    #[test]
    fn test_get_last_for_task() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();

        session_service.start(task.id, None).unwrap();
        session_service.stop().unwrap();

        session_service.start(task.id, None).unwrap();
        session_service.stop().unwrap();

        let last = session_service.get_last_for_task(task.id).unwrap();
        assert!(last.is_some());
    }

    #[test]
    fn test_get_last_session_task() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("task-1").unwrap();
        let task2 = task_service.get_or_create("task-2").unwrap();

        session_service.start(task1.id, None).unwrap();
        session_service.stop().unwrap();

        session_service.start(task2.id, None).unwrap();
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

    #[test]
    fn test_log_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        let session = session_service
            .log_session(task.id, start, end, None)
            .unwrap();

        assert_eq!(session.task_id, task.id);
        assert!(session.end_time.is_some());
        assert_eq!(session.duration_minutes().unwrap(), 60);
    }

    #[test]
    fn test_get_date_range() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .log_session(task.id, start, end, None)
            .unwrap();

        let date = start.date_naive();
        let sessions = session_service.get_date_range(date, date).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].task.name, "test-task");
    }

    #[test]
    fn test_get_quadrant_time() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        task_service.set_em_quadrant(task.id, true, true).unwrap();

        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .log_session(task.id, start, end, None)
            .unwrap();

        let date = start.date_naive();
        let quadrant_times = session_service.get_quadrant_time(date, date).unwrap();

        assert_eq!(quadrant_times.len(), 1);
        assert_eq!(quadrant_times[0].total_seconds, 3600);
    }

    #[test]
    fn test_get_task_time() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();

        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .log_session(task.id, start, end, None)
            .unwrap();

        let date = start.date_naive();
        let task_times = session_service.get_task_time(date, date).unwrap();

        assert_eq!(task_times.len(), 1);
        assert_eq!(task_times[0].task_name, "test-task");
        assert_eq!(task_times[0].total_seconds, 3600);
        assert_eq!(task_times[0].session_count, 1);
    }

    #[test]
    fn test_get_tag_time() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("coding/feature").unwrap();

        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .log_session(task.id, start, end, None)
            .unwrap();

        let date = start.date_naive();
        let tag_times = session_service.get_tag_time(date, date).unwrap();

        assert!(!tag_times.is_empty());
        let tag_names: Vec<&str> = tag_times.iter().map(|t| t.tag.as_str()).collect();
        assert!(tag_names.contains(&"coding"));
        assert!(tag_names.contains(&"feature"));
    }

    #[test]
    fn test_get_tags_with_time() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("coding/feature").unwrap();

        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .log_session(task.id, start, end, None)
            .unwrap();

        let date = start.date_naive();
        let tags_with_time = session_service.get_tags_with_time(date, date).unwrap();

        assert!(!tags_with_time.is_empty());
        let coding_tag = tags_with_time.iter().find(|t| t.tag == "coding");
        assert!(coding_tag.is_some());
        let coding_tag = coding_tag.unwrap();
        assert_eq!(coding_tag.total_seconds, 3600);
        assert_eq!(coding_tag.session_count, 1);
    }

    #[test]
    fn test_get_sessions_by_tag() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("coding/feature").unwrap();
        let task2 = task_service.get_or_create("meeting/standup").unwrap();

        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        session_service
            .log_session(task1.id, start, end, None)
            .unwrap();

        let start2 = Utc::now() - chrono::Duration::hours(4);
        let end2 = Utc::now() - chrono::Duration::hours(3);
        session_service
            .log_session(task2.id, start2, end2, None)
            .unwrap();

        let date = start.date_naive();
        let coding_sessions = session_service
            .get_sessions_by_tag("coding", date, date)
            .unwrap();

        assert_eq!(coding_sessions.len(), 1);
        assert!(coding_sessions[0].task.name.contains("coding"));

        let meeting_sessions = session_service
            .get_sessions_by_tag("meeting", date, date)
            .unwrap();
        assert_eq!(meeting_sessions.len(), 1);
        assert!(meeting_sessions[0].task.name.contains("meeting"));
    }

    #[test]
    fn test_update_full() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id, None).unwrap();

        let new_start = (Utc::now() - chrono::Duration::hours(3))
            .with_nanosecond(0)
            .unwrap();
        let new_end = (Utc::now() - chrono::Duration::hours(1))
            .with_nanosecond(0)
            .unwrap();

        let updated = session_service
            .update_full(
                session.id,
                new_start,
                Some(new_end),
                Some("Test notes".to_string()),
                None,
            )
            .unwrap();

        assert_eq!(updated.start_time, new_start);
        assert_eq!(updated.end_time, Some(new_end));
        assert_eq!(updated.notes, Some("Test notes".to_string()));
    }

    #[test]
    fn test_update_full_with_active_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id, None).unwrap();

        let new_start = (Utc::now() - chrono::Duration::hours(1))
            .with_nanosecond(0)
            .unwrap();

        let updated = session_service
            .update_full(session.id, new_start, None, None, None)
            .unwrap();

        assert_eq!(updated.start_time, new_start);
        assert!(updated.end_time.is_none());
        assert!(updated.notes.is_none());
    }

    #[test]
    fn test_update_full_invalid_time_range() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id, None).unwrap();

        let start = Utc::now();
        let end = start - chrono::Duration::hours(1);

        let result = session_service.update_full(session.id, start, Some(end), None, None);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionServiceError::InvalidTimeRange
        ));
    }

    #[test]
    fn test_update_full_not_found() {
        let (session_service, _) = create_test_services();

        let result = session_service.update_full(9999, Utc::now(), None, None, None);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionServiceError::NotFound(9999)
        ));
    }

    #[test]
    fn test_delete_session() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("test-task").unwrap();
        let session = session_service.start(task.id, None).unwrap();

        let before_delete = session_service.get_by_id(session.id).unwrap();
        assert!(before_delete.is_some());

        session_service.delete(session.id).unwrap();

        let after_delete = session_service.get_by_id(session.id).unwrap();
        assert!(after_delete.is_none());
    }

    #[test]
    fn test_delete_session_not_found() {
        let (session_service, _) = create_test_services();

        let result = session_service.delete(9999);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionServiceError::NotFound(9999)
        ));
    }
}
