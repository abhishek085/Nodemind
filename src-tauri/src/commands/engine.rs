//! Engine lifecycle commands (setup, status, restart).
//!
//! `AppState` is managed as `Arc<AppState>` (see `lib.rs`) so async commands
//! can clone the handle and hand it to background tasks.

use std::sync::Arc;

use tauri::State;

use crate::error::Result;
use crate::state::{AppState, EngineStatus};

/// Current engine snapshot (phase, arch, models, server health).
#[tauri::command]
pub fn engine_status(state: State<'_, Arc<AppState>>) -> EngineStatus {
    state.engine_status()
}

/// Start first-launch setup: download any missing models, then boot the
/// servers. Returns immediately; progress streams over `model-progress` and
/// `engine-status` events.
#[tauri::command]
pub async fn begin_setup(state: State<'_, Arc<AppState>>) -> Result<()> {
    let s = state.inner().clone();
    crate::models::begin_setup(s);
    Ok(())
}

/// Re-verify models and (re)start the sidecars.
#[tauri::command]
pub async fn restart_engine(state: State<'_, Arc<AppState>>) -> Result<()> {
    let s = state.inner().clone();
    crate::engine::shutdown(&s);
    crate::engine::boot(s);
    Ok(())
}
