use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc};
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

    if unit_str.is_empty() || unit_str.starts_with(':') {
        return None;
    }

    let number: i64 = number_str.parse().ok()?;
    Some((number, unit_str))
}

pub fn parse_relative_time(input: &str) -> Result<DateTime<Utc>> {
    let input = input.trim().to_lowercase();
    let now = Local::now();

    if input == "now" {
        return Ok(now.with_timezone(&Utc));
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
            return Ok((now - duration).with_timezone(&Utc));
        }
    }

    // Try M/D H:MM format (e.g., "3/22 11:11") - before split_number_and_unit
    if let Some((date_part, time_part)) = input.split_once(' ') {
        if let Some((month, day)) = parse_month_day(date_part) {
            if let Some(time) = parse_time_str(time_part) {
                let year = now.year();
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    return Ok(datetime_from_local_date_and_time(date, time));
                }
            }
        }
    }

    // Try M/D format (e.g., "3/22" - midnight)
    if let Some((month, day)) = parse_month_day(&input) {
        let year = now.year();
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            return Ok(datetime_from_local_date_and_time(
                date,
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ));
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
        return Ok((now - duration).with_timezone(&Utc));
    }

    if input.starts_with("yesterday") {
        let rest = input.strip_prefix("yesterday").unwrap_or("").trim();
        if rest.is_empty() {
            let yesterday = now.date_naive() - chrono::Duration::days(1);
            return Ok(datetime_from_local_date_and_time(
                yesterday,
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ));
        }

        if let Ok(time) = NaiveTime::parse_from_str(rest, "%H:%M") {
            let yesterday = now.date_naive() - chrono::Duration::days(1);
            return Ok(datetime_from_local_date_and_time(yesterday, time));
        }

        if let Some((hour_str, min_str)) = rest.split_once(':') {
            if let (Ok(hour), Ok(min)) = (hour_str.parse::<u32>(), min_str.parse::<u32>()) {
                if let Some(time) = NaiveTime::from_hms_opt(hour, min, 0) {
                    let yesterday = now.date_naive() - chrono::Duration::days(1);
                    return Ok(datetime_from_local_date_and_time(yesterday, time));
                }
            }
        }
    }

    if let Ok(time) = NaiveTime::parse_from_str(&input, "%H:%M") {
        let today = now.date_naive();
        let dt = datetime_from_local_date_and_time(today, time);
        if dt > now.with_timezone(&Utc) {
            return Ok(datetime_from_local_date_and_time(
                today - chrono::Duration::days(1),
                time,
            ));
        }
        return Ok(dt);
    }

    if let Some((hour_str, min_str)) = input.split_once(':') {
        if let (Ok(hour), Ok(min)) = (hour_str.parse::<u32>(), min_str.parse::<u32>()) {
            if let Some(time) = NaiveTime::from_hms_opt(hour, min, 0) {
                let today = now.date_naive();
                let dt = datetime_from_local_date_and_time(today, time);
                if dt > now.with_timezone(&Utc) {
                    return Ok(datetime_from_local_date_and_time(
                        today - chrono::Duration::days(1),
                        time,
                    ));
                }
                return Ok(dt);
            }
        }
    }

    if let Ok(date) = NaiveDate::parse_from_str(&input, "%Y-%m-%d") {
        return Ok(datetime_from_local_date_and_time(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        ));
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&input) {
        let dt_utc: DateTime<Utc> = dt.with_timezone(&Utc);
        return Ok(dt_utc);
    }

    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&input, "%Y-%m-%d %H:%M") {
        return Ok(Local
            .from_local_datetime(&dt)
            .single()
            .unwrap()
            .with_timezone(&Utc));
    }

    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&input, "%Y-%m-%d %H:%M:%S") {
        return Ok(Local
            .from_local_datetime(&dt)
            .single()
            .unwrap()
            .with_timezone(&Utc));
    }

    Err(TimeParseError::InvalidFormat(input))
}

