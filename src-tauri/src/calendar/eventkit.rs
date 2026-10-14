//! Calendar via EventKit, through the Swift `nodemind-helper calendar` sidecar.
//!
//! Instant and reads every calendar account (iCloud, Google, Exchange)
//! without driving Calendar.app. `apple` (AppleScript) is the fallback when
//! the helper is absent.

use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::store::db::CalendarEvent;

#[derive(Deserialize)]
struct HelperEvent {
    id: String,
    title: String,
    start: String,
    end: String,
    calendar: String,
}

/// Outcome of asking the helper; `Unavailable` means "use the fallback".
pub enum Fetch {
    Events(Vec<CalendarEvent>),
    Unavailable,
}

/// Events for a local day (`YYYY-MM-DD`). Blocking — call off the async thread.
pub fn events_for_day(helper: &Path, day: &str) -> Result<Fetch> {
    if !helper.exists() {
        return Ok(Fetch::Unavailable);
    }
    let out = std::process::Command::new(helper)
        .args(["calendar", day])
        .output()
        .map_err(|e| Error::Calendar(format!("calendar helper: {e}")))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // 2 = access denied: report it rather than trying AppleScript, which
        // would just ask for a second (Automation) permission.
        return match out.status.code() {
            Some(2) => Err(Error::Calendar(msg)),
            _ => {
                eprintln!("[calendar] helper failed ({msg}), falling back to AppleScript");
                Ok(Fetch::Unavailable)
            }
        };
    }
    let parsed: Vec<HelperEvent> = serde_json::from_slice(&out.stdout)?;
    Ok(Fetch::Events(parse(parsed, day)))
}

fn parse(events: Vec<HelperEvent>, day: &str) -> Vec<CalendarEvent> {
    events
        .into_iter()
        .filter(|e| !e.title.trim().is_empty())
        .map(|e| CalendarEvent {
            id: e.id,
            title: e.title.trim().to_string(),
            start: e.start,
            end: e.end,
            calendar: e.calendar,
            day: day.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_helper_json() {
        let raw = r#"[{"id":"a","title":"Standup","start":"2026-10-14T09:30:00-05:00","end":"2026-10-14T09:45:00-05:00","calendar":"Work","allDay":false},
                      {"id":"b","title":" ","start":"x","end":"y","calendar":"Home","allDay":true}]"#;
        let events = parse(serde_json::from_str(raw).unwrap(), "2026-10-14");
        assert_eq!(events.len(), 1, "untitled events are skipped");
        assert_eq!(events[0].title, "Standup");
        assert!((crate::calendar::apple::scheduled_hours(&events) - 0.25).abs() < 1e-9);
    }
}
