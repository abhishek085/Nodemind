//! Model management: a static manifest + a one-time downloader.
//!
//! Resolution order for each model:
//!   1. `app_data/models/<file>`   — downloaded at first launch (writable)
//!   2. `resources/models/<file>`  — bundled inside the .app if the builder
//!      ran `scripts/fetch-models.sh` (true batteries-included)
//!   3. download to (1) with progress events, then continue
//!
//! Everything after the first launch is fully offline.

use std::io::{Read, Write};
use std::sync::Arc;

use futures_util::StreamExt;
use sha2::Digest;
use tauri::Emitter;

use crate::error::{Error, Result};
use crate::state::AppState;

pub mod manifest;

pub use manifest::{host_arch, ModelDef, ModelKey, MANIFEST};

/// Mutable bookkeeping for a model download (surfaces in `engine-status`).
#[derive(Debug, Default, Clone)]
pub struct DownloadState {
    pub downloading: bool,
    pub downloaded_bytes: u64,
    pub expected_bytes: u64,
    pub error: Option<String>,
}

/// Kick off the one-time setup: download every missing model, then boot the
/// engine. Idempotent — if models are already present it just boots.
///
/// Streams progress to the UI via `model-progress` / `engine-status` events.
pub fn begin_setup(state: Arc<AppState>) {
    let missing: Vec<ModelKey> = MANIFEST
        .iter()
        .filter(|def| state.resolve_model(def.key).is_none())
        .map(|def| def.key)
        .collect();

    if missing.is_empty() {
        crate::engine::boot(Arc::clone(&state));
        return;
    }

    tauri::async_runtime::spawn(async move {
        let mut all_ok = true;
        for def in MANIFEST.iter().filter(|d| missing.contains(&d.key)) {
            if !download_one(state.clone(), def).await {
                all_ok = false;
            }
        }
        if all_ok {
            // Companion (CoreML encoder) is optional — best effort, background.
            let s2 = Arc::clone(&state);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = download_companion(s2).await {
                    eprintln!("[models] companion download failed: {e}");
                }
            });
            crate::engine::boot(Arc::clone(&state));
        }
    });
}

fn set_download_state(state: &AppState, key: ModelKey, ds: DownloadState) {
    state.downloads.lock().unwrap().insert(key, ds);
    state.push_status();
}

/// Download a single model file (resume + sha256 verify). Returns success.
async fn download_one(state: Arc<AppState>, def: &ModelDef) -> bool {
    let dest = state.models_dir.join(def.filename);
    let part = state.models_dir.join(format!(".{}", def.filename));

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .user_agent(format!(
            "Nodemind/{}",
            crate::store::db::app_version()
        ))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            set_download_state(
                &state,
                def.key,
                DownloadState {
                    expected_bytes: def.expected_bytes,
                    error: Some(e.to_string()),
                    ..Default::default()
                },
            );
            return false;
        }
    };

    let mut resume_from = 0u64;
    if part.exists() {
        resume_from = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    }

    set_download_state(
        &state,
        def.key,
        DownloadState {
            downloading: true,
            downloaded_bytes: resume_from,
            expected_bytes: def.expected_bytes,
            error: None,
        },
    );

    let mut req = client.get(def.url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }

    let resp = match req.send().await {
        Ok(r) => match r.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                set_download_state(
                    &state,
                    def.key,
                    DownloadState {
                        error: Some(e.to_string()),
                        ..Default::default()
                    },
                );
                return false;
            }
        },
        Err(e) => {
            set_download_state(
                &state,
                def.key,
                DownloadState {
                    error: Some(format!("request failed: {e}")),
                    ..Default::default()
                },
            );
            return false;
        }
    };

    let total = resp
        .content_length()
        .map(|c| c.saturating_sub(resume_from))
        .unwrap_or(def.expected_bytes);
    let expected = total.saturating_add(resume_from);

    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part)
    {
        Ok(f) => f,
        Err(e) => {
            set_download_state(
                &state,
                def.key,
                DownloadState {
                    error: Some(e.to_string()),
                    ..Default::default()
                },
            );
            return false;
        }
    };
    let mut writer = std::io::BufWriter::new(file);

    // (Re)hash the whole file — cheap relative to the transfer.
    let mut hasher = sha2::Sha256::new();
    if resume_from > 0 {
        if let Ok(mut f) = std::fs::File::open(&part) {
            let mut buf = vec![0u8; 1 << 20];
            loop {
                match f.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => hasher.update(&buf[..n]),
                    Err(_) => break,
                }
            }
        }
    }

    let mut stream = resp.bytes_stream();
    let mut written = resume_from;
    let mut last_emit = std::time::Instant::now();

    let result: std::result::Result<(), String> = async {
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| e.to_string())?;
            hasher.update(&chunk);
            writer
                .write_all(&chunk)
                .map_err(|e| e.to_string())?;
            written += chunk.len() as u64;
            if last_emit.elapsed().as_millis() > 250 || written >= expected {
                last_emit = std::time::Instant::now();
                let s = state.clone();
                let key = def.key; // Copy — the spawned task must be 'static
                tauri::async_runtime::spawn(async move {
                    *s.downloads
                        .lock()
                        .unwrap()
                        .entry(key)
                        .or_default() = DownloadState {
                        downloading: true,
                        downloaded_bytes: written,
                        expected_bytes: expected,
                        error: None,
                    };
                    let _ = s.app.emit(
                        "model-progress",
                        serde_json::json!({ "key": key.as_str(), "done": written, "total": expected }),
                    );
                    s.push_status();
                });
            }
        }
        writer
            .flush()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;

    drop(writer);

    if let Err(e) = result {
        set_download_state(
            &state,
            def.key,
            DownloadState {
                error: Some(e),
                ..Default::default()
            },
        );
        return false;
    }

    if let Some(want) = def.sha256 {
        let got = hexify(&hasher.finalize());
        if got != want.to_lowercase() {
            let _ = std::fs::remove_file(&part);
            set_download_state(
                &state,
                def.key,
                DownloadState {
                    error: Some(format!(
                        "checksum mismatch for {} — delete it and try again (setup will re-download)",
                        def.filename
                    )),
                    ..Default::default()
                },
            );
            return false;
        }
    }

    match std::fs::rename(&part, &dest) {
        Ok(()) => {
            eprintln!(
                "[models] ✓ {} ready ({} bytes)",
                def.filename,
                dest.metadata().map(|m| m.len()).unwrap_or(0)
            );
            set_download_state(
                &state,
                def.key,
                DownloadState {
                    downloaded_bytes: expected,
                    expected_bytes: expected,
                    ..Default::default()
                },
            );
            true
        }
        Err(e) => {
            set_download_state(
                &state,
                def.key,
                DownloadState {
                    error: Some(format!("could not move {}: {e}", def.filename)),
                    ..Default::default()
                },
            );
            false
        }
    }
}

