//! # lampyris-ingest
//!
//! Turns external data into [`lampyris_core::Post`]s and pushes them into a
//! bounded channel. The channel is the backpressure point: if the detector falls
//! behind, ingestion waits instead of buffering without limit, and the Jetstream
//! client resumes from its cursor if the server drops a slow connection.

pub mod jetstream;
pub mod replay;

pub use jetstream::{parse_jetstream, JetstreamConfig, DEFAULT_ENDPOINTS};
pub use replay::replay_file;
