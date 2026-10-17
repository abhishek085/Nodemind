//! Pause-based speech segmenter for live transcription.
//!
//! Splits a 16 kHz mono stream into utterances at natural pauses so each one
//! can be transcribed while the user keeps talking. A simple adaptive energy
//! gate decides speech vs. silence per 20 ms frame; stretches with no speech
//! are dropped, which also keeps whisper from hallucinating on silence.

/// 20 ms at 16 kHz.
const FRAME: usize = 320;
const FRAME_MS: u32 = 20;
/// Never cut a segment shorter than this (whisper needs some context).
const MIN_SEGMENT_MS: u32 = 2_000;
/// Hard cap so a monologue without pauses still streams.
const MAX_SEGMENT_MS: u32 = 20_000;
/// Less speech than this in a segment is treated as noise and dropped.
const MIN_SPEECH_MS: u32 = 300;
/// Audio kept from before speech starts (soft consonants live here).
const PRE_ROLL_MS: u32 = 300;
/// Absolute floor for "speech" RMS, whatever the noise estimate says.
const MIN_SPEECH_RMS: f32 = 0.008;

/// How long a pause must be to end an utterance of `seg_ms`. Short
/// utterances wait for a clear breath; longer ones take any sentence break,
/// so fluent speech still streams in ~5–10 s pieces.
fn required_pause_ms(seg_ms: u32) -> u32 {
    match seg_ms {
        0..=5_999 => 600,
        6_000..=11_999 => 260,
        _ => 140,
    }
}

pub struct Segmenter {
    buf: Vec<f32>,
    frame: Vec<f32>,
    speech_ms: u32,
    trailing_silence_ms: u32,
    noise_floor: f32,
}

impl Default for Segmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Segmenter {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(16 * MAX_SEGMENT_MS as usize),
            frame: Vec::with_capacity(FRAME),
            speech_ms: 0,
            trailing_silence_ms: 0,
            noise_floor: 0.004,
        }
    }

    /// Feed samples; returns any utterances completed by them.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        let mut out = Vec::new();
        for &s in samples {
            self.frame.push(s);
            if self.frame.len() == FRAME {
                if let Some(seg) = self.process_frame() {
                    out.push(seg);
                }
            }
        }
        out
    }

    /// End of stream: the final utterance, if it holds any speech.
    pub fn finish(mut self) -> Option<Vec<f32>> {
        let rest = std::mem::take(&mut self.frame);
        self.buf.extend_from_slice(&rest);
        self.take_segment()
    }

    fn process_frame(&mut self) -> Option<Vec<f32>> {
        let rms = (self.frame.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt();
        let is_speech = rms > MIN_SPEECH_RMS.max(self.noise_floor * 3.0);
        // The floor tracks background noise only; letting speech nudge it
        // would, over a long monologue, raise it until speech reads as silence.
        if !is_speech {
            self.noise_floor += (rms - self.noise_floor) * 0.05;
        }

        self.buf.extend_from_slice(&self.frame);
        self.frame.clear();
        if is_speech {
            self.speech_ms += FRAME_MS;
            self.trailing_silence_ms = 0;
        } else {
            self.trailing_silence_ms += FRAME_MS;
        }

        let seg_ms = (self.buf.len() / 16) as u32;
        if self.speech_ms == 0 && seg_ms > PRE_ROLL_MS {
            // Nothing said yet: keep only the pre-roll.
            let keep = 16 * PRE_ROLL_MS as usize;
            self.buf.drain(..self.buf.len() - keep);
            return None;
        }
        let at_pause =
            seg_ms >= MIN_SEGMENT_MS && self.trailing_silence_ms >= required_pause_ms(seg_ms);
        if at_pause || seg_ms >= MAX_SEGMENT_MS {
            return self.take_segment();
        }
        None
    }

    fn take_segment(&mut self) -> Option<Vec<f32>> {
        let has_speech = self.speech_ms >= MIN_SPEECH_MS;
        // Trim the trailing pause down to a short tail.
        let excess_ms = self.trailing_silence_ms.saturating_sub(PRE_ROLL_MS);
        let trim = (16 * excess_ms as usize).min(self.buf.len());
        self.buf.truncate(self.buf.len() - trim);
        let seg = std::mem::take(&mut self.buf);
        self.speech_ms = 0;
        self.trailing_silence_ms = 0;
        has_speech.then_some(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: u32) -> Vec<f32> {
        (0..16 * ms as usize)
            .map(|i| 0.2 * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / 16_000.0).sin())
            .collect()
    }

    fn silence(ms: u32) -> Vec<f32> {
        vec![0.0; 16 * ms as usize]
    }

    #[test]
    fn silence_yields_nothing() {
        let mut s = Segmenter::new();
        assert!(s.push(&silence(5_000)).is_empty());
        assert!(s.finish().is_none());
    }

    #[test]
    fn cuts_at_pauses() {
        let mut s = Segmenter::new();
        let mut segs = s.push(&tone(2_500));
        segs.extend(s.push(&silence(800)));
        assert_eq!(segs.len(), 1, "first utterance should be cut at the pause");
        segs.extend(s.push(&tone(1_000)));
        let last = s.finish().expect("tail utterance");
        // ~1 s of speech plus a short pre-roll.
        assert!(last.len() >= 16_000 && last.len() <= 16 * 1_400);
    }

    #[test]
    fn short_phrase_is_not_cut_early() {
        let mut s = Segmenter::new();
        let mut segs = s.push(&tone(800));
        segs.extend(s.push(&silence(700)));
        assert!(segs.is_empty(), "segments under MIN_SEGMENT_MS wait for more speech");
    }

    #[test]
    fn long_monologue_is_capped() {
        let mut s = Segmenter::new();
        let segs = s.push(&tone(45_000));
        assert_eq!(segs.len(), 2);
        assert!(segs.iter().all(|seg| seg.len() <= 16 * MAX_SEGMENT_MS as usize));
    }

    /// Real-speech check: `NM_WAV=/path/16k-mono.wav cargo test segment_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn segment_real_wav() {
        let path = std::env::var("NM_WAV").expect("set NM_WAV");
        let mut r = hound::WavReader::open(path).unwrap();
        let samples: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
        let mut seg = Segmenter::new();
        let mut segs = Vec::new();
        for chunk in samples.chunks(1_600) {
            segs.extend(seg.push(chunk));
        }
        segs.extend(seg.finish());
        for (i, s) in segs.iter().enumerate() {
            println!("utterance {i}: {:.1}s", s.len() as f32 / 16_000.0);
        }
        assert!(!segs.is_empty());
    }
}
