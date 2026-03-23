use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimeParseError {
    #[error("Invalid time format: {0}")]
    InvalidFormat(String),
    #[error("Time cannot be in the future")]
    FutureTime,
}

pub type Result<T> = std::result::Result<T, TimeParseError>;

fn split_number_and_unit(input: &str) -> Option<(i64, &str)> {
    let input = input.trim();

    let mut split_idx = 0;
    for (idx, c) in input.char_indices() {
        if c.is_ascii_digit() {
            split_idx = idx + c.len_utf8();
        } else {
            break;
        }
    }

    if split_idx == 0 {
        return None;
    }

    let number_str = &input[..split_idx];
    let unit_str = input[split_idx..].trim_start();

    if unit_str.is_empty() {
        return None;
    }

    let number: i64 = number_str.parse().ok()?;
    Some((number, unit_str))
}

pub fn parse_relative_time(input: &str) -> Result<DateTime<Utc>> {
    let input = input.trim().to_lowercase();
    let now = Utc::now();

    if input == "now" {
        return Ok(now);
    }

    if let Some(rest) = input.strip_suffix(" ago") {
        if let Some((amount, unit)) = split_number_and_unit(rest) {
            let duration = match unit {
                "s" | "sec" | "second" | "seconds" => chrono::Duration::seconds(amount),
                "m" | "min" | "minute" | "minutes" => chrono::Duration::minutes(amount),
                "h" | "hr" | "hour" | "hours" => chrono::Duration::hours(amount),
                "d" | "day" | "days" => chrono::Duration::days(amount),
                "w" | "week" | "weeks" => chrono::Duration::weeks(amount),
                _ => return Err(TimeParseError::InvalidFormat(input)),
            };
            return Ok(now - duration);
        }
    }

    if let Some((amount, unit)) = split_number_and_unit(&input) {
        let duration = match unit {
            "s" | "sec" | "second" | "seconds" => chrono::Duration::seconds(amount),
            "m" | "min" | "minute" | "minutes" => chrono::Duration::minutes(amount),
            "h" | "hr" | "hour" | "hours" => chrono::Duration::hours(amount),
            "d" | "day" | "days" => chrono::Duration::days(amount),
            "w" | "week" | "weeks" => chrono::Duration::weeks(amount),
            _ => return Err(TimeParseError::InvalidFormat(input)),
        };
        return Ok(now - duration);
    }

    if input.starts_with("yesterday") {
        let rest = input.strip_prefix("yesterday").unwrap_or("").trim();
        if rest.is_empty() {
            let yesterday = now.date_naive() - chrono::Duration::days(1);
            return Ok(datetime_from_date_and_time(
                yesterday,
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ));
        }

        if let Ok(time) = NaiveTime::parse_from_str(rest, "%H:%M") {
            let yesterday = now.date_naive() - chrono::Duration::days(1);
            return Ok(datetime_from_date_and_time(yesterday, time));
        }
    }

    if let Ok(time) = NaiveTime::parse_from_str(&input, "%H:%M") {
        let today = now.date_naive();
        let dt = datetime_from_date_and_time(today, time);
        if dt > now {
            return Ok(datetime_from_date_and_time(
                today - chrono::Duration::days(1),
                time,
            ));
        }
        return Ok(dt);
    }

    if let Ok(date) = NaiveDate::parse_from_str(&input, "%Y-%m-%d") {
        return Ok(datetime_from_date_and_time(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        ));
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&input) {
        let dt_utc: DateTime<Utc> = dt.with_timezone(&Utc);
        return Ok(dt_utc);
    }

    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&input, "%Y-%m-%d %H:%M") {
        return Ok(dt.and_utc());
    }

    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&input, "%Y-%m-%d %H:%M:%S") {
        return Ok(dt.and_utc());
    }

    Err(TimeParseError::InvalidFormat(input))
}

fn datetime_from_date_and_time(date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    chrono::NaiveDateTime::new(date, time).and_utc()
}

pub fn parse_date(input: &str) -> Result<NaiveDate> {
    let input = input.trim().to_lowercase();
    let today = Utc::now().date_naive();

    match input.as_str() {
        "today" => Ok(today),
        "yesterday" => Ok(today - chrono::Duration::days(1)),
        _ => NaiveDate::parse_from_str(&input, "%Y-%m-%d")
            .map_err(|_| TimeParseError::InvalidFormat(input)),
    }
}

pub fn parse_period(period: &str) -> Result<(NaiveDate, NaiveDate)> {
    let period = period.trim().to_lowercase();
    let today = Utc::now().date_naive();

    match period.as_str() {
        "day" | "today" => Ok((today, today)),
        "week" | "7d" | "7days" => {
            let start = today - chrono::Duration::days(6);
            Ok((start, today))
        }
        "month" | "30d" | "30days" => {
            let start = today - chrono::Duration::days(29);
            Ok((start, today))
        }
        _ => match parse_date(&period) {
            Ok(date) => Ok((date, date)),
            Err(_) => Err(TimeParseError::InvalidFormat(format!(
                "Unknown period: {}",
                period
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_relative_time_now() {
        let now = Utc::now();
        let parsed = parse_relative_time("now").unwrap();
        let diff = (parsed - now).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_hours_ago() {
        let result = parse_relative_time("2h ago").unwrap();
        let expected = Utc::now() - chrono::Duration::hours(2);
        let diff = (result - expected).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_minutes_ago() {
        let result = parse_relative_time("30m ago").unwrap();
        let expected = Utc::now() - chrono::Duration::minutes(30);
        let diff = (result - expected).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_days_ago() {
        let result = parse_relative_time("2d ago").unwrap();
        let expected = Utc::now() - chrono::Duration::days(2);
        let diff = (result - expected).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_hour_only() {
        let result = parse_relative_time("2h").unwrap();
        let expected = Utc::now() - chrono::Duration::hours(2);
        let diff = (result - expected).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_invalid() {
        assert!(parse_relative_time("invalid").is_err());
    }

    #[test]
    fn test_parse_date_today() {
        let result = parse_date("today").unwrap();
        let today = Utc::now().date_naive();
        assert_eq!(result, today);
    }

    #[test]
    fn test_parse_date_yesterday() {
        let result = parse_date("yesterday").unwrap();
        let yesterday = Utc::now().date_naive() - chrono::Duration::days(1);
        assert_eq!(result, yesterday);
    }

    #[test]
    fn test_parse_date_iso() {
        let result = parse_date("2024-01-15").unwrap();
        assert_eq!(result.to_string(), "2024-01-15");
    }

    #[test]
    fn test_parse_period_day() {
        let (start, end) = parse_period("day").unwrap();
        let today = Utc::now().date_naive();
        assert_eq!(start, today);
        assert_eq!(end, today);
    }

    #[test]
    fn test_parse_period_week() {
        let (start, end) = parse_period("week").unwrap();
        let today = Utc::now().date_naive();
        assert_eq!(end, today);
        assert_eq!((end - start).num_days(), 6);
    }

    #[test]
    fn test_parse_period_month() {
        let (start, end) = parse_period("month").unwrap();
        let today = Utc::now().date_naive();
        assert_eq!(end, today);
        assert_eq!((end - start).num_days(), 29);
    }
}
