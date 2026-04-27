use serde::Deserialize;
use std::collections::HashMap;

/// Partial typed view over a Wikidata entity dump line.
///
/// Only the fields the linter needs are typed; unknown fields are ignored.
#[derive(Debug, Deserialize)]
pub struct Entity {
    pub id: String,
    #[serde(default)]
    pub labels: HashMap<String, MonolingualText>,
    #[serde(default)]
    pub descriptions: HashMap<String, MonolingualText>,
    #[serde(default)]
    pub aliases: HashMap<String, Vec<MonolingualText>>,
    #[serde(default)]
    pub claims: HashMap<String, Vec<Claim>>,
}

#[derive(Debug, Deserialize)]
pub struct MonolingualText {
    pub language: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct Claim {
    pub mainsnak: Snak,
}

#[derive(Debug, Deserialize)]
pub struct Snak {
    pub datavalue: Option<DataValue>,
}

#[derive(Debug, Deserialize)]
pub struct DataValue {
    pub value: serde_json::Value,
}

impl Entity {
    /// Returns the `id` of the first P31 (instance-of) claim's value, if any.
    pub fn first_p31_id(&self) -> Option<&str> {
        self.claims
            .get("P31")?
            .first()?
            .mainsnak
            .datavalue
            .as_ref()?
            .value
            .get("id")?
            .as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_entity() {
        let json = r#"{
            "id": "Q42",
            "labels": {"en": {"language":"en","value":"Douglas Adams"}},
            "descriptions": {"en": {"language":"en","value":"English writer"}},
            "aliases": {"en": [{"language":"en","value":"Douglas N. Adams"}]},
            "claims": {}
        }"#;
        let e: Entity = serde_json::from_str(json).unwrap();
        assert_eq!(e.id, "Q42");
        assert_eq!(e.labels["en"].value, "Douglas Adams");
        assert_eq!(e.descriptions["en"].value, "English writer");
        assert_eq!(e.aliases["en"][0].value, "Douglas N. Adams");
        assert!(e.first_p31_id().is_none());
    }

    #[test]
    fn extracts_p31_id() {
        let json = r#"{
            "id": "Q42",
            "claims": {"P31": [{"mainsnak":{"datavalue":{"value":{"id":"Q5"}}}}]}
        }"#;
        let e: Entity = serde_json::from_str(json).unwrap();
        assert_eq!(e.first_p31_id(), Some("Q5"));
    }

    #[test]
    fn ignores_unknown_fields() {
        let json = r#"{"id":"Q1","type":"item","sitelinks":{},"lastrevid":1}"#;
        let e: Entity = serde_json::from_str(json).unwrap();
        assert_eq!(e.id, "Q1");
    }
}
