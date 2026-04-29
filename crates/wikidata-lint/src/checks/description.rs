use std::collections::HashSet;

use wd_core::{
    Details, Field, Issue,
    entity::{Entity, MonolingualText},
    is_english, text,
};

use super::CheckCtx;

fn english_descs(entity: &Entity) -> impl Iterator<Item = (&String, &MonolingualText)> {
    entity
        .descriptions
        .iter()
        .filter(|(lang, _)| is_english(lang))
}

/// Per SPEC §"Language handling": for `descriptions[lang]`, look up
/// `labels[lang]`; if absent, fall back to `labels["en"]`; otherwise None.
fn label_for_lang<'a>(entity: &'a Entity, lang: &str) -> Option<&'a str> {
    entity
        .labels
        .get(lang)
        .or_else(|| entity.labels.get("en"))
        .map(|m| m.value.as_str())
}

fn emit(
    out: &mut Vec<Issue>,
    entity: &Entity,
    lang: &str,
    value: &str,
    check: &str,
    suggestion: Option<String>,
    details: Option<Details>,
) {
    out.push(Issue {
        qid: entity.id.clone(),
        lang: lang.to_string(),
        field: Field::Description,
        check: check.to_string(),
        value: value.to_string(),
        suggestion,
        details,
    });
}

// --- Predicates (shared with the composite check) ---

pub(super) fn pred_too_long(value: &str, max: usize) -> bool {
    value.chars().count() > max
}

pub(super) fn pred_starts_capitalized(value: &str) -> bool {
    value.chars().next().is_some_and(|c| c.is_uppercase())
}

pub(super) fn pred_ends_with_punctuation(value: &str, exempt_suffixes: &[String]) -> bool {
    let Some(last) = value.chars().next_back() else {
        return false;
    };
    if !last.is_ascii_punctuation() {
        return false;
    }
    // Exemption: balanced `(`/`)` and the description happens to end at a
    // closing paren — common Wikidata pattern, e.g. "ABC (band)".
    if last == ')' && has_balanced_parens(value) {
        return false;
    }
    // Exemption: trailing token is a multi-period acronym, e.g. "R.O.C.",
    // "U.S.A.", "i.e.", "e.g.". Detected structurally so users don't have
    // to enumerate every possible initialism in the suffix list.
    if last == '.' && trailing_token_is_acronym(value) {
        return false;
    }
    // Exemption: trailing ellipsis ("..."). Sometimes a truncation marker,
    // sometimes part of a name (e.g. the band "In the Woods..."). The
    // Unicode ellipsis "…" (U+2026) is already exempt because it's not
    // ASCII punctuation.
    if last == '.' && ends_with_ascii_ellipsis(value) {
        return false;
    }
    // Exemption: configured literal end-of-description suffixes,
    // e.g. "Inc.", "Ltd.", honorifics like "Jr.".
    if exempt_suffixes.iter().any(|s| value.ends_with(s.as_str())) {
        return false;
    }
    true
}

fn has_balanced_parens(s: &str) -> bool {
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn trailing_token_is_acronym(value: &str) -> bool {
    value
        .split_whitespace()
        .next_back()
        .is_some_and(is_acronym_token)
}

/// True when the value ends with three or more consecutive ASCII
/// periods — the classic ellipsis pattern, used as a truncation marker
/// or as part of a name like the band "In the Woods..."
fn ends_with_ascii_ellipsis(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[bytes.len() - 1] == b'.'
        && bytes[bytes.len() - 2] == b'.'
        && bytes[bytes.len() - 3] == b'.'
}

/// Matches strings of the form `(<ascii-letter>.)+` with at least 2
/// letter+period pairs — i.e., classic dotted initialisms.
fn is_acronym_token(token: &str) -> bool {
    let bytes = token.as_bytes();
    if bytes.len() < 4 {
        // Need at least "X.Y." (2 pairs).
        return false;
    }
    let mut i = 0;
    let mut pairs = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            return false;
        }
        i += 1;
        if i >= bytes.len() || bytes[i] != b'.' {
            return false;
        }
        i += 1;
        pairs += 1;
    }
    pairs >= 2
}

