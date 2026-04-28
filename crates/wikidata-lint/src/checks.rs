use std::collections::HashSet;

use wd_core::{Issue, entity::Entity};

use crate::matchers::CompiledRules;

pub mod description;
pub mod streaming;

/// Every check ID this scanner knows about. Order matches the spec.
///
/// Adding a check requires (a) a new ID here and (b) a dispatch line in
/// `run_all`.
pub const ALL: &[&str] = &[
    "description.too_long",
    "description.starts_with_label",
    "description.starts_capitalized",
    "description.ends_with_punctuation",
    "description.contains_trademark",
    "description.contains_html_entity",
    "description.contains_double_space",
    "description.contains_obituary",
    "description.space_before_comma",
    "description.marketing_imperative",
    "description.promotional",
    "description.composite",
    "description.multi_sentence",
    "description.misspelled",
    "description.starts_with_lowercase_nationality",
    "description.contains_lowercase_nationality",
    // bad_start is dispatched LAST among description checks so the
    // strip runs after suggestion-based fixes (misspelled,
    // starts_with_lowercase_nationality) that would otherwise clobber
    // a prior strip with their original-derived suggestion.
    "description.bad_start",
    "aliases.long",
    "descriptions.long",
];

#[derive(Debug, Clone)]
pub struct EnabledChecks {
    enabled: HashSet<String>,
}

impl EnabledChecks {
    pub fn all() -> Self {
        Self {
            enabled: ALL.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    pub fn from_list<I, S>(items: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = HashSet::new();
        for item in items {
            let id = item.as_ref().trim();
            if id.is_empty() {
                continue;
            }
            if !ALL.contains(&id) {
                anyhow::bail!("unknown check id: {id}");
            }
            set.insert(id.to_string());
        }
        Ok(Self { enabled: set })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.enabled.contains(id)
    }
}

pub struct CheckCtx<'a> {
    pub compiled: &'a CompiledRules,
    pub enabled: &'a EnabledChecks,
}

pub fn run_all(entity: &Entity, ctx: &CheckCtx<'_>, out: &mut Vec<Issue>) {
    let e = ctx.enabled;
    if e.contains("description.too_long") {
        description::too_long(entity, ctx, out);
    }
    if e.contains("description.starts_with_label") {
        description::starts_with_label(entity, ctx, out);
    }
    if e.contains("description.starts_capitalized") {
        description::starts_capitalized(entity, ctx, out);
    }
    if e.contains("description.ends_with_punctuation") {
        description::ends_with_punctuation(entity, ctx, out);
    }
    if e.contains("description.contains_trademark") {
        description::contains_trademark(entity, ctx, out);
    }
    if e.contains("description.contains_html_entity") {
        description::contains_html_entity(entity, ctx, out);
    }
    if e.contains("description.contains_double_space") {
        description::contains_double_space(entity, ctx, out);
    }
    if e.contains("description.contains_obituary") {
        description::contains_obituary(entity, ctx, out);
    }
    if e.contains("description.space_before_comma") {
        description::space_before_comma(entity, ctx, out);
    }
    if e.contains("description.marketing_imperative") {
        description::marketing_imperative(entity, ctx, out);
    }
    if e.contains("description.promotional") {
        description::promotional(entity, ctx, out);
    }
    if e.contains("description.composite") {
        description::composite(entity, ctx, out);
    }
    if e.contains("description.multi_sentence") {
        description::multi_sentence(entity, ctx, out);
    }
    if e.contains("description.misspelled") {
        description::misspelled(entity, ctx, out);
    }
    if e.contains("description.starts_with_lowercase_nationality") {
        description::starts_with_lowercase_nationality(entity, ctx, out);
    }
    if e.contains("description.contains_lowercase_nationality") {
        description::contains_lowercase_nationality(entity, ctx, out);
    }
    if e.contains("description.bad_start") {
        description::bad_start(entity, ctx, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_set_contains_every_known_id() {
        let e = EnabledChecks::all();
        for id in ALL {
            assert!(e.contains(id), "ALL missing {id}");
        }
    }

    #[test]
    fn from_list_accepts_known() {
        let e = EnabledChecks::from_list(["description.too_long"]).unwrap();
        assert!(e.contains("description.too_long"));
    }

    #[test]
    fn from_list_rejects_unknown() {
        let err = EnabledChecks::from_list(["bogus.check"]).unwrap_err();
        assert!(err.to_string().contains("bogus.check"));
    }

    #[test]
    fn from_list_ignores_empty_entries() {
        let e = EnabledChecks::from_list(["", " description.too_long ", ""]).unwrap();
        assert!(e.contains("description.too_long"));
    }
}
