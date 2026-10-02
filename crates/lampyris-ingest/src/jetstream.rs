//! Bluesky Jetstream client.
//!
//! Jetstream is a lightweight JSON view of the Bluesky firehose, served over a
//! public WebSocket with no account or API key. We subscribe to new posts only
//! (`app.bsky.feed.post`) and keep the last event time as a cursor, so after a
//! disconnect we resume where we left off instead of losing data.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use lampyris_core::Post;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// Public Jetstream instances operated by Bluesky. The client rotates through
/// them when a connection fails.
pub const DEFAULT_ENDPOINTS: &[&str] = &[
    "wss://jetstream2.us-east.bsky.network/subscribe",
    "wss://jetstream1.us-east.bsky.network/subscribe",
    "wss://jetstream1.us-west.bsky.network/subscribe",
    "wss://jetstream2.us-west.bsky.network/subscribe",
];

const POST_COLLECTION: &str = "app.bsky.feed.post";
/// Rewind applied to the cursor on reconnect so no event falls in the gap.
const CURSOR_REWIND_US: i64 = 2_000_000;

#[derive(Debug, Clone)]
pub struct JetstreamConfig {
    pub endpoints: Vec<String>,
    /// Start from this event time (µs since epoch). `None` means "live".
    pub cursor: Option<i64>,
    /// If set, every raw message is appended to this JSONL file for later replay.
    pub record_to: Option<std::path::PathBuf>,
}

impl Default for JetstreamConfig {
    fn default() -> Self {
        Self {
            endpoints: DEFAULT_ENDPOINTS.iter().map(|s| s.to_string()).collect(),
            cursor: None,
            record_to: None,
        }
    }
}

// --- wire format -----------------------------------------------------------

#[derive(Deserialize)]
struct Event {
    did: String,
    time_us: i64,
    kind: String,
    commit: Option<Commit>,
}

#[derive(Deserialize)]
struct Commit {
    operation: String,
    collection: String,
    rkey: String,
    record: Option<Record>,
}

#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    text: String,
    #[serde(default)]
    langs: Vec<String>,
    embed: Option<Embed>,
}

#[derive(Deserialize)]
struct Embed {
    #[serde(rename = "$type", default)]
    kind: String,
    media: Option<Box<Embed>>,
}

impl Embed {
    fn is_external(&self) -> bool {
        self.kind.contains("embed.external") || self.media.as_ref().is_some_and(|m| m.is_external())
    }
}

/// Parse one Jetstream message. Returns `Ok(None)` for anything that is not a
/// newly created post (likes, deletes, identity events, ...).
pub fn parse_jetstream(raw: &str) -> Result<Option<(Post, i64)>> {
    let ev: Event = serde_json::from_str(raw).context("invalid Jetstream JSON")?;
    let cursor = ev.time_us;
    if ev.kind != "commit" {
        return Ok(None);
    }
    let Some(commit) = ev.commit else {
        return Ok(None);
    };
    if commit.operation != "create" || commit.collection != POST_COLLECTION {
        return Ok(None);
    }
    let Some(record) = commit.record else {
        return Ok(None);
    };
    let has_external_link = record.embed.as_ref().is_some_and(|e| e.is_external());
    Ok(Some((
        Post {
            author: ev.did,
            id: commit.rkey,
            time_us: ev.time_us,
            text: record.text,
            langs: record.langs,
            has_external_link,
        },
        cursor,
    )))
}

fn subscribe_url(endpoint: &str, cursor: Option<i64>) -> Result<url::Url> {
    let mut url = url::Url::parse(endpoint).with_context(|| format!("bad endpoint {endpoint}"))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("wantedCollections", POST_COLLECTION);
        if let Some(c) = cursor {
            q.append_pair("cursor", &c.to_string());
        }
    }
    Ok(url)
}

