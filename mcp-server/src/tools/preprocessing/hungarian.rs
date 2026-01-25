//! Hungarian language preprocessor plugin
//!
//! Provides Hungarian-specific query preprocessing for RAG:
//! - Stop word removal (vannak, voltak, hogy, etc.)
//! - Question word removal (milyen, hogyan, miért, etc.)
//! - Suffix stripping (Hungarian grammatical endings)

use super::{PreprocessResult, QueryPreprocessor};
use std::collections::HashSet;

/// Hungarian language preprocessor
pub struct HungarianPreprocessor;

impl HungarianPreprocessor {
    /// Hungarian suffixes sorted by length (longest first for greedy matching)
    ///
    /// Hungarian is an agglutinative language with complex suffix combinations.
    /// Order matters: longer suffixes must come first for greedy matching.
    const SUFFIXES: &'static [&'static str] = &[
        // ============ 7+ karakteres összetett ragok ============
        "jaikkal", "jeikkel", // birtokos többes eszközhatározó
        "jaikat", "jeiket", // birtokos többes tárgyeset
        "okban", "ekben", "ökben", "akban", // helyhatározó többes
        // ============ 6 karakteres ragok ============
        "jeiket", "aikat", "jeink", "aink", "jeibe", "aiba", "aitok", "eitek", "jaink", "jeink",
        "ukkal", "ükkel", // birtokos eszközhatározó
        "unkat", "ünket", // birtokos tárgyeset (mi)
        // ============ 5 karakteres ragok ============
        "ekkel", "okkal", "akkal", "ükkel", "aikat", "eiket", // birtokos tárgyeset többes
        "jának", "jének", // birtokos részes
        "ához", "éhez", // birtokos -hoz
        "ával", "ével", // birtokos eszközhatározó
        "okat", "akat", "eket", "öket", // tárgyeset többes (KRITIKUS!)
        // ============ 4 karakteres ragok ============
        "ként", "kor", "ból", "ből", "tól", "től", "ról", "ről", "nak", "nek", "val", "vel", "hoz",
        "hez", "höz", "ban", "ben", "nál", "nél", "ait", "eit", "jait",
        "jeit", // birtokos tárgyeset
        "ját", "jét", // egyes 3. személyű birtokos tárgyeset
        "unk", "ünk", // birtokos többes 1. személy
        "tok", "tek", "tök", // birtokos többes 2. személy
        // ============ 3 karakteres ragok ============
        "ért",
        // Note: ból/ből/tól/től/nak/nek/ban/ben/hoz/hez/höz/val/vel/nál/nél/ról/ről
        // are in 4-char section above (they're 4 bytes due to accents, but 3 chars)
        // ============ 2 karakteres ragok ============
        "ba", "be", "ra", "re", "ig", "at", "et", "ot", "öt", // tárgyeset egyes
        "ok", "ek", "ök", "ak", // többes szám
        "ja", "je", "ai", "ei", // birtokos
        "ám", "ém", "om", "em", "öm", // birtokos 1. személy
        "ád", "éd", "od", "ed", "öd", // birtokos 2. személy
        "uk", "ük", // birtokos 3. személy többes
    ];

    /// Hungarian stop words (common words without semantic value)
    const STOP_WORDS: &'static [&'static str] = &[
        // Létigék
        "vannak",
        "voltak",
        "lenne",
        "lennének",
        "lesz",
        "lesznek",
        "volt",
        "van",
        "vagy",
        "vagyok",
        "vagyunk",
        "vagytok",
        // Kötőszavak, módosítószavak
        "hogy",
        "mint",
        "amely",
        "amelyek",
        "amit",
        "amiket",
        "ahol",
        "amikor",
        "mivel",
        "mert",
        "tehát",
        "vagyis",
        "illetve",
        "valamint",
        "pedig",
        "csak",
        "még",
        "már",
        "minden",
        "összes",
        "egyik",
        "másik",
        "szerint",
        "között",
        "után",
        "előtt",
        "alatt",
        "felett",
        "mellett",
        "által",
        "felé",
        "ellen",
        "miatt",
        "helyett",
        "ezek",
        "azok",
        "ilyen",
        "olyan",
        "igen",
        "nem",
        "és",
        "egy",
        "kell",
        "kéne",
        "lehet",
        "lehetne",
        // Névelők és mutató névmások
        "az",
        "ezt",
        "azt",
        "ezen",
        "azon",
        "ebben",
        "abban",
        "ennek",
        "annak",
        // Segédigék
        "tudja",
        "tudják",
        "tudjuk",
        "tud",
        "fog",
        "fogja",
        "fogják",
        "fogjuk",
        // Gyakori rövid szavak
        "meg",
        "fel",
        "le",
        "ki",
        "be", // igekötők önállóan
    ];

    /// Hungarian question words
    const QUESTION_WORDS: &'static [&'static str] = &[
        "milyen", "milyenek", "hogyan", "miért", "mikor", "hol", "honnan", "hová", "mennyi",
        "hány", "melyik", "melyeket", "kik", "kiket", "mik", "miket", "mit", "mi",
        "ki", // rövid kérdőszavak
    ];

    /// Strip the longest matching suffix from a word
    fn strip_suffix(word: &str) -> (String, Option<String>) {
        let word_chars = word.chars().count();
        for suffix in Self::SUFFIXES {
            let suffix_chars = suffix.chars().count();
            // Only strip if word is longer than suffix + 2 chars (preserve stem of min 3 chars)
            if word_chars > suffix_chars + 2 && word.ends_with(suffix) {
                let stemmed = &word[..word.len() - suffix.len()];

                // Don't strip -at/-et/-ot/-öt if stem ends in 't' (causative verbs: -tat/-tet)
                // e.g., "különböztet" should NOT become "különbözt"
                if matches!(*suffix, "at" | "et" | "ot" | "öt") && stemmed.ends_with('t') {
                    continue;
                }

                return (stemmed.to_string(), Some(word.to_string()));
            }
        }
        (word.to_string(), None)
    }
}

