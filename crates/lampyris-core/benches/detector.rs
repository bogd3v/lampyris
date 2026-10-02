//! Throughput benchmark on a synthetic firehose.
//!
//! Run with `cargo bench -p lampyris-core`.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use lampyris_core::{Detector, DetectorConfig, Post};

const WORDS: &[&str] = &[
    "the",
    "people",
    "city",
    "today",
    "vote",
    "new",
    "music",
    "coffee",
    "rain",
    "game",
    "team",
    "love",
    "work",
    "train",
    "late",
    "again",
    "book",
    "reading",
    "friends",
    "weekend",
    "photo",
    "sunset",
    "election",
    "media",
    "truth",
    "government",
    "cat",
    "dog",
    "garden",
    "bread",
    "football",
    "match",
    "season",
    "song",
    "album",
    "movie",
    "night",
    "morning",
    "school",
    "class",
    "hoy",
    "gente",
    "ciudad",
    "lluvia",
    "trabajo",
    "amigos",
    "fin",
    "semana",
    "noche",
    "partido",
];

const CAMPAIGN: &str =
    "They do not want you to know what really happened, share this before it gets deleted";

/// Tiny deterministic PRNG so the benchmark needs no extra dependencies.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn synthetic_stream(n: usize) -> Vec<Post> {
    let mut rng = Lcg(42);
    (0..n)
        .map(|i| {
            // ~50 posts per second, like a busy firehose slice.
            let t = (i as i64) * 20_000;
            let author = format!("did:plc:{:x}", rng.next() % 2_000_000);
            let text = if i % 200 == 0 {
                CAMPAIGN.to_string()
            } else {
                let len = 6 + (rng.next() % 20) as usize;
                (0..len)
                    .map(|_| WORDS[(rng.next() as usize) % WORDS.len()])
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            Post::new(author, format!("{i}"), t, text)
        })
        .collect()
}

fn bench_detector(c: &mut Criterion) {
    let posts = synthetic_stream(100_000);
    let mut group = c.benchmark_group("detector");
    group.throughput(Throughput::Elements(posts.len() as u64));
    group.sample_size(10);
    group.bench_function("push_100k_posts", |b| {
        b.iter(|| {
            let mut d = Detector::new(DetectorConfig::default());
            let mut events = 0usize;
            for p in &posts {
                events += d.push(p).len();
            }
            events
        })
    });
    group.finish();
}

criterion_group!(benches, bench_detector);
criterion_main!(benches);
