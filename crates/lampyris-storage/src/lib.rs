//! # lampyris-storage
//!
//! Keeps detected clusters on the local machine, in a single SQLite file.
//!
//! Only clusters and *references* to their posts are stored — never the full
//! firehose — which keeps the database small and limits how much personal data
//! a local install holds. Old rows are pruned with [`Store::prune_before`].

use std::path::Path;

use anyhow::{Context, Result};
use lampyris_core::{ClusterSnapshot, PostRef, Score, Signals};
use rusqlite::{params, Connection, OptionalExtension};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS clusters (
    run_id         TEXT    NOT NULL,
    cluster_id     INTEGER NOT NULL,
    first_time_us  INTEGER NOT NULL,
    last_time_us   INTEGER NOT NULL,
    accounts       INTEGER NOT NULL,
    posts          INTEGER NOT NULL,
    score          REAL    NOT NULL,
    model          TEXT    NOT NULL,
    closed         INTEGER NOT NULL,
    sample_text    TEXT    NOT NULL,
    signals_json   TEXT    NOT NULL,
    score_json     TEXT    NOT NULL,
    PRIMARY KEY (run_id, cluster_id)
);
CREATE INDEX IF NOT EXISTS clusters_last_time ON clusters(last_time_us);

CREATE TABLE IF NOT EXISTS cluster_posts (
    run_id      TEXT    NOT NULL,
    cluster_id  INTEGER NOT NULL,
    author      TEXT    NOT NULL,
    post_id     TEXT    NOT NULL,
    time_us     INTEGER NOT NULL,
    PRIMARY KEY (run_id, cluster_id, author, post_id),
    FOREIGN KEY (run_id, cluster_id) REFERENCES clusters(run_id, cluster_id) ON DELETE CASCADE
);
"#;

/// A cluster as stored, with the run that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCluster {
    pub run_id: String,
    pub snapshot: ClusterSnapshot,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Insert or update a cluster snapshot and its post references.
    pub fn upsert(&mut self, run_id: &str, s: &ClusterSnapshot) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"INSERT INTO clusters (run_id, cluster_id, first_time_us, last_time_us, accounts,
                   posts, score, model, closed, sample_text, signals_json, score_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
               ON CONFLICT(run_id, cluster_id) DO UPDATE SET
                   first_time_us = excluded.first_time_us,
                   last_time_us  = excluded.last_time_us,
                   accounts      = excluded.accounts,
                   posts         = excluded.posts,
                   score         = excluded.score,
                   model         = excluded.model,
                   closed        = excluded.closed,
                   signals_json  = excluded.signals_json,
                   score_json    = excluded.score_json"#,
            params![
                run_id,
                s.id as i64,
                s.first_time_us,
                s.last_time_us,
                s.signals.accounts as i64,
                s.signals.posts as i64,
                s.score.value,
                s.score.model,
                s.closed,
                s.sample_text,
                serde_json::to_string(&s.signals)?,
                serde_json::to_string(&s.score)?,
            ],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO cluster_posts (run_id, cluster_id, author, post_id, time_us)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for p in &s.samples {
                stmt.execute(params![run_id, s.id as i64, p.author, p.id, p.time_us])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Most recently active clusters first.
    pub fn recent(&self, limit: usize) -> Result<Vec<StoredCluster>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, cluster_id, first_time_us, last_time_us, closed, sample_text,
                    signals_json, score_json
             FROM clusters ORDER BY last_time_us DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (run_id, id, first, last, closed, text, signals_json, score_json) in rows {
            let signals: Signals = serde_json::from_str(&signals_json)?;
            let score: Score = serde_json::from_str(&score_json)?;
            let samples = self.samples(&run_id, id)?;
            out.push(StoredCluster {
                run_id,
                snapshot: ClusterSnapshot {
                    id: id as u64,
                    first_time_us: first,
                    last_time_us: last,
                    sample_text: text,
                    samples,
                    signals,
                    score,
                    closed,
                },
            });
        }
        Ok(out)
    }

    fn samples(&self, run_id: &str, cluster_id: i64) -> Result<Vec<PostRef>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT author, post_id, time_us FROM cluster_posts
             WHERE run_id = ?1 AND cluster_id = ?2 ORDER BY time_us",
        )?;
        let refs = stmt
            .query_map(params![run_id, cluster_id], |r| {
                Ok(PostRef {
                    author: r.get(0)?,
                    id: r.get(1)?,
                    time_us: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(refs)
    }

    /// Delete clusters whose last activity is older than `time_us`.
    /// Returns how many clusters were removed.
    pub fn prune_before(&mut self, time_us: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM clusters WHERE last_time_us < ?1",
            params![time_us],
        )?)
    }

    pub fn count(&self) -> Result<usize> {
        let n: Option<i64> = self
            .conn
            .query_row("SELECT COUNT(*) FROM clusters", [], |r| r.get(0))
            .optional()?;
        Ok(n.unwrap_or(0) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampyris_core::{Detector, DetectorConfig, DetectorEvent, Post};

    fn some_cluster() -> ClusterSnapshot {
        let mut d = Detector::new(DetectorConfig::default());
        let text = "share this everywhere before they delete it, the truth must come out today";
        let mut last = None;
        for i in 0..6 {
            for ev in d.push(&Post::new(
                format!("did:plc:{i}"),
                format!("r{i}"),
                i * 1_000_000,
                text,
            )) {
                if let DetectorEvent::ClusterReported(s) = ev {
                    last = Some(s);
                }
            }
        }
        last.expect("cluster reported")
    }

    #[test]
    fn roundtrip_and_update() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = some_cluster();
        store.upsert("run1", &s).unwrap();
        s.closed = true;
        store.upsert("run1", &s).unwrap();

        let got = store.recent(10).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].run_id, "run1");
        assert_eq!(got[0].snapshot, s);
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn prune_removes_old_clusters_and_their_posts() {
        let mut store = Store::open_in_memory().unwrap();
        let s = some_cluster();
        store.upsert("run1", &s).unwrap();
        assert_eq!(store.prune_before(s.last_time_us + 1).unwrap(), 1);
        assert_eq!(store.count().unwrap(), 0);
        let orphans: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM cluster_posts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0);
    }
}
