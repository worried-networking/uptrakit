use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Status of an update batch.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for status strings received from a
/// newer peer that this binary does not yet know about. Serde deserialization
/// is infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older clients to survive rolling upgrades without dropping
/// entire messages.
///
/// `FromStr` remains strict for DB lookups and URL parameters where callers
/// need to distinguish known variants from unknown ones.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum BatchStatus {
    /// The batch has updates still pending or in progress.
    InProgress,
    /// All updates in the batch completed successfully.
    Completed,
    /// All updates finished but at least one failed.
    PartiallyCompleted,
    /// An unknown status received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl BatchStatus {
    /// Returns the string representation.
    ///
    /// For [`BatchStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::PartiallyCompleted => "partially_completed",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for BatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid batch status string.
#[derive(Debug)]
pub struct ParseBatchStatusError;

impl fmt::Display for ParseBatchStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid batch status value")
    }
}

impl std::error::Error for ParseBatchStatusError {}

impl FromStr for BatchStatus {
    type Err = ParseBatchStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "partially_completed" => Ok(Self::PartiallyCompleted),
            _ => Err(ParseBatchStatusError),
        }
    }
}

impl From<String> for BatchStatus {
    /// Converts a snake_case string to a batch status.
    ///
    /// Unknown strings map to [`BatchStatus::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "partially_completed" => Self::PartiallyCompleted,
            _ => Self::Other(s),
        }
    }
}

// ── Serde: infallible string-based serialization ──────────────────────────────
//
// Custom Serialize/Deserialize are implemented manually rather than via derive
// so that unknown strings deserialize to `Other(String)` rather than failing.
// This makes rolling upgrades wire-safe: a payload containing a new batch
// status from a newer server can be fully parsed by an older client without
// dropping the enclosing struct.

impl Serialize for BatchStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BatchStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize as a plain string, then convert via From<String>.
        // Unknown strings become Other(s) — this conversion is infallible.
        String::deserialize(deserializer).map(BatchStatus::from)
    }
}

// ── SeaORM integration ────────────────────────────────────────────────────────
//
// BatchStatus is used as a typed column in update_batch entity. The manual
// implementations replace the removed DeriveActiveEnum derive.
//
// TryGetable uses FromStr (strict): an unexpected value stored in the database
// is always an explicit DbErr::Type, not silently promoted to Other. A DB
// value that this binary does not recognise is a data integrity issue.

#[cfg(feature = "sea-orm")]
mod sea_orm_impl {
    use super::BatchStatus;
    use sea_orm::entity::prelude::*;
    use sea_orm::sea_query::ValueType;
    use sea_orm::{TryGetError, TryGetable};

    impl From<BatchStatus> for Value {
        fn from(s: BatchStatus) -> Self {
            Value::String(Some(s.as_str().to_string()))
        }
    }

    impl TryGetable for BatchStatus {
        fn try_get_by<I: sea_orm::ColIdx>(
            res: &QueryResult,
            index: I,
        ) -> std::result::Result<Self, TryGetError> {
            match <Option<String> as TryGetable>::try_get_by(res, index) {
                Ok(Some(val)) => val.parse::<BatchStatus>().map_err(|_| {
                    TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                        "unexpected BatchStatus value in database: {val:?}"
                    )))
                }),
                Ok(None) => Err(TryGetError::Null(index.as_str().unwrap_or("").to_string())),
                Err(e) => Err(e),
            }
        }
    }

    impl ValueType for BatchStatus {
        fn try_from(v: Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
            match v {
                Value::String(Some(s)) => s
                    .parse::<BatchStatus>()
                    .map_err(|_| sea_orm::sea_query::ValueTypeErr),
                _ => Err(sea_orm::sea_query::ValueTypeErr),
            }
        }

        fn type_name() -> String {
            "BatchStatus".to_string()
        }

        fn array_type() -> sea_orm::sea_query::ArrayType {
            sea_orm::sea_query::ArrayType::String
        }

        fn column_type() -> sea_orm::ColumnType {
            sea_orm::ColumnType::String(sea_orm::sea_query::StringLen::None)
        }
    }

    impl sea_orm::sea_query::Nullable for BatchStatus {
        fn null() -> Value {
            Value::String(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            BatchStatus::InProgress,
            BatchStatus::Completed,
            BatchStatus::PartiallyCompleted,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: BatchStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            BatchStatus::InProgress,
            BatchStatus::Completed,
            BatchStatus::PartiallyCompleted,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            BatchStatus::InProgress,
            BatchStatus::Completed,
            BatchStatus::PartiallyCompleted,
        ] {
            let s = variant.as_str();
            let parsed: BatchStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<BatchStatus>().is_err());
        assert!("".parse::<BatchStatus>().is_err());
    }

    #[test]
    fn unknown_status_deserializes_to_other() {
        let deserialized: BatchStatus =
            serde_json::from_str(r#""future_status""#).expect("deserialize unknown");
        assert_eq!(
            deserialized,
            BatchStatus::Other("future_status".to_string())
        );
    }

    #[test]
    fn other_serializes_to_inner_string() {
        let status = BatchStatus::Other("future_status".to_string());
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""future_status""#);
    }

    #[test]
    fn other_round_trip() {
        let original = r#""new_batch_state""#;
        let deserialized: BatchStatus = serde_json::from_str(original).expect("deserialize");
        assert_eq!(
            deserialized,
            BatchStatus::Other("new_batch_state".to_string())
        );
        let reserialized = serde_json::to_string(&deserialized).expect("serialize");
        assert_eq!(reserialized, original);
    }

    #[test]
    fn from_str_still_strict_for_unknown() {
        // FromStr must reject unknown strings even though Deserialize accepts them.
        assert!("future_status".parse::<BatchStatus>().is_err());
        assert!("other".parse::<BatchStatus>().is_err());
    }
}
