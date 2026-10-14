//! Auto-update commands (Tauri updater plugin).
//!
//! The updater checks the configured release endpoint (GitHub Releases) and,
//! on a valid signed release, offers a one-click download + restart.
//! Releasing: `scripts/release.sh` signs artifacts with the private key.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

use crate::error::Result;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// Check the release endpoint for a newer signed build.
/// Returns `None` when up to date (or the endpoint is unreachable).
/// Skipped in dev builds, which can't install updates anyway.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<UpdateInfo>> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    let updater = app.updater().map_err(|e| crate::error::Error::Engine(e.to_string()))?;
    match updater.check().await {
        Ok(Some(update)) => {
            eprintln!("[updater] new version available: {}", update.version);
            Ok(Some(UpdateInfo {
                version: update.version.clone(),
                notes: update.body.clone(),
            }))
        }
        Ok(None) => Ok(None),
        Err(e) => {
            // Not fatal: offline machines simply never see updates.
            eprintln!("[updater] check failed: {e}");
            Ok(None)
        }
    }
}

/// Download the update, install it, and relaunch.
///
/// On macOS the plugin replaces the .app bundle; we then start the new build
/// from the same executable path and exit the old process.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<()> {
    let updater = app.updater().map_err(|e| crate::error::Error::Engine(e.to_string()))?;
    match updater.check().await {
        Ok(Some(update)) => {
            update
                .download_and_install(
                    |done: usize, _total: Option<u64>| {
                        eprintln!("[updater] downloaded {done} bytes");
                    },
                    || eprintln!("[updater] download finished"),
                )
                .await
                .map_err(|e| crate::error::Error::Engine(e.to_string()))?;
            relaunch(&app)
        }
        Ok(None) => Err(crate::error::Error::Engine("no update available".into())),
        Err(e) => Err(crate::error::Error::Engine(e.to_string())),
    }
}

/// Spawn the freshly-installed app bundle, then exit this process.
fn relaunch(_app: &AppHandle) -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(&exe).spawn();
    } else {
        eprintln!("[updater] cannot resolve current executable — not relaunching");
    }
    // Give the child a moment to detach before we go.
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::process::exit(0);
}
