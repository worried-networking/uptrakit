use std::fmt;

use serde::{Deserialize, Serialize};
use time::UtcDateTime;

/// Unix epoch timestamp in milliseconds.
pub type Timestamp = i64;

/// Returns the current time as Unix epoch milliseconds.
pub fn now_millis() -> Timestamp {
    let now = UtcDateTime::now();
    now.unix_timestamp() * 1000 + i64::from(now.millisecond())
}

/// Final status of an update execution.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for status strings received from a newer
/// agent that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older controllers to survive rolling upgrades without
/// dropping the enclosing `UpdateResult` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(strum::EnumIter))]
pub enum UpdateFinalStatus {
    Completed,
    Failed,
    /// An unknown status received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl UpdateFinalStatus {
    /// Returns the string representation.
    ///
    /// For [`UpdateFinalStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for UpdateFinalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for UpdateFinalStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for UpdateFinalStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdateFinalStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(UpdateFinalStatus::from)
    }
}

/// Default timeout for update execution (2 hours).
pub const DEFAULT_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(7200);

/// Default timeout for update execution.
pub(crate) fn default_update_timeout() -> std::time::Duration {
    DEFAULT_UPDATE_TIMEOUT
}

/// Reason for service disconnection.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for reason strings received from a newer
/// peer that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing rolling upgrades without dropping the `Disconnecting` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(strum::EnumIter))]
pub enum DisconnectReason {
    /// SIGTERM/SIGINT - clean exit.
    Shutdown,
    /// SIGHUP - will reconnect after external restart.
    Restart,
    /// An unknown reason received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl DisconnectReason {
    /// Returns the string representation.
    ///
    /// For [`DisconnectReason::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Restart => "restart",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for DisconnectReason {
    fn from(s: String) -> Self {
        match s.as_str() {
            "shutdown" => Self::Shutdown,
            "restart" => Self::Restart,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for DisconnectReason {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DisconnectReason {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(DisconnectReason::from)
    }
}

// ── JSON Schema impls for custom-serde enums ──────────────────────────────────
//
// `derive(schemars::JsonSchema)` would document the Rust variant identifiers
// rather than the wire strings — a silent semantic bug (spec §1). These
// hand-written impls emit an OPEN string schema: `"type": "string"` with known
// wire strings in the description and NO `"enum"` array, because the
// `Other(String)` catch-all makes the value space open-ended.
//
// Known-value lists are derived via `strum::EnumIter` from the same `as_str()`
// the `Serialize` impl uses — a hardcoded list here would drift silently.

#[cfg(feature = "schema")]
impl schemars::JsonSchema for UpdateFinalStatus {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("UpdateFinalStatus")
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        use strum::IntoEnumIterator;
        let known: Vec<String> = UpdateFinalStatus::iter()
            .filter(|v| !matches!(v, Self::Other(_)))
            .map(|v| v.as_str().to_string())
            .collect();
        schemars::json_schema!({
            "type": "string",
            "description": format!(
                "Open wire string (unknown values are forward-compatible). Known values: {}.",
                known.join(", ")
            ),
        })
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for DisconnectReason {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("DisconnectReason")
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        use strum::IntoEnumIterator;
        let known: Vec<String> = DisconnectReason::iter()
            .filter(|v| !matches!(v, Self::Other(_)))
            .map(|v| v.as_str().to_string())
            .collect();
        schemars::json_schema!({
            "type": "string",
            "description": format!(
                "Open wire string (unknown values are forward-compatible). Known values: {}.",
                known.join(", ")
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "schema")]
    mod schema_tests {
        use super::super::*;

        fn assert_open_string_schema<T: schemars::JsonSchema>(known: &[&str]) {
            let schema = schemars::schema_for!(T);
            let value = serde_json::to_value(&schema).expect("schema to JSON");
            assert_eq!(value["type"], "string");
            assert!(
                value.get("enum").is_none(),
                "must be an open string schema, found closed enum list: {value}"
            );
            let desc = value["description"].as_str().expect("description present");
            for k in known {
                assert!(
                    desc.contains(k),
                    "known value {k} missing from description: {desc}"
                );
            }
        }

        #[test]
        fn update_final_status_schema_is_open_string_with_known_values() {
            assert_open_string_schema::<UpdateFinalStatus>(&["completed", "failed"]);
        }

        #[test]
        fn disconnect_reason_schema_is_open_string_with_known_values() {
            assert_open_string_schema::<DisconnectReason>(&["shutdown", "restart"]);
        }
    }
}