pub(super) fn pred_bad_start(value: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| value.starts_with(p.as_str()))
}

// --- Per-check functions ---

pub fn too_long(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let max = ctx.compiled.thresholds.description_max_len;
    for (lang, mt) in english_descs(entity) {
        if pred_too_long(&mt.value, max) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.too_long",
                None,
                None,
            );
        }
    }
}

pub fn starts_with_label(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if let Some(label) = label_for_lang(entity, lang)
            && label_match_at_boundary(&mt.value, label).is_some()
        {
            // Pre-compute the canonical fix here so the fixer (which only
            // sees Issue records, not the entity) can apply it without
            // re-finding the label.
            let suggestion =
                compute_starts_with_label_fix(&mt.value, label, &ctx.compiled.nationalities);
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.starts_with_label",
                suggestion,
                None,
            );
        }
    }
}

/// Returns the slice of `value` after `label` *only when* `label` is a
/// non-empty literal prefix and the character immediately after the
/// label is a word boundary (whitespace, ASCII punctuation, or end of
/// string).
///
/// This avoids orthographic false positives like "Korean family name"
/// matching a label of "Ko".
fn label_match_at_boundary<'a>(value: &'a str, label: &str) -> Option<&'a str> {
    if label.is_empty() {
        return None;
    }
    let after = value.strip_prefix(label)?;
    let is_boundary = match after.chars().next() {
        None => true,
        Some(c) => c.is_whitespace() || c.is_ascii_punctuation(),
    };
    is_boundary.then_some(after)
}

/// Per SPEC §"description.starts_with_label" (fixer): strip the leading
/// label, then any leading copular ("is a", "is an", "was a", "was an",
/// "are", "were") that's followed by whitespace, then trim, then
/// lowercase the first character — *unless* the first word is a proper
/// adjective from the configured nationalities/proper-adjectives set,
/// in which case its capitalization is preserved (so "is a Guinean-born
/// musician" becomes "Guinean-born musician", not "guinean-born…").
///
/// Also strips leading separator punctuation (`,`/`;`/`:`/`-`/`–`/`—`)
/// and surrounding whitespace immediately after the label, so a
/// description shaped like `"Label, the rest"` doesn't leave `", the
/// rest"` behind.
///
/// Returns `None` ("would_blank") when the result is empty.
fn compute_starts_with_label_fix(
    value: &str,
    label: &str,
    proper_adjectives: &HashSet<String>,
) -> Option<String> {
    let after_label = label_match_at_boundary(value, label)?;
    let after_label = after_label.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '–' | '—')
    });
    let after_copular = strip_copular_prefix(after_label);
    let trimmed = after_copular.trim();
    if trimmed.is_empty() {
        return None;
    }
    if first_token_is_proper_adjective(trimmed, proper_adjectives) {
        Some(trimmed.to_string())
    } else {
        Some(text::lowerfirst(trimmed))
    }
}

/// Returns true when the first whitespace-bounded token of `s` —
/// matched in either its full lowercase form or its first hyphen-half —
/// is in `proper_adjectives`. This lets the fix preserve the original
/// case of proper adjectives like `Guinean`, `Cambodian-born`, etc.
fn first_token_is_proper_adjective(s: &str, proper_adjectives: &HashSet<String>) -> bool {
    let Some(first) = s.split_whitespace().next() else {
        return false;
    };
    let lower = first.to_lowercase();
    if proper_adjectives.contains(&lower) {
        return true;
    }
    if let Some((head, _tail)) = lower.split_once('-')
        && proper_adjectives.contains(head)
    {
        return true;
    }
    false
}

