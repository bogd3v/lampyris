use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use lampyris::{default_db_path, format, Pipeline, PipelineConfig, PipelineMsg, Source};
use lampyris_core::DetectorConfig;
use lampyris_ingest::JetstreamConfig;
use lampyris_storage::Store;
use tracing_subscriber::EnvFilter;

/// Lampyris — local-first detector of coordinated bot campaigns.
///
/// Many small, independent lights that together make the dark visible.
#[derive(Parser)]
#[command(name = "lampyris", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Watch the live Bluesky firehose for coordinated posting.
    Watch {
        #[command(flatten)]
        detect: DetectArgs,
        #[command(flatten)]
        out: OutputArgs,
        /// Jetstream endpoint(s). Defaults to Bluesky's public instances.
        #[arg(long = "endpoint")]
        endpoints: Vec<String>,
        /// Also save every raw message to this JSONL file, for replay.
        #[arg(long)]
        record: Option<PathBuf>,
        /// Start from this event time (µs since epoch) instead of live.
        #[arg(long)]
        cursor: Option<i64>,
    },
    /// Run the detector over a recording made with `watch --record`.
    Replay {
        file: PathBuf,
        #[command(flatten)]
        detect: DetectArgs,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// List clusters stored in the local database.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        db: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
struct DetectArgs {
    /// Seconds of history each new post is compared against.
    #[arg(long, default_value_t = 600)]
    window: u64,
    /// Distinct accounts needed to report a cluster.
    #[arg(long, default_value_t = 5)]
    min_accounts: usize,
    /// Max differing SimHash bits to count as the same text (higher = looser).
    #[arg(long, default_value_t = 3)]
    max_hamming: u32,
    /// Ignore posts shorter than this many words.
    #[arg(long, default_value_t = 5)]
    min_tokens: usize,
    /// Only analyse these languages (repeatable, e.g. --lang es --lang pt).
    #[arg(long = "lang")]
    langs: Vec<String>,
}

#[derive(Args)]
struct OutputArgs {
    /// SQLite database for clusters. Defaults to the user data dir.
    #[arg(long)]
    db: Option<PathBuf>,
    /// Do not store anything on disk.
    #[arg(long, conflicts_with = "db")]
    no_db: bool,
    /// Days to keep clusters in the database.
    #[arg(long, default_value_t = 30)]
    retention_days: u32,
    /// Print events as JSON lines instead of text.
    #[arg(long)]
    json: bool,
    /// Post links to print per reported cluster.
    #[arg(long, default_value_t = 3)]
    links: usize,
    /// Print throughput statistics every few seconds.
    #[arg(long)]
    stats: bool,
}

impl DetectArgs {
    fn config(&self) -> DetectorConfig {
        DetectorConfig {
            window_secs: self.window,
            min_accounts: self.min_accounts.max(2),
            max_hamming: self.max_hamming.min(16),
            min_tokens: self.min_tokens.max(1),
            langs: self.langs.clone(),
            ..Default::default()
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Command::Watch {
            detect,
            out,
            endpoints,
            record,
            cursor,
        } => {
            let mut js = JetstreamConfig {
                cursor,
                record_to: record,
                ..Default::default()
            };
            if !endpoints.is_empty() {
                js.endpoints = endpoints;
            }
            run(Source::Jetstream(js), &detect, &out)
        }
        Command::Replay { file, detect, out } => run(Source::Replay(file), &detect, &out),
        Command::List { limit, db, json } => list(db.unwrap_or_else(default_db_path), limit, json),
    }
}

fn run(source: Source, detect: &DetectArgs, out: &OutputArgs) -> Result<()> {
    let db_path = if out.no_db {
        None
    } else {
        Some(out.db.clone().unwrap_or_else(default_db_path))
    };
    if let Some(p) = &db_path {
        tracing::info!(db = %p.display(), "storing clusters");
    }
    let pipeline = Pipeline::start(PipelineConfig {
        source,
        detector: detect.config(),
        db_path,
        retention_days: out.retention_days,
    })?;

    // Ctrl-C asks the pipeline to stop; it then flushes and reports `Finished`.
    let stopper = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stopper = stopper.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::spawn(move || {
            rt.block_on(async {
                let _ = tokio::signal::ctrl_c().await;
            });
            stopper.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    let mut stop_sent = false;
    loop {
        if !stop_sent && stopper.load(std::sync::atomic::Ordering::SeqCst) {
            eprintln!("stopping…");
            pipeline.stop();
            stop_sent = true;
        }
        let Some(msg) = pipeline.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        match msg {
            PipelineMsg::Event(ev) => {
                if out.json {
                    println!("{}", serde_json::to_string(&ev)?);
                } else {
                    println!("{}\n", format::event(&ev, out.links));
                }
            }
            PipelineMsg::Stats(s) => {
                if out.stats {
                    eprintln!("{}", format::stats(&s));
                }
            }
            PipelineMsg::Error(e) => eprintln!("error: {e}"),
            PipelineMsg::Finished(s) => {
                eprintln!("done · {}", format::stats(&s));
                break;
            }
        }
    }
    pipeline.join();
    Ok(())
}

fn list(db: PathBuf, limit: usize, json: bool) -> Result<()> {
    if !db.exists() {
        eprintln!(
            "no database at {} yet — run `lampyris watch` first",
            db.display()
        );
        return Ok(());
    }
    let store = Store::open(&db)?;
    for c in store.recent(limit)? {
        if json {
            println!(
                "{}",
                serde_json::json!({ "run_id": c.run_id, "cluster": c.snapshot })
            );
        } else {
            println!("[run {}] {}\n", c.run_id, format::snapshot(&c.snapshot, 3));
        }
    }
    Ok(())
}
