//! Voice capture commands.
//!
//! The backend owns the microphone (see `audio::session`); the UI only
//! starts/stops and renders the `capture-*` events. The tray and the global
//! hotkey drive the same session, so every window stays in sync.

use std::sync::Arc;

use tauri::State;

use crate::audio::session::{self, CaptureMode, CaptureStatus};
use crate::engine::pipeline::CaptureResult;
use crate::error::Result;
use crate::state::AppState;

/// Start recording: a brain-dump (mic) or meeting notes (mic + call audio).
#[tauri::command]
pub fn start_capture(
    state: State<'_, Arc<AppState>>,
    language: Option<String>,
    mode: Option<CaptureMode>,
) -> Result<()> {
    let language = language.unwrap_or_else(|| crate::commands::settings::current_language(&state));
    session::start(state.inner(), &language, mode.unwrap_or(CaptureMode::Note))
}

/// Stop recording and organize. Resolves when the result is ready; stages
/// stream over `capture-stage` meanwhile.
#[tauri::command]
pub async fn stop_capture(state: State<'_, Arc<AppState>>) -> Result<CaptureResult> {
    session::stop(state.inner().clone()).await
}

/// Discard the current recording.
#[tauri::command]
pub fn cancel_capture(state: State<'_, Arc<AppState>>) -> Result<()> {
    session::cancel(state.inner())
}

/// Whether a capture is running, for windows that open mid-recording.
#[tauri::command]
pub fn capture_status(state: State<'_, Arc<AppState>>) -> CaptureStatus {
    session::status(&state)
}

/// Open the full Nodemind window, optionally on a specific view.
#[tauri::command]
pub fn open_main_window(app: tauri::AppHandle, view: Option<String>) {
    crate::tray::show_main(&app, view.as_deref());
}
