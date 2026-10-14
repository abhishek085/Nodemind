//! The local AI engine: bundled C++ sidecars + their client helpers.
//!
//! - [`sidecar`]  — spawn/health/kill for the two `llama-server` instances.
//! - [`whisper`]  — run the bundled `whisper-cli` on a wav file.
//! - [`llm`]      — chat + embeddings over llama-server's OpenAI-compatible API.
//! - [`pipeline`] — the full capture pipeline: wav → text → checklist + graph.

pub mod llm;
pub mod pipeline;
pub mod sidecar;
pub mod whisper;

use std::sync::Arc;

use crate::state::{AppState, EnginePhase};

/// Boot the engine: start both llama-servers and wait for health.
///
/// Called once when models become available (either pre-bundled or after the
/// first-launch download completes). Safe to call when already running.
pub fn boot(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // Never run two boots concurrently.
        if state.chat_running.load(std::sync::atomic::Ordering::SeqCst)
            && state.embed_running.load(std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }

        state.set_phase(EnginePhase::Starting);
        state.push_status();

        let chat_model = state.resolve_model(crate::models::ModelKey::Llm);
        let embed_model = state.resolve_model(crate::models::ModelKey::Embed);
        let whisper_model = state.resolve_model(crate::models::ModelKey::Whisper);

        let missing = if crate::models::models_ready(&state) {
            Vec::new()
        } else {
            missing_names(&chat_model, &embed_model, &whisper_model)
        };
        if !missing.is_empty() {
            let msg = format!(
                "models missing: {} — run setup to download them",
                missing.join(", ")
            );
            state.model_error.lock().unwrap().replace(msg.clone());
            state.set_phase(EnginePhase::Setup);
            state.push_status();
            return;
        }

        // Embedding server first (smaller, starts faster).
        let embed_ok = sidecar::ensure_embed_server(&state).await;
        let chat_ok = sidecar::ensure_chat_server(&state).await;

        if embed_ok && chat_ok {
            state.set_phase(EnginePhase::Ready);
            state.model_error.lock().unwrap().take();
            eprintln!("[engine] ✓ ready ({})", crate::models::host_arch());
        } else {
            let msg = if !chat_ok {
                "chat model server failed to start — see app logs".to_string()
            } else {
                "embedding server failed to start — see app logs".to_string()
            };
            state.model_error.lock().unwrap().replace(msg.clone());
            state.set_phase(EnginePhase::Failed);
        }
        state.push_status();
    });
}

fn missing_names(
    chat: &Option<std::path::PathBuf>,
    embed: &Option<std::path::PathBuf>,
    whisper: &Option<std::path::PathBuf>,
) -> Vec<String> {
    let mut out = Vec::new();
    if chat.is_none() {
        out.push("llm".into());
    }
    if embed.is_none() {
        out.push("embed".into());
    }
    if whisper.is_none() {
        out.push("whisper".into());
    }
    out
}

/// Shut the engine down (kill sidecars). Called on app exit.
pub fn shutdown(state: &AppState) {
    sidecar::kill_all(state);
    state.set_phase(EnginePhase::Setup);
}
