//! Sidecar process management.
//!
//! Two `llama-server` instances run on loopback:
//!   - port 8642 — the chat model (Llama 3.2 1B) for cleanup/annotation
//!   - port 8643 — the embedding model (EmbeddingGemma 2) for the vector store
//!
//! `whisper-cli` is *not* a resident server — it is spawned per transcription.
//! Sidecars are killed automatically on app exit (see `engine::shutdown`).

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use crate::models::ModelKey;
use crate::state::AppState;

pub const CHAT_PORT: u16 = 8642;
pub const EMBED_PORT: u16 = 8643;
const HOST: &str = "127.0.0.1";

/// Open (or create) an append-mode log file in the app-data `logs` dir.
fn log_file(state: &AppState, name: &str) -> Option<std::fs::File> {
    let dir = state.models_dir.parent()?.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(name))
        .ok()
}

fn sidecar_bin(state: &AppState, name: &str) -> Option<std::path::PathBuf> {
    let p = state.sidecar_dir.join(name);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn is_port_alive(port: u16) -> bool {
    std::net::TcpStream::connect((HOST, port))
        .map(|_| true)
        .unwrap_or(false)
}

/// Wait until the llama-server at `port` reports healthy, or the deadline.
async fn wait_healthy(port: u16, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    while start.elapsed() < deadline {
        let ok = client
            .get(format!("http://{HOST}:{port}/health"))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .is_ok();
        if ok {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    false
}

/// Make sure the embedding server is up. Returns true when healthy.
pub async fn ensure_embed_server(state: &Arc<AppState>) -> bool {
    if is_port_alive(EMBED_PORT) {
        state.embed_running.store(true, std::sync::atomic::Ordering::SeqCst);
        return true;
    }

    let Some(model) = state.resolve_model(ModelKey::Embed) else {
        eprintln!("[sidecar] embed model missing");
        return false;
    };
    let Some(bin) = sidecar_bin(state, "llama-server") else {
        eprintln!("[sidecar] llama-server binary missing — run scripts/build-sidecars.sh");
        return false;
    };

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("-m")
        .arg(&model)
        .args(["--host", HOST, "--port", &EMBED_PORT.to_string()])
        .args(["-ngl", "99", "--ctx-size", "8192", "--no-webui"])
        .env("GGML_LOG_LEVEL", "0") // silence ggml chatter
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(file) = log_file(state, "embed-server.log") {
        cmd.stderr(Stdio::from(file));
    } else {
        cmd.stderr(Stdio::null());
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[sidecar] embed spawn failed: {e}");
            return false;
        }
    };

    state.embed_server.lock().unwrap().replace(child);
    let healthy = wait_healthy(EMBED_PORT, Duration::from_secs(180)).await;
    state.embed_running.store(healthy, std::sync::atomic::Ordering::SeqCst);
    eprintln!("[sidecar] embed server {} (port {EMBED_PORT})", if healthy { "up" } else { "DOWN" });
    healthy
}

/// Make sure the chat server is up. Returns true when healthy.
pub async fn ensure_chat_server(state: &Arc<AppState>) -> bool {
    if is_port_alive(CHAT_PORT) {
        state.chat_running.store(true, std::sync::atomic::Ordering::SeqCst);
        return true;
    }

    let Some(model) = state.resolve_model(ModelKey::Llm) else {
        eprintln!("[sidecar] llm model missing");
        return false;
    };
    let Some(bin) = sidecar_bin(state, "llama-server") else {
        eprintln!("[sidecar] llama-server binary missing — run scripts/build-sidecars.sh");
        return false;
    };

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("-m")
        .arg(&model)
        .args(["--host", HOST, "--port", &CHAT_PORT.to_string()])
        .args(["-ngl", "99", "--ctx-size", "8192", "--no-webui"])
        .env("GGML_LOG_LEVEL", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(file) = log_file(state, "chat-server.log") {
        cmd.stderr(Stdio::from(file));
    } else {
        cmd.stderr(Stdio::null());
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[sidecar] chat spawn failed: {e}");
            return false;
        }
    };

    state.chat_server.lock().unwrap().replace(child);
    let healthy = wait_healthy(CHAT_PORT, Duration::from_secs(240)).await;
    state.chat_running.store(healthy, std::sync::atomic::Ordering::SeqCst);
    eprintln!("[sidecar] chat server {} (port {CHAT_PORT})", if healthy { "up" } else { "DOWN" });
    healthy
}

/// Kill every sidecar we own. Called on exit.
pub fn kill_all(state: &AppState) {
    for slot in [state.chat_server.lock().unwrap().take(), state.embed_server.lock().unwrap().take()] {
        if let Some(mut c) = slot {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    // Ports may be held by processes we didn't spawn (a previous crashed run
    // whose child outlived us). Best-effort cleanup.
    for port in [CHAT_PORT, EMBED_PORT] {
        if is_port_alive(port) {
            let _ = std::process::Command::new("pkill")
                .args(["-f", &format!("llama-server .*--port {port}")])
                .output();
        }
    }
    state.chat_running.store(false, std::sync::atomic::Ordering::SeqCst);
    state.embed_running.store(false, std::sync::atomic::Ordering::SeqCst);
}
