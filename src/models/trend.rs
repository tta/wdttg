//! Trend report types and pure bucketing logic.
//!
//! Shared by the stdout renderer (`print_trend_stdout` in main.rs) and the
//! interactive TUI (`TrendView` in ui/trend_view.rs). Both paths call
//! `compute_trend_buckets` to turn `SessionSplitTime` rows into per-item
//! per-bucket sums, then render with their own representation.

use crate::models::{EmQuadrant, SessionSplitTime};
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

/// Which axis to group time along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendDimension {
    Task,
    Tag,
    Quadrant,
    SplitAttention,
}

impl TrendDimension {
    pub fn label(self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Tag => "Tag",
            Self::Quadrant => "Quadrant",
            Self::SplitAttention => "Split-Attention",
        }
    }

    pub fn header_label(self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Tag => "Tag",
            Self::Quadrant => "EM Quadrant",
            Self::SplitAttention => "Split-Attention",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Task => Self::Tag,
            Self::Tag => Self::Quadrant,
            Self::Quadrant => Self::SplitAttention,
            Self::SplitAttention => Self::Task,
        }
    }
}

/// Bucket granularity for the trend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketBy {
    Day,
    Week,
}

impl BucketBy {
    pub fn parse_bucket(s: &str) -> Option<Self> {
        match s {
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "daily",
            Self::Week => "weekly",
        }
    }

    pub fn bucket_days(self) -> i64 {
        match self {
            Self::Day => 1,
            Self::Week => 7,
        }
    }
}

/// One row in the trend output.
#[derive(Debug, Clone)]
pub struct TrendItem {
    pub name: String,
    /// Seconds per bucket. Same length as `TrendResult.bucket_starts`.
    pub buckets: Vec<i64>,
    /// Sum of `buckets`. Precomputed for sorting.
    pub total: i64,
    /// For SplitAttention rows: total overlap across all buckets.
    /// For other dimensions: 0. Used by the TUI to color the sparkline red
    /// when overlap is trending up.
    pub overlap_total: i64,
}

/// The result of bucketing sessions for a trend view.
#[derive(Debug, Clone)]
pub struct TrendResult {
    /// Items sorted by `total` descending, truncated to `top`.
    pub items: Vec<TrendItem>,
    /// Per-bucket grand totals (across all items, before truncation).
    pub totals: Vec<i64>,
    /// The NaiveDate that begins each bucket. For `BucketBy::Week`, this is
    /// the Monday of the week. Same length as each item's `buckets`.
    pub bucket_starts: Vec<NaiveDate>,
}

impl TrendResult {
    pub fn bucket_count(&self) -> usize {
        self.bucket_starts.len()
    }

    pub fn total_grand(&self) -> i64 {
        self.totals.iter().sum()
    }

    /// Human-readable label for the bucket at `idx`.
    pub fn bucket_label(&self, idx: usize, bucket_by: BucketBy) -> String {
        let d = self.bucket_starts[idx];
        match bucket_by {
            BucketBy::Day => d.format("%Y-%m-%d").to_string(),
            BucketBy::Week => format!("week of {}", d.format("%Y-%m-%d")),
        }
    }
}

