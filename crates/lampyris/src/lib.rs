//! # lampyris
//!
//! Wires the pieces together: a source (live Jetstream or a recording) feeds the
//! detector through a bounded channel; detected clusters are stored locally and
//! forwarded to whoever is listening — the command line or the desktop UI.
//!
//! ```text
//!  source ──posts──▶ [bounded channel] ──▶ detector thread ──events──▶ store + listener
//! ```
//!
//! The pipeline owns its threads and its async runtime, so callers (including the
//! Qt event loop) never need to run async code themselves.

pub mod format;
pub mod pipeline;

pub use pipeline::{default_db_path, Pipeline, PipelineConfig, PipelineMsg, Source};
