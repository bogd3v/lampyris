//! 64-bit SimHash fingerprints.
//!
//! SimHash maps similar texts to fingerprints that differ in few bits, so
//! "almost the same message" becomes "small Hamming distance" — a single XOR and
//! popcount, cheap enough to run on every post of the firehose.

use crate::normalize::{is_unspaced_script, normalize};
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

/// Number of bits that differ between two fingerprints.
#[inline]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Fingerprint of a raw post text, or `None` when the text is too short to say
/// anything meaningful ("gm", "lol", a lone emoji). Short texts are identical by
/// chance far too often to count as evidence of coordination.
pub fn fingerprint(text: &str, min_tokens: usize) -> Option<u64> {
    fingerprint_normalized(&normalize(text), min_tokens)
}

/// Fingerprint of an already normalized text (see [`normalize`]).
pub fn fingerprint_normalized(normalized: &str, min_tokens: usize) -> Option<u64> {
    let mut acc = Accumulator::default();
    if is_unspaced_script(normalized) {
        // Character bigrams. Roughly two characters carry what one word does in
        // spaced languages, so the minimum length is scaled accordingly.
        let chars: Vec<char> = normalized.chars().filter(|c| !c.is_whitespace()).collect();
        if chars.len() < min_tokens * 2 {
            return None;
        }
        let mut buf = [0u8; 4];
        for c in &chars {
            acc.add(xxh3_64(c.encode_utf8(&mut buf).as_bytes()));
        }
        for pair in chars.windows(2) {
            let seed = xxh3_64(pair[0].encode_utf8(&mut buf).as_bytes());
            acc.add(xxh3_64_with_seed(
                pair[1].encode_utf8(&mut buf).as_bytes(),
                seed,
            ));
        }
    } else {
        let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
        if words.len() < min_tokens {
            return None;
        }
        // Unigrams capture vocabulary, bigrams capture word order.
        for w in &words {
            acc.add(xxh3_64(w.as_bytes()));
        }
        for pair in words.windows(2) {
            acc.add(xxh3_64_with_seed(
                pair[1].as_bytes(),
                xxh3_64(pair[0].as_bytes()),
            ));
        }
    }
    Some(acc.finish())
}

struct Accumulator {
    counts: [i32; 64],
}

impl Default for Accumulator {
    fn default() -> Self {
        Self { counts: [0; 64] }
    }
}

impl Accumulator {
    #[inline]
    fn add(&mut self, h: u64) {
        for (bit, count) in self.counts.iter_mut().enumerate() {
            if (h >> bit) & 1 == 1 {
                *count += 1;
            } else {
                *count -= 1;
            }
        }
    }

    fn finish(&self) -> u64 {
        self.counts
            .iter()
            .enumerate()
            .filter(|(_, &c)| c > 0)
            .fold(0u64, |acc, (bit, _)| acc | (1 << bit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: usize = 5;

    fn fp(t: &str) -> u64 {
        fingerprint(t, MIN).expect("long enough")
    }

    #[test]
    fn identical_texts_match_exactly() {
        let t = "Mañana todos a votar por el único candidato honesto del país";
        assert_eq!(fp(t), fp(t));
    }

    #[test]
    fn cosmetic_variants_match_exactly() {
        let a = "Mañana todos a votar por el único candidato honesto del país";
        let b = "MAÑANA todos a votar por el único candidato honesto del país!!! 🇨🇴 https://x.co/1 @fan";
        assert_eq!(fp(a), fp(b));
    }

    #[test]
    fn small_edits_stay_close() {
        let a = "the election was stolen and the media is hiding the truth from all of you, share before they delete it";
        let b = "the election was stolen and the media is hiding the real truth from all of you, share before they delete it";
        let d = hamming(fp(a), fp(b));
        assert!(d <= 8, "distance {d}");
    }

    #[test]
    fn unrelated_texts_are_far_apart() {
        let a = "the election was stolen and the media is hiding the truth from all of you";
        let b = "just baked sourdough bread for the first time and it came out surprisingly well";
        let d = hamming(fp(a), fp(b));
        assert!(d >= 16, "distance {d}");
    }

    #[test]
    fn short_texts_are_ignored() {
        assert_eq!(fingerprint("gm gm", MIN), None);
        assert_eq!(fingerprint("🔥🔥🔥", MIN), None);
    }

    #[test]
    fn cjk_texts_get_fingerprints() {
        assert!(fingerprint("今日は選挙の日ですみんな投票に行こう", MIN).is_some());
        assert_eq!(fingerprint("投票", MIN), None);
    }
}