fn strip_copular_prefix(s: &str) -> &str {
    const COPULARS: &[&str] = &["is an", "was an", "is a", "was a", "were", "are"];
    for cop in COPULARS {
        if let Some(prefix) = s.get(..cop.len())
            && prefix.eq_ignore_ascii_case(cop)
            && s[cop.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace())
        {
            return &s[cop.len()..];
        }
    }
    s
}

pub fn starts_capitalized(entity: &Entity, _ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if pred_starts_capitalized(&mt.value) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.starts_capitalized",
                None,
                None,
            );
        }
    }
}

pub fn ends_with_punctuation(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let exempts = &ctx.compiled.ends_with_punctuation_exempt_suffixes;
    for (lang, mt) in english_descs(entity) {
        if pred_ends_with_punctuation(&mt.value, exempts) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.ends_with_punctuation",
                None,
                None,
            );
        }
    }
}

pub fn contains_trademark(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let chars = &ctx.compiled.trademark_chars;
    if chars.is_empty() {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        if chars.iter().any(|c| mt.value.contains(c.as_str())) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.contains_trademark",
                None,
                None,
            );
        }
    }
}

pub fn contains_html_entity(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if ctx.compiled.html_entity_substrings.is_match(&mt.value) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.contains_html_entity",
                None,
                None,
            );
        }
    }
}

pub fn contains_double_space(entity: &Entity, _ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if mt.value.contains("  ") {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.contains_double_space",
                None,
                None,
            );
        }
    }
}

pub fn contains_obituary(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if ctx.compiled.obituary_markers.is_match(&mt.value) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.contains_obituary",
                None,
                None,
            );
        }
    }
}

pub fn space_before_comma(entity: &Entity, _ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if mt.value.contains(" ,") {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.space_before_comma",
                None,
                None,
            );
        }
    }
}

pub fn bad_start(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let prefixes = &ctx.compiled.bad_starts_descriptions;
    if prefixes.is_empty() {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        if pred_bad_start(&mt.value, prefixes) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.bad_start",
                None,
                None,
            );
        }
    }
}

pub fn marketing_imperative(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    for (lang, mt) in english_descs(entity) {
        if ctx.compiled.marketing_imperatives.is_match(&mt.value) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.marketing_imperative",
                None,
                None,
            );
        }
    }
}

pub fn promotional(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    if ctx
        .compiled
        .skip_qids_for("promotional")
        .is_some_and(|s| s.contains(&entity.id))
    {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        if !ctx.compiled.promotional_substrings.is_match(&mt.value) {
            continue;
        }
        let lower = mt.value.to_lowercase();
        if ctx.compiled.promotional_exempt_lower.is_match(&lower) {
            continue;
        }
        emit(
            out,
            entity,
            lang,
            &mt.value,
            "description.promotional",
            None,
            None,
        );
    }
}

pub fn multi_sentence(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    if entity.id.starts_with('P') {
        return;
    }
    if ctx
        .compiled
        .skip_qids_for("multi_sentence")
        .is_some_and(|s| s.contains(&entity.id))
    {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        if ctx.compiled.multi_sentence_markers.is_match(&mt.value) {
            emit(
                out,
                entity,
                lang,
                &mt.value,
                "description.multi_sentence",
                None,
                None,
            );
        }
    }
}

pub fn misspelled(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let map = &ctx.compiled.misspellings;
    if map.is_empty() {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        let value = &mt.value;
        let mut matched = false;
        let mut suggestion = String::with_capacity(value.len());
        for (is_ws, seg) in text::whitespace_segments(value) {
            if is_ws {
                suggestion.push_str(seg);
                continue;
            }
            // Lookup order per SPEC: literal, lowercase, capfirst.
            // Replacement preserves the matched form.
            if let Some(v) = map.get(seg) {
                suggestion.push_str(v);
                matched = true;
                continue;
            }
            let lower = seg.to_lowercase();
            if let Some(v) = map.get(&lower) {
                suggestion.push_str(&v.to_lowercase());
                matched = true;
                continue;
            }
            let cap = text::capfirst(seg);
            if let Some(v) = map.get(&cap) {
                suggestion.push_str(&text::capfirst(v));
                matched = true;
                continue;
            }
            suggestion.push_str(seg);
        }
        if matched {
            emit(
                out,
                entity,
                lang,
                value,
                "description.misspelled",
                Some(suggestion),
                None,
            );
        }
    }
}

