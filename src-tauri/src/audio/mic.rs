//! Microphone capture via CoreAudio (cpal).
//!
//! The cpal stream is not `Send`, so it lives on a dedicated thread for the
//! whole session. The realtime callback only downmixes to mono and hands the
//! buffer over a channel; resampling to 16 kHz and the caller's processing run
//! on the capture thread, never in the audio callback.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::error::{Error, Result};

/// Sample rate everything downstream (whisper, the segmenter) expects.
pub const TARGET_RATE: u32 = 16_000;

/// A running capture. Dropping it without `stop()` also stops the thread.
pub struct MicCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MicCapture {
    /// Open the default input device and stream 16 kHz mono `f32` into
    /// `on_audio` (called on the capture thread). `on_end` runs on the same
    /// thread after the stream closes, so it can flush whatever it buffered.
    pub fn start<F, E>(mut on_audio: F, on_end: E) -> Result<Self>
    where
        F: FnMut(&[f32]) + Send + 'static,
        E: FnOnce() + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stop_flag = stop.clone();

        let thread = std::thread::Builder::new()
            .name("nodemind-mic".into())
            .spawn(move || {
                let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<f32>>();
                let (stream, in_rate) = match open_stream(pcm_tx) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));

                let mut resampler = Resampler::new(in_rate, TARGET_RATE);
                let mut out = Vec::with_capacity(4_096);
                while !stop_flag.load(Ordering::SeqCst) {
                    match pcm_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(mono) => {
                            out.clear();
                            resampler.process(&mono, &mut out);
                            if !out.is_empty() {
                                on_audio(&out);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                drop(stream);
                // Drain what the callback delivered before the stream closed.
                for mono in pcm_rx.try_iter() {
                    out.clear();
                    resampler.process(&mono, &mut out);
                    on_audio(&out);
                }
                on_end();
            })
            .map_err(|e| Error::Capture(format!("cannot start capture thread: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(Error::Capture("capture thread exited during startup".into())),
        }
    }

    /// Stop the stream and wait for `on_end` to finish. Blocking (briefly).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MicCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn open_stream(tx: mpsc::Sender<Vec<f32>>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| Error::Capture("no microphone found — connect one or pick an input in System Settings → Sound".into()))?;
    let supported = device
        .default_input_config()
        .map_err(|e| Error::Capture(format!("cannot read microphone format: {e}")))?;
    let config = supported.config();
    let rate = config.sample_rate;
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, tx),
        SampleFormat::I16 => build::<i16>(&device, &config, tx),
        SampleFormat::I32 => build::<i32>(&device, &config, tx),
        SampleFormat::U16 => build::<u16>(&device, &config, tx),
        other => return Err(Error::Capture(format!("unsupported microphone format {other:?}"))),
    }?;
    stream
        .play()
        .map_err(|e| Error::Capture(format!("cannot start microphone: {e}")))?;
    Ok((stream, rate))
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, tx: mpsc::Sender<Vec<f32>>) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    device
        .build_input_stream::<T, _, _>(
            config.clone(),
            move |data: &[T], _| {
                let mono: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / channels as f32)
                    .collect();
                let _ = tx.send(mono);
            },
            |e| eprintln!("[mic] stream error: {e}"),
            None,
        )
        .map_err(|e| Error::Capture(format!("cannot open microphone: {e}")))
}

/// Streaming box-filter downsampler (averages each output period). Crude but
/// alias-safe enough for speech, and stateful across buffers.
pub(crate) struct Resampler {
    ratio: f64,
    acc: f64,
    frac: f64,
}

impl Resampler {
    pub(crate) fn new(in_rate: u32, out_rate: u32) -> Self {
        Self { ratio: in_rate as f64 / out_rate as f64, acc: 0.0, frac: 0.0 }
    }

    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if (self.ratio - 1.0).abs() < f64::EPSILON {
            out.extend_from_slice(input);
            return;
        }
        for &s in input {
            self.acc += s as f64;
            self.frac += 1.0;
            if self.frac >= self.ratio {
                out.push((self.acc / self.frac) as f32);
                self.acc = 0.0;
                self.frac -= self.ratio;
            }
        }
    }
}

/// Perceptual 0..1 level for the UI meter (RMS, square-rooted again so normal
/// speech sits mid-scale).
pub fn level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    (rms.sqrt() * 1.6).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_hits_target_rate() {
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = Vec::new();
        for _ in 0..10 {
            r.process(&vec![0.5; 4_800], &mut out); // 1 s in 100 ms buffers
        }
        assert!((out.len() as i64 - 16_000).abs() <= 1);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-4));
    }

    #[test]
    fn resampler_handles_fractional_ratio() {
        let mut r = Resampler::new(44_100, 16_000);
        let mut out = Vec::new();
        r.process(&vec![0.0; 44_100], &mut out);
        assert!((out.len() as i64 - 16_000).abs() <= 1);
    }
}
