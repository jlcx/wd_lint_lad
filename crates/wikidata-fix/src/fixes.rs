//! Per-check fix functions.
//!
//! Each fix takes the current `working` value (state of the cell so
//! far in stage-1 coalescing) and returns a `FixOutcome`. Detection-only
//! checks return `DetectionOnly`; fixes that can't be applied (e.g.
//! ends_with_punctuation on a non-period suffix) return `Skipped`.

use std::collections::HashSet;

use wd_core::{Details, Issue, text};

/// Per SPEC §[fixer] — checks for which a canonical fix is defined.
pub const FIXABLE_CHECKS: &[&str] = &[
    "description.misspelled",
    "description.starts_with_lowercase_nationality",
    "description.contains_lowercase_nationality",
    "description.contains_html_entity",
    "description.contains_double_space",
    "description.space_before_comma",
    "description.contains_trademark",
    "description.ends_with_punctuation",
    "description.starts_with_label",
    "description.bad_start",
    "description.composite",
];

pub fn is_fixable(check_id: &str) -> bool {
    FIXABLE_CHECKS.contains(&check_id)
}

/// Per SPEC §"Safety": maximum length for labels and aliases.
pub const LABEL_ALIAS_MAX_LEN: usize = 250;

#[derive(Debug)]
pub enum FixOutcome {
    Applied(String),
    DetectionOnly,
    Skipped(SkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    PartialHtml,
    NonperiodPunct,
    WouldBlank,
    CompositePartial,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::PartialHtml => "partial_html",
            SkipReason::NonperiodPunct => "nonperiod_punct",
            SkipReason::WouldBlank => "would_blank",
            SkipReason::CompositePartial => "composite_partial",
        }
    }
}

pub struct FixCtx {
    pub nationalities: HashSet<String>,
    pub trademark_chars: Vec<String>,
    /// Sorted by length descending so longest match wins on prefix scan.
    pub bad_start_strip_prefixes: Vec<String>,
}

/// Context for post-fix validation: the result of stage-1 coalescing
/// is checked against these guideline rules before it's emitted. If
/// any rule fires, the group is routed to the unfixable report rather
/// than producing a half-fixed CSV row.
pub struct PostFixCtx {
    pub bad_starts: Vec<String>,
}

/// Returns `Some(reason)` when the post-fix value still violates a
/// description guideline that the fix didn't (or couldn't) address;
/// `None` when the value is clean.
pub fn post_fix_violation(
    value: &str,
    field: wd_core::Field,
    ctx: &PostFixCtx,
) -> Option<&'static str> {
    if !matches!(field, wd_core::Field::Description) {
        return None;
    }
    if ctx
        .bad_starts
        .iter()
        .any(|p| value.starts_with(p.as_str()))
    {
        return Some("post_fix_bad_start");
    }
    None
}

pub fn apply(check_id: &str, issue: &Issue, working: &str, ctx: &FixCtx) -> FixOutcome {
    match check_id {
        "description.misspelled" => match issue.suggestion.as_deref() {
            Some(s) => FixOutcome::Applied(s.to_string()),
            None => FixOutcome::Skipped(SkipReason::WouldBlank),
        },
        "description.starts_with_label" => match issue.suggestion.as_deref() {
            Some(s) => FixOutcome::Applied(s.to_string()),
            None => FixOutcome::Skipped(SkipReason::WouldBlank),
        },
        "description.starts_with_lowercase_nationality" => {
            FixOutcome::Applied(text::capfirst(working))
        }
        "description.contains_lowercase_nationality" => FixOutcome::Applied(
            fix_contains_lowercase_nationality(working, &ctx.nationalities),
        ),
        "description.contains_html_entity" => match decode_html_entities(working) {
            Some(s) => FixOutcome::Applied(s),
            None => FixOutcome::Skipped(SkipReason::PartialHtml),
        },
        "description.contains_double_space" => FixOutcome::Applied(collapse_double_spaces(working)),
        "description.space_before_comma" => FixOutcome::Applied(working.replace(" ,", ",")),
        "description.contains_trademark" => FixOutcome::Applied(
            strip_trademark_chars(working, &ctx.trademark_chars)
                .trim()
                .to_string(),
        ),
        "description.ends_with_punctuation" => match strip_trailing_period(working) {
            Some(s) => FixOutcome::Applied(s),
            None => FixOutcome::Skipped(SkipReason::NonperiodPunct),
        },
        "description.bad_start" => fix_bad_start(working, &ctx.bad_start_strip_prefixes),
        "description.composite" => fix_composite(issue, working, ctx),
        _ => FixOutcome::DetectionOnly,
    }
}

