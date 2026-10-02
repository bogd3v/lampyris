//! Text normalization.
//!
//! Coordinated campaigns vary their texts just enough to dodge exact matching:
//! changed capitalization, emojis, different links, added mentions, accents,
//! full-width letters, look-alike characters from other scripts and invisible
//! characters. Normalization folds all of that away *before* hashing.
//!
//! The transformation is lossy on purpose. It is never shown to people; it only
//! has to be the same for two texts that a human would call "the same message".

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Normalize a post text for near-duplicate detection.
///
/// - Unicode NFKC (full-width and "math bold" letters, ligatures, ...).
/// - Drops URLs and @mentions (they differ per copy of a template).
/// - Removes diacritics, zero-width and other format characters.
/// - Maps common look-alike Cyrillic/Greek letters to Latin.
/// - Lowercases; turns punctuation, symbols and emojis into separators.
pub fn normalize(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    let mut out = String::with_capacity(nfkc.len());

    for raw_token in nfkc.split_whitespace() {
        if is_url(raw_token) || is_mention(raw_token) {
            continue;
        }
        let mut wrote_any = false;
        for c in raw_token.nfkd() {
            if is_combining_mark(c) || is_invisible(c) {
                continue;
            }
            for lc in c.to_lowercase() {
                let lc = fold_confusable(lc);
                if lc.is_alphanumeric() {
                    if !wrote_any && !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                    out.push(lc);
                    wrote_any = true;
                } else if wrote_any && !out.ends_with(' ') {
                    // Punctuation inside a token ("hola,mundo") splits it.
                    out.push(' ');
                }
            }
        }
        if wrote_any && !out.ends_with(' ') {
            out.push(' ');
        }
    }

    while out.ends_with(' ') {
        out.pop();
    }
    out
}

fn is_url(token: &str) -> bool {
    let t = token.trim_start_matches(|c: char| !c.is_alphanumeric());
    let lower: String = t.chars().take(8).collect::<String>().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("www.")
        || token.contains("://")
}

fn is_mention(token: &str) -> bool {
    token.starts_with('@') && token.chars().count() > 1
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'               // soft hyphen
        | '\u{034F}'             // combining grapheme joiner
        | '\u{061C}'             // arabic letter mark
        | '\u{115F}' | '\u{1160}'
        | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}' // zero-width space/joiners, LRM/RLM
        | '\u{202A}'..='\u{202E}' // bidi embeddings/overrides
        | '\u{2060}'..='\u{206F}' // word joiner, invisible operators
        | '\u{3164}'             // hangul filler
        | '\u{FE00}'..='\u{FE0F}' // variation selectors
        | '\u{FEFF}'             // BOM / zero-width no-break space
        | '\u{FFA0}'
        | '\u{E0000}'..='\u{E007F}' // tag characters
    )
}

/// Map lowercase letters that are visually identical to Latin ones.
///
/// Applied to every text, including texts genuinely written in Cyrillic or
/// Greek: because the mapping is deterministic, two copies of the same message
/// still produce the same output, and spoofed Latin text collapses back onto
/// the original.
fn fold_confusable(c: char) -> char {
    match c {
        // Cyrillic
        'а' => 'a',
        'е' => 'e',
        'о' => 'o',
        'р' => 'p',
        'с' => 'c',
        'у' => 'y',
        'х' => 'x',
        'і' => 'i',
        'ј' => 'j',
        'ѕ' => 's',
        'ԁ' => 'd',
        'ӏ' => 'l',
        'ԛ' => 'q',
        'ԝ' => 'w',
        'ѵ' => 'v',
        'һ' => 'h',
        'к' => 'k',
        // Greek
        'ο' => 'o',
        'ν' => 'v',
        'ι' => 'i',
        'κ' => 'k',
        'ρ' => 'p',
        'υ' => 'u',
        'α' => 'a',
        _ => c,
    }
}

/// Whether a normalized text is written mostly in scripts without spaces between
/// words (Chinese, Japanese, Thai, ...), where shingles must be built from
/// characters instead of words. Korean uses spaces but its words are long, so it
/// is grouped here too.
pub fn is_unspaced_script(normalized: &str) -> bool {
    let mut letters = 0usize;
    let mut unspaced = 0usize;
    for c in normalized.chars().filter(|c| !c.is_whitespace()) {
        letters += 1;
        if is_unspaced_char(c) {
            unspaced += 1;
        }
    }
    letters > 0 && unspaced * 10 >= letters * 3
}

fn is_unspaced_char(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}'   // Hiragana, Katakana
        | '\u{3400}'..='\u{4DBF}' // CJK ext A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified
        | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
        | '\u{0E00}'..='\u{0E7F}' // Thai
        | '\u{0E80}'..='\u{0EFF}' // Lao
        | '\u{1000}'..='\u{109F}' // Myanmar
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FA1F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_punctuation_and_emoji() {
        assert_eq!(normalize("¡Hola, MUNDO!!! 🔥🔥"), "hola mundo");
    }

    #[test]
    fn drops_urls_and_mentions() {
        assert_eq!(
            normalize("@someone.bsky.social mira esto https://ejemplo.com/a?b=1 ya"),
            "mira esto ya"
        );
        assert_eq!(normalize("(www.example.org) texto"), "texto");
    }

    #[test]
    fn keeps_hashtag_words() {
        assert_eq!(normalize("#Elecciones2026 ahora"), "elecciones2026 ahora");
    }

    #[test]
    fn removes_diacritics() {
        assert_eq!(normalize("Información pública"), "informacion publica");
    }

    #[test]
    fn folds_full_width_and_math_letters() {
        assert_eq!(normalize("ｈｏｌａ 𝐦𝐮𝐧𝐝𝐨"), "hola mundo");
    }

    #[test]
    fn folds_cyrillic_lookalikes() {
        // "vote" with Cyrillic о and е.
        assert_eq!(normalize("vоtе"), "vote");
    }

    #[test]
    fn strips_invisible_characters() {
        assert_eq!(normalize("vo\u{200B}te no\u{FEFF}w"), "vote now");
    }

    #[test]
    fn splits_on_inner_punctuation() {
        assert_eq!(normalize("hola,mundo"), "hola mundo");
    }

    #[test]
    fn detects_unspaced_scripts() {
        assert!(is_unspaced_script(&normalize("今日は選挙の日です")));
        assert!(!is_unspaced_script(&normalize("hoy es día de elecciones")));
    }
}
