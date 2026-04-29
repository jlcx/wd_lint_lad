//! Compiled view of the rules file.
//!
//! Built once at startup and shared across worker threads via `Arc`.
//! Holds the AhoCorasick automatons, lookup tables, and threshold
//! values the per-entity checks need at runtime.

use std::collections::{HashMap, HashSet};

use aho_corasick::AhoCorasick;
use wd_core::{Rules, Thresholds};

/// A possibly-empty set of substring patterns.
///
/// Empty sets always return `false` for `is_match`, sidestepping
/// AhoCorasick's empty-pattern constructor.
pub struct SubstringSet {
    inner: Option<AhoCorasick>,
}

impl SubstringSet {
    pub fn new<I, S>(patterns: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u8]>,
    {
        let collected: Vec<_> = patterns.into_iter().collect();
        if collected.is_empty() {
            return Ok(Self { inner: None });
        }
        let ac = AhoCorasick::new(&collected)?;
        Ok(Self { inner: Some(ac) })
    }

    pub fn is_match(&self, hay: &str) -> bool {
        self.inner.as_ref().is_some_and(|ac| ac.is_match(hay))
    }
}

pub struct CompiledRules {
    pub thresholds: Thresholds,
    pub skip_qids: HashMap<String, HashSet<String>>,
    /// Used by `aliases.long` in step 4. Kept here so all rule data lives
    /// in one place; suppress dead-code warning until then.
    #[allow(dead_code)]
    pub excluded_p31_for_long_aliases: HashSet<String>,

    pub bad_starts_descriptions: Vec<String>,
    pub trademark_chars: Vec<String>,
    pub ends_with_punctuation_exempt_suffixes: Vec<String>,

    pub marketing_imperatives: SubstringSet,
    pub promotional_substrings: SubstringSet,
    pub promotional_exempt_lower: SubstringSet,
    pub html_entity_substrings: SubstringSet,
    pub multi_sentence_markers: SubstringSet,
    pub obituary_markers: SubstringSet,

    pub misspellings: HashMap<String, String>,
    pub nationalities: HashSet<String>,
}

impl CompiledRules {
    pub fn compile(rules: &Rules) -> anyhow::Result<Self> {
        let promo_exempt_lower: Vec<String> = rules
            .promotional_exempt_substrings
            .iter()
            .map(|s| s.to_lowercase())
            .collect();

        Ok(Self {
            thresholds: rules.thresholds.clone(),
            skip_qids: rules.skip_qids.clone(),
            excluded_p31_for_long_aliases: rules.excluded_p31_for_long_aliases.clone(),
            bad_starts_descriptions: rules.bad_starts_descriptions.clone(),
            trademark_chars: rules.trademark_chars.clone(),
            ends_with_punctuation_exempt_suffixes: rules
                .ends_with_punctuation_exempt_suffixes
                .clone(),
            marketing_imperatives: SubstringSet::new(rules.marketing_imperatives.iter().map(String::as_str))?,
            promotional_substrings: SubstringSet::new(rules.promotional_substrings.iter().map(String::as_str))?,
            promotional_exempt_lower: SubstringSet::new(promo_exempt_lower.iter().map(String::as_str))?,
            html_entity_substrings: SubstringSet::new(rules.html_entity_substrings.iter().map(String::as_str))?,
            multi_sentence_markers: SubstringSet::new(rules.multi_sentence_markers.iter().map(String::as_str))?,
            obituary_markers: SubstringSet::new(rules.obituary_markers.iter().map(String::as_str))?,
            misspellings: rules.misspellings.clone(),
            // Union the two configured lists into a single runtime set:
            // both lists feed the same nationality/proper-adjective checks.
            nationalities: rules
                .nationalities_lower
                .iter()
                .chain(rules.proper_adjectives_lower.iter())
                .cloned()
                .collect(),
        })
    }

    pub fn skip_qids_for(&self, key: &str) -> Option<&HashSet<String>> {
        self.skip_qids.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_from(json: &str) -> Rules {
        serde_json::from_str(json).unwrap()
    }

    fn empty_rules() -> Rules {
        rules_from(
            r#"{
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
            }"#,
        )
    }

    #[test]
    fn empty_substring_set_never_matches() {
        let s = SubstringSet::new(Vec::<&str>::new()).unwrap();
        assert!(!s.is_match("anything"));
    }

    #[test]
    fn substring_set_finds_any_pattern() {
        let s = SubstringSet::new(["foo", "bar"]).unwrap();
        assert!(s.is_match("a foo b"));
        assert!(s.is_match("rebar"));
        assert!(!s.is_match("baz"));
    }

    #[test]
    fn compile_lowercases_promotional_exempts() {
        let mut rules = empty_rules();
        rules.promotional_exempt_substrings = vec!["AWARD".into()];
        let c = CompiledRules::compile(&rules).unwrap();
        assert!(c.promotional_exempt_lower.is_match("won an award"));
    }

    #[test]
    fn compile_preserves_skip_qids_and_p31_exclusions() {
        let mut rules = empty_rules();
        rules.skip_qids.insert("promotional".into(), ["Q1".to_string()].into_iter().collect());
        rules.excluded_p31_for_long_aliases = ["Q42".to_string()].into_iter().collect();
        let c = CompiledRules::compile(&rules).unwrap();
        assert!(c.skip_qids_for("promotional").unwrap().contains("Q1"));
        assert!(c.excluded_p31_for_long_aliases.contains("Q42"));
    }
}
