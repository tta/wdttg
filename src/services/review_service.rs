use crate::db::Database;
use crate::models::{EmQuadrant, QuadrantTarget, SplitTask, WeekSummary, WeeklyReview};
use crate::time_parser::local_to_utc_safe;
use chrono::{Datelike, NaiveDate, NaiveDateTime};
use rusqlite::params;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReviewServiceError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, ReviewServiceError>;

pub struct ReviewService {
    db: Arc<Database>,
}

impl ReviewService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    pub fn has_active_session(&self, week_start: NaiveDate) -> bool {
        let (start_str, end_str) = week_bounds(week_start);
        let conn = self.db.connection();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions
                 WHERE start_time >= ?1 AND start_time < ?2
                   AND end_time IS NULL",
                params![start_str, end_str],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        count > 0
    }

    pub fn get_week_summary(&self, week_start: NaiveDate) -> Result<WeekSummary> {
        let (start_str, end_str) = week_bounds(week_start);
        let mut summary = WeekSummary::empty(week_start);

        {
            let conn = self.db.connection();

            let mut stmt = conn.prepare(
                "SELECT t.em_urgent, t.em_important,
                        SUM(CAST(strftime('%s', COALESCE(s.end_time, datetime('now'))) AS INTEGER)
                            - CAST(strftime('%s', s.start_time) AS INTEGER)) AS total_secs
                 FROM sessions s JOIN tasks t ON s.task_id = t.id
                 WHERE s.start_time >= ?1 AND s.start_time < ?2
                   AND s.is_background = 0
                 GROUP BY t.em_urgent, t.em_important",
            )?;

            let rows = stmt.query_map(params![start_str, end_str], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;

            for row in rows {
                let (em_urgent, em_important, total_secs) = row?;
                let quadrant = EmQuadrant::new(em_urgent != 0, em_important != 0);
                summary.quadrant_actuals.insert(quadrant, total_secs);
                summary.total_seconds += total_secs;
            }
        }

        let targets = self.get_targets(week_start)?;
        if targets.is_empty() {
            summary.has_targets = false;
            for (quadrant, minutes) in crate::models::DEFAULT_TARGETS {
                summary.quadrant_targets.insert(quadrant, minutes * 60);
            }
        } else {
            summary.has_targets = true;
            for target in &targets {
                summary
                    .quadrant_targets
                    .insert(target.quadrant, target.target_minutes * 60);
            }
        }

        let (total_split, top_split) = self.get_split_attention(week_start)?;
        summary.split_attention_seconds = total_split;
        summary.top_split_tasks = top_split;

        Ok(summary)
    }

    pub fn get_split_attention(
        &self,
        week_start: NaiveDate,
    ) -> Result<(i64, Vec<SplitTask>)> {
        let (start_str, end_str) = week_bounds(week_start);
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT bt.name AS bg_task,
                    SUM(CAST(strftime('%s', MIN(COALESCE(a.end_time, datetime('now')), COALESCE(b.end_time, datetime('now')))) AS INTEGER)
                        - CAST(strftime('%s', MAX(a.start_time, b.start_time)) AS INTEGER)) AS overlap_secs,
                    COUNT(*) AS session_count
             FROM sessions a JOIN sessions b
               ON a.start_time < COALESCE(b.end_time, datetime('now'))
               AND b.start_time < COALESCE(a.end_time, datetime('now'))
             JOIN tasks bt ON b.task_id = bt.id
             WHERE a.start_time >= ?1 AND a.start_time < ?2
               AND b.start_time >= ?1 AND b.start_time < ?2
               AND a.is_background = 0
               AND b.is_background = 1
             GROUP BY bt.name
             ORDER BY overlap_secs DESC",
        )?;

        let rows = stmt.query_map(params![start_str, end_str], |row| {
            Ok(SplitTask {
                name: row.get::<_, String>(0)?,
                overlap_seconds: row.get::<_, i64>(1)?,
                session_count: row.get::<_, i64>(2)? as usize,
            })
        })?;

        let mut all_tasks: Vec<SplitTask> = rows.collect::<std::result::Result<_, _>>()?;
        let total: i64 = all_tasks.iter().map(|t| t.overlap_seconds).sum();
        all_tasks.truncate(5);

        Ok((total, all_tasks))
    }

    pub fn get_targets(&self, week_start: NaiveDate) -> Result<Vec<QuadrantTarget>> {
        let conn = self.db.connection();
        let week_str = week_start.format("%Y-%m-%d").to_string();

        let mut stmt = conn.prepare(
            "SELECT week_start, em_urgent, em_important, target_minutes
             FROM weekly_targets
             WHERE week_start = ?1",
        )?;

        let rows = stmt.query_map(params![week_str], |row| {
            let ws: String = row.get(0)?;
            let em_urgent: i64 = row.get(1)?;
            let em_important: i64 = row.get(2)?;
            Ok(QuadrantTarget {
                week_start: NaiveDate::parse_from_str(&ws, "%Y-%m-%d").unwrap_or(week_start),
                quadrant: EmQuadrant::new(em_urgent != 0, em_important != 0),
                target_minutes: row.get(3)?,
            })
        })?;

        rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
    }

    pub fn set_target(
        &self,
        week_start: NaiveDate,
        quadrant: EmQuadrant,
        target_minutes: i64,
    ) -> Result<()> {
        let conn = self.db.connection();
        let week_str = week_start.format("%Y-%m-%d").to_string();
        let (em_urgent, em_important) = quadrant.flags();

        conn.execute(
            "INSERT INTO weekly_targets (week_start, em_urgent, em_important, target_minutes)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (week_start, em_urgent, em_important)
             DO UPDATE SET target_minutes = excluded.target_minutes",
            params![week_str, em_urgent as i32, em_important as i32, target_minutes],
        )?;

        Ok(())
    }

    pub fn save_review(&self, week_start: NaiveDate, notes: &str) -> Result<()> {
        let conn = self.db.connection();
        let week_str = week_start.format("%Y-%m-%d").to_string();

        conn.execute(
            "INSERT INTO weekly_reviews (week_start, notes)
             VALUES (?1, ?2)
             ON CONFLICT (week_start)
             DO UPDATE SET notes = excluded.notes",
            params![week_str, notes],
        )?;

        Ok(())
    }

    pub fn get_review(&self, week_start: NaiveDate) -> Result<Option<WeeklyReview>> {
        let conn = self.db.connection();
        let week_str = week_start.format("%Y-%m-%d").to_string();

        let result = conn.query_row(
            "SELECT week_start, notes, created_at FROM weekly_reviews WHERE week_start = ?1",
            params![week_str],
            |row| {
                let ws: String = row.get(0)?;
                let notes: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
                let created_str: String = row.get(2)?;
                let created = NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S")
                    .map(|dt| dt.and_utc())
                    .unwrap_or_else(|_| chrono::Utc::now());
                Ok(WeeklyReview {
                    week_start: NaiveDate::parse_from_str(&ws, "%Y-%m-%d")
                        .unwrap_or(week_start),
                    notes,
                    created_at: created,
                })
            },
        );

        match result {
            Ok(review) => Ok(Some(review)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_review_history(&self, limit: usize) -> Result<Vec<WeeklyReview>> {
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT week_start, notes, created_at FROM weekly_reviews ORDER BY week_start DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let ws: String = row.get(0)?;
            let notes: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
            let created_str: String = row.get(2)?;
            let created = NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S")
                .map(|dt| dt.and_utc())
                .unwrap_or_else(|_| chrono::Utc::now());
            Ok(WeeklyReview {
                week_start: NaiveDate::parse_from_str(&ws, "%Y-%m-%d").unwrap_or_default(),
                notes,
                created_at: created,
            })
        })?;

        rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
    }

    pub fn get_8week_trend(&self, week_start: NaiveDate) -> Result<Vec<WeekSummary>> {
        let mut trend = Vec::with_capacity(8);
        for i in (0..8).rev() {
            let ws = week_start - chrono::Duration::weeks(i as i64);
            trend.push(self.get_week_summary(ws)?);
        }
        Ok(trend)
    }
}

