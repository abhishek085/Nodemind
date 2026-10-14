//! Speech-to-text via the bundled whisper.cpp sidecars.
//!
//! Live path (per utterance, while the user is still talking):
//!   16 kHz mono f32 → in-memory wav → `whisper-server` /inference → text
//! Fallback when the server isn't running:
//!   wav file → `whisper-cli` → JSON (segments) → text
//!
//! Both run on Metal; the resident server keeps the model loaded.

use std::sync::Arc;
use std::time::Instant;

use crate::models::ModelKey;
use crate::prompts;
use crate::state::AppState;

/// Transcribe one utterance (16 kHz mono f32). `context` is the transcript so
/// far; its tail is passed as whisper's prompt so sentences continue
/// naturally across utterance boundaries.
pub async fn transcribe_segment(
    state: &Arc<AppState>,
    samples: &[f32],
    language: &str,
    context: &str,
) -> crate::error::Result<String> {
    let wav = wav_bytes(samples)?;
    if state.whisper_running.load(std::sync::atomic::Ordering::SeqCst) {
        match transcribe_via_server(wav.clone(), language, context).await {
            Ok(text) => return Ok(text),
            Err(e) => eprintln!("[stt] server failed, falling back to whisper-cli: {e}"),
        }
    }
    let s = state.clone();
    let language = language.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let tmp = tempfile::Builder::new()
            .prefix("nm-utt-")
            .suffix(".wav")
            .tempfile()
            .map_err(|e| crate::error::Error::Capture(format!("tempfile: {e}")))?;
        std::fs::write(tmp.path(), &wav)?;
        transcribe(&s, tmp.path(), &language)
    })
    .await
    .map_err(|e| crate::error::Error::Engine(format!("transcription task: {e}")))?
}

async fn transcribe_via_server(
    wav: Vec<u8>,
    language: &str,
    context: &str,
) -> crate::error::Result<String> {
    let tail: String = {
        let chars: Vec<char> = context.chars().collect();
        chars[chars.len().saturating_sub(200)..].iter().collect()
    };
    let mut prompt = tail;
    if language == "hi" {
        let hint = prompts::get("stt.hindi_initial_prompt");
        prompt = format!("{hint} {prompt}");
    }
    let file = reqwest::multipart::Part::bytes(wav)
        .file_name("utterance.wav")
        .mime_str("audio/wav")
        .map_err(crate::error::Error::Http)?;
    let form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("response_format", "json")
        .text("language", "en")
        .text("temperature", "0")
        .text("no_timestamps", "true")
        .text("prompt", prompt.trim().to_string());
    let resp = server_client()
        .post(format!(
            "http://127.0.0.1:{}/inference",
            crate::engine::sidecar::WHISPER_PORT
        ))
        .multipart(form)
        .send()
        .await?
        .error_for_status()?;
    let body: serde_json::Value = resp.json().await?;
    Ok(strip_markers(body["text"].as_str().unwrap_or("")))
}

fn server_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("reqwest client")
    })
}

/// Encode 16 kHz mono f32 samples as a 16-bit wav, in memory.
fn wav_bytes(samples: &[f32]) -> crate::error::Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    {
        let mut w = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| crate::error::Error::Capture(format!("wav: {e}")))?;
        for &s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .map_err(|e| crate::error::Error::Capture(format!("wav: {e}")))?;
        }
        w.finalize()
            .map_err(|e| crate::error::Error::Capture(format!("wav: {e}")))?;
    }
    Ok(cursor.into_inner())
}

/// Remove whisper's non-speech annotations (`[BLANK_AUDIO]`, `(music)`, …)
/// from a whole-utterance transcript.
fn strip_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if is_non_speech_marker(&out) {
        String::new()
    } else {
        out
    }
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

    let transcript = transcript_from_json(&parsed);
    let words = transcript.split_whitespace().count() as u64;
    eprintln!(
        "[stt] {} words in {:.1}s ({}s audio)",
        words,
        started.elapsed().as_secs_f64(),
        wav_duration_estimate(std::fs::metadata(wav_path).map(|m| m.len()).unwrap_or(0))
    );
    Ok(transcript)
}

/// Join whisper-cli's `-oj` output into one transcript.
///
/// `transcription` is an array of segments (`{ "text": " Hello", ... }`).
/// Non-speech markers whisper emits for silence/noise (`[BLANK_AUDIO]`,
/// `(music)`, …) are dropped so an empty recording yields an empty string.
fn transcript_from_json(parsed: &serde_json::Value) -> String {
    let segments: Vec<&str> = match &parsed["transcription"] {
        serde_json::Value::Array(segs) => segs.iter().filter_map(|s| s["text"].as_str()).collect(),
        serde_json::Value::String(s) => vec![s.as_str()],
        _ => Vec::new(),
    };
    segments
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_non_speech_marker(s))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_non_speech_marker(s: &str) -> bool {
    let bracketed = (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('(') && s.ends_with(')'));
    bracketed && !s[1..s.len() - 1].contains(['[', ']', '(', ')'])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_segment_array() {
        let v = serde_json::json!({ "transcription": [
            { "text": " Remind me to call Priya." },
            { "text": " Then finish the slides." }
        ]});
        assert_eq!(
            transcript_from_json(&v),
            "Remind me to call Priya. Then finish the slides."
        );
    }

    #[test]
    fn drops_non_speech_markers() {
        let v = serde_json::json!({ "transcription": [
            { "text": " [BLANK_AUDIO]" },
            { "text": " (keyboard clicking)" }
        ]});
        assert_eq!(transcript_from_json(&v), "");
    }

    #[test]
    fn strips_server_markers() {
        assert_eq!(strip_markers(" [BLANK_AUDIO]"), "");
        assert_eq!(strip_markers(" (wind blowing)"), "");
        assert_eq!(strip_markers(" Call Priya. [MUSIC] Then  the slides."), "Call Priya. Then the slides.");
    }

    #[test]
    fn wav_roundtrip() {
        let wav = wav_bytes(&[0.0, 0.5, -0.5]).unwrap();
        let r = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn accepts_plain_string() {
        let v = serde_json::json!({ "transcription": " hello " });
        assert_eq!(transcript_from_json(&v), "hello");
    }

    /// Live path against a running whisper-server on WHISPER_PORT:
    /// `NM_WAV=/path/16k-mono.wav cargo test live_path -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_path_over_real_speech() {
        let path = std::env::var("NM_WAV").expect("set NM_WAV");
        let mut r = hound::WavReader::open(path).unwrap();
        let samples: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
        let mut seg = crate::audio::segmenter::Segmenter::new();
        let mut utterances = Vec::new();
        for chunk in samples.chunks(1_600) {
            utterances.extend(seg.push(chunk));
        }
        utterances.extend(seg.finish());
        let mut text = String::new();
        for u in &utterances {
            let t0 = std::time::Instant::now();
            let t = transcribe_via_server(wav_bytes(u).unwrap(), "en", &text).await.unwrap();
            println!("{:>5.1}s audio → {:.2}s: {t}", u.len() as f32 / 16_000.0, t0.elapsed().as_secs_f32());
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&t);
        }
        assert!(text.to_lowercase().contains("priya"));
    }
}
