//! Two-stage coalescing per SPEC §[fixer] Coalescing.
//!
//! Stage 1: group input issues by `(qid, lang, field)`, apply fixes in
//! input order to a shared working string, validate against safety
//! bounds.
//!
//! Stage 2: assemble cells into one CSV row per `qid` (first-seen
//! order), with columns `qid`, then `L<lang>`/`D<lang>`/`A<lang>`
//! sorted alphabetically.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use wd_core::{Field, Issue};

use crate::fixes::{self, FixCtx, FixOutcome, LABEL_ALIAS_MAX_LEN};

/// Caller-supplied configuration for stage 1.
pub struct ProcessConfig {
    pub fix_ctx: FixCtx,
    pub enabled_checks: HashSet<String>,
    pub description_max_len: usize,
}

#[derive(Debug)]
pub struct ProcessResult {
    /// One entry per emitted cell. Order is the first-seen order of
    /// the underlying `(qid, lang, field)` group, which matches input
    /// order from the scanner.
    pub cells: Vec<CellOut>,
    pub unfixable: Vec<UnfixableEntry>,
    pub suppressed_count: usize,
}

#[derive(Debug, Clone)]
pub struct CellOut {
    pub qid: String,
    pub column: String,
    pub value: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(untagged)]
pub enum UnfixableEntry {
    /// A successfully-parsed issue we declined to fix.
    Parsed {
        #[serde(flatten)]
        issue: Issue,
        reason: String,
    },
    /// A line that failed JSON parsing — passed through verbatim.
    Unparsed { raw: String, reason: String },
}

/// Stage-1 internal state per `(qid, lang, field)` group.
struct GroupState {
    qid: String,
    lang: String,
    field: Field,
    original: String,
    working: String,
    contributing_checks: Vec<String>,
    issues: Vec<Issue>,
    rejected: Option<String>,
}

pub fn process(issues: Vec<Issue>, parse_failures: Vec<String>, config: &ProcessConfig) -> ProcessResult {
    // Stage 1 — group + apply fixes per group, preserving group input order.
    let mut groups: Vec<GroupState> = Vec::new();
    let mut group_index: HashMap<(String, String, Field), usize> = HashMap::new();

    for issue in issues {
        let key = (issue.qid.clone(), issue.lang.clone(), issue.field);
        if let Some(&idx) = group_index.get(&key) {
            apply_one(&mut groups[idx], issue, config);
        } else {
            let mut g = GroupState {
                qid: key.0.clone(),
                lang: key.1.clone(),
                field: key.2,
                original: issue.value.clone(),
                working: issue.value.clone(),
                contributing_checks: Vec::new(),
                issues: Vec::new(),
                rejected: None,
            };
            apply_one(&mut g, issue, config);
            group_index.insert(key, groups.len());
            groups.push(g);
        }
    }

    // Walk groups in first-seen order. For each survivor, emit a CellOut.
    // For each rejected, route its issues to the unfixable report.
    let mut cells: Vec<CellOut> = Vec::new();
    let mut unfixable: Vec<UnfixableEntry> = parse_failures
        .into_iter()
        .map(|raw| UnfixableEntry::Unparsed {
            raw,
            reason: "parse_error".into(),
        })
        .collect();
    let mut suppressed_count = 0usize;

    for mut g in groups {
        if let Some(reason) = g.rejected.take() {
            for issue in g.issues {
                unfixable.push(UnfixableEntry::Parsed {
                    issue,
                    reason: reason.clone(),
                });
            }
            continue;
        }
        // Safety pass.
        if g.working.is_empty() {
            reject_group(&mut unfixable, g, "safety_bounds");
            continue;
        }
        if g.working.chars().any(|c| c.is_control()) {
            reject_group(&mut unfixable, g, "control_chars");
            continue;
        }
        let max = match g.field {
            Field::Description => config.description_max_len,
            Field::Label | Field::Alias => LABEL_ALIAS_MAX_LEN,
        };
        if g.working.chars().count() > max {
            reject_group(&mut unfixable, g, "safety_bounds");
            continue;
        }
        if g.working == g.original {
            // No-op suppression — silent.
            suppressed_count += 1;
            continue;
        }
        cells.push(CellOut {
            qid: g.qid,
            column: column_name(g.field, &g.lang),
            value: g.working,
        });
    }

    ProcessResult {
        cells,
        unfixable,
        suppressed_count,
    }
}

fn apply_one(g: &mut GroupState, issue: Issue, config: &ProcessConfig) {
    if g.rejected.is_some() {
        // Even after rejection, retain the issue so it's reported.
        g.issues.push(issue);
        return;
    }
    let check_id = issue.check.clone();
    if !config.enabled_checks.contains(&check_id) {
        let reason = if fixes::is_fixable(&check_id) {
            "disabled"
        } else {
            "detection_only"
        };
        g.rejected = Some(reason.to_string());
        g.issues.push(issue);
        return;
    }
    let outcome = fixes::apply(&check_id, &issue, &g.working, &config.fix_ctx);
    g.issues.push(issue);
    match outcome {
        FixOutcome::Applied(next) => {
            g.working = next;
            g.contributing_checks.push(check_id);
        }
        FixOutcome::DetectionOnly => {
            g.rejected = Some("detection_only".into());
        }
        FixOutcome::Skipped(reason) => {
            g.rejected = Some(reason.as_str().to_string());
        }
    }
}

fn reject_group(unfixable: &mut Vec<UnfixableEntry>, g: GroupState, reason: &str) {
    for issue in g.issues {
        unfixable.push(UnfixableEntry::Parsed {
            issue,
            reason: reason.to_string(),
        });
    }
}

