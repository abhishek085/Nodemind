//! Calendar integration (read-only).
//!
//! - [`eventkit`] — EventKit via the Swift helper (preferred, instant).
//! - [`apple`]    — AppleScript fallback when the helper isn't available.

pub mod apple;
pub mod eventkit;