/// Strip a leading copular/safe prefix from the description.
///
/// `strip_prefixes` is expected to be sorted by length descending so
/// longer patterns ("is an ") win against shorter ones ("is a ") when
/// both technically match.
///
/// Idempotent on no-match: if no configured prefix matches the
/// working value (either because a prior fix in the group already
/// stripped it, or the prefix is one we don't safely strip like an
/// article), this returns `Applied(unchanged)`. The post-fix
/// `bad_start` guideline check runs afterward and rejects with
/// `post_fix_bad_start` if the value still has a bad start.
pub fn fix_bad_start(value: &str, strip_prefixes: &[String]) -> FixOutcome {
    for prefix in strip_prefixes {
        if value.starts_with(prefix.as_str()) {
            let after = &value[prefix.len()..];
            let trimmed = after.trim_start();
            if trimmed.is_empty() {
                return FixOutcome::Skipped(SkipReason::WouldBlank);
            }
            // No first-letter casing change: stripping "is a" from
            // "is a Guinean-born ..." should leave "Guinean-born ..."
            // with its proper-adjective capitalization intact.
            return FixOutcome::Applied(trimmed.to_string());
        }
    }
    FixOutcome::Applied(value.to_string())
}

// --- Per-check fix implementations ---

pub fn fix_contains_lowercase_nationality(value: &str, nationalities: &HashSet<String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut token_index = 0usize;
    for (is_ws, seg) in text::whitespace_segments(value) {
        if is_ws {
            out.push_str(seg);
            continue;
        }
        token_index += 1;
        if token_index == 1 {
            // First token is handled by description.starts_with_lowercase_nationality.
            out.push_str(seg);
            continue;
        }
        if nationalities.contains(seg) {
            out.push_str(&text::capfirst(seg));
            continue;
        }
        // Single-hyphen token: capfirst whichever halves match.
        let parts: Vec<&str> = seg.split('-').collect();
        if parts.len() == 2
            && (nationalities.contains(parts[0]) || nationalities.contains(parts[1]))
        {
            let new_a = if nationalities.contains(parts[0]) {
                text::capfirst(parts[0])
            } else {
                parts[0].to_string()
            };
            let new_b = if nationalities.contains(parts[1]) {
                text::capfirst(parts[1])
            } else {
                parts[1].to_string()
            };
            out.push_str(&new_a);
            out.push('-');
            out.push_str(&new_b);
            continue;
        }
        out.push_str(seg);
    }
    out
}

pub fn decode_html_entities(value: &str) -> Option<String> {
    // Two passes: first reject if any unsupported entity is present; then decode.
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            // Look for a closing ';' within a small window — HTML entities are short.
            let mut j = i + 1;
            let limit = (i + 10).min(bytes.len());
            while j < limit {
                if bytes[j] == b';' {
                    let entity = &value[i..=j];
                    if !is_known_entity(entity) {
                        let inner = &value[i + 1..j];
                        if is_entity_inner(inner) {
                            return None;
                        }
                    }
                    break;
                }
                j += 1;
            }
        }
        i += 1;
    }
    Some(
        value
            .replace("&amp;", "&")
            .replace("&#91;", "[")
            .replace("&#93;", "]"),
    )
}

fn is_known_entity(s: &str) -> bool {
    matches!(s, "&amp;" | "&#91;" | "&#93;")
}

fn is_entity_inner(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '#')
}

