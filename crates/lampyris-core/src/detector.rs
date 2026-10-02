//! Sliding-window detector of coordinated near-duplicate posting.
//!
//! Every post gets a SimHash fingerprint. Fingerprints of the last
//! `window_secs` seconds live in an LSH index split into `max_hamming + 1` bands:
//! by the pigeonhole principle, two fingerprints within `max_hamming` bits of each
//! other share at least one band exactly, so a handful of hash lookups finds every
//! near-duplicate without comparing against the whole window.
//!
//! Matching posts are grouped into clusters. A cluster is *reported* once it
//! reaches `min_accounts` distinct accounts, and again each time that number
//! doubles, so a growing campaign shows up early and stays updated without
//! flooding the output.

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::post::{Post, PostRef};
use crate::score::{Score, Signals};
use crate::simhash::{fingerprint, hamming};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectorConfig {
    /// How far back (in event time) a new post is compared against.
    pub window_secs: u64,
    /// Maximum Hamming distance between fingerprints to count as "the same text".
    /// Higher values catch looser paraphrases at the cost of more comparisons and
    /// more false positives. 3 is a conservative default.
    pub max_hamming: u32,
    /// Minimum words (or ~2 characters each in unspaced scripts) for a post to be
    /// considered at all.
    pub min_tokens: usize,
    /// Distinct accounts needed before a cluster is reported.
    pub min_accounts: usize,
    /// Hard cap on posts kept in the window, to bound memory on traffic spikes.
    pub max_window_posts: usize,
    /// Post references kept per cluster as evidence.
    pub max_samples: usize,
    /// If non-empty, posts that declare languages and none of them is in this
    /// list are skipped. Posts without declared languages are always kept.
    pub langs: Vec<String>,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            window_secs: 600,
            max_hamming: 3,
            min_tokens: 5,
            min_accounts: 5,
            max_window_posts: 1_000_000,
            max_samples: 50,
            langs: Vec::new(),
        }
    }
}

/// A point-in-time view of a cluster, suitable for display, storage and export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterSnapshot {
    pub id: u64,
    pub first_time_us: i64,
    pub last_time_us: i64,
    /// Raw text of the first post in the cluster (truncated), for humans.
    pub sample_text: String,
    /// References to member posts (up to `max_samples`), for re-verification.
    pub samples: Vec<PostRef>,
    pub signals: Signals,
    pub score: Score,
    /// `true` once the cluster went quiet for a whole window and was closed.
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DetectorEvent {
    /// A cluster crossed `min_accounts`, or doubled in size since last reported.
    ClusterReported(ClusterSnapshot),
    /// A previously reported cluster had no new posts for a whole window.
    ClusterClosed(ClusterSnapshot),
}