fn column_name(field: Field, lang: &str) -> String {
    let prefix = match field {
        Field::Label => 'L',
        Field::Description => 'D',
        Field::Alias => 'A',
    };
    format!("{prefix}{lang}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProcessConfig {
        ProcessConfig {
            fix_ctx: FixCtx {
                nationalities: ["irish".to_string()].into_iter().collect(),
                trademark_chars: vec!["™".into()],
            },
            enabled_checks: fixes::FIXABLE_CHECKS.iter().map(|s| (*s).to_string()).collect(),
            description_max_len: 140,
        }
    }

    fn issue(qid: &str, lang: &str, check: &str, value: &str, suggestion: Option<&str>) -> Issue {
        Issue {
            qid: qid.into(),
            lang: lang.into(),
            field: Field::Description,
            check: check.into(),
            value: value.into(),
            suggestion: suggestion.map(|s| s.into()),
            details: None,
        }
    }

    #[test]
    fn single_misspelled_produces_one_cell() {
        let issues = vec![issue(
            "Q1",
            "en",
            "description.misspelled",
            "the abandonned ship",
            Some("the abandoned ship"),
        )];
        let r = process(issues, vec![], &config());
        assert_eq!(r.cells.len(), 1);
        assert_eq!(r.cells[0].qid, "Q1");
        assert_eq!(r.cells[0].column, "Den");
        assert_eq!(r.cells[0].value, "the abandoned ship");
        assert!(r.unfixable.is_empty());
    }

    #[test]
    fn detection_only_routes_entire_group_to_unfixable() {
        let issues = vec![
            issue("Q1", "en", "description.misspelled", "foo bar", Some("foo baz")),
            issue("Q1", "en", "description.too_long", "foo bar", None),
        ];
        let r = process(issues, vec![], &config());
        assert!(r.cells.is_empty());
        assert_eq!(r.unfixable.len(), 2);
        for entry in &r.unfixable {
            match entry {
                UnfixableEntry::Parsed { reason, .. } => assert_eq!(reason, "detection_only"),
                _ => panic!("unexpected variant"),
            }
        }
    }

    #[test]
    fn safety_bounds_rejects_too_long_value() {
        let mut cfg = config();
        cfg.description_max_len = 5;
        let issues = vec![issue(
            "Q1",
            "en",
            "description.misspelled",
            "abc",
            Some("abcdefghi"),
        )];
        let r = process(issues, vec![], &cfg);
        assert!(r.cells.is_empty());
        assert_eq!(r.unfixable.len(), 1);
        match &r.unfixable[0] {
            UnfixableEntry::Parsed { reason, .. } => assert_eq!(reason, "safety_bounds"),
            _ => panic!(),
        }
    }

    #[test]
    fn noop_suppresses_silently() {
        // Suggestion equals original → no-op.
        let issues = vec![issue(
            "Q1",
            "en",
            "description.misspelled",
            "the abandoned ship",
            Some("the abandoned ship"),
        )];
        let r = process(issues, vec![], &config());
        assert!(r.cells.is_empty());
        assert!(r.unfixable.is_empty());
        assert_eq!(r.suppressed_count, 1);
    }

    #[test]
    fn parse_failures_are_passed_through() {
        let r = process(vec![], vec!["not json".into()], &config());
        assert_eq!(r.unfixable.len(), 1);
        match &r.unfixable[0] {
            UnfixableEntry::Unparsed { raw, reason } => {
                assert_eq!(raw, "not json");
                assert_eq!(reason, "parse_error");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn multiple_qids_preserve_first_seen_order() {
        let issues = vec![
            issue("Q2", "en", "description.misspelled", "x", Some("xx")),
            issue("Q1", "en", "description.misspelled", "y", Some("yy")),
            issue("Q3", "en", "description.misspelled", "z", Some("zz")),
        ];
        let r = process(issues, vec![], &config());
        assert_eq!(r.cells.len(), 3);
        assert_eq!(r.cells[0].qid, "Q2");
        assert_eq!(r.cells[1].qid, "Q1");
        assert_eq!(r.cells[2].qid, "Q3");
    }

    #[test]
    fn distinct_columns_get_distinct_cells() {
        let issues = vec![
            issue("Q1", "en-gb", "description.misspelled", "a", Some("aa")),
            issue("Q1", "en", "description.misspelled", "b", Some("bb")),
        ];
        let r = process(issues, vec![], &config());
        assert_eq!(r.cells.len(), 2);
        // Each cell carries its own (qid, column, value) — no sparse columns.
        assert!(r.cells.iter().any(|c| c.column == "Den" && c.value == "bb"));
        assert!(r.cells.iter().any(|c| c.column == "Den-gb" && c.value == "aa"));
    }

    #[test]
    fn coalesces_two_fixes_in_input_order() {
        // First trademark strip, then double-space collapse.
        let i1 = Issue {
            qid: "Q1".into(),
            lang: "en".into(),
            field: Field::Description,
            check: "description.contains_trademark".into(),
            value: "foo™  bar".into(),
            suggestion: None,
            details: None,
        };
        let i2 = Issue {
            qid: "Q1".into(),
            lang: "en".into(),
            field: Field::Description,
            check: "description.contains_double_space".into(),
            value: "foo™  bar".into(),
            suggestion: None,
            details: None,
        };
        let r = process(vec![i1, i2], vec![], &config());
        assert_eq!(r.cells.len(), 1);
        assert_eq!(r.cells[0].value, "foo bar");
    }
}
