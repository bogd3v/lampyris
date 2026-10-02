# Contributing to Lampyris

Thanks for helping. Bots and manipulation campaigns are a global problem, so
contributions in any language, from anywhere, are welcome.

## Pseudonymous contributions are welcome

People working on disinformation can become targets. You do **not** need to use
your legal name. If you contribute under a pseudonym:

- Configure Git per repository: `git config user.name "your-handle"` and
  `git config user.email "your-handle@users.noreply.github.com"` (or your
  forge's no-reply address).
- Commit timestamps carry your time zone. Consider `TZ=UTC git commit …`.
- Check that logs, paths and screenshots don't reveal your home directory or
  username.

Maintainers will never ask contributors to reveal their identity.

## Development

```sh
cargo test               # core, ingest, storage, pipeline
cargo clippy --all-targets
cargo fmt --all
cargo bench -p lampyris-core   # detector throughput
```

The Qt/Kirigami app is built separately: `cargo build -p lampyris-ui`
(needs Qt 6 and KF6 Kirigami; see the README).

`LAMPYRIS_QML=crates/lampyris-ui/qml/main.qml cargo run -p lampyris-ui` loads
the QML from disk, so you can iterate on the interface without rebuilding.

## Ground rules for detection changes

- Keep `lampyris-core` free of I/O and async code.
- Every new signal must be explainable in one sentence and listed in
  [docs/METHODOLOGY.md](docs/METHODOLOGY.md).
- Changing the score formula means bumping `SCORE_MODEL_VERSION`.
- Add a test for each false positive or false negative you fix, ideally from a
  small anonymized recording.
- Never add code that targets, profiles or deanonymizes specific individuals.

## Pull requests

Small, focused PRs with a clear description are easiest to review. CI runs
formatting, clippy and tests.

## Conduct

Be kind and assume good faith. Harassment of anyone, including people whose
accounts appear in a cluster, is not tolerated in this project's spaces.
