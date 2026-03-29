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
    #[error("A background task is already running. Stop it first with 'wdttg stop --bg'")]
    BackgroundAlreadyRunning,
    #[error("A foreground task is already running. Stop it first with 'wdttg stop'")]
    ForegroundAlreadyRunning,
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
    pub is_background: bool,
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

        let (em_urgent, em_important): (i32, i32) = conn
            .query_row(
                "SELECT em_urgent, em_important FROM tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| SessionServiceError::TaskNotFound(task_id))?;

        conn.execute(
            "UPDATE sessions SET end_time = datetime('now') WHERE end_time IS NULL AND is_background = 0",
            [],
        )?;

        conn.execute(
            "INSERT INTO sessions (task_id, start_time, link, em_urgent, em_important) VALUES (?1, datetime('now'), ?2, ?3, ?4)",
            params![task_id, link, em_urgent, em_important],
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
                "SELECT id FROM sessions WHERE end_time IS NULL AND is_background = 0 ORDER BY start_time DESC LIMIT 1",
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
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
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

    pub fn get_active_foreground(&self) -> Result<Option<SessionWithTask>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.end_time IS NULL AND s.is_background = 0
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

    pub fn get_active_background(&self) -> Result<Option<SessionWithTask>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.end_time IS NULL AND s.is_background = 1
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

    pub fn start_background(&self, task_id: i64, link: Option<&str>) -> Result<Session> {
        let conn = self.db.connection();

        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM sessions WHERE end_time IS NULL AND is_background = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;

        if existing.is_some() {
            return Err(SessionServiceError::BackgroundAlreadyRunning);
        }

        let (em_urgent, em_important): (i32, i32) = conn
            .query_row(
                "SELECT em_urgent, em_important FROM tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| SessionServiceError::TaskNotFound(task_id))?;

        conn.execute(
            "INSERT INTO sessions (task_id, start_time, link, is_background, em_urgent, em_important) VALUES (?1, datetime('now'), ?2, 1, ?3, ?4)",
            params![task_id, link, em_urgent, em_important],
        )?;

        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_by_id(id)?.ok_or(SessionServiceError::NotFound(id))
    }

    pub fn stop_background(&self) -> Result<Option<Session>> {
        let conn = self.db.connection();

        let active_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM sessions WHERE end_time IS NULL AND is_background = 1 ORDER BY start_time DESC LIMIT 1",
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

    pub fn stop_all(&self) -> Result<(Option<Session>, Option<Session>)> {
        let fg = self.stop()?;
        let bg = self.stop_background()?;
        Ok((fg, bg))
    }

    pub fn get_overlapping_sessions(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let start_str = format_sqlite_datetime(&start);
        let end_str = format_sqlite_datetime(&end);

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.start_time < ?2 
               AND (s.end_time IS NULL OR s.end_time > ?1)
             ORDER BY s.start_time ASC",
        )?;

        let sessions = stmt
            .query_map(params![start_str, end_str], |row| {
                self.row_to_session_with_task(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    pub fn get_split_times(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::SessionSplitTime>> {
        use crate::models::SessionSplitTime;
        use std::collections::HashMap;

        let sessions = self.get_date_range(start, end)?;

        if sessions.is_empty() {
            return Ok(vec![]);
        }

        let mut results: HashMap<i64, SessionSplitTime> = HashMap::new();

        let foreground: Vec<_> = sessions
            .iter()
            .filter(|s| !s.session.is_background)
            .collect();
        let background: Vec<_> = sessions
            .iter()
            .filter(|s| s.session.is_background)
            .collect();

        for swt in &foreground {
            let total = swt.session.duration_seconds().unwrap_or(0);
            let mut overlap_total = 0i64;

            for bg in &background {
                if let Some(overlap) = Self::calculate_overlap(&swt.session, &bg.session) {
                    overlap_total += overlap;
                }
            }

            let split = total - overlap_total / 2;
            results.insert(
                swt.session.id,
                SessionSplitTime {
                    session_id: swt.session.id,
                    task_id: swt.task.id,
                    task_name: swt.task.name.clone(),
                    em_quadrant: swt.session.em_quadrant(),
                    total_seconds: total,
                    split_seconds: split,
                    overlap_seconds: overlap_total,
                },
            );
        }

        for bg in &background {
            let total = bg.session.duration_seconds().unwrap_or(0);
            let mut overlap_total = 0i64;

            for fg in &foreground {
                if let Some(overlap) = Self::calculate_overlap(&fg.session, &bg.session) {
                    overlap_total += overlap;
                }
            }

            let split = (total - overlap_total) + overlap_total / 2;
            results.insert(
                bg.session.id,
                SessionSplitTime {
                    session_id: bg.session.id,
                    task_id: bg.task.id,
                    task_name: bg.task.name.clone(),
                    em_quadrant: bg.session.em_quadrant(),
                    total_seconds: total,
                    split_seconds: split,
                    overlap_seconds: overlap_total,
                },
            );
        }

        Ok(results.into_values().collect())
    }

    fn calculate_overlap(a: &Session, b: &Session) -> Option<i64> {
        let a_end = a.end_time?;
        let b_end = b.end_time?;

        let overlap_start = a.start_time.max(b.start_time);
        let overlap_end = a_end.min(b_end);

        if overlap_start < overlap_end {
            Some((overlap_end - overlap_start).num_seconds())
        } else {
            None
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
        is_background: Option<bool>,
        em_quadrant: Option<crate::models::EmQuadrant>,
    ) -> Result<Session> {
        if let Some(end_time) = end {
            if end_time <= start {
                return Err(SessionServiceError::InvalidTimeRange);
            }
        }

        if let Some(new_bg) = is_background {
            let current = self
                .get_by_id(id)?
                .ok_or(SessionServiceError::NotFound(id))?;

            if new_bg != current.is_background && current.end_time.is_none() {
                if new_bg {
                    let existing = self.get_active_background()?;
                    if let Some(bg) = existing {
                        if bg.session.id != id {
                            return Err(SessionServiceError::BackgroundAlreadyRunning);
                        }
                    }
                } else {
                    let existing = self.get_active_foreground()?;
                    if let Some(fg) = existing {
                        if fg.session.id != id {
                            return Err(SessionServiceError::ForegroundAlreadyRunning);
                        }
                    }
                }
            }
        }

        let conn = self.db.connection();

        let start_str = format_sqlite_datetime(&start);
        let end_str = end.map(|dt| format_sqlite_datetime(&dt));

        let rows_affected = match (is_background, em_quadrant) {
            (Some(bg), Some(em)) => {
                conn.execute(
                    "UPDATE sessions SET start_time = ?1, end_time = ?2, notes = ?3, link = ?4, is_background = ?5, em_urgent = ?6, em_important = ?7 WHERE id = ?8",
                    params![start_str, end_str, notes, link, bg as i32, em.is_urgent() as i32, em.is_important() as i32, id],
                )?
            }
            (Some(bg), None) => {
                conn.execute(
                    "UPDATE sessions SET start_time = ?1, end_time = ?2, notes = ?3, link = ?4, is_background = ?5 WHERE id = ?6",
                    params![start_str, end_str, notes, link, bg as i32, id],
                )?
            }
            (None, Some(em)) => {
                conn.execute(
                    "UPDATE sessions SET start_time = ?1, end_time = ?2, notes = ?3, link = ?4, em_urgent = ?5, em_important = ?6 WHERE id = ?7",
                    params![start_str, end_str, notes, link, em.is_urgent() as i32, em.is_important() as i32, id],
                )?
            }
            (None, None) => {
                conn.execute(
                    "UPDATE sessions SET start_time = ?1, end_time = ?2, notes = ?3, link = ?4 WHERE id = ?5",
                    params![start_str, end_str, notes, link, id],
                )?
            }
        };

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
            "SELECT id, task_id, start_time, end_time, notes, link, is_background, em_urgent, em_important, created_at FROM sessions WHERE id = ?1",
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
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
                    t.id, t.name, t.notes, t.em_urgent, t.em_important, t.status, t.recurring, t.created_at, t.updated_at
             FROM sessions s
             INNER JOIN tasks t ON s.task_id = t.id
             WHERE s.start_time <= ?2 AND (s.end_time IS NULL OR s.end_time >= ?1)
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

            let clamped_start = swt.session.start_time.max(start_of_day);
            let clamped_end = session_end.min(end_of_day);

            if clamped_start >= clamped_end {
                continue;
            }

            if let Some(prev_end) = last_end {
                if clamped_start > prev_end {
                    blocks.push(TimeBlock {
                        session_id: None,
                        start: prev_end,
                        end: Some(clamped_start),
                        task: None,
                        quadrant: None,
                        link: None,
                        is_background: false,
                    });
                }
            } else if clamped_start > start_of_day {
                blocks.push(TimeBlock {
                    session_id: None,
                    start: start_of_day,
                    end: Some(clamped_start),
                    task: None,
                    quadrant: None,
                    link: None,
                    is_background: false,
                });
            }

            let quadrant = swt.session.em_quadrant();
            blocks.push(TimeBlock {
                session_id: Some(swt.session.id),
                start: clamped_start,
                end: Some(clamped_end),
                task: Some(swt.task),
                quadrant: Some(quadrant),
                link: swt.session.link.clone(),
                is_background: swt.session.is_background,
            });

            last_end = Some(clamped_end);
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
                    is_background: false,
                });
            }
        }

        Ok(blocks)
    }

    pub fn get_recent(&self, limit: i64) -> Result<Vec<SessionWithTask>> {
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
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
            "SELECT id, task_id, start_time, end_time, notes, link, is_background, em_urgent, em_important, created_at 
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
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
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

        let (em_urgent, em_important): (i32, i32) = conn
            .query_row(
                "SELECT em_urgent, em_important FROM tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| SessionServiceError::TaskNotFound(task_id))?;

        let start_str = format_sqlite_datetime(&start);
        let end_str = format_sqlite_datetime(&end);

        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, link, em_urgent, em_important) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![task_id, start_str, end_str, link, em_urgent, em_important],
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
            "SELECT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
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
        use crate::models::QuadrantTime;
        use std::collections::HashMap;

        let split_times = self.get_split_times(start, end)?;

        let mut by_quadrant: HashMap<crate::models::EmQuadrant, i64> = HashMap::new();
        for st in split_times {
            *by_quadrant.entry(st.em_quadrant).or_insert(0) += st.split_seconds;
        }

        let results: Vec<QuadrantTime> = by_quadrant
            .into_iter()
            .map(|(quadrant, total_seconds)| QuadrantTime {
                quadrant,
                total_seconds,
            })
            .collect();

        Ok(results)
    }

    pub fn get_task_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::TaskTime>> {
        use crate::models::TaskTime;
        use std::collections::HashMap;

        let split_times = self.get_split_times(start, end)?;

        let mut by_task_em: HashMap<(i64, crate::models::EmQuadrant), (String, i64, i64)> =
            HashMap::new();
        let mut task_totals: HashMap<i64, i64> = HashMap::new();
        for st in &split_times {
            let entry = by_task_em.entry((st.task_id, st.em_quadrant)).or_insert((
                st.task_name.clone(),
                0,
                0,
            ));
            entry.1 += st.split_seconds;
            entry.2 += 1;
            *task_totals.entry(st.task_id).or_insert(0) += st.split_seconds;
        }

        let mut results: Vec<TaskTime> = by_task_em
            .into_iter()
            .map(
                |((task_id, em_quadrant), (task_name, total_seconds, session_count))| TaskTime {
                    task_id,
                    task_name,
                    total_seconds,
                    session_count,
                    em_quadrant,
                },
            )
            .collect();

        results.sort_by(|a, b| {
            let total_a = task_totals.get(&a.task_id).copied().unwrap_or(0);
            let total_b = task_totals.get(&b.task_id).copied().unwrap_or(0);
            match total_b.cmp(&total_a) {
                std::cmp::Ordering::Equal => b.total_seconds.cmp(&a.total_seconds),
                other => other,
            }
        });
        Ok(results)
    }

    pub fn get_tag_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::TagTime>> {
        use std::collections::HashMap;

        let split_times = self.get_split_times(start, end)?;

        if split_times.is_empty() {
            return Ok(vec![]);
        }

        let conn = self.db.connection();
        let mut task_tags: HashMap<i64, Vec<String>> = HashMap::new();

        let mut stmt = conn.prepare(
            "SELECT tt.task_id, tg.name
             FROM task_tags tt
             INNER JOIN tags tg ON tt.tag_id = tg.id",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (task_id, tag_name) = row?;
            task_tags.entry(task_id).or_default().push(tag_name);
        }

        let mut by_tag: HashMap<String, i64> = HashMap::new();
        for st in &split_times {
            if let Some(tags) = task_tags.get(&st.task_id) {
                for tag in tags {
                    *by_tag.entry(tag.clone()).or_insert(0) += st.split_seconds;
                }
            }
        }

        let mut results: Vec<crate::models::TagTime> = by_tag
            .into_iter()
            .map(|(tag, total_seconds)| crate::models::TagTime { tag, total_seconds })
            .collect();

        results.sort_by(|a, b| b.total_seconds.cmp(&a.total_seconds));
        Ok(results)
    }

    pub fn get_tags_with_time(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<crate::models::TagWithTime>> {
        use std::collections::HashMap;

        let split_times = self.get_split_times(start, end)?;

        if split_times.is_empty() {
            return Ok(vec![]);
        }

        let conn = self.db.connection();
        let mut task_tags: HashMap<i64, Vec<String>> = HashMap::new();

        let mut stmt = conn.prepare(
            "SELECT tt.task_id, tg.name
             FROM task_tags tt
             INNER JOIN tags tg ON tt.tag_id = tg.id",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (task_id, tag_name) = row?;
            task_tags.entry(task_id).or_default().push(tag_name);
        }

        let mut by_tag: HashMap<String, (i64, i64)> = HashMap::new();
        for st in &split_times {
            if let Some(tags) = task_tags.get(&st.task_id) {
                for tag in tags {
                    let entry = by_tag.entry(tag.clone()).or_insert((0, 0));
                    entry.0 += st.split_seconds;
                    entry.1 += 1;
                }
            }
        }

        let mut results: Vec<crate::models::TagWithTime> = by_tag
            .into_iter()
            .map(
                |(tag, (total_seconds, session_count))| crate::models::TagWithTime {
                    tag,
                    total_seconds,
                    session_count,
                },
            )
            .collect();

        results.sort_by(|a, b| b.total_seconds.cmp(&a.total_seconds));
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
            "SELECT DISTINCT s.id, s.task_id, s.start_time, s.end_time, s.notes, s.link, s.is_background, s.em_urgent, s.em_important, s.created_at,
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
        let em_urgent: i32 = row.get(7)?;
        let em_important: i32 = row.get(8)?;
        let created_at_str: String = row.get(9)?;

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
            em_urgent: em_urgent != 0,
            em_important: em_important != 0,
            created_at,
        })
    }

    fn row_to_session_with_task(
        &self,
        row: &rusqlite::Row,
    ) -> std::result::Result<SessionWithTask, rusqlite::Error> {
        let session = self.row_to_session(row)?;

        let task_id: i64 = row.get(10)?;
        let task_name: String = row.get(11)?;
        let task_notes: Option<String> = row.get(12)?;
        let em_urgent: i32 = row.get(13)?;
        let em_important: i32 = row.get(14)?;
        let status_str: String = row.get(15)?;
        let recurring: i32 = row.get(16)?;
        let task_created_at_str: String = row.get(17)?;
        let task_updated_at_str: String = row.get(18)?;

        let status: TaskStatus = status_str.parse().map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                15,
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
    fn test_get_tag_time_with_split_times() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();

        let fg_task = task_service.get_or_create("coding/feature").unwrap();
        let bg_task = task_service.get_or_create("meeting/standup").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let fg_start = base + chrono::Duration::hours(1);
        let fg_end = base + chrono::Duration::hours(2);
        let bg_start = base;
        let bg_end = base + chrono::Duration::hours(3);

        let conn = session_service.db.connection();
        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, is_background, em_urgent, em_important) VALUES (?1, ?2, ?3, 0, 0, 0)",
            params![fg_task.id, format_sqlite_datetime(&fg_start), format_sqlite_datetime(&fg_end)],
        ).unwrap();
        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, is_background, em_urgent, em_important) VALUES (?1, ?2, ?3, 1, 0, 0)",
            params![bg_task.id, format_sqlite_datetime(&bg_start), format_sqlite_datetime(&bg_end)],
        ).unwrap();
        drop(conn);

        let date = base.date_naive();
        let tag_times = session_service.get_tag_time(date, date).unwrap();

        assert!(!tag_times.is_empty());

        let coding_tag = tag_times.iter().find(|t| t.tag == "coding");
        assert!(coding_tag.is_some());
        let coding_time = coding_tag.unwrap().total_seconds;
        assert_eq!(coding_time, 1800);

        let meeting_tag = tag_times.iter().find(|t| t.tag == "meeting");
        assert!(meeting_tag.is_some());
        let meeting_time = meeting_tag.unwrap().total_seconds;
        assert_eq!(meeting_time, 9000);
    }

    #[test]
    fn test_get_tags_with_time_with_split_times() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();

        let fg_task = task_service.get_or_create("coding/feature").unwrap();
        let bg_task = task_service.get_or_create("meeting/standup").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let fg_start = base + chrono::Duration::hours(1);
        let fg_end = base + chrono::Duration::hours(2);
        let bg_start = base;
        let bg_end = base + chrono::Duration::hours(3);

        let conn = session_service.db.connection();
        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, is_background, em_urgent, em_important) VALUES (?1, ?2, ?3, 0, 0, 0)",
            params![fg_task.id, format_sqlite_datetime(&fg_start), format_sqlite_datetime(&fg_end)],
        ).unwrap();
        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, is_background, em_urgent, em_important) VALUES (?1, ?2, ?3, 1, 0, 0)",
            params![bg_task.id, format_sqlite_datetime(&bg_start), format_sqlite_datetime(&bg_end)],
        ).unwrap();
        drop(conn);

        let date = base.date_naive();
        let tags_with_time = session_service.get_tags_with_time(date, date).unwrap();

        assert!(!tags_with_time.is_empty());

        let coding_tag = tags_with_time.iter().find(|t| t.tag == "coding");
        assert!(coding_tag.is_some());
        let coding = coding_tag.unwrap();
        assert_eq!(coding.total_seconds, 1800);
        assert_eq!(coding.session_count, 1);

        let meeting_tag = tags_with_time.iter().find(|t| t.tag == "meeting");
        assert!(meeting_tag.is_some());
        let meeting = meeting_tag.unwrap();
        assert_eq!(meeting.total_seconds, 9000);
        assert_eq!(meeting.session_count, 1);
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
                None,
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
            .update_full(session.id, new_start, None, None, None, None, None)
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

        let result =
            session_service.update_full(session.id, start, Some(end), None, None, None, None);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionServiceError::InvalidTimeRange
        ));
    }

    #[test]
    fn test_update_full_not_found() {
        let (session_service, _) = create_test_services();

        let result = session_service.update_full(9999, Utc::now(), None, None, None, None, None);

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

    #[test]
    fn test_start_background_succeeds_when_none_running() {
        let (session_service, task_service) = create_test_services();

        let task = task_service.get_or_create("bg-task").unwrap();
        let session = session_service.start_background(task.id, None).unwrap();

        assert!(session.end_time.is_none());
        assert!(session.is_background);
        assert_eq!(session.task_id, task.id);
    }

    #[test]
    fn test_start_background_fails_when_already_running() {
        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("bg-task-1").unwrap();
        let task2 = task_service.get_or_create("bg-task-2").unwrap();

        session_service.start_background(task1.id, None).unwrap();

        let result = session_service.start_background(task2.id, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionServiceError::BackgroundAlreadyRunning
        ));
    }

    #[test]
    fn test_start_foreground_does_not_stop_background() {
        let (session_service, task_service) = create_test_services();

        let bg_task = task_service.get_or_create("bg-task").unwrap();
        let fg_task = task_service.get_or_create("fg-task").unwrap();

        let bg_session = session_service.start_background(bg_task.id, None).unwrap();
        assert!(bg_session.end_time.is_none());

        let _fg_session = session_service.start(fg_task.id, None).unwrap();

        let bg_after = session_service.get_by_id(bg_session.id).unwrap().unwrap();
        assert!(bg_after.end_time.is_none());
    }

    #[test]
    fn test_get_active_foreground_and_background() {
        let (session_service, task_service) = create_test_services();

        let bg_task = task_service.get_or_create("bg-task").unwrap();
        let fg_task = task_service.get_or_create("fg-task").unwrap();

        session_service.start_background(bg_task.id, None).unwrap();
        session_service.start(fg_task.id, None).unwrap();

        let fg = session_service.get_active_foreground().unwrap();
        assert!(fg.is_some());
        assert_eq!(fg.unwrap().task.name, "fg-task");

        let bg = session_service.get_active_background().unwrap();
        assert!(bg.is_some());
        assert_eq!(bg.unwrap().task.name, "bg-task");
    }

    #[test]
    fn test_stop_background_only() {
        let (session_service, task_service) = create_test_services();

        let bg_task = task_service.get_or_create("bg-task").unwrap();
        let fg_task = task_service.get_or_create("fg-task").unwrap();

        session_service.start_background(bg_task.id, None).unwrap();
        session_service.start(fg_task.id, None).unwrap();

        let stopped = session_service.stop_background().unwrap();
        assert!(stopped.is_some());

        let bg_after = session_service.get_active_background().unwrap();
        assert!(bg_after.is_none());

        let fg_after = session_service.get_active_foreground().unwrap();
        assert!(fg_after.is_some());
    }

    #[test]
    fn test_stop_all() {
        let (session_service, task_service) = create_test_services();

        let bg_task = task_service.get_or_create("bg-task").unwrap();
        let fg_task = task_service.get_or_create("fg-task").unwrap();

        session_service.start_background(bg_task.id, None).unwrap();
        session_service.start(fg_task.id, None).unwrap();

        let (fg, bg) = session_service.stop_all().unwrap();
        assert!(fg.is_some());
        assert!(bg.is_some());

        assert!(session_service.get_active_foreground().unwrap().is_none());
        assert!(session_service.get_active_background().unwrap().is_none());
    }

    #[test]
    fn test_calculate_overlap_no_overlap() {
        use chrono::TimeZone;

        let task1 = crate::models::Task {
            id: 1,
            name: "task1".to_string(),
            notes: None,
            em_urgent: false,
            em_important: false,
            status: TaskStatus::Active,
            recurring: false,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let task2 = crate::models::Task {
            id: 2,
            name: "task2".to_string(),
            notes: None,
            em_urgent: false,
            em_important: false,
            status: TaskStatus::Active,
            recurring: false,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let session1 = Session {
            id: 1,
            task_id: task1.id,
            start_time: Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap(),
            end_time: Some(Utc.with_ymd_and_hms(2024, 1, 1, 11, 0, 0).unwrap()),
            notes: None,
            link: None,
            is_background: false,
            em_urgent: false,
            em_important: false,
            created_at: Utc::now(),
        };

        let session2 = Session {
            id: 2,
            task_id: task2.id,
            start_time: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            end_time: Some(Utc.with_ymd_and_hms(2024, 1, 1, 13, 0, 0).unwrap()),
            notes: None,
            link: None,
            is_background: true,
            em_urgent: false,
            em_important: false,
            created_at: Utc::now(),
        };

        let overlap = SessionService::calculate_overlap(&session1, &session2);
        assert!(overlap.is_none());
    }

    #[test]
    fn test_calculate_overlap_with_overlap() {
        use chrono::TimeZone;

        let session1 = Session {
            id: 1,
            task_id: 1,
            start_time: Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap(),
            end_time: Some(Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap()),
            notes: None,
            link: None,
            is_background: false,
            em_urgent: false,
            em_important: false,
            created_at: Utc::now(),
        };

        let session2 = Session {
            id: 2,
            task_id: 2,
            start_time: Utc.with_ymd_and_hms(2024, 1, 1, 9, 30, 0).unwrap(),
            end_time: Some(Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap()),
            notes: None,
            link: None,
            is_background: true,
            em_urgent: false,
            em_important: false,
            created_at: Utc::now(),
        };

        let overlap = SessionService::calculate_overlap(&session1, &session2);
        assert_eq!(overlap, Some(1800));
    }

    #[test]
    fn test_get_split_times_single_overlap() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();

        let bg_task = task_service.get_or_create("bg-task").unwrap();
        let fg_task = task_service.get_or_create("fg-task").unwrap();

        let bg_start = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let bg_end = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();

        let fg_start = Utc.with_ymd_and_hms(2024, 1, 1, 9, 30, 0).unwrap();
        let fg_end = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();

        let conn = session_service.db.connection();
        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, is_background, em_urgent, em_important) VALUES (?1, ?2, ?3, 1, 0, 0)",
            params![bg_task.id, format_sqlite_datetime(&bg_start), format_sqlite_datetime(&bg_end)],
        ).unwrap();
        conn.execute(
            "INSERT INTO sessions (task_id, start_time, end_time, is_background, em_urgent, em_important) VALUES (?1, ?2, ?3, 0, 0, 0)",
            params![fg_task.id, format_sqlite_datetime(&fg_start), format_sqlite_datetime(&fg_end)],
        ).unwrap();
        drop(conn);

        let date = bg_start.date_naive();
        let split_times = session_service.get_split_times(date, date).unwrap();

        assert_eq!(split_times.len(), 2);

        let fg_split = split_times
            .iter()
            .find(|s| s.task_name == "fg-task")
            .unwrap();
        assert_eq!(fg_split.total_seconds, 1800);
        assert_eq!(fg_split.overlap_seconds, 1800);
        assert_eq!(fg_split.split_seconds, 900);

        let bg_split = split_times
            .iter()
            .find(|s| s.task_name == "bg-task")
            .unwrap();
        assert_eq!(bg_split.total_seconds, 3600);
        assert_eq!(bg_split.overlap_seconds, 1800);
        assert_eq!(bg_split.split_seconds, 2700);
    }

    #[test]
    fn test_get_overlapping_sessions() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();

        let task1 = task_service.get_or_create("existing-task").unwrap();

        let existing_start = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let existing_end = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();

        session_service
            .log_session(task1.id, existing_start, existing_end, None)
            .unwrap();

        let new_start = Utc.with_ymd_and_hms(2024, 1, 1, 9, 30, 0).unwrap();
        let new_end = Utc.with_ymd_and_hms(2024, 1, 1, 10, 30, 0).unwrap();

        let overlaps = session_service
            .get_overlapping_sessions(new_start, new_end)
            .unwrap();

        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].task.name, "existing-task");
    }

    #[test]
    fn test_get_day_returns_session_spanning_previous_midnight() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();
        let local_tz = Local::now().timezone();

        let task = task_service.get_or_create("overnight-task").unwrap();

        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let start_local = yesterday.and_hms_opt(23, 0, 0).unwrap();
        let end_local = today.and_hms_opt(1, 0, 0).unwrap();

        let start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task.id, start_utc, end_utc, None)
            .unwrap();

        let sessions = session_service.get_day(today).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].task.name, "overnight-task");
    }

    #[test]
    fn test_get_day_returns_session_starting_today_spanning_midnight() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();
        let local_tz = Local::now().timezone();

        let task = task_service.get_or_create("overnight-task").unwrap();

        let today = Local::now().date_naive();
        let tomorrow = today + chrono::Duration::days(1);

        let start_local = today.and_hms_opt(23, 0, 0).unwrap();
        let end_local = tomorrow.and_hms_opt(1, 0, 0).unwrap();

        let start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task.id, start_utc, end_utc, None)
            .unwrap();

        let sessions = session_service.get_day(today).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].task.name, "overnight-task");
    }

    #[test]
    fn test_get_day_with_gaps_clamps_session_from_previous_day() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();
        let local_tz = Local::now().timezone();

        let task = task_service.get_or_create("overnight-task").unwrap();

        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let start_local = yesterday.and_hms_opt(23, 0, 0).unwrap();
        let end_local = today.and_hms_opt(1, 0, 0).unwrap();

        let start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task.id, start_utc, end_utc, None)
            .unwrap();

        let blocks = session_service.get_day_with_gaps(today).unwrap();

        let session_blocks: Vec<&TimeBlock> = blocks.iter().filter(|b| !b.is_gap()).collect();
        assert_eq!(session_blocks.len(), 1);

        let block = session_blocks[0];
        let local_start = Local.from_utc_datetime(&block.start.naive_utc());
        let local_end = Local.from_utc_datetime(&block.end.unwrap().naive_utc());

        assert_eq!(local_start.hour(), 0);
        assert_eq!(local_start.minute(), 0);
        assert_eq!(local_end.hour(), 1);
        assert_eq!(local_end.minute(), 0);
        assert_eq!(block.duration_seconds(), Some(3600));
    }

    #[test]
    fn test_get_day_with_gaps_clamps_session_spanning_to_next_day() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();
        let local_tz = Local::now().timezone();

        let task = task_service.get_or_create("overnight-task").unwrap();

        let today = Local::now().date_naive();
        let tomorrow = today + chrono::Duration::days(1);

        let start_local = today.and_hms_opt(23, 0, 0).unwrap();
        let end_local = tomorrow.and_hms_opt(1, 0, 0).unwrap();

        let start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task.id, start_utc, end_utc, None)
            .unwrap();

        let blocks = session_service.get_day_with_gaps(today).unwrap();

        let session_blocks: Vec<&TimeBlock> = blocks.iter().filter(|b| !b.is_gap()).collect();
        assert_eq!(session_blocks.len(), 1);

        let block = session_blocks[0];
        let local_start = Local.from_utc_datetime(&block.start.naive_utc());
        let local_end = Local.from_utc_datetime(&block.end.unwrap().naive_utc());

        assert_eq!(local_start.hour(), 23);
        assert_eq!(local_start.minute(), 0);
        assert_eq!(local_end.hour(), 23);
        assert_eq!(local_end.minute(), 59);
    }

    #[test]
    fn test_get_day_with_gaps_multi_day_session() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();
        let local_tz = Local::now().timezone();

        let task = task_service.get_or_create("multi-day-task").unwrap();

        let day_minus_1 = Local::now().date_naive() - chrono::Duration::days(1);
        let today = Local::now().date_naive();
        let day_plus_1 = today + chrono::Duration::days(1);

        let start_local = day_minus_1.and_hms_opt(23, 0, 0).unwrap();
        let end_local = day_plus_1.and_hms_opt(1, 0, 0).unwrap();

        let start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task.id, start_utc, end_utc, None)
            .unwrap();

        let blocks_prev = session_service.get_day_with_gaps(day_minus_1).unwrap();
        let session_blocks_prev: Vec<&TimeBlock> =
            blocks_prev.iter().filter(|b| !b.is_gap()).collect();
        assert_eq!(session_blocks_prev.len(), 1);
        let prev = session_blocks_prev[0];
        let prev_start = Local.from_utc_datetime(&prev.start.naive_utc());
        let prev_end = Local.from_utc_datetime(&prev.end.unwrap().naive_utc());
        assert_eq!(prev_start.hour(), 23);
        assert_eq!(prev_end.hour(), 23);
        assert_eq!(prev_end.minute(), 59);

        let blocks_today = session_service.get_day_with_gaps(today).unwrap();
        let session_blocks_today: Vec<&TimeBlock> =
            blocks_today.iter().filter(|b| !b.is_gap()).collect();
        assert_eq!(session_blocks_today.len(), 1);
        let cur = session_blocks_today[0];
        let cur_start = Local.from_utc_datetime(&cur.start.naive_utc());
        let cur_end = Local.from_utc_datetime(&cur.end.unwrap().naive_utc());
        assert_eq!(cur_start.hour(), 0);
        assert_eq!(cur_start.minute(), 0);
        assert_eq!(cur_end.hour(), 23);
        assert_eq!(cur_end.minute(), 59);

        let blocks_next = session_service.get_day_with_gaps(day_plus_1).unwrap();
        let session_blocks_next: Vec<&TimeBlock> =
            blocks_next.iter().filter(|b| !b.is_gap()).collect();
        assert_eq!(session_blocks_next.len(), 1);
        let nxt = session_blocks_next[0];
        let nxt_start = Local.from_utc_datetime(&nxt.start.naive_utc());
        let nxt_end = Local.from_utc_datetime(&nxt.end.unwrap().naive_utc());
        assert_eq!(nxt_start.hour(), 0);
        assert_eq!(nxt_start.minute(), 0);
        assert_eq!(nxt_end.hour(), 1);
        assert_eq!(nxt_end.minute(), 0);
        assert_eq!(nxt.duration_seconds(), Some(3600));
    }

    #[test]
    fn test_get_day_with_gaps_midnight_session_with_other_sessions() {
        use chrono::TimeZone;

        let (session_service, task_service) = create_test_services();
        let local_tz = Local::now().timezone();

        let task1 = task_service.get_or_create("overnight-task").unwrap();
        let task2 = task_service.get_or_create("morning-task").unwrap();

        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let overnight_start_local = yesterday.and_hms_opt(23, 0, 0).unwrap();
        let overnight_end_local = today.and_hms_opt(1, 0, 0).unwrap();
        let overnight_start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&overnight_start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let overnight_end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&overnight_end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task1.id, overnight_start_utc, overnight_end_utc, None)
            .unwrap();

        let morning_start_local = today.and_hms_opt(10, 0, 0).unwrap();
        let morning_end_local = today.and_hms_opt(11, 0, 0).unwrap();
        let morning_start_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&morning_start_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let morning_end_utc: DateTime<Utc> = local_tz
            .from_local_datetime(&morning_end_local)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        session_service
            .log_session(task2.id, morning_start_utc, morning_end_utc, None)
            .unwrap();

        let blocks = session_service.get_day_with_gaps(today).unwrap();

        let session_blocks: Vec<&TimeBlock> = blocks.iter().filter(|b| !b.is_gap()).collect();
        assert_eq!(session_blocks.len(), 2);

        let gap_blocks: Vec<&TimeBlock> = blocks
            .iter()
            .filter(|b| b.is_gap() && b.session_id.is_none())
            .collect();
        assert!(gap_blocks.len() >= 1);

        let gap_between = gap_blocks
            .iter()
            .find(|g| {
                let gap_start = Local.from_utc_datetime(&g.start.naive_utc());
                let gap_end = Local.from_utc_datetime(&g.end.unwrap().naive_utc());
                gap_start.hour() == 1 && gap_end.hour() == 10
            })
            .expect("should have gap between overnight and morning sessions");
        assert_eq!(gap_between.duration_seconds(), Some(9 * 3600));
    }
}
