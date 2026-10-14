//! Menu-bar shell: the tray icon, the quick-capture popover, and the main
//! window's show/hide lifecycle.
//!
//! Nodemind lives in the menu bar. Left-click the icon for the popover
//! (record + review); the full window — mind map, Today, settings — opens on
//! demand, and the Dock icon only shows while it is open.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{ActivationPolicy, App, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::engine::pipeline::CaptureResult;
use crate::state::{AppState, EnginePhase};

pub const TRAY_ID: &str = "main-tray";
pub const QUICK: &str = "quick";
pub const MAIN: &str = "main";
const QUICK_SIZE: (f64, f64) = (360.0, 440.0);

/// When the popover last hid itself on blur. Clicking the tray icon while
/// the popover is open blurs it first; without this the click would
/// immediately re-open it.
static LAST_QUICK_HIDE: Mutex<Option<Instant>> = Mutex::new(None);

pub enum TrayStatus {
    Idle,
    Recording(u64),
    Processing,
}

/// Reflect capture state next to the menu-bar icon ("● 0:12", "…").
pub fn set_status(state: &AppState, status: TrayStatus) {
    let Some(tray) = state.app.tray_by_id(TRAY_ID) else {
        return;
    };
    let title = match status {
        TrayStatus::Idle => None,
        TrayStatus::Recording(s) => Some(format!("● {}:{:02}", s / 60, s % 60)),
        TrayStatus::Processing => Some("…".to_string()),
    };
    let _ = tray.set_title(title);
}

/// After a capture finishes with no Nodemind window in view, say what
/// happened with a native notification instead of popping a window.
pub fn notify_if_hidden(state: &AppState, result: &CaptureResult) {
    let visible = [QUICK, MAIN].iter().any(|label| {
        state
            .app
            .get_webview_window(label)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false)
    });
    if visible {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    let body = match result.tasks.len() {
        _ if result.minutes.is_some() => format!(
            "Meeting notes ready: {}{}",
            result.minutes.as_ref().map(|m| m.title.as_str()).unwrap_or("meeting"),
            if result.tasks.is_empty() { String::new() } else { format!(" · {} action items added to Today", result.tasks.len()) }
        ),
        0 if result.transcript.is_empty() => "Didn't catch any speech.".to_string(),
        0 => "Saved to your mind map.".to_string(),
        n => {
            let titles: Vec<&str> = result.tasks.iter().take(3).map(|t| t.title.as_str()).collect();
            format!("Added {n} to Today: {}", titles.join(" · "))
        }
    };
    let _ = state.app.notification().builder().title("Nodemind").body(body).show();
}

/// Build the tray icon, the hidden popover window, and the menu.
pub fn setup(app: &mut App, state: Arc<AppState>) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(ActivationPolicy::Accessory);

    // The popover is created hidden at launch so it opens instantly.
    let quick = WebviewWindowBuilder::new(app, QUICK, WebviewUrl::default())
        .title("Nodemind")
        .inner_size(QUICK_SIZE.0, QUICK_SIZE.1)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()?;
    let q = quick.clone();
    quick.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let _ = q.hide();
            if let Ok(mut t) = LAST_QUICK_HIDE.lock() {
                *t = Some(Instant::now());
            }
        }
    });

    // Closing the main window hides it and returns to menu-bar-only mode.
    if let Some(main) = app.get_webview_window(MAIN) {
        let handle = app.handle().clone();
        let m = main.clone();
        main.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = m.hide();
                #[cfg(target_os = "macos")]
                let _ = handle.set_activation_policy(ActivationPolicy::Accessory);
            }
        });
    }

    let status = MenuItemBuilder::new("Nodemind — starting…").enabled(false).build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&status)
        .separator()
        .text("record", "Start / stop recording   ⌥⇧Space")
        .text("meeting", "Start meeting notes")
        .text("open", "Open Nodemind")
        .text("map", "Mind map")
        .separator()
        .text("restart", "Restart engine")
        .quit()
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;
    let s = state.clone();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Nodemind")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "record" => crate::audio::session::toggle(&s, crate::audio::session::CaptureMode::Note),
            "meeting" => crate::audio::session::toggle(&s, crate::audio::session::CaptureMode::Meeting),
            "open" => show_main(app, None),
            "map" => show_main(app, Some("graph")),
            "restart" => {
                crate::engine::shutdown(&s);
                crate::engine::boot(s.clone());
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_quick(tray.app_handle(), rect);
            }
        })
        .build(app)?;

    // Keep the menu's status line honest as the engine moves between phases.
    tauri::async_runtime::spawn(async move {
        loop {
            let label = match state.engine_status().phase {
                EnginePhase::Setup => "Nodemind — needs setup",
                EnginePhase::Starting => "Nodemind — starting engine…",
                EnginePhase::Ready => "Nodemind — ready",
                EnginePhase::Failed => "Nodemind — engine error",
            };
            let _ = status.set_text(label);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Ok(())
}

/// Show the popover just under the menu-bar icon, or hide it.
fn toggle_quick(app: &AppHandle, rect: tauri::Rect) {
    let Some(quick) = app.get_webview_window(QUICK) else {
        return;
    };
    if quick.is_visible().unwrap_or(false) {
        let _ = quick.hide();
        return;
    }
    let just_hidden = LAST_QUICK_HIDE
        .lock()
        .ok()
        .and_then(|t| *t)
        .is_some_and(|t| t.elapsed() < Duration::from_millis(250));
    if just_hidden {
        return; // this click is what blurred (and hid) it
    }
    let scale = quick.scale_factor().unwrap_or(1.0);
    let icon_pos = rect.position.to_physical::<f64>(scale);
    let icon_size = rect.size.to_physical::<f64>(scale);
    let width = QUICK_SIZE.0 * scale;
    let x = icon_pos.x + icon_size.width / 2.0 - width / 2.0;
    let y = icon_pos.y + icon_size.height + 4.0 * scale;
    let _ = quick.set_position(tauri::PhysicalPosition::new(x.max(0.0), y));
    let _ = quick.show();
    let _ = quick.set_focus();
}

/// Open the full window (optionally on a given view) and show the Dock icon.
pub fn show_main(app: &AppHandle, view: Option<&str>) {
    if let Some(quick) = app.get_webview_window(QUICK) {
        let _ = quick.hide();
    }
    let Some(main) = app.get_webview_window(MAIN) else {
        return;
    };
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(ActivationPolicy::Regular);
    let _ = main.show();
    let _ = main.unminimize();
    let _ = main.set_focus();
    if let Some(view) = view {
        let _ = main.emit("navigate", view);
    }
}
