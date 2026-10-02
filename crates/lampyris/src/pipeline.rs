use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use lampyris_core::{Detector, DetectorConfig, DetectorEvent, DetectorStats, Post};
use lampyris_ingest::JetstreamConfig;
use lampyris_storage::Store;
use tokio::sync::{mpsc, watch};

/// Posts buffered between ingestion and detection. When full, ingestion waits.
const POST_CHANNEL_CAPACITY: usize = 10_000;
const STATS_EVERY: Duration = Duration::from_secs(2);
const PRUNE_EVERY: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub enum Source {
    /// Live Bluesky firehose.
    Jetstream(JetstreamConfig),
    /// A JSONL file recorded with `lampyris watch --record`.
    Replay(PathBuf),
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub source: Source,
    pub detector: DetectorConfig,
    /// Where clusters are stored. `None` keeps nothing on disk.
    pub db_path: Option<PathBuf>,
    /// Clusters older than this (in event time) are deleted from the store.
    pub retention_days: u32,
}

#[derive(Debug, Clone)]
pub enum PipelineMsg {
    Event(DetectorEvent),
    Stats(DetectorStats),
    /// Something went wrong in a stage; the message is meant for humans.
    Error(String),
    /// The source ended (replay finished, or the pipeline was stopped) and all
    /// remaining clusters were flushed.
    Finished(DetectorStats),
}

/// Default database location: `$XDG_DATA_HOME/lampyris/lampyris.db`.
pub fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lampyris")
        .join("lampyris.db")
}

/// A running pipeline. Dropping it stops the pipeline.
pub struct Pipeline {
    pub run_id: String,
    rx: std_mpsc::Receiver<PipelineMsg>,
    stop_tx: watch::Sender<bool>,
    threads: Vec<JoinHandle<()>>,
}

impl Pipeline {
    pub fn start(config: PipelineConfig) -> Result<Self> {
        let run_id = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
        let store = match &config.db_path {
            Some(p) => Some(Store::open(p).with_context(|| format!("opening {}", p.display()))?),
            None => None,
        };

        let (post_tx, post_rx) = mpsc::channel::<Post>(POST_CHANNEL_CAPACITY);
        let (out_tx, out_rx) = std_mpsc::channel::<PipelineMsg>();
        let (stop_tx, stop_rx) = watch::channel(false);

        let ingest = spawn_ingest(config.source.clone(), post_tx, stop_rx, out_tx.clone())?;
        let detect = spawn_detector(
            config.detector.clone(),
            config.retention_days,
            store,
            run_id.clone(),
            post_rx,
            out_tx,
        )?;

        Ok(Self {
            run_id,
            rx: out_rx,
            stop_tx,
            threads: vec![ingest, detect],
        })
    }

    /// Blocking receive. Returns `None` once the pipeline has fully stopped.
    pub fn recv(&self) -> Option<PipelineMsg> {
        self.rx.recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<PipelineMsg> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn try_recv(&self) -> Option<PipelineMsg> {
        self.rx.try_recv().ok()
    }

    /// Ask the source to stop. The detector then flushes and sends `Finished`.
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Stop and wait for every thread to exit.
    pub fn join(mut self) {
        self.stop();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
    }
}

fn spawn_ingest(
    source: Source,
    post_tx: mpsc::Sender<Post>,
    mut stop_rx: watch::Receiver<bool>,
    out_tx: std_mpsc::Sender<PipelineMsg>,
) -> Result<JoinHandle<()>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building async runtime")?;
    Ok(std::thread::Builder::new()
        .name("lampyris-ingest".into())
        .spawn(move || {
            rt.block_on(async move {
                let work = async {
                    match source {
                        Source::Jetstream(cfg) => {
                            lampyris_ingest::jetstream::run(cfg, post_tx).await
                        }
                        Source::Replay(path) => {
                            lampyris_ingest::replay_file(&path, post_tx).await.map(|n| {
                                tracing::info!(posts = n, "replay finished");
                            })
                        }
                    }
                };
                tokio::select! {
                    res = work => {
                        if let Err(e) = res {
                            let _ = out_tx.send(PipelineMsg::Error(format!("{e:#}")));
                        }
                    }
                    _ = stop_rx.wait_for(|stop| *stop) => {
                        tracing::info!("ingestion stopped");
                    }
                }
                // Dropping the sender here lets the detector drain and finish.
            });
        })?)
}

