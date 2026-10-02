//! Human-readable rendering of detector output for the terminal.

use chrono::{DateTime, Utc};
use lampyris_core::{ClusterSnapshot, DetectorEvent, DetectorStats};

pub fn time_us(t: i64) -> String {
    DateTime::<Utc>::from_timestamp_micros(t)
        .map(|d| d.format("%Y-%m-%d %H:%M:%SZ").to_string())
        .unwrap_or_else(|| t.to_string())
}

fn one_line(text: &str, max_chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        flat
    } else {
        let mut s: String = flat.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

pub fn snapshot(s: &ClusterSnapshot, links: usize) -> String {
    let sig = &s.signals;
    let mut out = format!(
        "#{id}  {acc} accounts · {posts} posts · {span:.1} min · {rate:.1} acc/min · score {score:.2} ({model})\n    first {first}  last {last}\n    “{text}”",
        id = s.id,
        acc = sig.accounts,
        posts = sig.posts,
        span = sig.span_secs / 60.0,
        rate = sig.accounts_per_minute,
        score = s.score.value,
        model = s.score.model,
        first = time_us(s.first_time_us),
        last = time_us(s.last_time_us),
        text = one_line(&s.sample_text, 140),
    );
    if sig.external_link_ratio > 0.5 {
        out.push_str(
            "\n    note: most posts share a link card — could be people sharing the same headline",
        );
    }
    for p in s.samples.iter().take(links) {
        out.push_str("\n    ");
        out.push_str(&p.bsky_url());
    }
    out
}

pub fn event(ev: &DetectorEvent, links: usize) -> String {
    match ev {
        DetectorEvent::ClusterReported(s) => format!("▲ {}", snapshot(s, links)),
        DetectorEvent::ClusterClosed(s) => format!("■ closed {}", snapshot(s, 0)),
    }
}

pub fn stats(s: &DetectorStats) -> String {
    format!(
        "posts {} · indexed {} · short {} · other-lang {} · window {} · clusters {} (reported {})",
        s.posts_seen,
        s.posts_indexed,
        s.skipped_short,
        s.skipped_lang,
        s.window_posts,
        s.active_clusters,
        s.clusters_reported
    )
}
