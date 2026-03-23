use super::{EmQuadrant, Task};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: i64,
    pub task_id: i64,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Session {
    pub fn is_active(&self) -> bool {
        self.end_time.is_none()
    }

    pub fn duration_seconds(&self) -> Option<i64> {
        self.end_time
            .map(|end| (end - self.start_time).num_seconds())
    }

    pub fn duration_minutes(&self) -> Option<i64> {
        self.duration_seconds().map(|s| s / 60)
    }
}

#[derive(Debug, Clone)]
pub struct SessionWithTask {
    pub session: Session,
    pub task: Task,
}

impl SessionWithTask {
    pub fn is_active(&self) -> bool {
        self.session.is_active()
    }

    pub fn duration_seconds(&self) -> Option<i64> {
        self.session.duration_seconds()
    }
}

#[derive(Debug, Clone)]
pub struct QuadrantTime {
    pub quadrant: EmQuadrant,
    pub total_seconds: i64,
}

impl QuadrantTime {
    pub fn total_minutes(&self) -> i64 {
        self.total_seconds / 60
    }

    pub fn total_hours(&self) -> f64 {
        self.total_seconds as f64 / 3600.0
    }
}

#[derive(Debug, Clone)]
pub struct TagTime {
    pub tag: String,
    pub total_seconds: i64,
}

impl TagTime {
    pub fn total_minutes(&self) -> i64 {
        self.total_seconds / 60
    }

    pub fn total_hours(&self) -> f64 {
        self.total_seconds as f64 / 3600.0
    }
}

#[derive(Debug, Clone)]
pub struct TagWithTime {
    pub tag: String,
    pub total_seconds: i64,
    pub session_count: i64,
}

impl TagWithTime {
    pub fn total_minutes(&self) -> i64 {
        self.total_seconds / 60
    }

    pub fn total_hours(&self) -> f64 {
        self.total_seconds as f64 / 3600.0
    }
}

#[derive(Debug, Clone)]
pub struct TaskTime {
    pub task_name: String,
    pub total_seconds: i64,
    pub session_count: i64,
    pub em_quadrant: EmQuadrant,
}

impl TaskTime {
    pub fn total_minutes(&self) -> i64 {
        self.total_seconds / 60
    }

    pub fn total_hours(&self) -> f64 {
        self.total_seconds as f64 / 3600.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_session_is_active() {
        let active_session = Session {
            id: 1,
            task_id: 1,
            start_time: Utc::now(),
            end_time: None,
            notes: None,
            created_at: Utc::now(),
        };
        assert!(active_session.is_active());

        let completed_session = Session {
            id: 2,
            task_id: 1,
            start_time: Utc::now(),
            end_time: Some(Utc::now()),
            notes: None,
            created_at: Utc::now(),
        };
        assert!(!completed_session.is_active());
    }

    #[test]
    fn test_session_duration() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 11, 30, 0).unwrap();

        let session = Session {
            id: 1,
            task_id: 1,
            start_time: start,
            end_time: Some(end),
            notes: None,
            created_at: start,
        };

        assert_eq!(session.duration_seconds(), Some(5400));
        assert_eq!(session.duration_minutes(), Some(90));
    }

    #[test]
    fn test_session_duration_active() {
        let active_session = Session {
            id: 1,
            task_id: 1,
            start_time: Utc::now(),
            end_time: None,
            notes: None,
            created_at: Utc::now(),
        };

        assert!(active_session.duration_seconds().is_none());
        assert!(active_session.duration_minutes().is_none());
    }

    #[test]
    fn test_quadrant_time_helpers() {
        let qt = QuadrantTime {
            quadrant: EmQuadrant::DoFirst,
            total_seconds: 3661,
        };

        assert_eq!(qt.total_minutes(), 61);
        assert!((qt.total_hours() - 1.0169444444444444).abs() < 0.0001);
    }
}
