# Methodology

This document explains exactly what Lampyris measures, how the score is
computed, and where it can be wrong. If you publish findings made with
Lampyris, link here and state the detector settings you used.

## 1. What a "cluster" is

A cluster is a set of posts such that:

1. each post's normalized text has a 64-bit **SimHash** fingerprint within
   `max_hamming` bits (default 3) of another post in the set;
2. those posts were published within `window_secs` (default 600 s) of each other;
3. the set contains at least `min_accounts` **distinct accounts** (default 5).

One account repeating itself never forms a cluster on its own: only distinct
accounts count toward `min_accounts`.

A cluster is reported when it first reaches `min_accounts`, then again each
time its number of accounts doubles (5, 10, 20, 40, …). It is closed after a
whole window without new posts.

## 2. Normalization

Before hashing, text goes through, in order:

1. Unicode NFKC (full-width and stylized letters become plain letters).
2. Removal of URLs and @mentions (they usually differ between copies).
3. Removal of diacritics and invisible/format characters (zero-width spaces,
   bidi controls, variation selectors, tag characters…).
4. Folding of Cyrillic/Greek letters that look identical to Latin ones.
5. Lowercasing; punctuation, symbols and emojis become separators.

This is lossy on purpose: it is never shown to users, it only needs to map
"the same message" to the same string.

## 3. Fingerprints and matching

- Features: word unigrams + bigrams. For mostly unspaced scripts (CJK, Thai,
  Khmer…) character unigrams + bigrams.
- Texts with fewer than `min_tokens` words (default 5; ~2 characters per token
  in unspaced scripts) are ignored: short texts ("gm", "lol") collide by chance.
- Near-duplicate search uses locality-sensitive hashing: the 64 bits are split
  into `max_hamming + 1` bands. By the pigeonhole principle, any two
  fingerprints within `max_hamming` bits share at least one band exactly, so no
  match inside the window is missed.

## 4. Signals

| Signal | Meaning |
| --- | --- |
| `accounts` | distinct accounts in the cluster |
| `posts` | total posts |
| `span_secs` | time between first and last post |
| `accounts_per_minute` | `accounts / max(span in minutes, 1)` |
| `mean_distance` | mean SimHash distance to the cluster's anchor post (0 = identical after normalization) |
| `external_link_ratio` | share of posts embedding a link card |

## 5. Score (`heuristic-v0`)

```
size      = 1 − exp(−accounts / 15)
speed     = 1 − exp(−accounts_per_minute / 4)
sameness  = 1 − 0.5 · min(mean_distance / (max_hamming + 1), 1)
damping   = 1 − 0.5 · external_link_ratio
score     = size · speed · sameness · damping
```

**This is not a probability and it is not calibrated.** It is a transparent
ranking aid. The raw signals are always published next to it, and the model
name travels with every score so results stay comparable when the formula
changes.

## 6. Known false positives

- **Shared headlines**: many people posting a news link with its title as text.
  Damped through `external_link_ratio`, flagged in the UI.
- **Fandoms and memes**: song lyrics, catchphrases, "I voted" templates.
- **App-generated posts**: "I just joined…", game scores, cross-posting tools.
- **Legitimate organized action**: unions, NGOs or campaigns asking supporters to
  post a shared message. This *is* coordination, but not necessarily
  inauthentic. Lampyris cannot tell intent.

## 7. Known false negatives

- **AI-paraphrased campaigns**: varied wording defeats text similarity. This is
  the main reason behavioral signals are next on the roadmap.
- Campaigns slower than the window, or split across many small groups below
  `min_accounts`.
- Images, video and reposts without text (not analysed yet).

## 8. Responsible use

- Describe **patterns**, not people: "these 40 accounts posted this text within
  3 minutes", not "X is a bot farm paid by Y".
- Re-verify before publishing: every cluster stores references to public posts
  (`at://` URIs and web links) so anyone can check them.
- Publish your settings and, ideally, the recording (`watch --record`) so others
  can reproduce the result with `lampyris replay`.
- Correct mistakes publicly.

## 9. Data handled

- Input: public Bluesky posts, as published on the open firehose.
- Stored locally: cluster metadata, the text of one member post, and references
  (account DID, post id, time) to up to 50 member posts. Nothing else.
- Retention: 30 days by default (`--retention-days`).
- Recordings made with `--record` contain raw public posts; treat them with care.
