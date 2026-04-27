//! Streaming "high-water mark" checks (`aliases.long`, `descriptions.long`).
//!
//! These run on the writer thread so the running maximum stays monotonic
//! and ordering is deterministic. The worker pre-extracts the relevant
//! data per entity into a `StreamingCandidate`; the writer applies the
//! running maxima and emits issues.

use wd_core::{Details, Field, Issue, entity::Entity, is_english};

use crate::matchers::CompiledRules;

#[derive(Debug, Clone)]
pub struct LangValue {
    pub lang: String,
    pub value: String,
    pub char_len: usize,
}

#[derive(Debug, Clone)]
pub struct StreamingCandidate {
    pub qid: String,
    /// English aliases when the entity is eligible for `aliases.long`;
    /// `None` when ineligible (no P31 / excluded P31 / skip_qids hit /
    /// the check is disabled).
    pub aliases: Option<Vec<LangValue>>,
    /// English descriptions when eligible for `descriptions.long`.
    pub descriptions: Option<Vec<LangValue>>,
}

#[derive(Debug, Default)]
pub struct StreamingState {
    pub aliases_max: usize,
    pub descriptions_max: usize,
}

fn collect_english_descriptions(entity: &Entity) -> Vec<LangValue> {
    let mut langs: Vec<&String> = entity
        .descriptions
        .keys()
        .filter(|k| is_english(k))
        .collect();
    langs.sort();
    langs
        .into_iter()
        .filter_map(|lang| {
            entity.descriptions.get(lang).map(|mt| LangValue {
                lang: lang.clone(),
                value: mt.value.clone(),
                char_len: mt.value.chars().count(),
            })
        })
        .collect()
}

fn collect_english_aliases(entity: &Entity) -> Vec<LangValue> {
    let mut langs: Vec<&String> = entity.aliases.keys().filter(|k| is_english(k)).collect();
    langs.sort();
    let mut out = Vec::new();
    for lang in langs {
        if let Some(values) = entity.aliases.get(lang) {
            for mt in values {
                out.push(LangValue {
                    lang: lang.clone(),
                    value: mt.value.clone(),
                    char_len: mt.value.chars().count(),
                });
            }
        }
    }
    out
}

pub fn build_candidate(
    entity: &Entity,
    compiled: &CompiledRules,
    aliases_enabled: bool,
    descriptions_enabled: bool,
) -> Option<StreamingCandidate> {
    if !aliases_enabled && !descriptions_enabled {
        return None;
    }

    let aliases = if aliases_enabled {
        let skip = compiled
            .skip_qids_for("long_aliases")
            .is_some_and(|s| s.contains(&entity.id));
        let p31_qualifies = entity
            .first_p31_id()
            .is_some_and(|id| !compiled.excluded_p31_for_long_aliases.contains(id));
        if skip || !p31_qualifies {
            None
        } else {
            Some(collect_english_aliases(entity))
        }
    } else {
        None
    };

    let descriptions = if descriptions_enabled {
        let skip = compiled
            .skip_qids_for("long_descriptions")
            .is_some_and(|s| s.contains(&entity.id));
        if skip {
            None
        } else {
            Some(collect_english_descriptions(entity))
        }
    } else {
        None
    };

    if aliases.is_none() && descriptions.is_none() {
        return None;
    }

    Some(StreamingCandidate {
        qid: entity.id.clone(),
        aliases,
        descriptions,
    })
}

