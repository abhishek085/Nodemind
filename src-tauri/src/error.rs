//! Shared error type for the Nodemind backend.
//!
//! Every module returns `Result<T, Error>` and errors surface to the frontend
//! as a single human-readable message — no silent failures, no panics.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sidecar binary not found: {0} — run `scripts/build-sidecars.sh` once (one-time build)")]
    SidecarMissing(String),

    #[error("model not available: {0} ({1}) — run the first-launch setup to download it")]
    ModelMissing(String, String),

    #[error("engine error: {0}")]
    Engine(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("calendar error: {0}")]
    Calendar(String),

    #[error("capture error: {0}")]
    Capture(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Tauri commands return `Result<T, Error>`; the macro needs the error type
/// convertible to `InvokeError`. We surface the human-readable message.
impl From<Error> for tauri::ipc::InvokeError {
    fn from(e: Error) -> Self {
        tauri::ipc::InvokeError::from_error(e)
    }
}

impl Error {
    /// Best-effort one-line description for UI toasts.
    pub fn message(&self) -> String {
        self.to_string()
    }
}