pub fn starts_with_lowercase_nationality(
    entity: &Entity,
    ctx: &CheckCtx<'_>,
    out: &mut Vec<Issue>,
) {
    if ctx.compiled.nationalities.is_empty() {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        let value = &mt.value;
        if let Some(first) = value.split_whitespace().next()
            && ctx.compiled.nationalities.contains(first)
        {
            let suggestion = text::capfirst(value);
            emit(
                out,
                entity,
                lang,
                value,
                "description.starts_with_lowercase_nationality",
                Some(suggestion),
                None,
            );
        }
    }
}

pub fn contains_lowercase_nationality(
    entity: &Entity,
    ctx: &CheckCtx<'_>,
    out: &mut Vec<Issue>,
) {
    let nat = &ctx.compiled.nationalities;
    if nat.is_empty() {
        return;
    }
    for (lang, mt) in english_descs(entity) {
        let value = &mt.value;
        let mut tokens = value.split_whitespace();
        let _first = tokens.next();
        let mut hit = false;
        for token in tokens {
            if nat.contains(token) {
                hit = true;
                break;
            }
            // Per SPEC: also split a single-hyphen token and check halves.
            let parts: Vec<&str> = token.split('-').collect();
            if parts.len() == 2 && (nat.contains(parts[0]) || nat.contains(parts[1])) {
                hit = true;
                break;
            }
        }
        if hit {
            emit(
                out,
                entity,
                lang,
                value,
                "description.contains_lowercase_nationality",
                None,
                None,
            );
        }
    }
}

