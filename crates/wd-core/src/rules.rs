use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::{fs, io};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rules {
    pub nationalities_lower: Vec<String>,
    /// Additional lowercase tokens that should be capitalized in
    /// descriptions, beyond strict country/demonym terms — e.g.,
    /// continent adjectives ("european", "african") and religious
    /// adjectives ("christian", "muslim"). Merged with
    /// `nationalities_lower` at runtime; the two lists are treated as
    /// a single set by both `description.starts_with_lowercase_nationality`
    /// and `description.contains_lowercase_nationality`. Empty by default.
    #[serde(default)]
    pub proper_adjectives_lower: Vec<String>,
    pub misspellings: HashMap<String, String>,
    /// Detection-only counterpart to `misspellings`. Same `wrong → right`
    /// mapping shape, but matches on these keys are flagged via
    /// `description.misspelled_advisory` and never auto-applied by the
    /// fixer. Use this for keys where the "wrong" form is also a real
    /// surname, valid regional spelling, or otherwise plausible
    /// non-misspelling.
    #[serde(default)]
    pub misspellings_advisory: HashMap<String, String>,
    pub bad_starts_descriptions: Vec<String>,
    pub marketing_imperatives: Vec<String>,
    pub promotional_substrings: Vec<String>,
    pub promotional_exempt_substrings: Vec<String>,
    pub trademark_chars: Vec<String>,
    pub html_entity_substrings: Vec<String>,
    pub multi_sentence_markers: Vec<String>,
    pub obituary_markers: Vec<String>,
    #[serde(default)]
    pub skip_qids: HashMap<String, HashSet<String>>,
    #[serde(default)]
    pub excluded_p31_for_long_aliases: HashSet<String>,
    /// End-of-description literal suffixes that exempt a value from
    /// `description.ends_with_punctuation` (e.g. "Inc.", "Ltd."). Empty
    /// by default. The balanced-parenthesis exemption is hardcoded and
    /// not configured here.
    #[serde(default)]
    pub ends_with_punctuation_exempt_suffixes: Vec<String>,
    /// Subset of `bad_starts_descriptions` that the fixer is allowed
    /// to strip from the start of a description. Typically copular
    /// phrases like "is a ", "was an "; articles ("A "/"The ") are
    /// usually omitted because they're load-bearing for proper nouns.
    /// Empty by default — no auto-stripping unless configured.
    #[serde(default)]
    pub bad_start_strip_prefixes: Vec<String>,
    /// Maps language codes to expected script names (currently only
    /// `"latin"` is defined). Used by `label.wrong_script`,
    /// `alias.wrong_script`, and `description.mostly_foreign_script`.
    /// Lookup is hierarchical: `"en-gb"` falls back to `"en"` if not
    /// present. Empty by default.
    #[serde(default)]
    pub script_policies: HashMap<String, String>,
    pub thresholds: Thresholds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    pub description_max_len: usize,
    pub descgust_score_threshold: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("failed to read rules file {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse rules file {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub fn load_rules<P: AsRef<Path>>(path: P) -> Result<Rules, RulesError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| RulesError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| RulesError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

impl Rules {
    /// Returns the skip-list QIDs for a named check, or an empty set if absent.
    pub fn skip_qids_for(&self, check: &str) -> &HashSet<String> {
        static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        self.skip_qids
            .get(check)
            .unwrap_or_else(|| EMPTY.get_or_init(HashSet::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "nationalities_lower": ["palestine", "palestinian"],
        "misspellings": {"abandonned": "abandoned"},
        "bad_starts_descriptions": ["a ", "an ", "the "],
        "marketing_imperatives": ["Discover ", "Buy "],
        "promotional_substrings": ["the best ", " finest "],
        "promotional_exempt_substrings": ["award"],
        "trademark_chars": ["®", "™"],
        "html_entity_substrings": ["&amp;", "&#91;", "&#93;"],
        "multi_sentence_markers": [". The", ". A "],
        "obituary_markers": ["Obituary"],
        "skip_qids": {
            "promotional": ["Q749290"],
            "long_aliases": ["Q633110"]
        },
        "excluded_p31_for_long_aliases": ["Q13442814"],
        "thresholds": {
            "description_max_len": 140,
            "descgust_score_threshold": 4
        }
    }"#;

    #[test]
    fn parses_sample() {
        let r: Rules = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(r.thresholds.description_max_len, 140);
        assert_eq!(r.thresholds.descgust_score_threshold, 4);
        assert_eq!(r.misspellings.get("abandonned").unwrap(), "abandoned");
        assert!(r.excluded_p31_for_long_aliases.contains("Q13442814"));
    }

    #[test]
    fn skip_qids_for_known_and_unknown() {
        let r: Rules = serde_json::from_str(SAMPLE).unwrap();
        assert!(r.skip_qids_for("promotional").contains("Q749290"));
        assert!(r.skip_qids_for("nonexistent_check").is_empty());
    }

    #[test]
    fn skip_qids_optional() {
        let json = r#"{
            "nationalities_lower": [],
            "misspellings": {},
            "bad_starts_descriptions": [],
            "marketing_imperatives": [],
            "promotional_substrings": [],
            "promotional_exempt_substrings": [],
            "trademark_chars": [],
            "html_entity_substrings": [],
            "multi_sentence_markers": [],
            "obituary_markers": [],
            "thresholds": {"description_max_len": 140, "descgust_score_threshold": 4}
        }"#;
        let r: Rules = serde_json::from_str(json).unwrap();
        assert!(r.skip_qids.is_empty());
        assert!(r.excluded_p31_for_long_aliases.is_empty());
    }

    #[test]
    fn missing_required_field_errors() {
        let json = r#"{"nationalities_lower": []}"#;
        assert!(serde_json::from_str::<Rules>(json).is_err());
    }
}