fn datetime_from_local_date_and_time(date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    let naive = chrono::NaiveDateTime::new(date, time);
    Local
        .from_local_datetime(&naive)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn parse_month_day(input: &str) -> Option<(u32, u32)> {
    let input = input.trim();
    let (month_str, day_str) = input.split_once('/')?;
    let month: u32 = month_str.parse().ok()?;
    let day: u32 = day_str.parse().ok()?;
    if (1..=12).contains(&month) && (1..=31).contains(&day) {
        Some((month, day))
    } else {
        None
    }
}

fn parse_time_str(input: &str) -> Option<NaiveTime> {
    let input = input.trim();
    if let Ok(time) = NaiveTime::parse_from_str(input, "%H:%M") {
        return Some(time);
    }
    if let Ok(time) = NaiveTime::parse_from_str(input, "%H:%M:%S") {
        return Some(time);
    }
    if let Some((hour_str, min_str)) = input.split_once(':') {
        let hour: u32 = hour_str.parse().ok()?;
        let min: u32 = min_str.parse().ok()?;
        return NaiveTime::from_hms_opt(hour, min, 0);
    }
    None
}

pub fn parse_date(input: &str) -> Result<NaiveDate> {
    let input = input.trim().to_lowercase();
    let today = Local::now().date_naive();

    match input.as_str() {
        "today" => Ok(today),
        "yesterday" => Ok(today - chrono::Duration::days(1)),
        _ => NaiveDate::parse_from_str(&input, "%Y-%m-%d")
            .map_err(|_| TimeParseError::InvalidFormat(input)),
    }
}

pub fn parse_period(period: &str) -> Result<(NaiveDate, NaiveDate)> {
    let period = period.trim().to_lowercase();
    let today = Local::now().date_naive();

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
    use chrono::Timelike;

    #[test]
    fn test_parse_relative_time_now() {
        let now = Local::now();
        let parsed = parse_relative_time("now").unwrap();
        let diff = (parsed - now.with_timezone(&Utc)).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_hours_ago() {
        let result = parse_relative_time("2h ago").unwrap();
        let expected = Local::now() - chrono::Duration::hours(2);
        let diff = (result - expected.with_timezone(&Utc)).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_minutes_ago() {
        let result = parse_relative_time("30m ago").unwrap();
        let expected = Local::now() - chrono::Duration::minutes(30);
        let diff = (result - expected.with_timezone(&Utc)).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_days_ago() {
        let result = parse_relative_time("2d ago").unwrap();
        let expected = Local::now() - chrono::Duration::days(2);
        let diff = (result - expected.with_timezone(&Utc)).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_hour_only() {
        let result = parse_relative_time("2h").unwrap();
        let expected = Local::now() - chrono::Duration::hours(2);
        let diff = (result - expected.with_timezone(&Utc)).num_seconds().abs();
        assert!(diff < 2);
    }

    #[test]
    fn test_parse_relative_time_invalid() {
        assert!(parse_relative_time("invalid").is_err());
    }

    #[test]
    fn test_parse_relative_time_hm_without_leading_zero() {
        let result = parse_relative_time("9:30").unwrap();
        let local_result = result.with_timezone(&Local);
        assert_eq!(local_result.hour(), 9);
        assert_eq!(local_result.minute(), 30);
    }

    #[test]
    fn test_parse_date_today() {
        let result = parse_date("today").unwrap();
        let today = Local::now().date_naive();
        assert_eq!(result, today);
    }

    #[test]
    fn test_parse_date_yesterday() {
        let result = parse_date("yesterday").unwrap();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);
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
        let today = Local::now().date_naive();
        assert_eq!(start, today);
        assert_eq!(end, today);
    }

    #[test]
    fn test_parse_period_week() {
        let (start, end) = parse_period("week").unwrap();
        let today = Local::now().date_naive();
        assert_eq!(end, today);
        assert_eq!((end - start).num_days(), 6);
    }

    #[test]
    fn test_parse_period_month() {
        let (start, end) = parse_period("month").unwrap();
        let today = Local::now().date_naive();
        assert_eq!(end, today);
        assert_eq!((end - start).num_days(), 29);
    }

    #[test]
    fn test_parse_month_day_time() {
        let result = parse_relative_time("3/22 11:30").unwrap();
        let local_result = result.with_timezone(&Local);
        assert_eq!(local_result.month(), 3);
        assert_eq!(local_result.day(), 22);
        assert_eq!(local_result.hour(), 11);
        assert_eq!(local_result.minute(), 30);
    }

    #[test]
    fn test_parse_month_day_only() {
        let result = parse_relative_time("3/22").unwrap();
        let local_result = result.with_timezone(&Local);
        assert_eq!(local_result.month(), 3);
        assert_eq!(local_result.day(), 22);
        assert_eq!(local_result.hour(), 0);
        assert_eq!(local_result.minute(), 0);
    }
}