pub fn monday_of_week(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn week_bounds(week_start: NaiveDate) -> (String, String) {
    let start_local = week_start.and_hms_opt(0, 0, 0).unwrap();
    let week_end = week_start + chrono::Duration::days(7);
    let end_local = week_end.and_hms_opt(0, 0, 0).unwrap();

    let start_utc = local_to_utc_safe(start_local);
    let end_utc = local_to_utc_safe(end_local);

    (
        crate::services::format_sqlite_datetime(&start_utc),
        crate::services::format_sqlite_datetime(&end_utc),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{SessionService, TaskService};
    use chrono::Local;

    #[test]
    fn test_monday_of_week() {
        let wed = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        let mon = monday_of_week(wed);
        assert_eq!(mon, NaiveDate::from_ymd_opt(2026, 6, 8).unwrap());

        let mon2 = monday_of_week(mon);
        assert_eq!(mon2, mon);
    }

    #[test]
    fn test_week_summary_empty() {
        let ws = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let summary = WeekSummary::empty(ws);
        assert_eq!(summary.total_seconds, 0);
        assert_eq!(summary.split_attention_seconds, 0);
        assert!(summary.top_split_tasks.is_empty());
        for q in EmQuadrant::all() {
            assert_eq!(summary.quadrant_actuals.get(q), Some(&0));
        }
    }

    #[test]
    fn test_percentage_computation() {
        let ws = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let mut summary = WeekSummary::empty(ws);
        summary.total_seconds = 3600 * 10;
        summary.quadrant_actuals.insert(EmQuadrant::DoFirst, 3600 * 4);
        summary.quadrant_actuals.insert(EmQuadrant::Schedule, 3600 * 6);

        assert_eq!(summary.actual_pct(EmQuadrant::DoFirst), 40.0);
        assert_eq!(summary.actual_pct(EmQuadrant::Schedule), 60.0);

        summary.quadrant_targets.insert(EmQuadrant::DoFirst, 1200);
        summary.quadrant_targets.insert(EmQuadrant::Schedule, 1800);
        summary.quadrant_targets.insert(EmQuadrant::Delegate, 300);
        summary.quadrant_targets.insert(EmQuadrant::Eliminate, 300);

        assert!((summary.target_pct(EmQuadrant::DoFirst) - 33.33).abs() < 0.1);
    }

    #[test]
    fn test_percentage_division_by_zero() {
        let ws = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let summary = WeekSummary::empty(ws);
        assert_eq!(summary.actual_pct(EmQuadrant::DoFirst), 0.0);
        assert_eq!(summary.target_pct(EmQuadrant::DoFirst), 0.0);
    }

    fn create_test_services() -> (ReviewService, SessionService, TaskService) {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        let db = Arc::new(db);
        (
            ReviewService::new(Arc::clone(&db)),
            SessionService::new(Arc::clone(&db)),
            TaskService::new(db),
        )
    }

    #[test]
    fn test_target_upsert() {
        let db = Database::open_memory().expect("db");
        let svc = ReviewService::new(Arc::new(db));
        let ws = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();

        svc.set_target(ws, EmQuadrant::DoFirst, 120).unwrap();
        svc.set_target(ws, EmQuadrant::DoFirst, 240).unwrap();

        let targets = svc.get_targets(ws).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].target_minutes, 240);
    }

    #[test]
    fn test_cold_start_no_targets() {
        let db = Database::open_memory().expect("db");
        let svc = ReviewService::new(Arc::new(db));
        let ws = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();

        let targets = svc.get_targets(ws).unwrap();
        assert!(targets.is_empty());
    }

    #[test]
    fn test_save_and_get_review() {
        let db = Database::open_memory().expect("db");
        let svc = ReviewService::new(Arc::new(db));
        let ws = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();

        svc.save_review(ws, "Great week").unwrap();
        let review = svc.get_review(ws).unwrap();
        assert!(review.is_some());
        assert_eq!(review.unwrap().notes, "Great week");

        svc.save_review(ws, "Updated notes").unwrap();
        let review2 = svc.get_review(ws).unwrap();
        assert_eq!(review2.unwrap().notes, "Updated notes");
    }

    #[test]
    fn test_get_week_summary_zero_fill() {
        let (review_svc, sess_svc, task_svc) = create_test_services();

        let task = task_svc.get_or_create("test-task").unwrap();
        let start = chrono::Utc::now();
        sess_svc
            .log_session(task.id, start, Some(start + chrono::Duration::hours(2)), None)
            .unwrap();

        let ws = monday_of_week(Local::now().date_naive());
        let summary = review_svc.get_week_summary(ws).unwrap();

        for q in EmQuadrant::all() {
            assert!(summary.quadrant_actuals.contains_key(q));
        }
    }
}
