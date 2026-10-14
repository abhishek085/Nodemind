//! Global application state shared across all Tauri commands.
//!
//! Layout follows the design doc:
//!   audio (CoreAudio, backend) → engine (sidecars) → store (SQLite + vectors) → UI
//!
//! Everything in this struct is `Send + Sync` so it can live in `tauri::State`.

use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use rusqlite::Connection;
use tauri::Emitter;
use tauri::Manager;

use tauri::AppHandle;

use crate::models::ModelKey;
use crate::store::db::open_database;

/// Status of a single bundled model, as seen by the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusView {
    pub key: String,
    /// human label, e.g. "Whisper (speech → text)"
    pub label: String,
    pub present: bool,
    /// where the model currently lives, if anywhere
    pub path: Option<String>,
    pub size_bytes: u64,
    /// expected size from the manifest (0 = unknown)
    pub expected_bytes: u64,
    pub downloading: bool,
    /// bytes downloaded so far while `downloading`
    pub downloaded_bytes: u64,
    pub error: Option<String>,
}

/// Coarse engine state for the UI status dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EnginePhase {
    /// Before first launch: models missing, nothing running.
    Setup,
    /// Models present, sidecars starting up.
    Starting,
    /// Both servers healthy — the app is fully functional.
    Ready,
    /// Something failed; details in `error`.
    Failed,
}

/// Snapshot of the whole engine, emitted as the `engine-status` event.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub phase: EnginePhase,
    pub arch: String,
    pub chat_running: bool,
    pub embed_running: bool,
    /// CoreML whisper encoder detected next to the model (faster STT).
    pub coreml: bool,
    pub models: Vec<ModelStatusView>,
    pub error: Option<String>,
}

/// The one global state object, managed by Tauri.
pub struct AppState {
    pub app: AppHandle,
    /// Single SQLite connection (rusqlite `Connection` is not `Send`, so it is
    /// wrapped in a Mutex; a personal-scale app serializes fine on one conn).
    pub db: Mutex<Connection>,
    /// Directory for downloaded models (app data / models)
    pub models_dir: PathBuf,
    /// Directory of bundled resources (Contents/Resources on macOS)
    pub resources_dir: PathBuf,
    /// Directory containing the bundled sidecar executables.
    pub sidecar_dir: PathBuf,
    /// llama-server serving the chat model (port 8642)
    pub chat_server: Mutex<Option<Child>>,
    /// llama-server serving the embedding model (port 8643)
    pub embed_server: Mutex<Option<Child>>,
    /// whisper-server keeping the speech model loaded (port 8644)
    pub whisper_server: Mutex<Option<Child>>,
    pub whisper_running: AtomicBool,
    pub phase: AtomicU64,
    pub chat_running: AtomicBool,
    pub embed_running: AtomicBool,
    pub model_error: Mutex<Option<String>>,
    /// the live capture session, if recording (only one at a time)
    pub capture: Mutex<Option<crate::audio::session::Session>>,
    /// per-model download bookkeeping for progress events
    pub downloads: Mutex<std::collections::HashMap<ModelKey, crate::models::DownloadState>>,
}

impl AppState {
    /// Build the state from a running app. Called once from `setup`.
    pub fn init(app: AppHandle) -> Result<Self, crate::error::Error> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| crate::error::Error::Store(format!("cannot resolve app data dir: {e}")))?;
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| crate::error::Error::Store(format!("cannot create app data dir: {e}")))?;
        std::fs::create_dir_all(data_dir.join("models"))
            .map_err(|e| crate::error::Error::Store(format!("cannot create models dir: {e}")))?;

        let models_dir = data_dir.join("models");

        // Sidecars live next to the executable in a built app (Contents/MacOS).
        // In dev (`cargo run` / `tauri dev`) they sit next to the debug binary.
        let exe = std::env::current_exe().unwrap_or_default();
        let sidecar_dir = exe
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.clone());

        let resources_dir = app
            .path()
            .resource_dir()
            .unwrap_or_else(|_| data_dir.clone());

        let db = open_database(&data_dir)?;

        Ok(Self {
            app,
            db: Mutex::new(db),
            models_dir,
            resources_dir,
            sidecar_dir,
            chat_server: Mutex::new(None),
            embed_server: Mutex::new(None),
            whisper_server: Mutex::new(None),
            whisper_running: AtomicBool::new(false),
            phase: AtomicU64::new(EnginePhase::Setup as u64),
            chat_running: AtomicBool::new(false),
            embed_running: AtomicBool::new(false),
            model_error: Mutex::new(None),
            capture: Mutex::new(None),
            downloads: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Resolve the path of a model file, preferring the user's app-data copy
    /// (writable, downloaded) over the bundled read-only copy.
    pub fn resolve_model(&self, key: ModelKey) -> Option<std::path::PathBuf> {
        let name = key.filename();
        let in_data = self.models_dir.join(name);
        if in_data.is_file() && in_data.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            return Some(in_data);
        }
        // tauri.conf.json bundles `resources/models/**` with its path prefix,
        // so it lands at Contents/Resources/resources/models.
        let in_bundle = self.resources_dir.join("resources/models").join(name);
        if in_bundle.is_file() {
            return Some(in_bundle);
        }
        None
    }

    pub fn set_phase(&self, phase: EnginePhase) {
        self.phase.store(phase as u64, Ordering::SeqCst);
    }

    pub fn engine_status(&self) -> EngineStatus {
        let phase = match self.phase.load(Ordering::SeqCst) {
            0 => EnginePhase::Setup,
            1 => EnginePhase::Starting,
            2 => EnginePhase::Ready,
            _ => EnginePhase::Failed,
        };
        let models = crate::models::MANIFEST
            .iter()
            .map(|def| {
                let present = self.resolve_model(def.key).is_some();
                let downloads = self.downloads.lock().unwrap();
                let status = downloads.get(&def.key);
                ModelStatusView {
                    key: def.key.as_str().to_string(),
                    label: def.label.to_string(),
                    present,
                    path: self
                        .resolve_model(def.key)
                        .map(|p| p.display().to_string()),
                    size_bytes: present
                        .then(|| self.resolve_model(def.key).and_then(|p| p.metadata().ok()))
                        .flatten()
                        .map(|m| m.len())
                        .unwrap_or(0),
                    // The live content-length wins over the manifest estimate.
                    expected_bytes: status
                        .map(|s| s.expected_bytes)
                        .filter(|&e| e > 0)
                        .unwrap_or(def.expected_bytes),
                    downloading: status.map(|s| s.downloading).unwrap_or(false),
                    downloaded_bytes: status.map(|s| s.downloaded_bytes).unwrap_or(0),
                    error: status.and_then(|s| s.error.clone()),
                }
            })
            .collect();
        EngineStatus {
            phase,
            arch: crate::models::host_arch(),
            chat_running: self.chat_running.load(Ordering::SeqCst),
            embed_running: self.embed_running.load(Ordering::SeqCst),
            coreml: crate::engine::whisper::coreml_enabled(self),
            models,
            error: self.model_error.lock().unwrap().clone(),
        }
    }

    /// Emit the current engine status to the frontend.
    pub fn push_status(&self) {
        let _ = self
            .app
            .emit("engine-status", self.engine_status());
    }

    pub fn emit(&self, event: &str, payload: &serde_json::Value) {
        let _ = self.app.emit(event, payload);
    }

}


