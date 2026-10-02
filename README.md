# Lampyris

*Many small, independent lights that together make the dark visible.*

[Español](README.es.md)

Lampyris is a **local-first, open-source detector of coordinated inauthentic
behavior** on social networks. It watches public posts and surfaces groups of
distinct accounts that publish almost the same text within minutes — the
fingerprint of bot networks, astroturfing and copy-paste campaigns — and shows
the evidence so anyone can check it.

It runs entirely on your own machine. No server, no account, no API key.

> **A cluster is a pattern worth looking into, not proof of bots.** Fans
> quoting a song, people sharing the same headline, or a viral meme can look
> similar. Lampyris shows the signals behind every score so a human can judge.
> See [docs/METHODOLOGY.md](docs/METHODOLOGY.md).

## Status

Early MVP (v0.1). Source: **Bluesky** via its public
[Jetstream](https://github.com/bluesky-social/jetstream) firehose. Platform:
**Linux**. Interface: **command line** and a **Qt / KDE Kirigami** desktop app.

## What it detects today

**Coordinated near-duplicate posting**: N distinct accounts posting
(near-)identical text inside a sliding time window.

- Text is normalized first, so cosmetic variations don't hide a campaign:
  capitalization, emojis, links, @mentions, accents, full-width and
  "math bold" letters, look-alike Cyrillic/Greek characters and invisible
  characters.
- Works in any language. Scripts without spaces (Chinese, Japanese, Thai…)
  use character shingles instead of words.
- Every reported cluster comes with an explainable score and the raw signals:
  accounts, posts, time span, accounts per minute, text distance and share of
  link cards.

Throughput on a laptop-class CPU core: ~100k posts/s for the detector alone
(`cargo bench -p lampyris-core`), ~45k posts/s end to end including JSON
parsing. The whole Bluesky firehose is a small fraction of that.

## Quick start

Requirements: Rust ≥ 1.85.

```sh
cargo build --release

# Watch the live Bluesky firehose (Ctrl-C to stop)
./target/release/lampyris watch

# Only Spanish and Portuguese, report from 10 accounts, keep a recording
./target/release/lampyris watch --lang es --lang pt --min-accounts 10 --record today.jsonl

# Re-run the detector on a recording (deterministic: same input, same clusters)
./target/release/lampyris replay today.jsonl --json > clusters.jsonl

# Clusters stored locally (~/.local/share/lampyris/lampyris.db)
./target/release/lampyris list
```

Run `lampyris <command> --help` for every option (window size, similarity
threshold, minimum words, retention…).

### Desktop app (Qt 6 + KDE Kirigami)

Needs Qt 6 development files and KDE Frameworks 6 Kirigami at runtime
(package names vary between releases; these are the usual ones):

| Distro | Packages |
| --- | --- |
| Fedora | `qt6-qtbase-devel qt6-qtdeclarative-devel kf6-kirigami` |
| Arch | `qt6-base qt6-declarative kirigami` |
| Debian testing / Ubuntu 25.04+ | `qt6-base-dev qt6-declarative-dev qml6-module-org-kde-kirigami` |
| KDE neon / openSUSE | Qt 6 + KF6 Kirigami from the distro |

```sh
cargo run --release -p lampyris-ui
```

The UI crate is excluded from plain `cargo build` so the core builds anywhere.
Set `QMAKE=/path/to/qmake6` if `qmake6` is not on your `PATH`.

## Architecture

```
crates/
├── lampyris-core     detection engine — no I/O, no async; testable, replayable, WASM-ready
├── lampyris-ingest   sources: Bluesky Jetstream client (cursor + reconnect), JSONL replay
├── lampyris-storage  local SQLite: clusters and post references only, with retention
├── lampyris          pipeline (threads + bounded channels) and the `lampyris` CLI
└── lampyris-ui       Qt Quick + Kirigami app via CXX-Qt
```

```
source ──posts──▶ [bounded channel] ──▶ detector thread ──events──▶ SQLite + CLI/UI
```

- The bounded channel is the backpressure point. If the detector falls behind,
  ingestion waits; if Jetstream drops the connection, the client resumes from
  its cursor.
- The detector uses event time only, never the wall clock, so replays are
  exactly reproducible.
- Only cluster metadata and *references* to public posts are stored, not the
  firehose itself.

## Roadmap

- **Behavioral signals** that survive AI-generated, non-identical text:
  burst account creation, posting-time entropy, co-amplification graphs.
- **More sources**: Mastodon / the fediverse, Telegram public channels.
- **Signed, verifiable reports** that anyone can re-check against the public
  data.
- **Federation**: self-hosted nodes that exchange signed findings, with local
  trust and independent re-verification (Veilid is on the radar once it matures).
- **WebAssembly build** of the core for a zero-install browser version.
- Flatpak packaging.

## Contributing

Contributions are welcome from anyone, including under a pseudonym. See
[CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
