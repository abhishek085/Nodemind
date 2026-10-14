//! The embedded data store: one SQLite database + an embedded vector index.
//!
//! - [`db`]  — schema and CRUD for thoughts, tasks, calendar cache, focus.
//! - [`vec_`] — vector embeddings stored as BLOBs in SQLite, brute-force
//!              cosine kNN (fine for personal scale: thousands of rows).
//! - [`graph`] — the thought-graph with fuzzy entity merging via vectors.

pub mod db;
pub mod graph;
#[allow(dead_code)]
pub mod vec_;
