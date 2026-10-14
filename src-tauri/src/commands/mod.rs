//! The Tauri command surface — the only API the frontend talks to.
//!
//! One command per responsibility; each stays small. Async commands run on
//! the async runtime (never the UI thread); blocking work inside is moved to
//! `spawn_blocking`.

pub mod calendar;
pub mod capture;
pub mod engine;
pub mod settings;
pub mod thoughts;
pub mod updates;
