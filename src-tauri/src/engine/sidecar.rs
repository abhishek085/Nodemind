//! Sidecar process management.
//!
//! Three servers run on loopback:
//!   - port 8642 — `llama-server`, the chat model (Qwen3.5 0.8B) for cleanup/annotation/minutes
//!   - port 8643 — `llama-server`, the embedding model (EmbeddingGemma 2) for the vector store
//!   - port 8644 — `whisper-server`, speech → text with the model kept loaded,
//!     so each live utterance transcribes in ~0.2 s instead of reloading
//!
//! `whisper-cli` remains as a fallback when the server binary is absent.
//! Sidecars are killed automatically on app exit (see `engine::shutdown`).

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use crate::models::ModelKey;
use crate::state::AppState;

pub const CHAT_PORT: u16 = 8642;
pub const EMBED_PORT: u16 = 8643;
pub const WHISPER_PORT: u16 = 8644;
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

/// Make `child` die when this app does, however the app ends (quit, crash,
/// SIGTERM, `kill -9`): a tiny detached shell polls our pid and kills the
/// child once we're gone. Without it, killed apps left ~1.5 GB of servers
/// running with no owner.
fn bind_lifetime(child: &std::process::Child) {
    let script = format!(
        "while kill -0 {parent} 2>/dev/null; do sleep 2; done; kill {child} 2>/dev/null",
        parent = std::process::id(),
        child = child.id()
    );
    let _ = Command::new("/bin/sh")
        .args(["-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
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
        // Without --embeddings the server answers /v1/embeddings with 501.
        .args(["-ngl", "99", "--ctx-size", "8192", "--no-webui", "--embeddings"])
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

    bind_lifetime(&child);
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
        // --jinja: use the model's own chat template (Qwen needs it to honour
        // `enable_thinking: false`, see llm::chat).
        .args(["-ngl", "99", "--ctx-size", "8192", "--no-webui", "--jinja"])
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

    bind_lifetime(&child);
    state.chat_server.lock().unwrap().replace(child);
    let healthy = wait_healthy(CHAT_PORT, Duration::from_secs(240)).await;
    state.chat_running.store(healthy, std::sync::atomic::Ordering::SeqCst);
    eprintln!("[sidecar] chat server {} (port {CHAT_PORT})", if healthy { "up" } else { "DOWN" });
    healthy
}

/// Make sure the whisper server is up. Returns true when healthy.
///
/// Optional: when it is missing or fails, transcription falls back to
/// spawning `whisper-cli` per utterance (slower, still correct).
pub async fn ensure_whisper_server(state: &Arc<AppState>) -> bool {
    if is_port_alive(WHISPER_PORT) {
        state.whisper_running.store(true, std::sync::atomic::Ordering::SeqCst);
        return true;
    }
    let Some(model) = state.resolve_model(ModelKey::Whisper) else {
        return false;
    };
    let Some(bin) = sidecar_bin(state, "whisper-server") else {
        eprintln!("[sidecar] whisper-server missing — using whisper-cli per utterance");
        return false;
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("-m")
        .arg(&model)
        .args(["--host", HOST, "--port", &WHISPER_PORT.to_string()])
        .args(["-t", &threads.to_string(), "-l", "en"])
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(file) = log_file(state, "whisper-server.log") {
        cmd.stderr(Stdio::from(file));
    } else {
        cmd.stderr(Stdio::null());
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[sidecar] whisper spawn failed: {e}");
            return false;
        }
    };
    bind_lifetime(&child);
    state.whisper_server.lock().unwrap().replace(child);
    let healthy = wait_healthy(WHISPER_PORT, Duration::from_secs(60)).await;
    state.whisper_running.store(healthy, std::sync::atomic::Ordering::SeqCst);
    eprintln!("[sidecar] whisper server {} (port {WHISPER_PORT})", if healthy { "up" } else { "DOWN" });
    healthy
}

/// Kill every sidecar we own. Called on exit.
pub fn kill_all(state: &AppState) {
    for slot in [
        state.chat_server.lock().unwrap().take(),
        state.embed_server.lock().unwrap().take(),
        state.whisper_server.lock().unwrap().take(),
    ] {
        if let Some(mut c) = slot {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    // Ports may be held by processes we didn't spawn (a previous crashed run
    // whose child outlived us). Best-effort cleanup.
    for (bin, port) in [("llama-server", CHAT_PORT), ("llama-server", EMBED_PORT), ("whisper-server", WHISPER_PORT)] {
        if is_port_alive(port) {
            let _ = std::process::Command::new("pkill")
                .args(["-f", &format!("{bin} .*--port {port}")])
                .output();
        }
    }
    state.chat_running.store(false, std::sync::atomic::Ordering::SeqCst);
    state.embed_running.store(false, std::sync::atomic::Ordering::SeqCst);
    state.whisper_running.store(false, std::sync::atomic::Ordering::SeqCst);
}
