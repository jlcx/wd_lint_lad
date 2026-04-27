use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    Label,
    Description,
    Alias,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub qid: String,
    pub lang: String,
    pub field: Field,
    pub check: String,
    pub value: String,
    pub suggestion: Option<String>,
    pub details: Option<Details>,
}

/// Per-record `details` payload.
///
/// `null` for most checks (encoded as `Option::None` on `Issue`),
/// an array of sub-check IDs for `description.composite`,
/// or `{"new_max_len": <int>}` for `aliases.long` / `descriptions.long`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Details {
    Composite(Vec<String>),
    NewMaxLen { new_max_len: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_round_trip_misspelled() {
        let json = r#"{"qid":"Q12345","lang":"en-gb","field":"description","check":"description.misspelled","value":"the abandonned ship","suggestion":"the abandoned ship","details":null}"#;
        let issue: Issue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.qid, "Q12345");
        assert_eq!(issue.field, Field::Description);
        assert_eq!(issue.suggestion.as_deref(), Some("the abandoned ship"));
        assert!(issue.details.is_none());
        assert_eq!(serde_json::to_string(&issue).unwrap(), json);
    }

    #[test]
    fn details_composite_round_trip() {
        let d = Details::Composite(vec!["description.too_long".into()]);
        let s = serde_json::to_string(&d).unwrap();
        assert_eq!(s, r#"["description.too_long"]"#);
        let back: Details = serde_json::from_str(&s).unwrap();
        match back {
            Details::Composite(v) => assert_eq!(v, vec!["description.too_long"]),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn details_new_max_len_round_trip() {
        let d = Details::NewMaxLen { new_max_len: 42 };
        let s = serde_json::to_string(&d).unwrap();
        assert_eq!(s, r#"{"new_max_len":42}"#);
        let back: Details = serde_json::from_str(&s).unwrap();
        match back {
            Details::NewMaxLen { new_max_len } => assert_eq!(new_max_len, 42),
            _ => panic!("wrong variant"),
        }
    }
}
