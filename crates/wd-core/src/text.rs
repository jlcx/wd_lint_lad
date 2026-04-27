//! Small text helpers shared between the scanner and the fixer.

/// Return `s` with the first character uppercased; the rest is unchanged.
///
/// Operates on Unicode code points. `capfirst("über") == "Über"`.
pub fn capfirst(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

/// Return `s` with the first character lowercased; the rest is unchanged.
pub fn lowerfirst(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().chain(chars).collect(),
    }
}

/// Split `s` into segments preserving boundaries between Unicode-whitespace
/// runs and non-whitespace runs. Each segment is `(is_whitespace, slice)`.
///
/// Empty input yields an empty `Vec`.
pub fn whitespace_segments(s: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut cur: Option<bool> = None;
    for (i, ch) in s.char_indices() {
        let is_ws = ch.is_whitespace();
        match cur {
            None => cur = Some(is_ws),
            Some(prev) if prev != is_ws => {
                out.push((prev, &s[start..i]));
                start = i;
                cur = Some(is_ws);
            }
            _ => {}
        }
    }
    if let Some(ws) = cur {
        out.push((ws, &s[start..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capfirst_basic() {
        assert_eq!(capfirst(""), "");
        assert_eq!(capfirst("hello"), "Hello");
        assert_eq!(capfirst("Hello"), "Hello");
        assert_eq!(capfirst("über"), "Über");
        // Unicode toupper of ß is "SS"; documented quirk, not a bug.
        assert_eq!(capfirst("ßeppo"), "SSeppo");
    }

    #[test]
    fn lowerfirst_basic() {
        assert_eq!(lowerfirst(""), "");
        assert_eq!(lowerfirst("Hello"), "hello");
        assert_eq!(lowerfirst("HELLO"), "hELLO");
    }

    #[test]
    fn segments_empty() {
        assert!(whitespace_segments("").is_empty());
    }

    #[test]
    fn segments_single_token() {
        assert_eq!(whitespace_segments("abc"), vec![(false, "abc")]);
    }

    #[test]
    fn segments_alternating() {
        assert_eq!(
            whitespace_segments("a b  c"),
            vec![
                (false, "a"),
                (true, " "),
                (false, "b"),
                (true, "  "),
                (false, "c"),
            ]
        );
    }

    #[test]
    fn segments_leading_and_trailing_whitespace() {
        assert_eq!(
            whitespace_segments(" a "),
            vec![(true, " "), (false, "a"), (true, " ")]
        );
    }
}
