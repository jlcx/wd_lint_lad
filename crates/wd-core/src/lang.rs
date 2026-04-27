/// Returns true for `en` and any `en-*` subtag, case-insensitive.
///
/// Per SPEC §"Language handling", every check operates on every English
/// variant present on an entity, not just `en`.
pub fn is_english(code: &str) -> bool {
    let prefix = code.split('-').next().unwrap_or("");
    prefix.eq_ignore_ascii_case("en")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_en_and_subtags() {
        assert!(is_english("en"));
        assert!(is_english("EN"));
        assert!(is_english("en-us"));
        assert!(is_english("en-GB"));
        assert!(is_english("en-ca"));
        assert!(is_english("en-simple"));
        assert!(is_english("en-x-foo"));
    }

    #[test]
    fn rejects_other_languages() {
        assert!(!is_english(""));
        assert!(!is_english("english"));
        assert!(!is_english("eng"));
        assert!(!is_english("fr"));
        assert!(!is_english("de-at"));
        assert!(!is_english("zen"));
    }
}