impl DetectorEvent {
    pub fn snapshot(&self) -> &ClusterSnapshot {
        match self {
            DetectorEvent::ClusterReported(s) | DetectorEvent::ClusterClosed(s) => s,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorStats {
    pub posts_seen: u64,
    pub posts_indexed: u64,
    pub skipped_short: u64,
    pub skipped_lang: u64,
    pub window_posts: usize,
    pub active_clusters: usize,
    pub clusters_reported: u64,
}

struct Entry {
    seq: u64,
    hash: u64,
    time_us: i64,
    cluster: Option<u64>,
    author_key: u64,
    has_link: bool,
    post_ref: PostRef,
}

struct Cluster {
    id: u64,
    anchor_hash: u64,
    first_time_us: i64,
    last_time_us: i64,
    authors: FxHashSet<u64>,
    posts: usize,
    link_posts: usize,
    distance_sum: u64,
    samples: Vec<PostRef>,
    sample_text: String,
    next_report: usize,
    reported: bool,
}

const SAMPLE_TEXT_CHARS: usize = 500;
const SWEEP_EVERY_US: i64 = 5_000_000;

pub struct Detector {
    config: DetectorConfig,
    bands: Vec<(u32, u32)>, // (bit offset, width)
    window: VecDeque<Entry>,
    next_seq: u64,
    index: FxHashMap<(u8, u64), VecDeque<u64>>,
    clusters: FxHashMap<u64, Cluster>,
    redirects: FxHashMap<u64, u64>,
    next_cluster_id: u64,
    now_us: i64,
    last_sweep_us: i64,
    stats: DetectorStats,
}

impl Detector {
    pub fn new(config: DetectorConfig) -> Self {
        let band_count = (config.max_hamming + 1).clamp(1, 64);
        let width = 64 / band_count;
        let bands = (0..band_count)
            .map(|i| {
                let offset = i * width;
                let w = if i == band_count - 1 {
                    64 - offset
                } else {
                    width
                };
                (offset, w)
            })
            .collect();
        Self {
            config,
            bands,
            window: VecDeque::new(),
            next_seq: 0,
            index: FxHashMap::default(),
            clusters: FxHashMap::default(),
            redirects: FxHashMap::default(),
            next_cluster_id: 1,
            now_us: i64::MIN,
            last_sweep_us: i64::MIN,
            stats: DetectorStats::default(),
        }
    }

    pub fn config(&self) -> &DetectorConfig {
        &self.config
    }

    pub fn stats(&self) -> DetectorStats {
        DetectorStats {
            window_posts: self.window.len(),
            active_clusters: self.clusters.len(),
            ..self.stats.clone()
        }
    }

    /// Feed one post. Returns the events it triggered (usually none).
    pub fn push(&mut self, post: &Post) -> Vec<DetectorEvent> {
        let mut events = Vec::new();
        self.stats.posts_seen += 1;

        if !self.lang_allowed(post) {
            self.stats.skipped_lang += 1;
            return events;
        }

        self.advance_clock(post.time_us, &mut events);

        let Some(hash) = fingerprint(&post.text, self.config.min_tokens) else {
            self.stats.skipped_short += 1;
            return events;
        };
        self.stats.posts_indexed += 1;

        // 1. Find near-duplicates in the window through the LSH bands.
        let mut checked: FxHashSet<u64> = FxHashSet::default();
        let mut matched_seqs: Vec<(u64, u32)> = Vec::new();
        for (band, key) in band_keys(&self.bands, hash) {
            if let Some(bucket) = self.index.get(&(band, key)) {
                for &seq in bucket {
                    if !checked.insert(seq) {
                        continue;
                    }
                    let entry = &self.window[self.slot(seq)];
                    let d = hamming(hash, entry.hash);
                    if d <= self.config.max_hamming {
                        matched_seqs.push((seq, d));
                    }
                }
            }
        }

        // 2. Decide which cluster the post joins, merging clusters it bridges.
        let mut cluster_id = None;
        if !matched_seqs.is_empty() {
            let mut matched_clusters: Vec<u64> = matched_seqs
                .iter()
                .filter_map(|(seq, _)| self.window[self.slot(*seq)].cluster)
                .filter_map(|c| self.resolve(c))
                .collect();
            matched_clusters.sort_unstable();
            matched_clusters.dedup();

            let target = match matched_clusters
                .iter()
                .copied()
                .max_by_key(|id| self.clusters[id].authors.len())
            {
                Some(id) => id,
                None => {
                    // Only unclustered posts matched: start a cluster anchored on the
                    // closest of them.
                    let (anchor_seq, _) = *matched_seqs.iter().min_by_key(|(_, d)| *d).unwrap();
                    let anchor = &self.window[self.slot(anchor_seq)];
                    self.new_cluster(anchor.hash, anchor.time_us, &post.text)
                }
            };

            for other in matched_clusters.into_iter().filter(|&c| c != target) {
                self.merge_into(target, other);
            }
            for (seq, _) in &matched_seqs {
                let slot = self.slot(*seq);
                if self.window[slot]
                    .cluster
                    .and_then(|c| self.resolve(c))
                    .is_none()
                {
                    self.window[slot].cluster = Some(target);
                    let (h, t, a, l, r) = {
                        let e = &self.window[slot];
                        (
                            e.hash,
                            e.time_us,
                            e.author_key,
                            e.has_link,
                            e.post_ref.clone(),
                        )
                    };
                    self.add_member(target, h, t, a, l, r);
                }
            }
            self.add_member(
                target,
                hash,
                post.time_us,
                author_key(&post.author),
                post.has_external_link,
                post.to_ref(),
            );
            cluster_id = Some(target);
        }

        // 3. Index the new post.
        let seq = self.next_seq;
        self.next_seq += 1;
        for (band, key) in band_keys(&self.bands, hash) {
            self.index.entry((band, key)).or_default().push_back(seq);
        }
        self.window.push_back(Entry {
            seq,
            hash,
            time_us: post.time_us,
            cluster: cluster_id,
            author_key: author_key(&post.author),
            has_link: post.has_external_link,
            post_ref: post.to_ref(),
        });
        if self.window.len() > self.config.max_window_posts {
            self.evict_front();
        }

        // 4. Report if the cluster crossed a reporting threshold.
        if let Some(id) = cluster_id {
            let min_accounts = self.config.min_accounts;
            let c = self.clusters.get_mut(&id).expect("cluster exists");
            let accounts = c.authors.len();
            if accounts >= min_accounts.max(c.next_report) {
                c.next_report = accounts.saturating_mul(2);
                if !c.reported {
                    c.reported = true;
                    self.stats.clusters_reported += 1;
                }
                events.push(DetectorEvent::ClusterReported(self.snapshot(id, false)));
            }
        }

        events
    }

    /// Close every reported cluster (e.g. at the end of a replay).
    pub fn flush(&mut self) -> Vec<DetectorEvent> {
        let mut ids: Vec<u64> = self.clusters.keys().copied().collect();
        ids.sort_unstable();
        let mut events = Vec::new();
        for id in ids {
            if self.clusters[&id].reported {
                events.push(DetectorEvent::ClusterClosed(self.snapshot(id, true)));
            }
        }
        self.clusters.clear();
        self.redirects.clear();
        for e in self.window.iter_mut() {
            e.cluster = None;
        }
        events
    }

    /// Current snapshots of every reported, still active cluster.
    pub fn active_reported(&self) -> Vec<ClusterSnapshot> {
        let mut v: Vec<ClusterSnapshot> = self
            .clusters
            .values()
            .filter(|c| c.reported)
            .map(|c| self.snapshot(c.id, false))
            .collect();
        v.sort_by_key(|s| s.id);
        v
    }

    // --- internals -------------------------------------------------------

    fn lang_allowed(&self, post: &Post) -> bool {
        if self.config.langs.is_empty() || post.langs.is_empty() {
            return true;
        }
        post.langs.iter().any(|l| {
            let primary = l.split(['-', '_']).next().unwrap_or(l);
            self.config
                .langs
                .iter()
                .any(|want| want.eq_ignore_ascii_case(l) || want.eq_ignore_ascii_case(primary))
        })
    }

    #[inline]
    fn slot(&self, seq: u64) -> usize {
        (seq - self.window.front().expect("window not empty").seq) as usize
    }

    fn advance_clock(&mut self, time_us: i64, events: &mut Vec<DetectorEvent>) {
        if time_us > self.now_us {
            self.now_us = time_us;
        }
        let cutoff = self
            .now_us
            .saturating_sub((self.config.window_secs as i64).saturating_mul(1_000_000));
        while self.window.front().is_some_and(|e| e.time_us < cutoff) {
            self.evict_front();
        }
        if self.last_sweep_us == i64::MIN || self.now_us - self.last_sweep_us >= SWEEP_EVERY_US {
            self.last_sweep_us = self.now_us;
            self.sweep_clusters(cutoff, events);
        }
    }

    fn evict_front(&mut self) {
        let Some(entry) = self.window.pop_front() else {
            return;
        };
        for (band, key) in band_keys(&self.bands, entry.hash) {
            if let Some(bucket) = self.index.get_mut(&(band, key)) {
                // Buckets are filled in insertion order and the window is FIFO,
                // so the evicted entry is always at the front of its buckets.
                debug_assert_eq!(bucket.front(), Some(&entry.seq));
                bucket.pop_front();
                if bucket.is_empty() {
                    self.index.remove(&(band, key));
                }
            }
        }
    }

    fn sweep_clusters(&mut self, cutoff: i64, events: &mut Vec<DetectorEvent>) {
        let mut expired: Vec<u64> = self
            .clusters
            .values()
            .filter(|c| c.last_time_us < cutoff)
            .map(|c| c.id)
            .collect();
        expired.sort_unstable();
        for id in expired {
            if self.clusters[&id].reported {
                events.push(DetectorEvent::ClusterClosed(self.snapshot(id, true)));
            }
            self.clusters.remove(&id);
        }
        let clusters = &self.clusters;
        self.redirects
            .retain(|_, target| clusters.contains_key(target));
    }

    fn resolve(&self, mut id: u64) -> Option<u64> {
        while let Some(&next) = self.redirects.get(&id) {
            id = next;
        }
        self.clusters.contains_key(&id).then_some(id)
    }

    fn new_cluster(&mut self, anchor_hash: u64, time_us: i64, text: &str) -> u64 {
        let id = self.next_cluster_id;
        self.next_cluster_id += 1;
        self.clusters.insert(
            id,
            Cluster {
                id,
                anchor_hash,
                first_time_us: time_us,
                last_time_us: time_us,
                authors: FxHashSet::default(),
                posts: 0,
                link_posts: 0,
                distance_sum: 0,
                samples: Vec::new(),
                sample_text: text.chars().take(SAMPLE_TEXT_CHARS).collect(),
                next_report: 0,
                reported: false,
            },
        );
        id
    }

    fn add_member(
        &mut self,
        id: u64,
        hash: u64,
        time_us: i64,
        author: u64,
        has_link: bool,
        post_ref: PostRef,
    ) {
        let max_samples = self.config.max_samples;
        let c = self.clusters.get_mut(&id).expect("cluster exists");
        c.first_time_us = c.first_time_us.min(time_us);
        c.last_time_us = c.last_time_us.max(time_us);
        c.posts += 1;
        c.link_posts += has_link as usize;
        c.distance_sum += hamming(c.anchor_hash, hash) as u64;
        let new_author = c.authors.insert(author);
        // Prefer one sample per account: more accounts is better evidence.
        if new_author && c.samples.len() < max_samples {
            c.samples.push(post_ref);
        }
    }

    fn merge_into(&mut self, target: u64, other: u64) {
        let Some(src) = self.clusters.remove(&other) else {
            return;
        };
        let max_samples = self.config.max_samples;
        let dst = self.clusters.get_mut(&target).expect("target exists");
        dst.first_time_us = dst.first_time_us.min(src.first_time_us);
        dst.last_time_us = dst.last_time_us.max(src.last_time_us);
        dst.posts += src.posts;
        dst.link_posts += src.link_posts;
        // Distances were measured against a different anchor; keep them as an
        // approximation rather than recomputing from evicted posts.
        dst.distance_sum += src.distance_sum;
        for a in src.authors {
            dst.authors.insert(a);
        }
        for s in src.samples {
            if dst.samples.len() >= max_samples {
                break;
            }
            if !dst.samples.contains(&s) {
                dst.samples.push(s);
            }
        }
        if src.reported && !dst.reported {
            dst.reported = true;
            dst.next_report = dst.next_report.max(src.next_report);
        }
        self.redirects.insert(other, target);
    }

    fn snapshot(&self, id: u64, closed: bool) -> ClusterSnapshot {
        let c = &self.clusters[&id];
        let span_secs = (c.last_time_us - c.first_time_us).max(0) as f64 / 1e6;
        let accounts = c.authors.len();
        let signals = Signals {
            accounts,
            posts: c.posts,
            span_secs,
            accounts_per_minute: accounts as f64 / (span_secs / 60.0).max(1.0),
            mean_distance: if c.posts == 0 {
                0.0
            } else {
                c.distance_sum as f64 / c.posts as f64
            },
            external_link_ratio: if c.posts == 0 {
                0.0
            } else {
                c.link_posts as f64 / c.posts as f64
            },
        };
        let score = signals.score(self.config.max_hamming);
        ClusterSnapshot {
            id,
            first_time_us: c.first_time_us,
            last_time_us: c.last_time_us,
            sample_text: c.sample_text.clone(),
            samples: c.samples.clone(),
            signals,
            score,
            closed,
        }
    }
}

fn band_keys(bands: &[(u32, u32)], hash: u64) -> impl Iterator<Item = (u8, u64)> + '_ {
    bands.iter().enumerate().map(move |(i, &(offset, width))| {
        let mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        (i as u8, (hash >> offset) & mask)
    })
}

fn author_key(author: &str) -> u64 {
    xxh3_64(author.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = 1_000_000;
    const CAMPAIGN: &str =
        "The election was rigged and the mainstream media will never tell you the truth about it";

    fn post(author: &str, t_secs: i64, text: &str) -> Post {
        Post::new(
            format!("did:plc:{author}"),
            format!("r{author}{t_secs}"),
            t_secs * SEC,
            text,
        )
    }

    fn reported(events: &[DetectorEvent]) -> Vec<&ClusterSnapshot> {
        events
            .iter()
            .filter_map(|e| match e {
                DetectorEvent::ClusterReported(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn reports_when_min_accounts_reached() {
        let mut d = Detector::new(DetectorConfig::default());
        let mut all = Vec::new();
        for i in 0..4 {
            all.extend(d.push(&post(&format!("a{i}"), i, CAMPAIGN)));
        }
        assert!(
            all.is_empty(),
            "4 accounts must not report with min_accounts=5"
        );
        let ev = d.push(&post("a4", 4, CAMPAIGN));
        let r = reported(&ev);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].signals.accounts, 5);
        assert_eq!(r[0].signals.posts, 5);
        assert_eq!(r[0].samples.len(), 5);
    }

    #[test]
    fn same_account_repeating_is_not_coordination() {
        let mut d = Detector::new(DetectorConfig::default());
        let mut all = Vec::new();
        for t in 0..20 {
            all.extend(d.push(&post("spammer", t, CAMPAIGN)));
        }
        assert!(reported(&all).is_empty());
    }

    #[test]
    fn cosmetic_variants_join_the_same_cluster() {
        let mut d = Detector::new(DetectorConfig::default());
        let variants = [
            CAMPAIGN.to_string(),
            CAMPAIGN.to_uppercase(),
            format!("{CAMPAIGN} 🔥🔥"),
            format!("@someone {CAMPAIGN} https://t.co/abc"),
            CAMPAIGN.replace('o', "о"), // Cyrillic о
        ];
        let mut ev = Vec::new();
        for (i, v) in variants.iter().enumerate() {
            ev.extend(d.push(&post(&format!("v{i}"), i as i64, v)));
        }
        let r = reported(&ev);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].signals.accounts, 5);
        assert_eq!(r[0].signals.mean_distance, 0.0);
    }

    #[test]
    fn reports_again_when_cluster_doubles() {
        let mut d = Detector::new(DetectorConfig::default());
        let mut sizes = Vec::new();
        for i in 0..25 {
            for s in reported(&d.push(&post(&format!("a{i}"), i, CAMPAIGN))) {
                sizes.push(s.signals.accounts);
            }
        }
        assert_eq!(sizes, vec![5, 10, 20]);
    }

    #[test]
    fn posts_outside_the_window_do_not_match() {
        let cfg = DetectorConfig {
            window_secs: 60,
            ..Default::default()
        };
        let mut d = Detector::new(cfg);
        let mut ev = Vec::new();
        // One post every two minutes: never two in the same window.
        for i in 0..10 {
            ev.extend(d.push(&post(&format!("a{i}"), i * 120, CAMPAIGN)));
        }
        assert!(reported(&ev).is_empty());
        assert_eq!(d.stats().window_posts, 1);
    }

    #[test]
    fn closes_reported_cluster_after_quiet_window() {
        let cfg = DetectorConfig {
            window_secs: 60,
            ..Default::default()
        };
        let mut d = Detector::new(cfg);
        for i in 0..5 {
            d.push(&post(&format!("a{i}"), i, CAMPAIGN));
        }
        let ev = d.push(&post(
            "other",
            1000,
            "a completely unrelated message about baking bread at home",
        ));
        let closed: Vec<_> = ev
            .iter()
            .filter(|e| matches!(e, DetectorEvent::ClusterClosed(_)))
            .collect();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].snapshot().closed);
        assert_eq!(d.stats().active_clusters, 0);
    }

    #[test]
    fn unrelated_posts_do_not_cluster() {
        let mut d = Detector::new(DetectorConfig::default());
        let texts = [
            "just baked sourdough bread for the first time and it came out great",
            "the train to the city was late again this morning, third time this week",
            "watching the football final tonight with friends, hope we win this time",
            "my cat knocked a glass of water onto my laptop keyboard this morning",
            "learning rust has been fun but the borrow checker humbles me every day",
            "the sunset over the mountains today was absolutely stunning, so many colors",
        ];
        let mut ev = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            ev.extend(d.push(&post(&format!("u{i}"), i as i64, t)));
        }
        assert!(ev.is_empty());
        assert_eq!(d.stats().active_clusters, 0);
    }

