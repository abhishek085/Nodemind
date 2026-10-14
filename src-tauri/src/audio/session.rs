//! One live capture session: audio → utterances → whisper, streamed.
//!
//! ```text
//! mic ─────16 kHz──▶ Segmenter ──┐ (You)
//!                                 ├─▶ transcriber task ──▶ `capture-partial`
//! system tap ─16 kHz─▶ Segmenter ─┘ (Them, meeting mode only)
//!     └─ levels (10 Hz) ──▶ `capture-level` {source}
//! ```
//!
//! Utterances are transcribed while the user keeps talking, so stopping only
//! waits for the last one (~0.2 s) before `pipeline::organize` runs.
//! Start/stop are callable from anywhere — UI commands, the tray, the hotkey.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audio::mic::{self, MicCapture};
use crate::audio::segmenter::Segmenter;
use crate::audio::system::SystemAudio;
use crate::engine::meeting::{self, Line, Speaker};
use crate::engine::pipeline::{self, CaptureResult};
use crate::engine::whisper;
use crate::error::{Error, Result};
use crate::state::AppState;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// What is being captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMode {
    /// A brain-dump: the mic only → checklist + mind map.
    Note,
    /// A call: mic ("You") + the Mac's audio output ("Them") → minutes.
    Meeting,
}

impl CaptureMode {
    fn max_secs(self) -> u64 {
        match self {
            CaptureMode::Note => pipeline::MAX_CAPTURE_SECS,
            CaptureMode::Meeting => 3 * 60 * 60,
        }
    }
}

pub struct Session {
    id: u64,
    mode: CaptureMode,
    mic: MicCapture,
    system: Option<SystemAudio>,
    transcriber: tauri::async_runtime::JoinHandle<String>,
    partial: Arc<Mutex<String>>,
    language: String,
    started_at_ms: i64,
    started: Instant,
}

/// Snapshot for a window that opens mid-capture.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub recording: bool,
    pub mode: Option<CaptureMode>,
    pub elapsed_secs: u64,
    pub partial: String,
}

struct Utterance {
    speaker: Speaker,
    at_ms: u64,
    samples: Vec<f32>,
}

/// Start capturing.
pub fn start(state: &Arc<AppState>, language: &str, mode: CaptureMode) -> Result<()> {
    let mut slot = lock(&state.capture)?;
    if slot.is_some() {
        return Err(Error::Capture("already recording".into()));
    }
    let language = if language == "hi" { "hi" } else { "en" }.to_string();
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let partial = Arc::new(Mutex::new(String::new()));
    let started = Instant::now();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Utterance>();
    let transcriber = spawn_transcriber(state.clone(), rx, language.clone(), mode, partial.clone());

    let (on_audio, on_end) = source_callbacks(state.clone(), tx.clone(), Speaker::You, "mic", started);
    let mic = match MicCapture::start(on_audio, on_end) {
        Ok(m) => m,
        Err(e) => {
            transcriber.abort();
            return Err(e);
        }
    };

    // The other side of the call. Failing here downgrades to mic-only rather
    // than losing the meeting.
    let system = if mode == CaptureMode::Meeting {
        let (on_audio, on_end) = source_callbacks(state.clone(), tx.clone(), Speaker::Them, "system", started);
        match SystemAudio::start(&state.sidecar_dir.join("nodemind-helper"), on_audio, on_end) {
            Ok(s) => Some(s),
            Err(e) => {
                state.emit(
                    "capture-warning",
                    &serde_json::json!({ "message": format!("Recording your mic only — couldn't capture the call audio: {}", e.message()) }),
                );
                None
            }
        }
    } else {
        None
    };
    drop(tx); // the sources hold the remaining senders
    let has_system = system.is_some();

    *slot = Some(Session {
        id,
        mode,
        mic,
        system,
        transcriber,
        partial,
        language,
        started_at_ms: crate::time::now_millis(),
        started,
    });
    drop(slot);

    crate::applog::write(state, "app.log", &format!("capture {id} started ({mode:?}, system audio: {})", has_system));
    state.emit("capture-state", &serde_json::json!({ "state": "recording", "mode": mode }));
    crate::tray::set_status(state, crate::tray::TrayStatus::Recording(0));
    spawn_ticker(state.clone(), id, mode);
    Ok(())
}