impl QueryPreprocessor for HungarianPreprocessor {
    fn language(&self) -> &'static str {
        "hungarian"
    }

    fn preprocess(&self, query: &str) -> PreprocessResult {
        let stop_set: HashSet<&str> = Self::STOP_WORDS
            .iter()
            .chain(Self::QUESTION_WORDS.iter())
            .copied()
            .collect();

        let mut removed_words = Vec::new();
        let mut stemmed_words = Vec::new();
        let mut result_words = Vec::new();

        // Clean query: lowercase, keep only alphanumeric and whitespace
        let cleaned: String = query
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();

        for word in cleaned.split_whitespace() {
            // Skip short words (< 3 chars) - use chars().count() for UTF-8 correctness
            if word.chars().count() < 3 {
                removed_words.push(word.to_string());
                continue;
            }

            // Skip stop/question words
            if stop_set.contains(word) {
                removed_words.push(word.to_string());
                continue;
            }

            // Strip suffix
            let (stemmed, original) = Self::strip_suffix(word);

            // Skip if stemmed result is too short (< 3 chars)
            if stemmed.chars().count() < 3 {
                removed_words.push(word.to_string());
                continue;
            }

            // Track stemming for debug/transparency
            if let Some(orig) = original {
                stemmed_words.push((orig, stemmed.clone()));
            }
            result_words.push(stemmed);
        }

        PreprocessResult {
            processed: result_words.join(" "),
            original: query.to_string(),
            removed_words,
            stemmed_words,
        }
    }

    fn stop_words(&self) -> &[&str] {
        Self::STOP_WORDS
    }

    fn question_words(&self) -> &[&str] {
        Self::QUESTION_WORDS
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn preprocessor() -> HungarianPreprocessor {
        HungarianPreprocessor
    }

    #[test]
    fn test_language_name() {
        assert_eq!(preprocessor().language(), "hungarian");
    }

    #[test]
    fn test_basic_preprocessing() {
        let result = preprocessor().preprocess("Milyen jellemzői vannak a kalibrálásnak?");
        // "Milyen" and "vannak" should be removed (question word, stop word)
        // "jellemzői" → "jellemz" (suffix stripping)
        // "kalibrálásnak" → "kalibrálás" (suffix stripping)
        assert!(!result.processed.contains("milyen"));
        assert!(!result.processed.contains("vannak"));
        assert!(result.processed.contains("jellemz"));
    }

    #[test]
    fn test_stop_word_removal() {
        let result = preprocessor().preprocess("hogy mivel mert tehát");
        assert_eq!(result.processed, "");
        assert_eq!(result.removed_words.len(), 4);
    }

    #[test]
    fn test_question_word_removal() {
        let result = preprocessor().preprocess("milyen hogyan miért mikor");
        assert_eq!(result.processed, "");
        assert_eq!(result.removed_words.len(), 4);
    }

    #[test]
    fn test_suffix_stripping() {
        // Test various suffixes
        let test_cases = vec![
            ("házból", "ház"),       // -ból
            ("könyvnek", "könyv"),   // -nek
            ("emberekkel", "ember"), // -ekkel
            ("kutyáknak", "kutyák"), // -nak (kutyák is kept as stem)
        ];

        for (input, expected_stem) in test_cases {
            let (stemmed, _) = HungarianPreprocessor::strip_suffix(input);
            assert!(
                stemmed.starts_with(expected_stem) || stemmed == expected_stem,
                "Expected '{}' to stem to something starting with '{}', got '{}'",
                input,
                expected_stem,
                stemmed
            );
        }
    }

    #[test]
    fn test_short_word_removal() {
        let result = preprocessor().preprocess("a az és ki mi");
        // All words are < 3 chars or stop words
        assert_eq!(result.processed, "");
    }

    #[test]
    fn test_preserves_meaningful_words() {
        let result = preprocessor().preprocess("kalibrálás mérés beállítás");
        // These should be preserved (possibly stemmed)
        assert!(!result.processed.is_empty());
    }

    #[test]
    fn test_stemmed_words_tracking() {
        let result = preprocessor().preprocess("kalibrálásnak beállításból");
        // Should track which words were stemmed
        assert!(!result.stemmed_words.is_empty());
        // Each entry is (original, stemmed)
        for (orig, stemmed) in &result.stemmed_words {
            assert!(orig.len() > stemmed.len());
        }
    }

    #[test]
    fn test_removed_words_tracking() {
        let result = preprocessor().preprocess("milyen jellemzői vannak");
        // "milyen" and "vannak" should be in removed_words
        assert!(result.removed_words.contains(&"milyen".to_string()));
        assert!(result.removed_words.contains(&"vannak".to_string()));
    }

    #[test]
    fn test_complex_query() {
        let result = preprocessor()
            .preprocess("Milyen gázokat kell használni a kalibráláshoz és hogyan állítsuk be?");
        // Should process complex Hungarian query
        assert!(!result.processed.is_empty());
        // Should not contain question words
        assert!(!result.processed.contains("milyen"));
        assert!(!result.processed.contains("hogyan"));
        // Should not contain stop words
        assert!(!result.processed.contains("kell"));
        assert!(!result.processed.contains("és"));
    }

    #[test]
    fn test_passthrough_non_hungarian() {
        // If used on English text, still processes (but may not be optimal)
        let result = preprocessor().preprocess("What are the characteristics?");
        // Should still produce some output
        assert!(!result.processed.is_empty());
    }

    #[test]
    fn test_utf8_char_counting() {
        // Verify UTF-8 character counting works correctly
        // "áz" is 3 bytes but only 2 characters - should be filtered
        let result = preprocessor().preprocess("áz éz őz");
        // All words are 2 characters, should be removed
        assert!(result.removed_words.contains(&"áz".to_string()));
        assert!(result.removed_words.contains(&"éz".to_string()));
        assert!(result.removed_words.contains(&"őz".to_string()));

        // "ház" is 4 bytes but 3 characters - should be kept
        let result = preprocessor().preprocess("ház tűz víz");
        assert!(result.processed.contains("ház"));
        assert!(result.processed.contains("tűz"));
        assert!(result.processed.contains("víz"));
    }

    #[test]
    fn test_causative_verbs_not_stemmed() {
        // -tat/-tet causative suffixes should NOT be stripped
        // "különböztet" = distinguish (causative verb) - should stay as-is
        let (stemmed, _) = HungarianPreprocessor::strip_suffix("különböztet");
        assert_eq!(
            stemmed, "különböztet",
            "causative -tet should not be stripped"
        );

        // "olvastat" = make someone read - should stay as-is
        let (stemmed, _) = HungarianPreprocessor::strip_suffix("olvastat");
        assert_eq!(stemmed, "olvastat", "causative -tat should not be stripped");

        // But regular accusative should still work:
        // "házat" = house (accusative) - should stem to "ház"
        let (stemmed, _) = HungarianPreprocessor::strip_suffix("házat");
        assert_eq!(stemmed, "ház", "accusative -at should be stripped");
    }
}
