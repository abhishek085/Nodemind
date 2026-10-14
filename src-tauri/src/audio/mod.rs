//! Audio capture, owned by the backend so recording works from the menu bar
//! and the global hotkey with no window open.
//!
//! - [`mic`]       — CoreAudio input → 16 kHz mono `f32`, on its own thread.
//! - [`segmenter`] — splits the stream into utterances at natural pauses.
//! - [`system`]    — the Mac's audio output (other side of a call), via the
//!   Swift `nodemind-helper tap` sidecar; used in meeting mode.
//! - [`session`]   — one live capture: utterances are transcribed as they
//!   finish, then the transcript is organized when the user stops.

pub mod mic;
pub mod segmenter;
pub mod session;
pub mod system;
