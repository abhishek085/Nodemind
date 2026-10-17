//! Plain-text app logs in app-data/logs/, for diagnosing problems after the
//! fact (a GUI app's stderr goes nowhere once it is launched from Finder).
//!
//! - `app.log` — capture lifecycle (start, stop, organize timings, errors)
//! - `ui.log`  — errors reported by the webviews (see `log_ui_error`)

use std::io::Write;

use crate::state::AppState;

/// Append one timestamped line to `logs/<file>` and mirror it to stderr.
pub fn write(state: &AppState, file: &str, line: &str) {
    eprintln!("[{file}] {line}");
    let Some(dir) = state.models_dir.parent().map(|d| d.join("logs")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(file)) {
        let _ = writeln!(f, "{} {line}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
    }
}
