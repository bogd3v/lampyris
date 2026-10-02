//! The bridge between the detection pipeline and QML.
//!
//! `LampyrisController` is a QObject exposed to QML. Starting it launches the
//! pipeline on background threads; a relay thread collects events and pushes
//! throttled UI updates back onto the Qt event loop with `qt_thread.queue`, so
//! the interface stays fluid no matter how busy the firehose is.

use core::pin::Pin;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use lampyris::{default_db_path, Pipeline, PipelineConfig, PipelineMsg, Source};
use lampyris_core::{ClusterSnapshot, DetectorConfig, DetectorEvent};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, running)]
        #[qproperty(QString, status)]
        #[qproperty(i32, posts_seen)]
        #[qproperty(i32, clusters_reported)]
        #[qproperty(QString, clusters_json)]
        type LampyrisController = super::LampyrisControllerRust;

        /// Start watching the live Bluesky firehose. `langs` is a comma or space
        /// separated list of language codes; empty means every language.
        #[qinvokable]
        fn start(self: Pin<&mut LampyrisController>, langs: &QString, min_accounts: i32);

        #[qinvokable]
        fn stop(self: Pin<&mut LampyrisController>);
    }

    impl cxx_qt::Threading for LampyrisController {}
}

pub struct LampyrisControllerRust {
    running: bool,
    status: QString,
    posts_seen: i32,
    clusters_reported: i32,
    clusters_json: QString,
    stop_flag: Option<Arc<AtomicBool>>,
}

impl Default for LampyrisControllerRust {
    fn default() -> Self {
        Self {
            running: false,
            status: QString::from("Ready"),
            posts_seen: 0,
            clusters_reported: 0,
            clusters_json: QString::from("[]"),
            stop_flag: None,
        }
    }
}

/// Clusters shown in the list at most, highest score first.
const MAX_SHOWN: usize = 200;
const UI_UPDATE_EVERY: Duration = Duration::from_millis(500);

impl qobject::LampyrisController {
    pub fn start(mut self: Pin<&mut Self>, langs: &QString, min_accounts: i32) {
        if *self.running() {
            return;
        }
        let langs: Vec<String> = langs
            .to_string()
            .split([',', ' '])
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect();

        let config = PipelineConfig {
            source: Source::Jetstream(Default::default()),
            detector: DetectorConfig {
                min_accounts: min_accounts.max(2) as usize,
                langs,
                ..Default::default()
            },
            db_path: Some(default_db_path()),
            retention_days: 30,
        };

        let pipeline = match Pipeline::start(config) {
            Ok(p) => p,
            Err(e) => {
                self.as_mut()
                    .set_status(QString::from(&format!("Could not start: {e:#}")));
                return;
            }
        };

        let stop = Arc::new(AtomicBool::new(false));
        self.as_mut().rust_mut().stop_flag = Some(stop.clone());
        self.as_mut().set_running(true);
        self.as_mut().set_posts_seen(0);
        self.as_mut().set_clusters_reported(0);
        self.as_mut().set_clusters_json(QString::from("[]"));
        self.as_mut()
            .set_status(QString::from("Connecting to Bluesky…"));

        let qt_thread = self.qt_thread();
        std::thread::Builder::new()
            .name("lampyris-ui-relay".into())
            .spawn(move || relay(pipeline, stop, qt_thread))
            .expect("spawn relay thread");
    }

    pub fn stop(mut self: Pin<&mut Self>) {
        if let Some(flag) = self.as_mut().rust_mut().stop_flag.take() {
            flag.store(true, Ordering::SeqCst);
            self.as_mut().set_status(QString::from("Stopping…"));
        }
    }
}

fn relay(
    pipeline: Pipeline,
    stop: Arc<AtomicBool>,
    qt_thread: cxx_qt::CxxQtThread<qobject::LampyrisController>,
) {
    let mut clusters: BTreeMap<u64, ClusterSnapshot> = BTreeMap::new();
    let mut posts_seen = 0u64;
    let mut reported = 0u64;
    // Errors and reconnects are logged by the ingest stage; here we only switch
    // from "connecting" to "watching" once posts actually arrive.
    let mut status = String::from("Connecting to Bluesky…");
    let mut dirty = true;
    let mut last_push = Instant::now() - UI_UPDATE_EVERY;
    let mut stop_sent = false;

    loop {
        if !stop_sent && stop.load(Ordering::SeqCst) {
            pipeline.stop();
            stop_sent = true;
        }
        let mut finished = false;
        if let Some(msg) = pipeline.recv_timeout(Duration::from_millis(100)) {
            match msg {
                PipelineMsg::Event(ev) => {
                    let snap = match ev {
                        DetectorEvent::ClusterReported(s) | DetectorEvent::ClusterClosed(s) => s,
                    };
                    clusters.insert(snap.id, snap);
                    dirty = true;
                }
                PipelineMsg::Stats(s) => {
                    if posts_seen == 0 && s.posts_seen > 0 {
                        status = String::from("Watching the Bluesky firehose");
                    }
                    posts_seen = s.posts_seen;
                    reported = s.clusters_reported;
                    dirty = true;
                }
                PipelineMsg::Error(e) => {
                    status = e;
                    dirty = true;
                }
                PipelineMsg::Finished(s) => {
                    posts_seen = s.posts_seen;
                    reported = s.clusters_reported;
                    finished = true;
                }
            }
        }

        if finished || (dirty && last_push.elapsed() >= UI_UPDATE_EVERY) {
            dirty = false;
            last_push = Instant::now();
            let json = clusters_to_json(&clusters);
            let status_now = if finished {
                "Stopped".to_string()
            } else {
                status.clone()
            };
            let posts = posts_seen.min(i32::MAX as u64) as i32;
            let rep = reported.min(i32::MAX as u64) as i32;
            let _ = qt_thread.queue(move |mut q| {
                q.as_mut().set_clusters_json(QString::from(&json));
                q.as_mut().set_posts_seen(posts);
                q.as_mut().set_clusters_reported(rep);
                q.as_mut().set_status(QString::from(&status_now));
                if finished {
                    q.as_mut().set_running(false);
                }
            });
        }
        if finished {
            break;
        }
    }
    pipeline.join();
}

fn clusters_to_json(clusters: &BTreeMap<u64, ClusterSnapshot>) -> String {
    let mut list: Vec<&ClusterSnapshot> = clusters.values().collect();
    list.sort_by(|a, b| {
        b.score
            .value
            .total_cmp(&a.score.value)
            .then(b.signals.accounts.cmp(&a.signals.accounts))
    });
    let items: Vec<serde_json::Value> = list
        .into_iter()
        .take(MAX_SHOWN)
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "text": c.sample_text,
                "accounts": c.signals.accounts,
                "posts": c.signals.posts,
                "minutes": c.signals.span_secs / 60.0,
                "rate": c.signals.accounts_per_minute,
                "score": c.score.value,
                "model": c.score.model,
                "link_ratio": c.signals.external_link_ratio,
                "closed": c.closed,
                "first": fmt_time(c.first_time_us),
                "last": fmt_time(c.last_time_us),
                "links": c.samples.iter().take(5).map(|p| p.bsky_url()).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::Value::Array(items).to_string()
}

fn fmt_time(t: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_micros(t)
        .map(|d| d.format("%H:%M:%S UTC").to_string())
        .unwrap_or_default()
}