    #[test]
    fn language_filter_skips_other_languages() {
        let cfg = DetectorConfig {
            langs: vec!["es".into()],
            ..Default::default()
        };
        let mut d = Detector::new(cfg);
        let mut p = post("a", 0, CAMPAIGN);
        p.langs = vec!["en".into()];
        d.push(&p);
        let mut q = post("b", 1, CAMPAIGN);
        q.langs = vec!["es-CO".into()];
        d.push(&q);
        let s = d.stats();
        assert_eq!(s.skipped_lang, 1);
        assert_eq!(s.posts_indexed, 1);
    }

    #[test]
    fn window_cap_bounds_memory() {
        let cfg = DetectorConfig {
            max_window_posts: 100,
            ..Default::default()
        };
        let mut d = Detector::new(cfg);
        for i in 0..1000 {
            d.push(&post(
                &format!("a{i}"),
                0,
                &format!("unique message number {i} with enough words in it"),
            ));
        }
        assert_eq!(d.stats().window_posts, 100);
    }

    #[test]
    fn flush_closes_reported_clusters() {
        let mut d = Detector::new(DetectorConfig::default());
        for i in 0..6 {
            d.push(&post(&format!("a{i}"), i, CAMPAIGN));
        }
        let ev = d.flush();
        assert_eq!(ev.len(), 1);
        assert!(matches!(ev[0], DetectorEvent::ClusterClosed(_)));
    }
}
