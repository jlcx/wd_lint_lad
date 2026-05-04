use wd_core::{Field, Issue, entity::Entity, script::is_predominantly_non_latin};

use super::CheckCtx;

pub fn wrong_script(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in &entity.labels {
        let Some(script) = ctx.compiled.script_for_lang(lang) else {
            continue;
        };
        if script == "latin" && is_predominantly_non_latin(&mt.value) {
            out.push(Issue {
                qid: entity.id.clone(),
                lang: lang.clone(),
                field: Field::Label,
                check: "label.wrong_script".to_string(),
                value: mt.value.clone(),
                suggestion: None,
                details: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::checks::EnabledChecks;
    use crate::matchers::CompiledRules;
    use wd_core::{Rules, entity::Entity};

    use super::*;

    fn rules_with_latin_en() -> Rules {
        let mut r: Rules = serde_json::from_str(
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
        r.script_policies
            .insert("en".to_string(), "latin".to_string());
        r
    }

    fn run(rules: Rules, entity: serde_json::Value) -> Vec<Issue> {
        let enabled = EnabledChecks::all();
        let compiled = CompiledRules::compile(&rules).unwrap();
        let ctx = CheckCtx {
            compiled: &compiled,
            enabled: &enabled,
        };
        let entity: Entity = serde_json::from_value(entity).unwrap();
        let mut out = Vec::new();
        wrong_script(&entity, &ctx, &mut out);
        out
    }

    #[test]
    fn flags_cjk_label_on_english_field() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"東京都"}}
            }),
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].check, "label.wrong_script");
        assert_eq!(issues[0].field, Field::Label);
    }

    #[test]
    fn does_not_flag_latin_label() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Douglas Adams"}}
            }),
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn does_not_flag_latin_with_accents() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Étienne de Silhouette"}}
            }),
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn subtag_inherits_base_lang_policy() {
        // "en-gb" not explicitly in policies; should fall back to "en"
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en-gb": {"language":"en-gb","value":"東京都"}}
            }),
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn unconfigured_language_not_flagged() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"ja": {"language":"ja","value":"東京都"}}
            }),
        );
        assert!(issues.is_empty(), "no policy for 'ja' — must not fire");
    }
}
