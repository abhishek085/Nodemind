//! Calendar command — reads today's Apple Calendar events into the local cache.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::calendar::apple;
use crate::error::Result;
use crate::state::AppState;
use crate::store::db::{self, CalendarEvent};
use crate::time;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSync {
    pub day: String,
    pub events: Vec<CalendarEvent>,
    pub scheduled_hours: f64,
    /// present when macOS blocked the Apple-Events read
    pub permission_hint: Option<String>,
}

/// Read today's events from Apple Calendar (may trigger the system permission
/// prompt on first use) and cache them locally.
#[tauri::command]
pub async fn sync_calendar(state: State<'_, Arc<AppState>>) -> Result<CalendarSync> {
    let day = time::local_day_key();

    // AppleScript is a blocking subprocess — off the async thread.
    let result = tauri::async_runtime::spawn_blocking({
        let day = day.clone();
        move || apple::events_for_day(&day)
    })
    .await
    .map_err(|e| crate::error::Error::Calendar(format!("calendar task: {e}")))?;

    let (events, permission_hint) = match result {
        Ok(e) => (e, None),
        Err(e) => {
            // Surface the error as a hint, not a failure — the rest of the
            // Today view still works without calendar data.
            return Ok(CalendarSync {
                day,
                events: Vec::new(),
                scheduled_hours: 0.0,
                permission_hint: Some(e.message()),
            });
        }
    };

    {
        let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
        let _ = db::replace_calendar_day(&conn, &day, &events);
    }
    state.emit(
        "calendar-synced",
        &serde_json::json!({ "day": day, "count": events.len() }),
    );

    Ok(CalendarSync {
        day,
        scheduled_hours: apple::scheduled_hours(&events),
        permission_hint,
        events,
    })
}
