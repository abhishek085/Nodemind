//! Nodemind — app entry point (library).
//!
//! Assembly line: plugins → state (Arc) → prompts → engine boot → tray →
//! window. Everything after first launch is fully offline.

mod calendar;
mod commands;
mod engine;
mod error;
mod models;
mod prompts;
mod state;
mod store;
mod time;

use std::sync::Arc;

use tauri::menu::MenuEvent;
use tauri::{App, AppHandle, Manager, RunEvent};

/// Builds the application and runs the event loop.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            setup_tray(app, state.clone());

            // Boot (or start first-launch download) in the background.
            models::begin_setup(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // engine
            commands::engine::engine_status,
            commands::engine::begin_setup,
            commands::engine::restart_engine,
            // capture
            commands::capture::begin_capture,
            commands::capture::append_audio,
            commands::capture::finish_capture,
            // thoughts / tasks / graph
            commands::thoughts::get_thoughts,
            commands::thoughts::get_thought,
            commands::thoughts::delete_thought,
            commands::thoughts::get_tasks,
            commands::thoughts::add_task,
            commands::thoughts::update_task,
            commands::thoughts::delete_task,
            commands::thoughts::get_graph,
            commands::thoughts::search_thoughts,
            // calendar
            commands::calendar::sync_calendar,
            // today
            commands::today::get_today_brief,
            commands::today::start_focus,
            commands::today::finish_focus,
            commands::today::get_focus_today,
            // settings
            commands::settings::get_settings,
            commands::settings::set_language,
            commands::settings::open_data_dir,
            commands::settings::reindex_embeddings,
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

/// Menu-bar presence: a quiet indicator of engine health.
fn setup_tray(app: &App, state: Arc<state::AppState>) {
    use tauri::menu::{MenuItemBuilder, MenuBuilder};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("default window icon");

    // Keep a handle to the status line — tauri 2 removed lookup of menu
    // items by id, so we hold the shared handle instead.
    let status = MenuItemBuilder::new("Nodemind — starting…")
        .build(app)
        .expect("status menu item");

    let on_menu_event = |app: &AppHandle, event: MenuEvent| match event.id().as_ref() {
        "show" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }
        "restart" => {
            let arc = app.state::<Arc<state::AppState>>().inner().clone();
            engine::shutdown(&arc);
            engine::boot(arc);
        }
        "quit" => app.exit(0),
        _ => {}
    };

    let menu = MenuBuilder::new(app)
        .item(&status)
        .separator()
        .text("show", "Show Nodemind")
        .text("restart", "Restart engine")
        .separator()
        .quit()
        .build()
        .expect("tray menu");

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Nodemind")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                // Left-click pops the window.
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)
        .expect("tray icon");

    // Keep the tray label honest as the engine moves between phases.
    let s = state.clone();
    let status_line = status.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let phase = s.engine_status().phase;
            let label = match phase {
                state::EnginePhase::Setup => "Nodemind — needs setup",
                state::EnginePhase::Starting => "Nodemind — starting engine…",
                state::EnginePhase::Ready => "Nodemind — ready",
                state::EnginePhase::Failed => "Nodemind — engine error",
            };
            if let Some(tray) = s.app.tray_by_id("main-tray") {
                let _ = tray.set_tooltip(Some(label));
            }
            let _ = status_line.set_text(label);
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}
