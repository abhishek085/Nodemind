//! Tiny time helpers shared across modules.

use chrono::{Local, TimeZone};

/// Unix time in milliseconds.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Unix time in seconds.
pub fn now_secs() -> i64 {
    (now_millis() / 1000) as i64
}

/// Local date as `YYYY-MM-DD`.
pub fn local_day_key() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Parse an osascript date string, e.g. "Wed Oct 13 09:00:00 2026".
///
/// The weekday prefix is stripped first — chrono 0.4.4x cannot match `%a`/`%A`
/// (locale-name parsing is broken for it), but month-name formats work.
pub fn parse_mac_date(s: &str) -> Option<chrono::DateTime<Local>> {
    const WEEKDAYS: &[&str] = &[
        "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun",
        "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
    ];
    let t = s
        .split_whitespace()
        .filter(|w| !WEEKDAYS.contains(w))
        .collect::<Vec<_>>()
        .join(" ");
    for fmt in ["%b %e %H:%M:%S %Y", "%B %e %H:%M:%S %Y", "%b %e %I:%M:%S %p %Y"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&t, fmt) {
            if let Some(d) = Local.from_local_datetime(&naive).single() {
                return Some(d);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osascript_date() {
        let d = parse_mac_date("Wed Oct 13 09:30:00 2026").expect("should parse");
        assert_eq!(d.format("%Y-%m-%d %H:%M").to_string(), "2026-10-13 09:30");
    }

    #[test]
    fn day_key_format() {
        assert_eq!(local_day_key().len(), 10);
    }
}
