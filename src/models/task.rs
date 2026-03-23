use chrono::{DateTime, Utc};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Active,
    Completed,
    Archived,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Active => "active",
            TaskStatus::Completed => "completed",
            TaskStatus::Archived => "archived",
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(TaskStatus::Active),
            "completed" => Ok(TaskStatus::Completed),
            "archived" => Ok(TaskStatus::Archived),
            _ => Err(format!(
                "Invalid task status: '{}'. Expected: active, completed, or archived",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmQuadrant {
    DoFirst,
    Schedule,
    Delegate,
    Eliminate,
}

impl EmQuadrant {
    pub fn new(urgent: bool, important: bool) -> Self {
        match (urgent, important) {
            (true, true) => EmQuadrant::DoFirst,
            (false, true) => EmQuadrant::Schedule,
            (true, false) => EmQuadrant::Delegate,
            (false, false) => EmQuadrant::Eliminate,
        }
    }

    pub fn is_urgent(&self) -> bool {
        matches!(self, EmQuadrant::DoFirst | EmQuadrant::Delegate)
    }

    pub fn is_important(&self) -> bool {
        matches!(self, EmQuadrant::DoFirst | EmQuadrant::Schedule)
    }

    pub fn code(&self) -> &'static str {
        match self {
            EmQuadrant::DoFirst => "11",
            EmQuadrant::Schedule => "01",
            EmQuadrant::Delegate => "10",
            EmQuadrant::Eliminate => "00",
        }
    }

    pub fn tag(&self) -> String {
        format!("#em{}", self.code())
    }

    pub fn description(&self) -> &'static str {
        match self {
            EmQuadrant::DoFirst => "Urgent + Important (Do First)",
            EmQuadrant::Schedule => "Not Urgent + Important (Schedule)",
            EmQuadrant::Delegate => "Urgent + Not Important (Delegate)",
            EmQuadrant::Eliminate => "Not Urgent + Not Important (Eliminate)",
        }
    }
}

impl fmt::Display for EmQuadrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.tag())
    }
}

impl FromStr for EmQuadrant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        let clean = normalized.trim_start_matches("#em");

        match clean {
            "11" => Ok(EmQuadrant::DoFirst),
            "01" => Ok(EmQuadrant::Schedule),
            "10" => Ok(EmQuadrant::Delegate),
            "00" => Ok(EmQuadrant::Eliminate),
            _ => Err(format!(
                "Invalid EM quadrant: '{}'. Use format: 00, 01, 10, or 11",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: i64,
    pub name: String,
    pub notes: Option<String>,
    pub em_urgent: bool,
    pub em_important: bool,
    pub status: TaskStatus,
    pub recurring: bool,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn em_quadrant(&self) -> EmQuadrant {
        EmQuadrant::new(self.em_urgent, self.em_important)
    }

    pub fn pickable(&self) -> bool {
        matches!(self.status, TaskStatus::Active) || self.recurring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_from_str() {
        assert_eq!(TaskStatus::from_str("active").unwrap(), TaskStatus::Active);
        assert_eq!(
            TaskStatus::from_str("completed").unwrap(),
            TaskStatus::Completed
        );
        assert_eq!(
            TaskStatus::from_str("archived").unwrap(),
            TaskStatus::Archived
        );
    }

    #[test]
    fn test_task_status_from_str_invalid() {
        assert!(TaskStatus::from_str("invalid").is_err());
        assert!(TaskStatus::from_str("").is_err());
        assert!(TaskStatus::from_str("ACTIVE").is_err());
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(format!("{}", TaskStatus::Active), "active");
        assert_eq!(format!("{}", TaskStatus::Completed), "completed");
        assert_eq!(format!("{}", TaskStatus::Archived), "archived");
    }

    #[test]
    fn test_em_quadrant_new() {
        assert_eq!(EmQuadrant::new(true, true), EmQuadrant::DoFirst);
        assert_eq!(EmQuadrant::new(false, true), EmQuadrant::Schedule);
        assert_eq!(EmQuadrant::new(true, false), EmQuadrant::Delegate);
        assert_eq!(EmQuadrant::new(false, false), EmQuadrant::Eliminate);
    }

    #[test]
    fn test_em_quadrant_from_str_valid() {
        assert_eq!(EmQuadrant::from_str("11").unwrap(), EmQuadrant::DoFirst);
        assert_eq!(EmQuadrant::from_str("01").unwrap(), EmQuadrant::Schedule);
        assert_eq!(EmQuadrant::from_str("10").unwrap(), EmQuadrant::Delegate);
        assert_eq!(EmQuadrant::from_str("00").unwrap(), EmQuadrant::Eliminate);
    }

    #[test]
    fn test_em_quadrant_from_str_with_hash_prefix() {
        assert_eq!(EmQuadrant::from_str("#em11").unwrap(), EmQuadrant::DoFirst);
        assert_eq!(EmQuadrant::from_str("#EM01").unwrap(), EmQuadrant::Schedule);
        assert_eq!(
            EmQuadrant::from_str(" #em10 ").unwrap(),
            EmQuadrant::Delegate
        );
    }

    #[test]
    fn test_em_quadrant_from_str_invalid() {
        assert!(EmQuadrant::from_str("12").is_err());
        assert!(EmQuadrant::from_str("invalid").is_err());
        assert!(EmQuadrant::from_str("").is_err());
        assert!(EmQuadrant::from_str("#em").is_err());
    }

    #[test]
    fn test_em_quadrant_code() {
        assert_eq!(EmQuadrant::DoFirst.code(), "11");
        assert_eq!(EmQuadrant::Schedule.code(), "01");
        assert_eq!(EmQuadrant::Delegate.code(), "10");
        assert_eq!(EmQuadrant::Eliminate.code(), "00");
    }

    #[test]
    fn test_em_quadrant_tag() {
        assert_eq!(EmQuadrant::DoFirst.tag(), "#em11");
        assert_eq!(EmQuadrant::Schedule.tag(), "#em01");
        assert_eq!(EmQuadrant::Delegate.tag(), "#em10");
        assert_eq!(EmQuadrant::Eliminate.tag(), "#em00");
    }

    #[test]
    fn test_em_quadrant_description() {
        assert!(EmQuadrant::DoFirst.description().contains("Do First"));
        assert!(EmQuadrant::Schedule.description().contains("Schedule"));
        assert!(EmQuadrant::Delegate.description().contains("Delegate"));
        assert!(EmQuadrant::Eliminate.description().contains("Eliminate"));
    }

    #[test]
    fn test_em_quadrant_flags() {
        assert!(EmQuadrant::DoFirst.is_urgent());
        assert!(EmQuadrant::DoFirst.is_important());

        assert!(!EmQuadrant::Schedule.is_urgent());
        assert!(EmQuadrant::Schedule.is_important());

        assert!(EmQuadrant::Delegate.is_urgent());
        assert!(!EmQuadrant::Delegate.is_important());

        assert!(!EmQuadrant::Eliminate.is_urgent());
        assert!(!EmQuadrant::Eliminate.is_important());
    }
}