pub fn composite(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let max = ctx.compiled.thresholds.description_max_len;
    let threshold = ctx.compiled.thresholds.descgust_score_threshold;
    for (lang, mt) in english_descs(entity) {
        let value = &mt.value;
        let mut score: u32 = 0;
        let mut details: Vec<String> = Vec::new();

        if pred_starts_capitalized(value) {
            score += 1;
            details.push("description.starts_capitalized".into());
        }
        if pred_too_long(value, max) {
            score += 1;
            details.push("description.too_long".into());
        }
        if let Some(label) = label_for_lang(entity, lang)
            && label_match_at_boundary(value, label).is_some()
        {
            score += 1;
            details.push("description.starts_with_label".into());
        }
        if pred_ends_with_punctuation(value, &ctx.compiled.ends_with_punctuation_exempt_suffixes) {
            score += 1;
            details.push("description.ends_with_punctuation".into());
        }
        // Trademark: each char counts separately for score, listed once in details.
        let trademark_hits = ctx
            .compiled
            .trademark_chars
            .iter()
            .filter(|c| value.contains(c.as_str()))
            .count() as u32;
        if trademark_hits > 0 {
            score += trademark_hits;
            details.push("description.contains_trademark".into());
        }
        if pred_bad_start(value, &ctx.compiled.bad_starts_descriptions) {
            score += 1;
            details.push("description.bad_start".into());
        }
        if value.contains("  ") {
            score += 1;
            details.push("description.contains_double_space".into());
        }
        if ctx.compiled.obituary_markers.is_match(value) {
            score += 1;
            details.push("description.contains_obituary".into());
        }
        // Composite-specific HTML-entity check: only `&amp;` per SPEC.
        if value.contains("&amp;") {
            score += 1;
            details.push("description.contains_html_entity".into());
        }
        if value.contains(" ,") {
            score += 1;
            details.push("description.space_before_comma".into());
        }

        if score >= threshold {
            out.push(Issue {
                qid: entity.id.clone(),
                lang: lang.clone(),
                field: Field::Description,
                check: "description.composite".to_string(),
                value: value.clone(),
                suggestion: None,
                details: Some(Details::Composite(details)),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::EnabledChecks;
    use crate::matchers::CompiledRules;
    use wd_core::Rules;

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

    fn entity_from_value(v: serde_json::Value) -> Entity {
        serde_json::from_value(v).unwrap()
    }

    fn run<F>(rules: Rules, entity: serde_json::Value, f: F) -> Vec<Issue>
    where
        F: Fn(&Entity, &CheckCtx<'_>, &mut Vec<Issue>),
    {
        let enabled = EnabledChecks::all();
        let compiled = CompiledRules::compile(&rules).unwrap();
        let ctx = CheckCtx {
            compiled: &compiled,
            enabled: &enabled,
        };
        let entity = entity_from_value(entity);
        let mut out = Vec::new();
        f(&entity, &ctx, &mut out);
        out
    }

    #[test]
    fn starts_with_label_suggestion_strips_label_copular_and_lowerfirsts() {
        let nat: HashSet<String> = HashSet::new();
        let cases = &[
            // (value, label, expected_suggestion)
            ("Foo is a thing", "Foo", Some("thing")),
            ("Foo IS A book", "Foo", Some("book")),
            ("Foo was an author", "Foo", Some("author")),
            ("Foo bar", "Foo", Some("bar")),
            ("Foo Bar", "Foo", Some("bar")),
            ("Foo", "Foo", None), // would-blank
            ("Foo are runners", "Foo", Some("runners")),
            // "is a" requires trailing whitespace; "alive" doesn't qualify, so nothing is stripped.
            ("Foo is alive", "Foo", Some("is alive")),
        ];
        for (value, label, expected) in cases {
            let got = compute_starts_with_label_fix(value, label, &nat);
            assert_eq!(
                got.as_deref(),
                *expected,
                "value={value:?} label={label:?}",
            );
        }
    }

    #[test]
    fn starts_with_label_requires_word_boundary_after_label() {
        // The label "Ko" is an orthographic prefix of "Korean" but
        // "Korean" doesn't have a word boundary after the "Ko". The
        // check must NOT fire.
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Ko"}},
                "descriptions": {"en": {"language":"en","value":"Korean family name"}}
            }),
            super::starts_with_label,
        );
        assert!(issues.is_empty(), "no boundary after 'Ko' — must not fire");

        // With a space boundary, the check fires.
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Ko"}},
                "descriptions": {"en": {"language":"en","value":"Ko Korean family name"}}
            }),
            super::starts_with_label,
        );
        assert_eq!(issues.len(), 1);

        // With a punctuation boundary, the check fires.
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Ko"}},
                "descriptions": {"en": {"language":"en","value":"Ko, Korean family name"}}
            }),
            super::starts_with_label,
        );
        assert_eq!(issues.len(), 1);

        // Description equals label exactly — end-of-string also counts as a boundary.
        // (Suggestion will be `None` since stripping leaves nothing — would-blank.)
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Ko"}},
                "descriptions": {"en": {"language":"en","value":"Ko"}}
            }),
            super::starts_with_label,
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].suggestion.is_none());
    }

    #[test]
    fn starts_with_label_preserves_case_for_proper_adjective_first_word() {
        let nat: HashSet<String> = ["guinean".into(), "cambodian".into()]
            .into_iter()
            .collect();
        let cases = &[
            // First word in nat → preserve.
            ("Foo is a Cambodian writer", "Foo", Some("Cambodian writer")),
            // Hyphen-half match → preserve.
            ("Foo, is a Guinean-born musician", "Foo", Some("Guinean-born musician")),
            // Not in nat → lowerfirst (existing behavior).
            ("Foo is a Doctor", "Foo", Some("doctor")),
            ("Foo is a teacher", "Foo", Some("teacher")),
        ];
        for (value, label, expected) in cases {
            let got = compute_starts_with_label_fix(value, label, &nat);
            assert_eq!(
                got.as_deref(),
                *expected,
                "value={value:?} label={label:?}",
            );
        }
    }

    #[test]
    fn starts_with_label_strips_leading_separator_punctuation() {
        let nat: HashSet<String> = HashSet::new();
        let cases = &[
            // (value, label, expected_suggestion)
            ("Foo, famous writer", "Foo", Some("famous writer")),
            ("Foo; the next thing", "Foo", Some("the next thing")),
            ("Foo: composer", "Foo", Some("composer")),
            ("Foo - artist", "Foo", Some("artist")),
            ("Foo – author", "Foo", Some("author")),    // en-dash
            ("Foo — author", "Foo", Some("author")),    // em-dash
            ("Foo,is a thing", "Foo", Some("thing")),    // comma + copular together
            // Empty proper-adjective set → first-word lowerfirst always applies.
            ("Foo, is a Guinean-born guitarist", "Foo", Some("guinean-born guitarist")),
        ];
        for (value, label, expected) in cases {
            let got = compute_starts_with_label_fix(value, label, &nat);
            assert_eq!(got.as_deref(), *expected, "value={value:?} label={label:?}");
        }
    }

    #[test]
    fn starts_with_label_emits_suggestion() {
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Foo"}},
                "descriptions": {"en": {"language":"en","value":"Foo is a thing"}}
            }),
            super::starts_with_label,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].suggestion.as_deref(), Some("thing"));
    }

    #[test]
    fn starts_with_label_uses_per_lang_then_falls_back_to_en() {
        let rules = empty_rules();
        // Per-lang label match
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en-gb": {"language":"en-gb","value":"Foo"}},
                "descriptions": {"en-gb": {"language":"en-gb","value":"Foo bar baz"}}
            }),
            super::starts_with_label,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].lang, "en-gb");

        // Fallback to en label
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "labels": {"en": {"language":"en","value":"Foo"}},
                "descriptions": {"en-us": {"language":"en-us","value":"Foo bar baz"}}
            }),
            super::starts_with_label,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].lang, "en-us");

        // No label at all
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Foo bar baz"}}
            }),
            super::starts_with_label,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn starts_capitalized_uses_unicode_uppercase() {
        let rules = empty_rules();
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Über alles"}}
            }),
            super::starts_capitalized,
        );
        assert_eq!(issues.len(), 1);

        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"über alles"}}
            }),
            super::starts_capitalized,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn ends_with_punctuation_exempts_balanced_parens() {
        // Balanced parens — exempt.
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Acme Corp (band)"}}
            }),
            super::ends_with_punctuation,
        );
        assert!(issues.is_empty());

        // Unbalanced — still flagged.
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"foo (a)) extra)"}}
            }),
            super::ends_with_punctuation,
        );
        assert_eq!(issues.len(), 1);

        // Bare trailing close-paren with no opener — flagged.
        let issues = run(
            empty_rules(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"foo)"}}
            }),
            super::ends_with_punctuation,
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn ends_with_punctuation_exempts_trailing_ellipsis() {
        let cases = &[
            // (description, expected_issue_count)
            ("In the Woods...", 0),       // band name with trailing ellipsis
            ("foo bar baz...", 0),        // truncation marker, 3 dots
            ("foo....", 0),               // 4 dots also exempt
            ("foo..", 1),                 // 2 dots — typo, still flags
            ("foo.", 1),                  // single period, still flags
        ];
        for (value, expected) in cases {
            let issues = run(
                empty_rules(),
                serde_json::json!({
                    "id": "Q1",
                    "descriptions": {"en": {"language":"en","value": value}}
                }),
                super::ends_with_punctuation,
            );
            assert_eq!(
                issues.len(),
                *expected,
                "value={value:?} expected {expected} got {}",
                issues.len()
            );
        }
    }

    #[test]
    fn ends_with_punctuation_exempts_dotted_acronyms() {
        let cases = &[
            // (description, expected_issue_count)
            ("government of R.O.C.", 0),       // 3-pair acronym
            ("ambassador to U.S.A.", 0),       // 3-pair
            ("president of U.S.", 0),          // 2-pair
            ("see e.g.", 0),                   // 2-pair lowercase
            ("known as a.k.a.", 0),            // 3-pair lowercase
            ("the USA.", 1),                   // single trailing period, no internal periods → still flagged
            ("ends with U.", 1),               // single pair only → not enough, flagged
            ("12.5.", 1),                      // digits, not letters → flagged
            ("Foo R.O.C", 0),                  // doesn't end with '.' so check doesn't fire at all
        ];
        for (value, expected) in cases {
            let issues = run(
                empty_rules(),
                serde_json::json!({
                    "id": "Q1",
                    "descriptions": {"en": {"language":"en","value": value}}
                }),
                super::ends_with_punctuation,
            );
            assert_eq!(
                issues.len(),
                *expected,
                "value={value:?} expected {expected} got {}",
                issues.len()
            );
        }
    }

    #[test]
    fn ends_with_punctuation_exempts_configured_suffixes() {
        let mut rules = empty_rules();
        rules.ends_with_punctuation_exempt_suffixes = vec!["Inc.".into(), "Jr.".into()];

        // "Inc." is in the list — exempt.
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Acme Inc."}}
            }),
            super::ends_with_punctuation,
        );
        assert!(issues.is_empty());

        // Suffix list is case-sensitive — "INC." does NOT match "Inc.".
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"ACME INC."}}
            }),
            super::ends_with_punctuation,
        );
        assert_eq!(issues.len(), 1);

        // Suffix not in list — still flagged.
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Acme Co."}}
            }),
            super::ends_with_punctuation,
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn ends_with_punctuation_strict_ascii() {
        let rules = empty_rules();
        for s in ["foo.", "foo!", "foo?", "foo,"] {
            let issues = run(
                rules.clone(),
                serde_json::json!({
                    "id": "Q1",
                    "descriptions": {"en": {"language":"en","value": s }}
                }),
                super::ends_with_punctuation,
            );
            assert_eq!(issues.len(), 1, "expected hit on `{s}`");
        }
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"foo"}}
            }),
            super::ends_with_punctuation,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn promotional_respects_exempt_and_skip_qids() {
        let mut rules = empty_rules();
        rules.promotional_substrings = vec!["the best".into()];
        rules.promotional_exempt_substrings = vec!["award".into()];
        rules
            .skip_qids
            .insert("promotional".into(), ["Q42".to_string()].into_iter().collect());

        // Plain match
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"the best taco"}}
            }),
            super::promotional,
        );
        assert_eq!(issues.len(), 1);

        // Exempt by case-insensitive substring
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"the best AWARD-winner"}}
            }),
            super::promotional,
        );
        assert!(issues.is_empty());

        // Skip QID
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q42",
                "descriptions": {"en": {"language":"en","value":"the best taco"}}
            }),
            super::promotional,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn multi_sentence_skips_properties_and_skip_qids() {
        let mut rules = empty_rules();
        rules.multi_sentence_markers = vec![". The".into()];
        rules
            .skip_qids
            .insert("multi_sentence".into(), ["Q7".to_string()].into_iter().collect());

        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"x. The next sentence"}}
            }),
            super::multi_sentence,
        );
        assert_eq!(issues.len(), 1);

        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "P31",
                "descriptions": {"en": {"language":"en","value":"x. The next sentence"}}
            }),
            super::multi_sentence,
        );
        assert!(issues.is_empty(), "P-prefix entity must be skipped");

        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q7",
                "descriptions": {"en": {"language":"en","value":"x. The next sentence"}}
            }),
            super::multi_sentence,
        );
        assert!(issues.is_empty(), "skip_qids hit must be skipped");
    }

    #[test]
    fn misspelled_picks_form_and_emits_full_corrected_value() {
        let mut rules = empty_rules();
        rules.misspellings.insert("abandonned".into(), "abandoned".into());

        // Exact match
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"the abandonned ship"}}
            }),
            super::misspelled,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].suggestion.as_deref(), Some("the abandoned ship"));

        // Lowercase form match (token "ABANDONNED" → use lowercase value)
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"the ABANDONNED ship"}}
            }),
            super::misspelled,
        );
        assert_eq!(issues[0].suggestion.as_deref(), Some("the abandoned ship"));

        // No misspelling → no issue
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"the abandoned ship"}}
            }),
            super::misspelled,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn misspelled_capfirst_form_capitalizes_value() {
        let mut rules = empty_rules();
        rules.misspellings.insert("Abandonned".into(), "Abandoned".into());

        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Abandonned ship"}}
            }),
            super::misspelled,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].suggestion.as_deref(), Some("Abandoned ship"));
    }

    #[test]
    fn starts_with_lowercase_nationality_capitalizes_first_char() {
        let mut rules = empty_rules();
        rules.nationalities_lower = vec!["palestinian".into()];
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"palestinian writer"}}
            }),
            super::starts_with_lowercase_nationality,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].suggestion.as_deref(), Some("Palestinian writer"));
    }

    #[test]
    fn contains_lowercase_nationality_matches_hyphen_halves() {
        let mut rules = empty_rules();
        rules.nationalities_lower = vec!["irish".into()];

        // Plain second-token match
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"writer who is irish"}}
            }),
            super::contains_lowercase_nationality,
        );
        assert_eq!(issues.len(), 1);

        // Hyphen-split match
        let issues = run(
            rules.clone(),
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"writer of anglo-irish descent"}}
            }),
            super::contains_lowercase_nationality,
        );
        assert_eq!(issues.len(), 1);

        // First token — must NOT trigger this check
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"irish writer"}}
            }),
            super::contains_lowercase_nationality,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn composite_emits_when_score_meets_threshold_and_records_subchecks() {
        let mut rules = empty_rules();
        // threshold 4: build a description that hits exactly 4 components.
        rules.thresholds.description_max_len = 5;
        rules.thresholds.descgust_score_threshold = 4;

        // "Foo  bar." — starts capitalized (+1), too long (+1, len=9>5),
        // ends with punctuation (+1), double space (+1) → score 4.
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"Foo  bar."}}
            }),
            super::composite,
        );
        assert_eq!(issues.len(), 1);
        let d = match &issues[0].details {
            Some(Details::Composite(v)) => v,
            other => panic!("expected composite details, got {other:?}"),
        };
        assert!(d.contains(&"description.starts_capitalized".to_string()));
        assert!(d.contains(&"description.too_long".to_string()));
        assert!(d.contains(&"description.ends_with_punctuation".to_string()));
        assert!(d.contains(&"description.contains_double_space".to_string()));
    }

    #[test]
    fn composite_below_threshold_does_not_emit() {
        let rules = empty_rules(); // threshold 4
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"a normal short description"}}
            }),
            super::composite,
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn composite_counts_each_trademark_char_separately() {
        let mut rules = empty_rules();
        rules.trademark_chars = vec!["®".into(), "™".into()];
        rules.thresholds.descgust_score_threshold = 2;

        // Description with both ® and ™ should hit threshold from trademark alone.
        let issues = run(
            rules,
            serde_json::json!({
                "id": "Q1",
                "descriptions": {"en": {"language":"en","value":"foo® bar™"}}
            }),
            super::composite,
        );
        assert_eq!(issues.len(), 1);
    }
}
