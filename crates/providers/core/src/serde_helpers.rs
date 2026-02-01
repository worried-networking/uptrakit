/// Serde module for `Option<OffsetDateTime>` using RFC 3339 format.
///
/// Use with `#[serde(default, with = "crate::serde_helpers::optional_rfc3339")]`.
pub mod optional_rfc3339 {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    pub fn serialize<S: Serializer>(
        dt: &Option<OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match dt {
            Some(dt) => {
                let s = dt.format(&Rfc3339).map_err(serde::ser::Error::custom)?;
                serializer.serialize_some(&s)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let dt = OffsetDateTime::parse(&s, &Rfc3339).map_err(serde::de::Error::custom)?;
                Ok(Some(dt))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use time::OffsetDateTime;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestStruct {
        #[serde(default, with = "super::optional_rfc3339")]
        pub timestamp: Option<OffsetDateTime>,
    }

    #[test]
    fn serialize_some() {
        let ts = OffsetDateTime::from_unix_timestamp(1706400000).expect("valid timestamp");
        let s = TestStruct {
            timestamp: Some(ts),
        };
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains("2024-01-28T"));
    }

    #[test]
    fn serialize_none() {
        let s = TestStruct { timestamp: None };
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains("null"));
    }

    #[test]
    fn deserialize_roundtrip() {
        let ts = OffsetDateTime::from_unix_timestamp(1706400000).expect("valid timestamp");
        let s = TestStruct {
            timestamp: Some(ts),
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let deserialized: TestStruct = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, s);
    }

    #[test]
    fn deserialize_null() {
        let json = r#"{"timestamp":null}"#;
        let s: TestStruct = serde_json::from_str(json).expect("deserialize");
        assert_eq!(s.timestamp, None);
    }

    #[test]
    fn deserialize_missing_field() {
        let json = r#"{}"#;
        let s: TestStruct = serde_json::from_str(json).expect("deserialize");
        assert_eq!(s.timestamp, None);
    }
}
