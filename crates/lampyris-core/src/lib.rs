//! # lampyris-core
//!
//! The detection engine of Lampyris. It is deliberately free of I/O: no network,
//! no disk, no async runtime. It takes posts in and returns events out, which makes
//! it easy to test, benchmark, replay and — later — compile to WebAssembly.
//!
//! The MVP signal is **coordinated near-duplicate posting**: many distinct accounts
//! publishing almost the same text inside a short time window.
//!
//! ```
//! use lampyris_core::{Detector, DetectorConfig, Post};
//!
//! let mut det = Detector::new(DetectorConfig { min_accounts: 3, ..Default::default() });
//! let text = "Vote for the people's candidate on Sunday, the only honest choice";
//! let mut events = Vec::new();
//! for i in 0..3 {
//!     let post = Post::new(format!("did:plc:account{i}"), format!("rkey{i}"), 1_000_000 * i, text);
//!     events.extend(det.push(&post));
//! }
//! assert_eq!(events.len(), 1);
//! ```

pub mod detector;
pub mod normalize;
pub mod post;
pub mod score;
pub mod simhash;

pub use detector::{ClusterSnapshot, Detector, DetectorConfig, DetectorEvent, DetectorStats};
pub use post::{Post, PostRef};
pub use score::{Score, Signals, SCORE_MODEL_VERSION};
