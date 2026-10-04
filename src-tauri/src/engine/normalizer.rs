use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036f}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe20}'..='\u{fe2f}'
    )
}

static FEAT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[\(\[]?\s*\b(?:feat\.?|ft\.?|featuring|with)\s+([^\)\]\-]+)[\)\]]?")
        .expect("valid feat regex")
});

static NOISE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:(?:\d{4}\s+)?re-?master(?:ed)?(?:\s+\d{4})?|deluxe(?:\s+edition)?|bonus(?:\s+track)?|anniversary(?:\s+edition)?|original\s+mix|radio\s+edit|single\s+version|mono(?:\s+version)?|stereo(?:\s+version)?)\b")
        .expect("valid noise regex")
});

static CLEAN_BRACKETS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    // Clean empty or dangling punctuation/brackets left after stripping
    Regex::new(r"[\(\[]\s*[\)\]]|\s*-\s*$|^\s*-\s*").expect("valid cleanup regex")
});

/// Canonicalizes punctuation, decomposes diacritics via NFKD, and maps symbols per D-11.
pub fn canonicalize_string(input: &str) -> String {
    // 1. Unicode NFKD decomposition and strip combining diacritical marks
    let decomposed: String = input.nfkd().filter(|c| !is_combining_mark(*c)).collect();

    // 2. Straighten quotes, map dashes, map ampersands to 'and'
    let mut replaced = String::with_capacity(decomposed.len());
    for c in decomposed.chars() {
        match c {
            '’' | '‘' | '`' | '´' => replaced.push('\''),
            '“' | '”' => replaced.push('"'),
            '–' | '—' | '−' => replaced.push('-'),
            '&' => replaced.push_str(" and "),
            _ => replaced.push(c),
        }
    }

    // 3. Lowercase and collapse consecutive spaces
    let mut result = String::new();
    let mut prev_space = false;
    for c in replaced.to_lowercase().chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }

    result.trim().to_string()
}

/// Normalizes track title per D-09, D-10, and D-11.
/// Strips remaster/edition noise while preserving 'Live', 'Acoustic', and 'Instrumental'.
/// Extracts featured artists to candidate search pool.
pub fn normalize_title(raw_title: &str) -> (String, Vec<String>) {
    let mut feat_artists = Vec::new();

    // 1. Extract featuring artists per D-10
    let mut working_title = raw_title.to_string();
    for cap in FEAT_REGEX.captures_iter(raw_title) {
        if let Some(artists_match) = cap.get(1) {
            let artists_raw = artists_match.as_str();
            for artist in artists_raw.split(&[',', '&', '/'][..]) {
                let cleaned = artist.trim();
                if !cleaned.is_empty() {
                    feat_artists.push(cleaned.to_string());
                }
            }
        }
    }
    working_title = FEAT_REGEX.replace_all(&working_title, "").to_string();

    // 2. Strip noise tokens (remaster, deluxe, etc.) while preserving Live/Acoustic per D-09
    working_title = NOISE_REGEX.replace_all(&working_title, "").to_string();

    // 3. Clean trailing hyphens or empty brackets
    let mut prev = String::new();
    while prev != working_title {
        prev = working_title.clone();
        working_title = CLEAN_BRACKETS_REGEX
            .replace_all(&working_title, "")
            .to_string();
        working_title = working_title
            .trim()
            .trim_end_matches('-')
            .trim()
            .to_string();
    }

    // 4. Canonicalize string (NFKD diacritics, ampersands, punctuation) per D-11
    let canonical = canonicalize_string(&working_title);

    (canonical, feat_artists)
}

/// Computes string similarity using Jaro-Winkler with token-sort heuristic per D-12.
pub fn calculate_similarity(s1: &str, s2: &str) -> f64 {
    let n1 = canonicalize_string(s1);
    let n2 = canonicalize_string(s2);

    if n1.is_empty() && n2.is_empty() {
        return 1.0;
    }
    if n1.is_empty() || n2.is_empty() {
        return 0.0;
    }
    if n1 == n2 {
        return 1.0;
    }

    // 1. Direct Jaro-Winkler similarity
    let direct_score = strsim::jaro_winkler(&n1, &n2);

    // 2. Token-sorted Jaro-Winkler similarity to handle transposed word ordering
    let mut tokens1: Vec<&str> = n1.split_whitespace().collect();
    let mut tokens2: Vec<&str> = n2.split_whitespace().collect();
    tokens1.sort_unstable();
    tokens2.sort_unstable();

    let sorted1 = tokens1.join(" ");
    let sorted2 = tokens2.join(" ");

    let token_sort_score = strsim::jaro_winkler(&sorted1, &sorted2);

    f64::max(direct_score, token_sort_score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_remaster_stripping() {
        let (title, _) = normalize_title("Hotel California - 2013 Remaster");
        assert_eq!(title, "hotel california");

        let (title2, _) = normalize_title("In the End (Deluxe Edition)");
        assert_eq!(title2, "in the end");

        let (title3, _) = normalize_title("Paranoid - Anniversary Edition");
        assert_eq!(title3, "paranoid");
    }

    #[test]
    fn test_preserve_live_and_acoustic() {
        let (live_title, _) = normalize_title("Comfortably Numb - Live at Pompeii");
        assert!(live_title.contains("live"));

        let (acoustic_title, _) = normalize_title("Creep (Acoustic)");
        assert!(acoustic_title.contains("acoustic"));

        let (instrumental_title, _) = normalize_title("Orion (Instrumental)");
        assert!(instrumental_title.contains("instrumental"));
    }

    #[test]
    fn test_featured_artist_extraction() {
        let (title, artists) = normalize_title("Under Pressure (feat. David Bowie)");
        assert_eq!(title, "under pressure");
        assert_eq!(artists, vec!["David Bowie"]);

        let (title2, artists2) = normalize_title("Levitating (feat. Daft Punk, Madonna)");
        assert_eq!(title2, "levitating");
        assert_eq!(artists2, vec!["Daft Punk", "Madonna"]);
    }

    #[test]
    fn test_nfkd_diacritic_folding_and_symbols() {
        assert_eq!(canonicalize_string("Báilame"), "bailame");
        assert_eq!(canonicalize_string("Café Tacvba"), "cafe tacvba");
        assert_eq!(canonicalize_string("Sigur Rós"), "sigur ros");
        assert_eq!(
            canonicalize_string("Simon & Garfunkel"),
            "simon and garfunkel"
        );
        assert_eq!(canonicalize_string("Don’t Stop"), "don't stop");
    }

    #[test]
    fn test_token_sorted_similarity() {
        // Transposed words should achieve high similarity score
        let score =
            calculate_similarity("Around the World Daft Punk", "Daft Punk Around the World");
        assert!(score >= 0.95);

        // Identical strings should be 1.0
        assert_eq!(calculate_similarity("Billie Jean", "Billie Jean"), 1.0);

        // Completely different strings should have low score
        let low_score = calculate_similarity("Hello", "Goodbye");
        assert!(low_score < 0.50);
    }
}