pub fn collapse_double_spaces(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev_space = false;
    for c in value.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

pub fn strip_trademark_chars(value: &str, chars: &[String]) -> String {
    let mut s = value.to_string();
    for c in chars {
        s = s.replace(c.as_str(), "");
    }
    s
}

/// Returns `Some(stripped)` when the value ends in `.`/`!`/`?`,
/// `Some(unchanged)` when there's no trailing punctuation at all
/// (lenient no-op; used so coalesced fixes after a prior strip don't
/// reject), and `None` only when there's a non-period trailing
/// punctuation that the spec says we won't touch.
pub fn strip_trailing_period(value: &str) -> Option<String> {
    let Some(last) = value.chars().next_back() else {
        return Some(value.to_string());
    };
    if matches!(last, '.' | '!' | '?') {
        let mut s = value.to_string();
        s.pop();
        Some(s)
    } else if last.is_ascii_punctuation() {
        None
    } else {
        Some(value.to_string())
    }
}

fn fix_composite(issue: &Issue, working: &str, ctx: &FixCtx) -> FixOutcome {
    let details = match &issue.details {
        Some(Details::Composite(v)) => v,
        _ => return FixOutcome::Skipped(SkipReason::CompositePartial),
    };
    let mut current = working.to_string();
    for sub_id in details {
        match apply_subfix(sub_id, &current, ctx) {
            FixOutcome::Applied(next) => current = next,
            FixOutcome::DetectionOnly | FixOutcome::Skipped(_) => {
                return FixOutcome::Skipped(SkipReason::CompositePartial);
            }
        }
    }
    FixOutcome::Applied(current)
}

fn apply_subfix(check_id: &str, working: &str, ctx: &FixCtx) -> FixOutcome {
    match check_id {
        "description.contains_trademark" => FixOutcome::Applied(
            strip_trademark_chars(working, &ctx.trademark_chars)
                .trim()
                .to_string(),
        ),
        "description.contains_double_space" => FixOutcome::Applied(collapse_double_spaces(working)),
        "description.contains_html_entity" => match decode_html_entities(working) {
            Some(s) => FixOutcome::Applied(s),
            None => FixOutcome::Skipped(SkipReason::PartialHtml),
        },
        "description.space_before_comma" => FixOutcome::Applied(working.replace(" ,", ",")),
        "description.ends_with_punctuation" => match strip_trailing_period(working) {
            Some(s) => FixOutcome::Applied(s),
            None => FixOutcome::Skipped(SkipReason::NonperiodPunct),
        },
        "description.bad_start" => fix_bad_start(working, &ctx.bad_start_strip_prefixes),
        // starts_with_label needs the label, which the composite doesn't carry; treat
        // as detection-only at the composite level so the whole record routes to
        // composite_partial.
        _ => FixOutcome::DetectionOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nat(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn collapse_double_spaces_only_collapses_runs_of_2_plus_ascii_spaces() {
        assert_eq!(collapse_double_spaces("a  b"), "a b");
        assert_eq!(collapse_double_spaces("a   b   c"), "a b c");
        assert_eq!(collapse_double_spaces("a b"), "a b"); // single space untouched
        // Other whitespace is left alone.
        assert_eq!(collapse_double_spaces("a\t\tb"), "a\t\tb");
    }

    #[test]
    fn strip_trademark_chars_removes_all_listed_chars_and_trims() {
        let chars = vec!["®".to_string(), "™".to_string()];
        assert_eq!(strip_trademark_chars("foo®bar™", &chars), "foobar");
        assert_eq!(strip_trademark_chars("®foo®", &chars), "foo");
    }

    #[test]
    fn strip_trailing_period_only_for_period_bang_question() {
        assert_eq!(strip_trailing_period("foo."), Some("foo".to_string()));
        assert_eq!(strip_trailing_period("foo!"), Some("foo".to_string()));
        assert_eq!(strip_trailing_period("foo?"), Some("foo".to_string()));
        // Other punctuation → reject (caller routes to unfixable).
        assert_eq!(strip_trailing_period("foo,"), None);
        assert_eq!(strip_trailing_period("foo)"), None);
        // No trailing punctuation → lenient no-op.
        assert_eq!(strip_trailing_period("foo"), Some("foo".to_string()));
        assert_eq!(strip_trailing_period(""), Some(String::new()));
    }

    #[test]
    fn decode_html_entities_decodes_known_and_rejects_unknown() {
        assert_eq!(
            decode_html_entities("AT&amp;T &#91;1&#93;").as_deref(),
            Some("AT&T [1]")
        );
        // Unknown entity → None.
        assert!(decode_html_entities("AT&unknown;T").is_none());
        // Bare '&' with no ';' is left alone.
        assert_eq!(
            decode_html_entities("AT&T no entity").as_deref(),
            Some("AT&T no entity")
        );
    }

    #[test]
    fn space_before_comma_fix_is_simple_replace() {
        // Use the apply() entry point with a contrived issue.
        let issue = Issue {
            qid: "Q1".into(),
            lang: "en".into(),
            field: wd_core::Field::Description,
            check: "description.space_before_comma".into(),
            value: "foo , bar".into(),
            suggestion: None,
            details: None,
        };
        let ctx = FixCtx {
            nationalities: HashSet::new(),
            trademark_chars: vec![],
            bad_start_strip_prefixes: vec![],
        };
        match apply(&issue.check, &issue, &issue.value, &ctx) {
            FixOutcome::Applied(s) => assert_eq!(s, "foo, bar"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn contains_lowercase_nationality_capfirsts_matching_tokens_and_hyphen_halves() {
        let nat = nat(&["irish", "anglo"]);
        assert_eq!(
            fix_contains_lowercase_nationality("writer who is irish", &nat),
            "writer who is Irish"
        );
        assert_eq!(
            fix_contains_lowercase_nationality("writer of anglo-irish descent", &nat),
            "writer of Anglo-Irish descent"
        );
        // First token NOT capitalized by this fix.
        assert_eq!(
            fix_contains_lowercase_nationality("irish writer", &nat),
            "irish writer"
        );
    }

    #[test]
    fn misspelled_uses_scanner_suggestion() {
        let issue = Issue {
            qid: "Q1".into(),
            lang: "en".into(),
            field: wd_core::Field::Description,
            check: "description.misspelled".into(),
            value: "the abandonned ship".into(),
            suggestion: Some("the abandoned ship".into()),
            details: None,
        };
        let ctx = FixCtx {
            nationalities: HashSet::new(),
            trademark_chars: vec![],
            bad_start_strip_prefixes: vec![],
        };
        match apply(&issue.check, &issue, &issue.value, &ctx) {
            FixOutcome::Applied(s) => assert_eq!(s, "the abandoned ship"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn detection_only_checks_return_detection_only() {
        let ctx = FixCtx {
            nationalities: HashSet::new(),
            trademark_chars: vec![],
            bad_start_strip_prefixes: vec![],
        };
        for id in [
            "description.too_long",
            "description.marketing_imperative",
            "description.promotional",
            "description.multi_sentence",
            "description.contains_obituary",
            "description.starts_capitalized",
            "aliases.long",
            "descriptions.long",
        ] {
            let issue = Issue {
                qid: "Q1".into(),
                lang: "en".into(),
                field: wd_core::Field::Description,
                check: id.into(),
                value: "x".into(),
                suggestion: None,
                details: None,
            };
            assert!(matches!(
                apply(id, &issue, &issue.value, &ctx),
                FixOutcome::DetectionOnly
            ));
        }
    }

    #[test]
    fn fix_bad_start_strips_safe_prefix_and_preserves_capitalization() {
        let prefixes = vec!["is a ".to_string()];
        match fix_bad_start("is a thing", &prefixes) {
            FixOutcome::Applied(s) => assert_eq!(s, "thing"),
            other => panic!("unexpected {other:?}"),
        }
        // Proper-adjective capitalization is preserved (no lowerfirst).
        match fix_bad_start("is a Guinean-born Canadian guitarist", &prefixes) {
            FixOutcome::Applied(s) => assert_eq!(s, "Guinean-born Canadian guitarist"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn fix_bad_start_longest_match_wins() {
        // Sorted by length descending so "is an " wins against "is a ".
        let prefixes = vec!["is an ".to_string(), "is a ".to_string()];
        match fix_bad_start("is an apple", &prefixes) {
            FixOutcome::Applied(s) => assert_eq!(s, "apple"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn fix_bad_start_no_match_is_idempotent() {
        // No-strip-prefix match → Applied(unchanged). Post-fix
        // validation handles non-strippable bad starts separately.
        let prefixes = vec!["is a ".to_string()];
        match fix_bad_start("The Beatles", &prefixes) {
            FixOutcome::Applied(s) => assert_eq!(s, "The Beatles"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn fix_bad_start_would_blank_rejects() {
        let prefixes = vec!["is a ".to_string()];
        match fix_bad_start("is a ", &prefixes) {
            FixOutcome::Skipped(SkipReason::WouldBlank) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn composite_applies_each_subfix_in_details_order() {
        let issue = Issue {
            qid: "Q1".into(),
            lang: "en".into(),
            field: wd_core::Field::Description,
            check: "description.composite".into(),
            value: "Foo  bar™ , baz.".into(),
            suggestion: None,
            details: Some(Details::Composite(vec![
                "description.contains_trademark".into(),
                "description.space_before_comma".into(),
                "description.contains_double_space".into(),
                "description.ends_with_punctuation".into(),
            ])),
        };
        let ctx = FixCtx {
            nationalities: HashSet::new(),
            trademark_chars: vec!["™".into()],
            bad_start_strip_prefixes: vec![],
        };
        match apply(&issue.check, &issue, &issue.value, &ctx) {
            FixOutcome::Applied(s) => assert_eq!(s, "Foo bar, baz"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn composite_with_detection_only_subcheck_routes_to_partial() {
        let issue = Issue {
            qid: "Q1".into(),
            lang: "en".into(),
            field: wd_core::Field::Description,
            check: "description.composite".into(),
            value: "X".into(),
            suggestion: None,
            details: Some(Details::Composite(vec![
                "description.too_long".into(), // detection-only
            ])),
        };
        let ctx = FixCtx {
            nationalities: HashSet::new(),
            trademark_chars: vec![],
            bad_start_strip_prefixes: vec![],
        };
        match apply(&issue.check, &issue, &issue.value, &ctx) {
            FixOutcome::Skipped(SkipReason::CompositePartial) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
