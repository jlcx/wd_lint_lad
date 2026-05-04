/// Returns true when `c` belongs to the Latin script.
///
/// Covers Basic Latin letters, Latin-1 Supplement, Latin Extended-A/B,
/// IPA Extensions, Latin Extended Additional, and the Extended-C/D/E
/// blocks — i.e., the full set of characters used in European Latin-
/// script writing systems.  Relies on the caller having already
/// established that `c` is alphabetic before using this to classify
/// script membership.
pub fn is_latin_script(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x0041..=0x007A   // Basic Latin: A-Z, a-z
        | 0x00C0..=0x024F // Latin-1 Supplement (letters) + Extended-A/B
        | 0x0250..=0x02AF // IPA Extensions (all Latin-derived)
        | 0x1E00..=0x1EFF // Latin Extended Additional
        | 0x2C60..=0x2C7F // Latin Extended-C
        | 0xA720..=0xA7FF // Latin Extended-D
        | 0xAB30..=0xAB6F // Latin Extended-E
    )
}

/// Returns true when the majority of *alphabetic* characters in `value`
/// are not Latin-script.
///
/// Digits, punctuation, and whitespace are neutral — they don't count
/// toward either side.  A tie (equal counts) is not flagged.
pub fn is_predominantly_non_latin(value: &str) -> bool {
    let mut latin: usize = 0;
    let mut non_latin: usize = 0;
    for c in value.chars() {
        if c.is_alphabetic() {
            if is_latin_script(c) {
                latin += 1;
            } else {
                non_latin += 1;
            }
        }
    }
    non_latin > latin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_script_covers_extended_forms() {
        // Basic ASCII letters
        assert!(is_latin_script('A'));
        assert!(is_latin_script('z'));
        // Accented forms (Latin Extended)
        assert!(is_latin_script('é'));
        assert!(is_latin_script('ü'));
        assert!(is_latin_script('ñ'));
        assert!(is_latin_script('Ø'));
        // IPA
        assert!(is_latin_script('ɐ'));
    }

    #[test]
    fn non_latin_scripts_not_classified_as_latin() {
        assert!(!is_latin_script('А')); // Cyrillic А
        assert!(!is_latin_script('α')); // Greek alpha
        assert!(!is_latin_script('东')); // CJK
        assert!(!is_latin_script('ع')); // Arabic
        assert!(!is_latin_script('あ')); // Hiragana
        assert!(!is_latin_script('ת')); // Hebrew
    }

    #[test]
    fn predominantly_non_latin_flags_when_non_latin_majority() {
        // All CJK → flag
        assert!(is_predominantly_non_latin("東京都"));
        // All Cyrillic → flag
        assert!(is_predominantly_non_latin("Москва"));
        // All Arabic → flag
        assert!(is_predominantly_non_latin("مدينة"));
        // Clearly Latin → no flag
        assert!(!is_predominantly_non_latin("English label"));
        assert!(!is_predominantly_non_latin("Étienne de Silhouette"));
        // Tie (equal counts) → no flag
        assert!(!is_predominantly_non_latin("АаBb")); // 2 Cyrillic, 2 Latin
        // Digits and punctuation are neutral
        assert!(!is_predominantly_non_latin("123 (foo)"));
        // Empty → no flag
        assert!(!is_predominantly_non_latin(""));
    }

    #[test]
    fn predominantly_non_latin_mixed_cases() {
        // "Tokyo (東京)" — "Tokyo" is 5 Latin, "東京" is 2 non-Latin → Latin wins → no flag
        assert!(!is_predominantly_non_latin("Tokyo (東京)"));
        // Parenthetical native form overwhelms the Latin → flag
        assert!(is_predominantly_non_latin("A (東京都知事)"));
    }
}
