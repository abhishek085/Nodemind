//! Speech-to-text via the bundled `whisper-cli` sidecar.
//!
//! Pipeline per capture:
//!   PCM s16le (16 kHz mono, from the frontend) → wav file → whisper-cli
//!   → JSON (segments) → cleaned transcript string.
//!
//! The CoreML encoder (`ggml-small-encoder.mlmodelc`, sibling of the model
//! file) is used automatically when present and the binary was built with
//! CoreML — otherwise it silently falls back to CPU.

use std::sync::Arc;
use std::time::Instant;

use crate::models::ModelKey;
use crate::prompts;
use crate::state::AppState;

/// Write a raw PCM s16le buffer (16 kHz mono) as a wav file.
pub fn write_wav(pcm: &[u8], out_path: &std::path::Path) -> crate::error::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out_path, spec)
        .map_err(|e| crate::error::Error::Capture(format!("cannot write wav: {e}")))?;
    // pcm is s16le bytes; hound wants i16 samples.
    let samples: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    // hound 3.x: the fast path is the buffered i16 writer (exactly `n` samples).
    let mut buf = writer.get_i16_writer(samples.len() as u32);
    for s in &samples {
        buf.write_sample(*s);
    }
    buf.flush()
        .map_err(|e| crate::error::Error::Capture(format!("wav write: {e}")))?;
    writer
        .finalize()
        .map_err(|e| crate::error::Error::Capture(format!("wav finalize: {e}")))?;
    Ok(())
}

/// Transcribe a wav file. Blocking — call from `spawn_blocking`.
pub fn transcribe(
    state: &Arc<AppState>,
    wav_path: &std::path::Path,
    language: &str,
) -> crate::error::Result<String> {
    let model = state
        .resolve_model(ModelKey::Whisper)
        .ok_or_else(|| {
            crate::error::Error::ModelMissing(
                "whisper".into(),
                "speech model not downloaded — run first-launch setup".into(),
            )
        })?;
    let bin = state.sidecar_dir.join("whisper-cli");
    if !bin.exists() {
        return Err(crate::error::Error::SidecarMissing(
            "whisper-cli".to_string(),
        ));
    }

    let tmp = tempfile::TempDir::new()
        .map_err(|e| crate::error::Error::Capture(format!("temp dir: {e}")))?;
    let json_path = tmp.path().join("out");

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("-m").arg(&model)
        .arg("-f").arg(wav_path)
        .arg("-t").arg(threads().to_string())
        .arg("-oj")
        .arg("-of").arg(&json_path)
        .arg("-np"); // no prints — JSON file only

    match language {
        "hi" => {
            // Hinglish: decode as English, bias toward translating Hindi words.
            cmd.arg("-l").arg("en");
            let prompt = prompts::get("stt.hindi_initial_prompt");
            if !prompt.is_empty() {
                cmd.arg("--prompt").arg(prompt);
            }
        }
        _ => {
            cmd.arg("-l").arg("en");
        }
    }

    let started = Instant::now();
    let output = cmd
        .output()
        .map_err(|e| crate::error::Error::Engine(format!("whisper spawn: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::Error::Engine(format!(
            "whisper failed: {}",
            stderr.chars().take(300).collect::<String>()
        )));
    }

    let json_text = std::fs::read_to_string(with_ext(&json_path, ".json"))
        .or_else(|_| std::fs::read_to_string(&json_path))
        .map_err(|e| crate::error::Error::Engine(format!("whisper produced no JSON: {e}")))?;

    let parsed: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| crate::error::Error::Engine(format!("whisper JSON: {e}")))?;

    let transcript = parsed["transcription"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let words = transcript.split_whitespace().count() as u64;
    eprintln!(
        "[stt] {} words in {:.1}s ({}s audio)",
        words,
        started.elapsed().as_secs_f64(),
        wav_duration_estimate(std::fs::metadata(wav_path).map(|m| m.len()).unwrap_or(0))
    );
    Ok(transcript)
}

fn with_ext(p: &std::path::Path, ext: &str) -> std::path::PathBuf {
    let mut s = p.as_os_str().to_string_lossy().into_owned();
    s.push_str(ext);
    std::path::PathBuf::from(s)
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

fn wav_duration_estimate(bytes: u64) -> f64 {
    bytes as f64 / (16_000.0 * 2.0)
}

/// True when the CoreML encoder sits next to the model file (fast path).
/// whisper.cpp auto-detects it; this is for status reporting only.
pub fn coreml_enabled(state: &AppState) -> bool {
    let Some(model) = state.resolve_model(ModelKey::Whisper) else {
        return false;
    };
    let stem = model
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = model.parent().unwrap_or_else(|| std::path::Path::new("."));
    dir.join(format!("{stem}-encoder.mlmodelc"))
        .join("model.mil")
        .exists()
}
