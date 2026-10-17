//! Global capture hotkey: ⌥⇧Space, from any app.
//!
//! - Hold it while you talk, release to finish (push-to-talk).
//! - Or tap it once to start and tap again to stop (hands-free).
//!
//! A press longer than `HOLD` counts as push-to-talk; anything shorter is a tap.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{Manager, Runtime};
use tauri_plugin_global_shortcut::{Builder, ShortcutState};

use crate::audio::session::{self, CaptureMode};
use crate::state::AppState;

pub const SHORTCUT: &str = "Alt+Shift+Space";
const HOLD: Duration = Duration::from_millis(450);

/// When the current hotkey press began, if it started a capture.
static PRESSED_AT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let builder = Builder::new().with_handler(|app, _shortcut, event| {
        let Some(state) = app.try_state::<Arc<AppState>>() else {
            return;
        };
        on_event(state.inner(), event.state);
    });
    match builder.with_shortcut(SHORTCUT) {
        Ok(b) => b.build(),
        Err(e) => {
            eprintln!("[hotkey] cannot register {SHORTCUT}: {e}");
            Builder::new().build()
        }
    }
}

fn on_event(state: &Arc<AppState>, key: ShortcutState) {
    let mut pressed = match PRESSED_AT.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    match key {
        ShortcutState::Pressed => {
            let recording = session::is_recording(state);
            session::toggle(state, CaptureMode::Note);
            // Only a press that *started* recording can become push-to-talk.
            *pressed = (!recording).then(Instant::now);
        }
        ShortcutState::Released => {
            if let Some(at) = pressed.take() {
                if at.elapsed() >= HOLD && session::is_recording(state) {
                    session::toggle(state, CaptureMode::Note);
                }
            }
        }
    }
}