/// Stream posts from Jetstream into `tx` until the receiver is dropped.
///
/// Reconnects forever with exponential backoff (1 s → 60 s), rotating endpoints
/// and resuming from the last seen event time.
pub async fn run(config: JetstreamConfig, tx: mpsc::Sender<Post>) -> Result<()> {
    anyhow::ensure!(
        !config.endpoints.is_empty(),
        "no Jetstream endpoints configured"
    );
    // More than one TLS backend can end up compiled in through dependencies;
    // pick one explicitly so rustls does not panic at connect time.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut recorder = match &config.record_to {
        Some(path) => Some(tokio::io::BufWriter::new(
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
                .with_context(|| format!("cannot open {}", path.display()))?,
        )),
        None => None,
    };

    let mut cursor = config.cursor;
    let mut backoff = Duration::from_secs(1);
    let mut attempt = 0usize;

    loop {
        let endpoint = &config.endpoints[attempt % config.endpoints.len()];
        attempt += 1;
        let resume = cursor.map(|c| c - CURSOR_REWIND_US);
        let url = subscribe_url(endpoint, resume)?;
        tracing::info!(%endpoint, cursor = ?resume, "connecting to Jetstream");

        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                tracing::info!(%endpoint, "connected");
                backoff = Duration::from_secs(1);
                while let Some(msg) = ws.next().await {
                    let text = match msg {
                        Ok(Message::Text(t)) => t,
                        Ok(Message::Close(frame)) => {
                            tracing::warn!(?frame, "server closed the connection");
                            break;
                        }
                        Ok(_) => continue, // ping/pong/binary
                        Err(e) => {
                            tracing::warn!(error = %e, "WebSocket error");
                            break;
                        }
                    };
                    if let Some(w) = recorder.as_mut() {
                        w.write_all(text.as_bytes()).await?;
                        w.write_all(b"\n").await?;
                    }
                    match parse_jetstream(&text) {
                        Ok(Some((post, ev_cursor))) => {
                            cursor = Some(ev_cursor);
                            if tx.send(post).await.is_err() {
                                if let Some(w) = recorder.as_mut() {
                                    w.flush().await?;
                                }
                                return Ok(()); // detector shut down
                            }
                        }
                        Ok(None) => {}
                        Err(e) => tracing::debug!(error = %e, "skipping malformed message"),
                    }
                }
            }
            Err(e) => tracing::warn!(%endpoint, error = %e, "connection failed"),
        }

        if let Some(w) = recorder.as_mut() {
            w.flush().await?;
        }
        if tx.is_closed() {
            return Ok(());
        }
        tracing::info!(delay = ?backoff, "reconnecting");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POST: &str = r#"{"did":"did:plc:abc","time_us":1727000000000000,"kind":"commit","commit":{"rev":"x","operation":"create","collection":"app.bsky.feed.post","rkey":"3l3qo2vutsw2b","record":{"$type":"app.bsky.feed.post","createdAt":"2026-10-02T12:00:00Z","langs":["es"],"text":"hola mundo"},"cid":"y"}}"#;

    #[test]
    fn parses_post_creation() {
        let (post, cursor) = parse_jetstream(POST).unwrap().unwrap();
        assert_eq!(post.author, "did:plc:abc");
        assert_eq!(post.id, "3l3qo2vutsw2b");
        assert_eq!(post.text, "hola mundo");
        assert_eq!(post.langs, vec!["es"]);
        assert!(!post.has_external_link);
        assert_eq!(cursor, 1727000000000000);
    }

    #[test]
    fn detects_external_link_embeds() {
        let raw = POST.replace(
            r#""text":"hola mundo""#,
            r#""text":"hola mundo","embed":{"$type":"app.bsky.embed.external","external":{"uri":"https://e.com"}}"#,
        );
        assert!(parse_jetstream(&raw).unwrap().unwrap().0.has_external_link);

        let nested = POST.replace(
            r#""text":"hola mundo""#,
            r#""text":"hola mundo","embed":{"$type":"app.bsky.embed.recordWithMedia","media":{"$type":"app.bsky.embed.external"}}"#,
        );
        assert!(
            parse_jetstream(&nested)
                .unwrap()
                .unwrap()
                .0
                .has_external_link
        );
    }

    #[test]
    fn ignores_non_post_events() {
        let delete = POST.replace(r#""operation":"create""#, r#""operation":"delete""#);
        assert!(parse_jetstream(&delete).unwrap().is_none());
        let like = POST.replace("app.bsky.feed.post\",\"rkey", "app.bsky.feed.like\",\"rkey");
        assert!(parse_jetstream(&like).unwrap().is_none());
        let identity = r#"{"did":"did:plc:abc","time_us":1,"kind":"identity","identity":{}}"#;
        assert!(parse_jetstream(identity).unwrap().is_none());
    }

    #[test]
    fn builds_subscribe_url() {
        let u = subscribe_url(DEFAULT_ENDPOINTS[0], Some(42)).unwrap();
        assert_eq!(
            u.query(),
            Some("wantedCollections=app.bsky.feed.post&cursor=42")
        );
    }
}
