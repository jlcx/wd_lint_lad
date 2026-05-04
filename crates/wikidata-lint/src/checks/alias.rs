use wd_core::{Field, Issue, entity::Entity, script::is_predominantly_non_latin};

use super::CheckCtx;

pub fn wrong_script(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, aliases) in &entity.aliases {
        let Some(script) = ctx.compiled.script_for_lang(lang) else {
            continue;
        };
        if script != "latin" {
            continue;
        }
        for alias in aliases {
            if is_predominantly_non_latin(&alias.value) {
                out.push(Issue {
                    qid: entity.id.clone(),
                    lang: lang.clone(),
                    field: Field::Alias,
                    check: "alias.wrong_script".to_string(),
                    value: alias.value.clone(),
                    suggestion: None,
                    details: None,
                });
            }
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
    fn flags_each_non_latin_alias_separately() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "aliases": {"en": [
                    {"language":"en","value":"Douglas Adams"},
                    {"language":"en","value":"東京都"},
                    {"language":"en","value":"Москва"}
                ]}
            }),
        );
        // Only the two non-Latin aliases fire.
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|i| i.field == Field::Alias));
    }

    #[test]
    fn does_not_flag_latin_aliases() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "aliases": {"en": [
                    {"language":"en","value":"D. Adams"},
                    {"language":"en","value":"D.N. Adams"}
                ]}
            }),
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn unconfigured_language_not_flagged() {
        let issues = run(
            rules_with_latin_en(),
            serde_json::json!({
                "id": "Q1",
                "aliases": {"ja": [{"language":"ja","value":"東京都"}]}
            }),
        );
        assert!(issues.is_empty());
    }
}
