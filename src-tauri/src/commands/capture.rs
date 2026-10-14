//! Voice capture commands.
//!
//! The frontend streams base64 PCM (16 kHz mono s16le) from its Web Audio
//! capture into the backend, which buffers it; `finish_capture` then runs the
//! full pipeline (whisper → LLM cleanup → graph → vector store).

use base64::Engine as _;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

use crate::engine::pipeline::{self, CaptureResult};
use crate::error::Result;
use crate::state::AppState;
use crate::time;

/// Open a new capture session. Resets the audio buffer.
#[tauri::command]
pub fn begin_capture(
    state: State<'_, Arc<AppState>>,
    language: Option<String>,
) -> Result<CaptureSession> {
    let language = language.unwrap_or_else(|| "en".into());
    let language = if language == "hi" { "hi" } else { "en" }.to_string();
    let mut cap = state
        .capture
        .lock()
        .map_err(|_| crate::error::Error::Capture("capture lock poisoned".into()))?;
    *cap = crate::state::CaptureBuffer::new(&language);
    Ok(CaptureSession {
        language,
        max_seconds: (pipeline::MAX_CAPTURE_BYTES / (16_000 * 2)) as u64,
        started_at: time::now_millis(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSession {
    pub language: String,
    pub max_seconds: u64,
    pub started_at: i64,
}

/// Append a base64-encoded chunk of 16 kHz mono s16le PCM.
#[tauri::command]
pub fn append_audio(state: State<'_, Arc<AppState>>, chunk: String) -> Result<u64> {
    // The frontend encodes cleanly, but be tolerant of any stray whitespace.
    let clean: String = chunk.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .map_err(|e| crate::error::Error::Capture(format!("bad audio chunk: {e}")))?;
    let mut cap = state
        .capture
        .lock()
        .map_err(|_| crate::error::Error::Capture("capture lock poisoned".into()))?;
    if !cap.active {
        return Err(crate::error::Error::Capture(
            "capture not started".into(),
        ));
    }
    cap.pcm.extend_from_slice(&bytes);
    if cap.pcm.len() > pipeline::MAX_CAPTURE_BYTES {
        let from = cap.pcm.len() - pipeline::MAX_CAPTURE_BYTES;
        cap.pcm = cap.pcm.split_off(from);
    }
    Ok(cap.duration_secs())
}

/// Finish the capture and run the full pipeline.
///
/// The promise resolves when everything is done (typically 10-60 s); stage
/// updates stream over the `capture-stage` event meanwhile.
#[tauri::command]
pub async fn finish_capture(
    state: State<'_, Arc<AppState>>,
) -> Result<CaptureResult> {
    let s = state.inner().clone();
    pipeline::process_capture(s).await
}
