//! Today commands: the friction brief + focus sessions.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::engine::llm;
use crate::error::Result;
use crate::state::AppState;
use crate::store::db::{self, FocusSession};
use crate::time;

/// The daily "friction" insight for the Today view.
///
/// Compares the mind's activity (thoughts, open tasks) against the day's
/// structure (calendar, focus). Falls back to a rule-based insight when the
/// LLM isn't ready, so Today always works.
#[tauri::command]
pub async fn get_today_brief(state: State<'_, Arc<AppState>>) -> Result<TodayBrief> {
    let day = time::local_day_key();
    let (open_tasks, thoughts, events, focus_minutes) = {
        let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
        let tasks = db::get_tasks(&conn, false).unwrap_or_default();
        let thoughts = db::get_thoughts_for_day(&conn, &day).unwrap_or_default();
        let events = db::get_calendar_day(&conn, &day).unwrap_or_default();
        let focus = db::focus_minutes_today(&conn, &day);
        (
            tasks,
            thoughts,
            events,
            focus,
        )
    };

    let scheduled_hours = crate::calendar::apple::scheduled_hours(&events);
    let task_list = open_tasks
        .iter()
        .take(12)
        .map(|t| format!("\"{}\"", t.title))
        .collect::<Vec<_>>()
        .join(", ");

    // Recent fog signals (from the meta table — written by the pipeline).
    let fog_json = state
        .db
        .lock()
        .map_err(|_| crate::error::Error::Store("db".into()))?
        .query_row(
            "SELECT value FROM meta WHERE key = 'fog_today'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "[]".to_string());

    let input = llm::BriefInput {
        open_tasks: open_tasks.len(),
        open_task_list: if task_list.is_empty() { "(none)".into() } else { task_list },
        thoughts_today: thoughts.len(),
        events_today: events.len(),
        scheduled_hours,
        focus_minutes,
        fog_json,
    };

    let llm_brief = llm::daily_brief(input).await.ok();

    let (friction, suggestion) = match llm_brief {
        Some(b) if b.friction.is_some() || b.suggestion.is_some() => (b.friction, b.suggestion),
        _ => rule_based_brief(&open_tasks, thoughts.len(), events.len(), scheduled_hours, focus_minutes),
    };

    state.emit(
        "today-brief",
        &serde_json::json!({ "friction": friction, "suggestion": suggestion }),
    );

    Ok(TodayBrief {
        friction,
        suggestion,
        open_tasks: open_tasks.len(),
        thoughts_today: thoughts.len(),
        events_today: events.len(),
        scheduled_hours,
        focus_minutes,
    })
}

/// Deterministic fallback when the LLM can't answer.
fn rule_based_brief(
    open_tasks: &[crate::store::db::Task],
    thoughts: usize,
    events: usize,
    scheduled_hours: f64,
    focus_minutes: i64,
) -> (Option<String>, Option<String>) {
    let loud_mind = thoughts >= 3 || open_tasks.len() >= 4;
    let empty_day = events == 0 && focus_minutes == 0;

    let friction = if loud_mind && empty_day {
        Some(format!(
            "Your mind is loud ({thoughts} brain-dumps, {} open tasks) but the day is empty — 0 h scheduled, 0 min focused.",
            open_tasks.len()
        ))
    } else if loud_mind && scheduled_hours >= 6.0 {
        Some(format!(
            "{scheduled_hours:.0} h of a day already committed while {} tasks wait — the calendar is outpacing the checklist.",
            open_tasks.len()
        ))
    } else {
        None
    };

    let suggestion = if let Some(task) = open_tasks.first() {
        Some(format!("Clear the top of the list: \"{}\"", truncate(&task.title, 40)))
    } else if empty_day {
        Some("Carve one 25-minute focus block — the mind settles when it has a container.".into())
    } else {
        Some("You're balanced. Ride the momentum.".into())
    };

    (friction, suggestion)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayBrief {
    pub friction: Option<String>,
    pub suggestion: Option<String>,
    pub open_tasks: usize,
    pub thoughts_today: usize,
    pub events_today: usize,
    pub scheduled_hours: f64,
    pub focus_minutes: i64,
}

// ── Focus sessions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusStarted {
    pub id: String,
    pub started_at: String,
    pub minutes: i64,
}

#[tauri::command]
pub fn start_focus(state: State<'_, Arc<AppState>>, minutes: Option<u32>) -> Result<FocusStarted> {
    let minutes = minutes.unwrap_or(25) as i64;
    let session = FocusSession {
        id: uuid::Uuid::new_v4().to_string(),
        title: None,
        day: time::local_day_key(),
        started_at: chrono::Utc::now().to_rfc3339(),
        ended_at: None,
        minutes,
    };
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::insert_focus(&conn, &session)?;
    Ok(FocusStarted {
        id: session.id,
        started_at: session.started_at,
        minutes,
    })
}

#[tauri::command]
pub fn finish_focus(state: State<'_, Arc<AppState>>, id: String, minutes: Option<i64>) -> Result<()> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    let ended = chrono::Utc::now().to_rfc3339();
    db::finish_focus(&conn, &id, &ended, minutes.unwrap_or(0))?;
    state.emit("focus-updated", &serde_json::json!({}));
    Ok(())
}

#[tauri::command]
pub fn get_focus_today(state: State<'_, Arc<AppState>>) -> Result<Vec<FocusSession>> {
    let conn = state.db.lock().map_err(|_| crate::error::Error::Store("db".into()))?;
    db::get_focus_today(&conn, &time::local_day_key())
}