/// Per-source audio handling: segment into utterances (stamped with when
/// they began) and report the input level. Returns (on_audio, on_end).
fn source_callbacks(
    state: Arc<AppState>,
    tx: tokio::sync::mpsc::UnboundedSender<Utterance>,
    speaker: Speaker,
    source: &'static str,
    started: Instant,
) -> (impl FnMut(&[f32]) + Send + 'static, impl FnOnce() + Send + 'static) {
    let segmenter = Arc::new(Mutex::new(Segmenter::new()));
    let stamp = move |samples: Vec<f32>| {
        let len_ms = (samples.len() / 16) as u64;
        let at_ms = (started.elapsed().as_millis() as u64).saturating_sub(len_ms);
        Utterance { speaker, at_ms, samples }
    };

    let on_audio = {
        let segmenter = segmenter.clone();
        let tx = tx.clone();
        let mut window: Vec<f32> = Vec::with_capacity(1_600);
        move |samples: &[f32]| {
            if let Ok(mut seg) = segmenter.lock() {
                for utterance in seg.push(samples) {
                    let _ = tx.send(stamp(utterance));
                }
            }
            window.extend_from_slice(samples);
            if window.len() >= 1_600 {
                // 100 ms at 16 kHz
                state.emit(
                    "capture-level",
                    &serde_json::json!({ "level": mic::level(&window), "source": source }),
                );
                window.clear();
            }
        }
    };
    let on_end = move || {
        let last = segmenter.lock().ok().map(|mut s| std::mem::take(&mut *s));
        if let Some(utterance) = last.and_then(Segmenter::finish) {
            let len_ms = (utterance.len() / 16) as u64;
            let at_ms = (started.elapsed().as_millis() as u64).saturating_sub(len_ms);
            let _ = tx.send(Utterance { speaker, at_ms, samples: utterance });
        }
        // tx drops here; once every source has ended the transcriber finishes.
    };
    (on_audio, on_end)
}

/// Utterances in, growing transcript out (one whisper call at a time).
fn spawn_transcriber(
    state: Arc<AppState>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Utterance>,
    language: String,
    mode: CaptureMode,
    partial: Arc<Mutex<String>>,
) -> tauri::async_runtime::JoinHandle<String> {
    tauri::async_runtime::spawn(async move {
        let mut lines: Vec<Line> = Vec::new();
        let mut rendered = String::new();
        while let Some(u) = rx.recv().await {
            // Whisper continues from what this speaker said last.
            let context: String = lines
                .iter()
                .filter(|l| l.speaker == u.speaker)
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            match whisper::transcribe_segment(&state, &u.samples, &language, &context).await {
                Ok(text) if !text.is_empty() => {
                    lines.push(Line { at_ms: u.at_ms, speaker: u.speaker, text });
                    rendered = match mode {
                        CaptureMode::Meeting => meeting::render(&lines),
                        CaptureMode::Note => lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" "),
                    };
                    if let Ok(mut p) = partial.lock() {
                        p.clone_from(&rendered);
                    }
                    state.emit("capture-partial", &serde_json::json!({ "text": rendered }));
                }
                Ok(_) => {}
                Err(e) => eprintln!("[capture] utterance failed: {e}"),
            }
        }
        rendered
    })
}