/// Monday of the week containing `date` (Mon-Sun).
fn monday_of(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn bucket_key(date: NaiveDate, bucket_by: BucketBy) -> NaiveDate {
    match bucket_by {
        BucketBy::Day => date,
        BucketBy::Week => monday_of(date),
    }
}

/// Build the list of bucket-start dates from `start` to `end` inclusive.
fn build_buckets(start: NaiveDate, end: NaiveDate, bucket_by: BucketBy) -> Vec<NaiveDate> {
    let step = bucket_by.bucket_days();
    let mut out: Vec<NaiveDate> = Vec::new();
    let mut d = start;
    while d <= end {
        let key = bucket_key(d, bucket_by);
        if out.last().copied() != Some(key) {
            out.push(key);
        }
        d += chrono::Duration::days(step);
    }
    out
}

/// Computes trend rows from raw `SessionSplitTime` data. Pure: no DB, no IO.
///
/// - For `TrendDimension::Task`, groups by `task_name`.
/// - For `TrendDimension::Tag`, groups by each tag in `task_tags[task_id]`.
///   `task_tags` MUST be `Some` for this dimension; otherwise the result is
///   empty (defensive — the caller is expected to fetch task_tags before
///   requesting the Tag dimension).
/// - For `TrendDimension::Quadrant`, groups by `EmQuadrant` (always 4 rows,
///   in the canonical order DoFirst, Schedule, Delegate, Eliminate).
/// - For `TrendDimension::SplitAttention`, groups by `task_name` but sums
///   `overlap_seconds` instead of `split_seconds`. Rows with zero total
///   overlap are dropped.
///
/// `top` truncates the item list after sorting by total descending. The
/// `totals` row is computed BEFORE truncation.
pub fn compute_trend_buckets(
    sessions: &[SessionSplitTime],
    task_tags: Option<&HashMap<i64, Vec<String>>>,
    dimension: TrendDimension,
    bucket_by: BucketBy,
    start: NaiveDate,
    end: NaiveDate,
    top: usize,
) -> TrendResult {
    let bucket_starts = build_buckets(start, end, bucket_by);
    let num_buckets = bucket_starts.len();

    let bucket_index = |date: NaiveDate| -> Option<usize> {
        let key = bucket_key(date, bucket_by);
        bucket_starts.iter().position(|&d| d == key)
    };

    // name -> (per-bucket seconds, per-bucket overlap)
    let mut acc: HashMap<String, (Vec<i64>, Vec<i64>)> = HashMap::new();
    let mut totals: Vec<i64> = vec![0; num_buckets];

    // For Quadrant, pre-seed the 4 canonical rows so missing quadrants
    // still appear with all-zero buckets.
    if dimension == TrendDimension::Quadrant {
        for q in EmQuadrant::all() {
            acc.insert(quadrant_row_name(*q), (vec![0; num_buckets], vec![0; num_buckets]));
        }
    }

    let push = |name: &str,
                idx: usize,
                secs: i64,
                overlap: i64,
                acc: &mut HashMap<String, (Vec<i64>, Vec<i64>)>,
                totals: &mut [i64]| {
        let entry = acc
            .entry(name.to_string())
            .or_insert_with(|| (vec![0; num_buckets], vec![0; num_buckets]));
        entry.0[idx] += secs;
        entry.1[idx] += overlap;
        totals[idx] += secs;
    };

    match dimension {
        TrendDimension::Task | TrendDimension::SplitAttention => {
            let field_is_overlap = dimension == TrendDimension::SplitAttention;
            for st in sessions {
                let Some(idx) = bucket_index(st.start_time.date_naive()) else {
                    continue;
                };
                let secs = if field_is_overlap {
                    st.overlap_seconds
                } else {
                    st.split_seconds
                };
                push(&st.task_name.clone(), idx, secs, st.overlap_seconds, &mut acc, &mut totals);
            }
        }
        TrendDimension::Tag => {
            let Some(tags_map) = task_tags else {
                return TrendResult {
                    items: vec![],
                    totals,
                    bucket_starts,
                };
            };
            for st in sessions {
                let Some(idx) = bucket_index(st.start_time.date_naive()) else {
                    continue;
                };
                if let Some(tags) = tags_map.get(&st.task_id) {
                    for tag in tags {
                        push(tag, idx, st.split_seconds, st.overlap_seconds, &mut acc, &mut totals);
                    }
                }
            }
        }
        TrendDimension::Quadrant => {
            for st in sessions {
                let Some(idx) = bucket_index(st.start_time.date_naive()) else {
                    continue;
                };
                let name = quadrant_row_name(st.em_quadrant);
                push(&name, idx, st.split_seconds, st.overlap_seconds, &mut acc, &mut totals);
            }
        }
    }

    let mut items: Vec<TrendItem> = acc
        .into_iter()
        .map(|(name, (buckets, overlap_buckets))| {
            let total: i64 = buckets.iter().sum();
            let overlap_total: i64 = overlap_buckets.iter().sum();
            TrendItem {
                name,
                buckets,
                total,
                overlap_total,
            }
        })
        .filter(|it| match dimension {
            // Split-attention drops rows that never had any overlap.
            TrendDimension::SplitAttention => it.overlap_total > 0,
            _ => true,
        })
        .collect();

    // Canonical order for Quadrant; sort by total desc for everything else.
    if dimension == TrendDimension::Quadrant {
        let order = |q: EmQuadrant| match q {
            EmQuadrant::DoFirst => 0,
            EmQuadrant::Schedule => 1,
            EmQuadrant::Delegate => 2,
            EmQuadrant::Eliminate => 3,
        };
        items.sort_by_key(|it| {
            quadrant_from_name(&it.name).map(order).unwrap_or(usize::MAX)
        });
    } else {
        items.sort_by(|a, b| b.total.cmp(&a.total));
    }

    items.truncate(top);

    TrendResult {
        items,
        totals,
        bucket_starts,
    }
}

/// Display name for a quadrant row.
pub fn quadrant_row_name(q: EmQuadrant) -> String {
    match q {
        EmQuadrant::DoFirst => "Q1 DoFirst".to_string(),
        EmQuadrant::Schedule => "Q2 Schedule".to_string(),
        EmQuadrant::Delegate => "Q3 Delegate".to_string(),
        EmQuadrant::Eliminate => "Q4 Eliminate".to_string(),
    }
}

/// Reverse of `quadrant_row_name`. Used to keep the canonical sort order.
fn quadrant_from_name(s: &str) -> Option<EmQuadrant> {
    match s {
        "Q1 DoFirst" => Some(EmQuadrant::DoFirst),
        "Q2 Schedule" => Some(EmQuadrant::Schedule),
        "Q3 Delegate" => Some(EmQuadrant::Delegate),
        "Q4 Eliminate" => Some(EmQuadrant::Eliminate),
        _ => None,
    }
}

/// ASCII-block sparkline characters, indexed 0..=7 (level 0 = empty placeholder).
pub const SPARK_CHARS: &[char] = &['·', '▁', '▂', '▃', '▄', '▅', '▆', '▇'];

/// Render a sparkline as a `String` (one char per bucket). Used by the
/// stdout renderer. The TUI uses `ui::trend_view::render_sparkline_spans`
/// so it can color individual buckets.
///
/// Scaling is per-row: `max_val` is the row's own maximum, so each row
/// fills its own range. This matches the existing stdout behavior at
/// main.rs (pre-refactor).
pub fn render_sparkline(values: &[i64], max_val: i64) -> String {
    if max_val == 0 {
        return values.iter().map(|_| SPARK_CHARS[0]).collect();
    }
    values
        .iter()
        .map(|&v| {
            if v == 0 {
                SPARK_CHARS[0]
            } else {
                let level = ((v as f64 / max_val as f64) * 7.0).ceil() as usize;
                SPARK_CHARS[level.min(7)]
            }
        })
        .collect()
}

/// Arrow + percentage comparing the last non-zero bucket to the mean of
/// all non-zero buckets. Empty string if fewer than 2 non-zero buckets.
pub fn format_trend_arrow(values: &[i64]) -> String {
    if values.len() < 2 {
        return String::new();
    }
    let nonzero: Vec<i64> = values.iter().filter(|&&v| v > 0).copied().collect();
    if nonzero.len() < 2 {
        return String::new();
    }
    let avg: f64 = nonzero.iter().sum::<i64>() as f64 / nonzero.len() as f64;
    if avg == 0.0 {
        return String::new();
    }
    let last = nonzero.last().unwrap();
    let pct = ((*last as f64 - avg) / avg * 100.0) as i64;
    match pct.cmp(&0) {
        std::cmp::Ordering::Greater => format!("↑ +{}%", pct),
        std::cmp::Ordering::Less => format!("↓ {}%", pct),
        std::cmp::Ordering::Equal => "→  0%".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EmQuadrant, SessionSplitTime};
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;

    fn split(id: i64, task_id: i64, name: &str, q: EmQuadrant, day: u32) -> SessionSplitTime {
        SessionSplitTime {
            session_id: id,
            task_id,
            task_name: name.to_string(),
            em_quadrant: q,
            start_time: Utc.with_ymd_and_hms(2026, 6, day, 9, 0, 0).unwrap(),
            total_seconds: 3600,
            split_seconds: 3600,
            overlap_seconds: 0,
            link: None,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn empty_sessions_produces_empty_items_with_totals_zero() {
        let r = compute_trend_buckets(
            &[],
            None,
            TrendDimension::Task,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 7),
            10,
        );
        assert!(r.items.is_empty());
        assert_eq!(r.totals, vec![0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(r.bucket_count(), 7);
    }

    #[test]
    fn task_dimension_groups_by_task_name() {
        let sessions = [
            split(1, 100, "coding/x", EmQuadrant::Schedule, 1),
            split(2, 100, "coding/x", EmQuadrant::Schedule, 1),
            split(3, 200, "meeting/y", EmQuadrant::Delegate, 2),
        ];
        let r = compute_trend_buckets(
            &sessions,
            None,
            TrendDimension::Task,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 2),
            10,
        );
        assert_eq!(r.items.len(), 2);
        // Sorted by total desc: coding/x (7200) > meeting/y (3600).
        assert_eq!(r.items[0].name, "coding/x");
        assert_eq!(r.items[0].total, 7200);
        assert_eq!(r.items[1].name, "meeting/y");
        // coding/x on day 1 only; day 2 is 0.
        assert_eq!(r.items[0].buckets, vec![7200, 0]);
        assert_eq!(r.items[1].buckets, vec![0, 3600]);
        // Totals across both days.
        assert_eq!(r.totals, vec![7200, 3600]);
    }

    #[test]
    fn top_truncation_keeps_highest() {
        let sessions = [
            split(1, 1, "a", EmQuadrant::Schedule, 1),
            split(2, 2, "b", EmQuadrant::Schedule, 1),
            split(3, 3, "c", EmQuadrant::Schedule, 1),
        ];
        let r = compute_trend_buckets(
            &sessions,
            None,
            TrendDimension::Task,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 1),
            2,
        );
        assert_eq!(r.items.len(), 2);
        // All three have equal totals; sort is stable on HashMap order, so
        // we only assert that exactly 2 survived truncation.
        assert!(r.totals[0] == 3600 * 3);
    }

    #[test]
    fn tag_dimension_uses_task_tags_map() {
        let sessions = [split(1, 100, "coding/x", EmQuadrant::Schedule, 1)];
        let mut tags = HashMap::new();
        tags.insert(100, vec!["work".to_string(), "deep".to_string()]);
        let r = compute_trend_buckets(
            &sessions,
            Some(&tags),
            TrendDimension::Tag,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 1),
            10,
        );
        // Two tag rows, each with the full split_seconds (tags don't divide).
        assert_eq!(r.items.len(), 2);
        for it in &r.items {
            assert_eq!(it.total, 3600);
        }
    }

    #[test]
    fn tag_dimension_without_map_returns_empty_items() {
        let sessions = [split(1, 100, "coding/x", EmQuadrant::Schedule, 1)];
        let r = compute_trend_buckets(
            &sessions,
            None,
            TrendDimension::Tag,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 1),
            10,
        );
        assert!(r.items.is_empty());
    }

    #[test]
    fn quadrant_dimension_always_has_four_rows_in_canonical_order() {
        let sessions = [
            split(1, 1, "x", EmQuadrant::Schedule, 1),
            split(2, 2, "y", EmQuadrant::DoFirst, 1),
        ];
        let r = compute_trend_buckets(
            &sessions,
            None,
            TrendDimension::Quadrant,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 1),
            10,
        );
        assert_eq!(r.items.len(), 4);
        assert_eq!(r.items[0].name, "Q1 DoFirst");
        assert_eq!(r.items[1].name, "Q2 Schedule");
        assert_eq!(r.items[2].name, "Q3 Delegate");
        assert_eq!(r.items[3].name, "Q4 Eliminate");
        assert_eq!(r.items[0].total, 3600); // DoFirst
        assert_eq!(r.items[1].total, 3600); // Schedule
        assert_eq!(r.items[2].total, 0); // Delegate absent
        assert_eq!(r.items[3].total, 0); // Eliminate absent
    }

    #[test]
    fn split_attention_drops_zero_overlap_rows() {
        // split() defaults to overlap_seconds=0, so the Task dimension
        // would include them but SplitAttention should drop them.
        let mut s_with_overlap = split(1, 100, "coding/x", EmQuadrant::Schedule, 1);
        s_with_overlap.overlap_seconds = 1800;
        let s_no_overlap = split(2, 200, "solo/y", EmQuadrant::Schedule, 1);
        let sessions = [s_with_overlap, s_no_overlap];

        let r = compute_trend_buckets(
            &sessions,
            None,
            TrendDimension::SplitAttention,
            BucketBy::Day,
            date(2026, 6, 1),
            date(2026, 6, 1),
            10,
        );
        // Only the overlapping task survives.
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].name, "coding/x");
        assert_eq!(r.items[0].buckets, vec![1800]);
        assert_eq!(r.items[0].overlap_total, 1800);
        // Totals reflect overlap sums (not split_seconds).
        assert_eq!(r.totals, vec![1800]);
    }

    #[test]
    fn weekly_buckets_collapse_to_monday_starts() {
        // A week Wed→Tue produces 2 weekly buckets (this week + next week).
        let sessions = [
            split(1, 1, "x", EmQuadrant::Schedule, 3),  // Wed Jun 3
            split(2, 1, "x", EmQuadrant::Schedule, 10), // Wed Jun 10
        ];
        let r = compute_trend_buckets(
            &sessions,
            None,
            TrendDimension::Task,
            BucketBy::Week,
            date(2026, 6, 3),
            date(2026, 6, 10),
            10,
        );
        // Two weekly buckets: Mon Jun 1 and Mon Jun 8.
        assert_eq!(r.bucket_count(), 2);
        assert_eq!(r.bucket_starts[0], date(2026, 6, 1));
        assert_eq!(r.bucket_starts[1], date(2026, 6, 8));
        assert_eq!(r.items[0].buckets, vec![3600, 3600]);
    }

    #[test]
    fn render_sparkline_scales_per_row() {
        let values = vec![0, 50, 100];
        let s = render_sparkline(&values, 100);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars[0], SPARK_CHARS[0]); // 0 → placeholder
        assert_eq!(chars[2], SPARK_CHARS[7]); // max → highest block
    }

    #[test]
    fn format_trend_arrow_handles_few_buckets() {
        assert_eq!(format_trend_arrow(&[]), "");
        assert_eq!(format_trend_arrow(&[100]), "");
        assert_eq!(format_trend_arrow(&[0, 0]), "");
    }

    #[test]
    fn format_trend_arrow_compares_last_to_mean() {
        // mean of [100, 200] = 150; last = 200; (200-150)/150 = +33%.
        let arrow = format_trend_arrow(&[100, 0, 200]);
        assert!(arrow.starts_with("↑ +"));
    }

    #[test]
    fn dimension_cycle_rotates_through_all_four() {
        let mut d = TrendDimension::Task;
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(d);
            d = d.cycle();
        }
        assert_eq!(
            seen,
            [
                TrendDimension::Task,
                TrendDimension::Tag,
                TrendDimension::Quadrant,
                TrendDimension::SplitAttention,
            ]
        );
        // After 4 cycles we're back.
        assert_eq!(d, TrendDimension::Task);
    }
}
