use super::EmQuadrant;
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SplitTask {
    pub name: String,
    pub overlap_seconds: i64,
    pub session_count: usize,
}

#[derive(Debug, Clone)]
pub struct QuadrantTarget {
    pub week_start: NaiveDate,
    pub quadrant: EmQuadrant,
    pub target_minutes: i64,
}

#[derive(Debug, Clone)]
pub struct WeeklyReview {
    pub week_start: NaiveDate,
    pub notes: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct WeekSummary {
    pub week_start: NaiveDate,
    pub quadrant_actuals: HashMap<EmQuadrant, i64>,
    pub quadrant_targets: HashMap<EmQuadrant, i64>,
    pub total_seconds: i64,
    pub split_attention_seconds: i64,
    pub top_split_tasks: Vec<SplitTask>,
    pub has_targets: bool,
}

impl WeekSummary {
    pub fn empty(week_start: NaiveDate) -> Self {
        let mut quadrant_actuals = HashMap::new();
        let mut quadrant_targets = HashMap::new();
        for q in EmQuadrant::all() {
            quadrant_actuals.insert(*q, 0);
            quadrant_targets.insert(*q, 0);
        }
        WeekSummary {
            week_start,
            quadrant_actuals,
            quadrant_targets,
            total_seconds: 0,
            split_attention_seconds: 0,
            top_split_tasks: vec![],
            has_targets: false,
        }
    }

    pub fn target_pct(&self, quadrant: EmQuadrant) -> f64 {
        let total: i64 = self.quadrant_targets.values().sum();
        if total == 0 {
            return 0.0;
        }
        let mins = self.quadrant_targets.get(&quadrant).copied().unwrap_or(0);
        mins as f64 / total as f64 * 100.0
    }

    pub fn actual_pct(&self, quadrant: EmQuadrant) -> f64 {
        if self.total_seconds == 0 {
            return 0.0;
        }
        let secs = self.quadrant_actuals.get(&quadrant).copied().unwrap_or(0);
        secs as f64 / self.total_seconds as f64 * 100.0
    }

    pub fn delta_pct(&self, quadrant: EmQuadrant) -> f64 {
        self.actual_pct(quadrant) - self.target_pct(quadrant)
    }
}

pub const DEFAULT_TARGETS: [(EmQuadrant, i64); 4] = [
    (EmQuadrant::DoFirst, 20),
    (EmQuadrant::Schedule, 75),
    (EmQuadrant::Delegate, 5),
    (EmQuadrant::Eliminate, 0),
];