fn spawn_detector(
    config: DetectorConfig,
    retention_days: u32,
    mut store: Option<Store>,
    run_id: String,
    mut post_rx: mpsc::Receiver<Post>,
    out_tx: std_mpsc::Sender<PipelineMsg>,
) -> Result<JoinHandle<()>> {
    Ok(std::thread::Builder::new()
        .name("lampyris-detector".into())
        .spawn(move || {
            let mut detector = Detector::new(config);
            let mut last_stats = Instant::now();
            let mut last_prune = Instant::now();
            let mut last_event_time = 0i64;

            let handle = |events: Vec<DetectorEvent>, store: &mut Option<Store>| {
                for ev in events {
                    if let Some(s) = store.as_mut() {
                        if let Err(e) = s.upsert(&run_id, ev.snapshot()) {
                            let _ = out_tx.send(PipelineMsg::Error(format!("storage: {e:#}")));
                        }
                    }
                    let _ = out_tx.send(PipelineMsg::Event(ev));
                }
            };

            while let Some(post) = post_rx.blocking_recv() {
                last_event_time = last_event_time.max(post.time_us);
                let events = detector.push(&post);
                handle(events, &mut store);

                if last_stats.elapsed() >= STATS_EVERY {
                    last_stats = Instant::now();
                    let _ = out_tx.send(PipelineMsg::Stats(detector.stats()));
                }
                if last_prune.elapsed() >= PRUNE_EVERY {
                    last_prune = Instant::now();
                    if let Some(s) = store.as_mut() {
                        let cutoff =
                            last_event_time - i64::from(retention_days) * 86_400 * 1_000_000;
                        match s.prune_before(cutoff) {
                            Ok(n) if n > 0 => tracing::info!(removed = n, "pruned old clusters"),
                            Ok(_) => {}
                            Err(e) => tracing::warn!(error = %e, "prune failed"),
                        }
                    }
                }
            }

            let events = detector.flush();
            handle(events, &mut store);
            let _ = out_tx.send(PipelineMsg::Finished(detector.stats()));
        })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(did: &str, rkey: &str, t_us: i64, text: &str) -> String {
        serde_json::json!({
            "did": did, "time_us": t_us, "kind": "commit",
            "commit": {"operation": "create", "collection": "app.bsky.feed.post", "rkey": rkey,
                       "record": {"text": text, "langs": ["en"]}}
        })
        .to_string()
    }

    #[test]
    fn replay_end_to_end_stores_clusters() {
        let dir = std::env::temp_dir().join(format!("lampyris-pipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rec = dir.join("rec.jsonl");
        let db = dir.join("test.db");
        let mut f = std::fs::File::create(&rec).unwrap();
        let campaign = "everyone is talking about the miracle cure the doctors are hiding from us";
        let humans = [
            "took the kids to the park and they would not stop chasing pigeons",
            "finally finished that novel, the ending made me cry on the bus",
            "new coffee shop downtown has the best croissants I have ever eaten",
            "rain all weekend so we stayed home and played board games",
            "my tomato plants are growing faster than I expected this spring",
            "anyone else watching the cycling race this afternoon on television",
            "spent three hours debugging only to find a missing semicolon again",
            "grandma turned ninety today and still beats everyone at cards",
            "the concert last night was loud, sweaty and absolutely perfect",
            "trying to learn the guitar at forty, fingers hurt but worth it",
            "lost my keys twice before nine in the morning, new personal record",
            "our neighborhood cleanup collected eleven bags of trash by noon",
        ];
        for i in 0..12 {
            writeln!(
                f,
                "{}",
                line(
                    &format!("did:plc:bot{i}"),
                    &format!("r{i}"),
                    i * 3_000_000,
                    campaign
                )
            )
            .unwrap();
            writeln!(
                f,
                "{}",
                line(
                    &format!("did:plc:human{i}"),
                    &format!("h{i}"),
                    i * 3_000_000 + 1,
                    humans[i as usize]
                )
            )
            .unwrap();
        }
        drop(f);

        let p = Pipeline::start(PipelineConfig {
            source: Source::Replay(rec),
            detector: DetectorConfig::default(),
            db_path: Some(db.clone()),
            retention_days: 30,
        })
        .unwrap();

        let mut reported = 0;
        let mut closed = 0;
        let mut finished = None;
        while let Some(msg) = p.recv_timeout(Duration::from_secs(10)) {
            match msg {
                PipelineMsg::Event(DetectorEvent::ClusterReported(_)) => reported += 1,
                PipelineMsg::Event(DetectorEvent::ClusterClosed(_)) => closed += 1,
                PipelineMsg::Finished(s) => {
                    finished = Some(s);
                    break;
                }
                PipelineMsg::Error(e) => panic!("{e}"),
                PipelineMsg::Stats(_) => {}
            }
        }
        p.join();

        let stats = finished.expect("pipeline finished");
        assert_eq!(stats.posts_seen, 24);
        assert_eq!(reported, 2, "reported at 5 and 10 accounts");
        assert_eq!(closed, 1);
        let store = Store::open(&db).unwrap();
        let rows = store.recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].snapshot.signals.accounts, 12);
        assert!(rows[0].snapshot.closed);
        std::fs::remove_dir_all(&dir).ok();
    }
}
