//! Replay of recorded Jetstream traffic.
//!
//! Recordings (`lampyris watch --record file.jsonl`) make investigations
//! reproducible: anyone with the same file and the same detector settings gets
//! exactly the same clusters, because the detector only uses event time.

use std::path::Path;

use anyhow::{Context, Result};
use lampyris_core::Post;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::jetstream::parse_jetstream;

/// Read a JSONL file of raw Jetstream messages and send its posts to `tx`.
/// Returns the number of posts sent.
pub async fn replay_file(path: &Path, tx: mpsc::Sender<Post>) -> Result<u64> {
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("cannot open {}", path.display()))?;
    let mut lines = BufReader::new(file).lines();
    let mut sent = 0u64;
    let mut line_no = 0u64;
    while let Some(line) = lines.next_line().await? {
        line_no += 1;
        if line.trim().is_empty() {
            continue;
        }
        match parse_jetstream(&line) {
            Ok(Some((post, _))) => {
                if tx.send(post).await.is_err() {
                    break;
                }
                sent += 1;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(line = line_no, error = %e, "skipping malformed line"),
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replays_posts_and_skips_noise() {
        let dir = std::env::temp_dir().join(format!("lampyris-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.jsonl");
        let post = r#"{"did":"did:plc:a","time_us":1,"kind":"commit","commit":{"operation":"create","collection":"app.bsky.feed.post","rkey":"r1","record":{"text":"hola"}}}"#;
        let like = r#"{"did":"did:plc:a","time_us":2,"kind":"commit","commit":{"operation":"create","collection":"app.bsky.feed.like","rkey":"r2","record":{}}}"#;
        std::fs::write(&path, format!("{post}\nnot json\n\n{like}\n{post}\n")).unwrap();

        let (tx, mut rx) = mpsc::channel(16);
        let sent = replay_file(&path, tx).await.unwrap();
        assert_eq!(sent, 2);
        assert_eq!(rx.recv().await.unwrap().id, "r1");
        std::fs::remove_dir_all(&dir).ok();
    }
}
