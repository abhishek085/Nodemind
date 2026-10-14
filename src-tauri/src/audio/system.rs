//! System audio capture (the other side of a call) via `nodemind-helper tap`.
//!
//! The helper owns the Core Audio process tap (Swift, macOS 14.2+) and
//! streams `u32 LE sample rate` then mono `f32 LE` samples on stdout. This
//! side resamples to 16 kHz on a reader thread and hands audio to the caller,
//! exactly like `mic::MicCapture`, so both feed the same segmenter design.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio::mic::{Resampler, TARGET_RATE};
use crate::error::{Error, Result};

pub struct SystemAudio {
    child: Child,
    stdin: Option<ChildStdin>,
    thread: Option<JoinHandle<()>>,
}

impl SystemAudio {
    /// Start the tap. `on_audio` gets 16 kHz mono on the reader thread;
    /// `on_end` runs there once the stream ends.
    pub fn start<F, E>(helper: &Path, mut on_audio: F, on_end: E) -> Result<Self>
    where
        F: FnMut(&[f32]) + Send + 'static,
        E: FnOnce() + Send + 'static,
    {
        if !helper.exists() {
            return Err(Error::SidecarMissing("nodemind-helper".into()));
        }
        let mut child = Command::new(helper)
            .arg("tap")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Capture(format!("cannot start system audio capture: {e}")))?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or_else(|| Error::Capture("no tap output".into()))?;
        let mut stderr = child.stderr.take();

        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32>>();
        let thread = std::thread::Builder::new()
            .name("nodemind-system-audio".into())
            .spawn(move || {
                let mut header = [0u8; 4];
                if stdout.read_exact(&mut header).is_err() {
                    let mut msg = String::new();
                    if let Some(e) = stderr.as_mut() {
                        let _ = e.read_to_string(&mut msg);
                    }
                    let msg = msg.trim();
                    let _ = ready_tx.send(Err(Error::Capture(if msg.is_empty() {
                        "system audio capture exited".into()
                    } else {
                        msg.to_string()
                    })));
                    return;
                }
                let rate = u32::from_le_bytes(header);
                let _ = ready_tx.send(Ok(rate));

                let mut resampler = Resampler::new(rate, TARGET_RATE);
                let mut buf = vec![0u8; 16_384];
                let mut carry: Vec<u8> = Vec::with_capacity(4);
                let mut samples: Vec<f32> = Vec::with_capacity(4_096);
                let mut out: Vec<f32> = Vec::with_capacity(4_096);
                loop {
                    let n = match stdout.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    // Reads can split a sample; carry the remainder over.
                    carry.extend_from_slice(&buf[..n]);
                    let whole = carry.len() / 4 * 4;
                    samples.clear();
                    samples.extend(
                        carry[..whole]
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    );
                    carry.drain(..whole);
                    out.clear();
                    resampler.process(&samples, &mut out);
                    if !out.is_empty() {
                        on_audio(&out);
                    }
                }
                on_end();
            })
            .map_err(|e| Error::Capture(format!("cannot start system audio thread: {e}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(_rate)) => Ok(Self { child, stdin, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = child.wait();
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(Error::Capture("system audio capture did not start".into()))
            }
        }
    }

    /// Stop the helper and wait for the reader to flush. Blocking (briefly).
    pub fn stop(mut self) {
        // Closing stdin is the helper's signal to tear the tap down cleanly.
        if let Some(mut s) = self.stdin.take() {
            let _ = s.flush();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