/// Stop, finish the last utterances, organize, and return the result.
pub async fn stop(state: Arc<AppState>) -> Result<CaptureResult> {
    let session = lock(&state.capture)?
        .take()
        .ok_or_else(|| Error::Capture("not recording".into()))?;
    state.emit("capture-state", &serde_json::json!({ "state": "processing" }));
    state.emit("capture-stage", &serde_json::json!({ "stage": "transcribing" }));
    crate::tray::set_status(&state, crate::tray::TrayStatus::Processing);

    let Session { id, mode, mic, system, transcriber, language, started_at_ms, started, .. } = session;
    let duration_secs = started.elapsed().as_secs() as i64;
    let t0 = Instant::now();
    // Joining the sources flushes their final utterances and closes the channel.
    let _ = tauri::async_runtime::spawn_blocking(move || {
        mic.stop();
        if let Some(s) = system {
            s.stop();
        }
    })
    .await;
    let transcript = transcriber.await.unwrap_or_default();
    let t_transcript = t0.elapsed();

    let result =
        pipeline::organize(state.clone(), transcript, language, started_at_ms, duration_secs, mode).await;

    crate::tray::set_status(&state, crate::tray::TrayStatus::Idle);
    state.emit("capture-state", &serde_json::json!({ "state": "idle" }));
    crate::applog::write(
        &state,
        "app.log",
        &match &result {
            Ok(r) => format!(
                "capture {id} done: {duration_secs}s audio, {} chars, {} tasks, minutes: {} — transcript tail {:.2}s, organize {:.2}s",
                r.transcript.len(),
                r.tasks.len(),
                r.minutes.is_some(),
                t_transcript.as_secs_f32(),
                (t0.elapsed() - t_transcript).as_secs_f32(),
            ),
            Err(e) => format!("capture {id} failed: {}", e.message()),
        },
    );
    match &result {
        Ok(r) => {
            state.emit("capture-complete", &serde_json::to_value(r).unwrap_or_default());
            crate::tray::notify_if_hidden(&state, r);
        }
        Err(e) => state.emit("capture-error", &serde_json::json!({ "message": e.message() })),
    }
    result
}

/// Throw the current capture away.
pub fn cancel(state: &Arc<AppState>) -> Result<()> {
    let Some(session) = lock(&state.capture)?.take() else {
        return Ok(());
    };
    session.transcriber.abort();
    let (mic, system) = (session.mic, session.system);
    std::thread::spawn(move || {
        mic.stop();
        if let Some(s) = system {
            s.stop();
        }
    });
    crate::tray::set_status(state, crate::tray::TrayStatus::Idle);
    state.emit("capture-state", &serde_json::json!({ "state": "idle" }));
    Ok(())
}

pub fn is_recording(state: &AppState) -> bool {
    state.capture.lock().map(|s| s.is_some()).unwrap_or(false)
}

pub fn status(state: &AppState) -> CaptureStatus {
    let slot = state.capture.lock();
    match slot.as_deref() {
        Ok(Some(s)) => CaptureStatus {
            recording: true,
            mode: Some(s.mode),
            elapsed_secs: s.started.elapsed().as_secs(),
            partial: s.partial.lock().map(|p| p.clone()).unwrap_or_default(),
        },
        _ => CaptureStatus { recording: false, mode: None, elapsed_secs: 0, partial: String::new() },
    }
}

/// Toggle used by the tray and hotkey: start if idle, stop if recording.
pub fn toggle(state: &Arc<AppState>, mode: CaptureMode) {
    if is_recording(state) {
        let s = state.clone();
        tauri::async_runtime::spawn(async move {
            let _ = stop(s).await;
        });
    } else if let Err(e) = start(state, &crate::commands::settings::current_language(state), mode) {
        state.emit("capture-error", &serde_json::json!({ "message": e.message() }));
    }
}

/// Once a second: update the menu-bar timer and enforce the length cap.
fn spawn_ticker(state: Arc<AppState>, id: u64, mode: CaptureMode) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let elapsed = match state.capture.lock().as_deref() {
                Ok(Some(s)) if s.id == id => s.started.elapsed().as_secs(),
                _ => return,
            };
            crate::tray::set_status(&state, crate::tray::TrayStatus::Recording(elapsed));
            if elapsed >= mode.max_secs() {
                let _ = stop(state.clone()).await;
                return;
            }
        }
    });
}

fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock().map_err(|_| Error::Capture("capture state poisoned".into()))
}