fn hexify(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Best-effort download of the whisper CoreML encoder, which makes STT
/// 5-10× faster on Apple Silicon. It ships as a zip from the canonical model
/// repo and is unzipped into `models/ggml-small-encoder.mlmodelc/`.
///
/// Purely optional — without it whisper runs CPU-only (slower, still works).
async fn download_companion(state: Arc<AppState>) -> Result<()> {
    const DIR: &str = "ggml-small-encoder.mlmodelc";
    const ZIP: &str = "ggml-small-encoder.mlmodelc.zip";
    const URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-encoder.mlmodelc.zip";
    const SHA: &str = "de43fb9fed471e95c19e60ae67575c2bf09e8fb607016da171b06ddad313988b";

    let dir_path = state.models_dir.join(DIR);
    if dir_path.join("model.mil").exists() {
        return Ok(()); // already present
    }

    let zip_path = state.models_dir.join(ZIP);
    let client = reqwest::Client::builder()
        .user_agent(format!("Nodemind/{}", crate::store::db::app_version()))
        .build()
        .map_err(Error::Http)?;

    if !zip_path.exists() {
        let resp = client
            .get(URL)
            .send()
            .await
            .map_err(Error::Http)?
            .error_for_status()
            .map_err(Error::Http)?;
        let bytes = resp.bytes().await.map_err(Error::Http)?;
        if hexify(&sha2::Sha256::digest(&bytes)) != SHA {
            return Err(Error::Engine(
                "CoreML encoder zip failed checksum — skipping (STT will run CPU-only)".into(),
            ));
        }
        std::fs::write(&zip_path, &bytes).map_err(Error::Io)?;
    }

    // Unzip (blocking: ~150MB of small files).
    let dir_exists = dir_path.exists();
    let _ = dir_exists;
    tokio::task::spawn_blocking({
        let zip_path = zip_path.clone();
        let dir_path = dir_path.clone();
        move || -> Result<()> {
            std::fs::create_dir_all(&dir_path).map_err(Error::Io)?;
            let file = std::fs::File::open(&zip_path).map_err(Error::Io)?;
            let mut zip =
                zip::ZipArchive::new(file).map_err(|e| Error::Engine(format!("zip: {e}")))?;
            for i in 0..zip.len() {
                let mut entry = zip
                    .by_index(i)
                    .map_err(|e| Error::Engine(format!("zip entry: {e}")))?;
                let name = entry
                    .enclosed_name()
                    .ok_or_else(|| Error::Engine("unsafe zip path".into()))?;
                let is_dir = name.to_string_lossy().ends_with('/');
                let dest = dir_path.join(&name);
                if is_dir {
                    std::fs::create_dir_all(&dest).map_err(Error::Io)?;
                } else {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent).map_err(Error::Io)?;
                    }
                    let mut out = std::fs::File::create(&dest).map_err(Error::Io)?;
                    std::io::copy(&mut entry, &mut out).map_err(Error::Io)?;
                }
            }
            Ok(())
        }
    })
    .await
    .map_err(|e| Error::Engine(format!("unzip task: {e}")))??;

    let _ = std::fs::remove_file(&zip_path);
    if dir_path.join("model.mil").exists() {
        eprintln!("[models] ✓ coreml encoder ready");
        Ok(())
    } else {
        Err(Error::Engine("CoreML encoder missing after unzip".into()))
    }
}

/// Whether the models required for a *fully* functional engine are present.
pub fn models_ready(state: &AppState) -> bool {
    MANIFEST.iter().all(|def| state.resolve_model(def.key).is_some())
}
