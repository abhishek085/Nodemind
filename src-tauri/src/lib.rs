//! Nodemind — app entry point (library).
//!
//! Assembly line: plugins → state (Arc) → prompts → engine boot → tray →
//! window. Everything after first launch is fully offline.
//!
//! Nodemind runs from the menu bar (see `tray`); the main window opens on
//! demand, or at launch only when first-run setup still has to happen.

mod applog;
mod audio;
mod calendar;
mod commands;
mod engine;
mod error;
mod hotkey;
mod models;
mod prompts;
#[cfg(debug_assertions)]
mod selftest;
mod state;
mod store;
mod time;
mod tray;

use std::sync::Arc;

use tauri::{Manager, RunEvent};

/// Builds the application and runs the event loop.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(hotkey::plugin())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = Arc::new(
                state::AppState::init(handle).expect(
                    "cannot initialise app state — is the app data dir writable?",
                ),
            );

            // Prompts: runtime file from the bundle, embedded fallback.
            // Synchronous — tiny TOML; the engine may use them immediately.
            prompts::init(&state.resources_dir);

            app.manage(state.clone());
            tray::setup(app, state.clone())?;
            // First run downloads the models — that needs the full window.
            if !models::models_ready(&state) {
                tray::show_main(app.handle(), None);
            }

            // SIGTERM / Ctrl-C: shut the servers down and exit cleanly.
            let for_signal = state.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(t) => t,
                    Err(_) => return,
                };
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
                engine::shutdown(&for_signal);
                handle.exit(0);
            });

            // Boot (or start first-launch download) in the background.
            models::begin_setup(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // engine
            commands::engine::engine_status,
            commands::engine::begin_setup,
            // capture
            commands::capture::start_capture,
            commands::capture::stop_capture,
            commands::capture::cancel_capture,
            commands::capture::capture_status,
            commands::capture::open_main_window,
            // thoughts / tasks / graph
            commands::thoughts::get_thought,
            commands::thoughts::delete_thought,
            commands::thoughts::add_task,
            commands::thoughts::update_task,
            commands::thoughts::delete_task,
            commands::thoughts::get_graph,
            commands::thoughts::get_today,
            commands::thoughts::search_memory,
            // calendar
            commands::calendar::sync_calendar,
            // settings
            commands::settings::get_settings,
            commands::settings::set_language,
            commands::settings::open_data_dir,
            commands::settings::reindex_embeddings,
            commands::settings::log_ui_error,
            // updates
            commands::updates::check_for_updates,
            commands::updates::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Nodemind");

    app.run(|app, event| {
        if matches!(event, RunEvent::Exit) {
            // Kill the llama-servers before we go.
            let arc = app.state::<Arc<state::AppState>>().inner().clone();
            engine::shutdown(&arc);
        }
    });
}