pub fn apply(candidate: &StreamingCandidate, state: &mut StreamingState, out: &mut Vec<Issue>) {
    if let Some(aliases) = &candidate.aliases {
        for em in aliases {
            if em.char_len > state.aliases_max {
                state.aliases_max = em.char_len;
                out.push(Issue {
                    qid: candidate.qid.clone(),
                    lang: em.lang.clone(),
                    field: Field::Alias,
                    check: "aliases.long".to_string(),
                    value: em.value.clone(),
                    suggestion: None,
                    details: Some(Details::NewMaxLen {
                        new_max_len: em.char_len as u64,
                    }),
                });
            }
        }
    }
    if let Some(descriptions) = &candidate.descriptions {
        for em in descriptions {
            if em.char_len > state.descriptions_max {
                state.descriptions_max = em.char_len;
                out.push(Issue {
                    qid: candidate.qid.clone(),
                    lang: em.lang.clone(),
                    field: Field::Description,
                    check: "descriptions.long".to_string(),
                    value: em.value.clone(),
                    suggestion: None,
                    details: Some(Details::NewMaxLen {
                        new_max_len: em.char_len as u64,
                    }),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lv(lang: &str, value: &str) -> LangValue {
        LangValue {
            lang: lang.into(),
            value: value.into(),
            char_len: value.chars().count(),
        }
    }

    #[test]
    fn apply_emits_only_on_strict_increase_and_updates_max() {
        let candidate = StreamingCandidate {
            qid: "Q1".into(),
            aliases: Some(vec![lv("en", "shorter"), lv("en", "longer one"), lv("en", "tiny")]),
            descriptions: None,
        };
        let mut state = StreamingState {
            aliases_max: 5,
            descriptions_max: 0,
        };
        let mut out = Vec::new();
        apply(&candidate, &mut state, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].value, "shorter");
        assert_eq!(out[1].value, "longer one");
        assert_eq!(state.aliases_max, 10);

        // Second pass: tied length must NOT emit (strict greater-than).
        let candidate2 = StreamingCandidate {
            qid: "Q2".into(),
            aliases: Some(vec![lv("en", "ten chars!")]), // 10 chars, ties existing max
            descriptions: None,
        };
        let mut out = Vec::new();
        apply(&candidate2, &mut state, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn apply_separate_counters_for_aliases_and_descriptions() {
        let candidate = StreamingCandidate {
            qid: "Q1".into(),
            aliases: Some(vec![lv("en", "abcde")]),       // 5
            descriptions: Some(vec![lv("en", "abc")]),    // 3
        };
        let mut state = StreamingState::default();
        let mut out = Vec::new();
        apply(&candidate, &mut state, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(state.aliases_max, 5);
        assert_eq!(state.descriptions_max, 3);
    }

    fn empty_compiled() -> CompiledRules {
        let rules: wd_core::Rules = serde_json::from_str(
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
        .unwrap();
        CompiledRules::compile(&rules).unwrap()
    }

    fn entity_from(json: serde_json::Value) -> Entity {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn build_returns_none_when_neither_check_enabled() {
        let compiled = empty_compiled();
        let entity = entity_from(serde_json::json!({"id": "Q1"}));
        assert!(build_candidate(&entity, &compiled, false, false).is_none());
    }

    #[test]
    fn aliases_require_eligible_p31() {
        let compiled = empty_compiled();
        // No P31 → ineligible for aliases. Enable descriptions too so the
        // candidate isn't elided by the both-None early return.
        let entity = entity_from(serde_json::json!({
            "id": "Q1",
            "descriptions": {"en": {"language":"en","value":"x"}},
            "aliases": {"en": [{"language":"en","value":"x"}]}
        }));
        let c = build_candidate(&entity, &compiled, true, true).unwrap();
        assert!(c.aliases.is_none());
        assert!(c.descriptions.is_some());

        // With P31 → eligible for aliases.
        let entity = entity_from(serde_json::json!({
            "id": "Q1",
            "claims": {"P31": [{"mainsnak":{"datavalue":{"value":{"id":"Q42"}}}}]},
            "aliases": {"en": [{"language":"en","value":"x"}]}
        }));
        let c = build_candidate(&entity, &compiled, true, false).unwrap();
        assert!(c.aliases.is_some());
    }

    #[test]
    fn aliases_excluded_when_p31_in_excluded_set() {
        let mut rules: wd_core::Rules = serde_json::from_str(
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
        .unwrap();
        rules.excluded_p31_for_long_aliases = ["Q42".to_string()].into_iter().collect();
        let compiled = CompiledRules::compile(&rules).unwrap();
        let entity = entity_from(serde_json::json!({
            "id": "Q1",
            "claims": {"P31": [{"mainsnak":{"datavalue":{"value":{"id":"Q42"}}}}]},
            "aliases": {"en": [{"language":"en","value":"x"}]}
        }));
        let c = build_candidate(&entity, &compiled, true, false);
        // Either None (no descriptions either) or aliases is None.
        assert!(c.is_none() || c.unwrap().aliases.is_none());
    }

    #[test]
    fn descriptions_skipped_when_qid_in_skip_set() {
        let mut rules: wd_core::Rules = serde_json::from_str(
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
        .unwrap();
        rules
            .skip_qids
            .insert("long_descriptions".into(), ["Q31".to_string()].into_iter().collect());
        let compiled = CompiledRules::compile(&rules).unwrap();
        let entity = entity_from(serde_json::json!({
            "id": "Q31",
            "descriptions": {"en": {"language":"en","value":"x"}}
        }));
        let c = build_candidate(&entity, &compiled, false, true);
        assert!(c.is_none() || c.unwrap().descriptions.is_none());
    }

    #[test]
    fn collects_only_english_variants() {
        let compiled = empty_compiled();
        let entity = entity_from(serde_json::json!({
            "id": "Q1",
            "descriptions": {
                "en": {"language":"en","value":"hi"},
                "de": {"language":"de","value":"hallo welt"},
                "en-gb": {"language":"en-gb","value":"howdy"}
            }
        }));
        let c = build_candidate(&entity, &compiled, false, true).unwrap();
        let descs = c.descriptions.unwrap();
        assert_eq!(descs.len(), 2);
        // Sorted by lang code: "en" then "en-gb".
        assert_eq!(descs[0].lang, "en");
        assert_eq!(descs[1].lang, "en-gb");
    }
}
