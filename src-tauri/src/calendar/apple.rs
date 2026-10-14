//! Apple Calendar integration via `osascript` (AppleScript).
//!
//! This is the "zero setup" native bridge the design calls for: the first
//! read triggers macOS's standard *Apple Events* permission prompt for the
//! Calendar app. No helper processes, no TCC hacks — just the system dialog.
//!
//! Only *reads* are performed. Nodemind never writes to your calendar.

use std::process::Command;

use crate::error::Result;
use crate::store::db::CalendarEvent;
use crate::time;

/// The AppleScript used for a one-day read. Dates are interpolated as
/// controlled values (we generate them; they never come from user input).
fn script(start: &str, end: &str) -> String {
    format!(
        r#"
tell application "Calendar"
    set sb to ""
    repeat with cal in (every calendar)
        repeat with ev in (events of cal whose start date ≥ date "{start}" and start date < date "{end}")
            set sb to sb & (summary of ev) & "\t" & ((start date of ev) as string) & "\t" & ((end date of ev) as string) & "\t" & (name of cal) & linefeed
        end repeat
    end repeat
    return sb
end tell
"#,
        start = start, end = end
    )
}

/// Fetch all events for a local day (`YYYY-MM-DD`).
///
/// Returns the events sorted by start time. The first call in a session may
/// show the system permission prompt — that is expected and required.
pub fn events_for_day(day: &str) -> Result<Vec<CalendarEvent>> {
    // Window: [day 00:00, next day 00:00) in local time.
    let start_dt = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|e| crate::error::Error::Calendar(format!("bad day '{day}': {e}")))?;
    let start = start_dt
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| crate::error::Error::Calendar("bad day".into()))?;
    let end = start + chrono::Duration::days(1);
    let start_s = start.format("%a %b %e %H:%M:%S %Y").to_string();
    let end_s = end.format("%a %b %e %H:%M:%S %Y").to_string();

    let output = Command::new("osascript")
        .arg("-e")
        .arg(script(&start_s, &end_s))
        .output()
        .map_err(|e| crate::error::Error::Calendar(format!("osascript unavailable: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim().to_string();
        // The common failure modes, with helpful hints:
        let hint = if msg.contains("not allowed to send apple events") || msg.contains("-1743") {
            format!(
                "{msg} — Grant Calendar access in System Settings → Privacy & Security → Automation (or try again to re-trigger the prompt)."
            )
        } else if msg.contains("Can't get") || msg.contains("-1728") {
            format!("{msg} — the Calendar app may not be installed or accessible.")
        } else if msg.is_empty() {
            String::new() // success with zero events also exits 0 with empty stdout
        } else {
            format!("{msg}")
        };
        if !hint.is_empty() {
            return Err(crate::error::Error::Calendar(hint));
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut events = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(4, '\t');
        let (title, start_raw, end_raw, cal) = match (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => continue,
        };
        let start = time::parse_mac_date(start_raw)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| start_raw.to_string());
        let end = time::parse_mac_date(end_raw)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| end_raw.to_string());
        let title = title.trim();
        if title.is_empty() {
            continue;
        }
        events.push(CalendarEvent {
            id: format!("{day}:{start}"),
            title: title.to_string(),
            start,
            end,
            calendar: cal.trim().to_string(),
            day: day.to_string(),
        });
    }
    events.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(events)
}

/// Total scheduled hours for a day, from cached events.
pub fn scheduled_hours(events: &[CalendarEvent]) -> f64 {
    events
        .iter()
        .map(|e| {
            let s = chrono::DateTime::parse_from_rfc3339(&e.start)
                .and_then(|d| chrono::DateTime::parse_from_rfc3339(&e.end).map(|e2| (e2 - d).num_milliseconds() as f64 / 3_600_000.0))
                .unwrap_or(0.0);
            s.clamp(0.0, 24.0)
        })
        .sum()
}
