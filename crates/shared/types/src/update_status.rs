use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Status of an individual update record.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(all(test, not(feature = "sea-orm")), derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    /// The update is queued and waiting for the host to become free. This
    /// applies to both batch items waiting for a preceding update on the same
    /// host, and to single (non-batch) updates triggered when the host already
    /// had an active update. Not an active state — no in-progress work on the
    /// host. Terminal states are [`Self::Completed`] and [`Self::Failed`].
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "queued"))]
    Queued,
    /// The update is waiting to be dispatched.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    /// The update is currently running on the agent.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "in_progress"))]
    InProgress,
    /// The update has finished but requires a system restart to take effect.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "awaiting_restart"))]
    AwaitingRestart,
    /// The update completed successfully.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "completed"))]
    Completed,
    /// The update failed.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "failed"))]
    Failed,
}

impl UpdateStatus {
    /// Returns the canonical string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::AwaitingRestart => "awaiting_restart",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for UpdateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`UpdateStatus`] string.
#[derive(Debug)]
pub struct ParseUpdateStatusError;

impl fmt::Display for ParseUpdateStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid update status value")
    }
}

impl std::error::Error for ParseUpdateStatusError {}

impl FromStr for UpdateStatus {
    type Err = ParseUpdateStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "awaiting_restart" => Ok(Self::AwaitingRestart),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseUpdateStatusError),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn serde_round_trip() {
        for variant in UpdateStatus::iter() {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: UpdateStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in UpdateStatus::iter() {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in UpdateStatus::iter() {
            let s = variant.as_str();
            let parsed: UpdateStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<UpdateStatus>().is_err());
        assert!("".parse::<UpdateStatus>().is_err());
        assert!("InProgress".parse::<UpdateStatus>().is_err());
        assert!("inprogress".parse::<UpdateStatus>().is_err());
    }

    #[test]
    fn as_str_values() {
        assert_eq!(UpdateStatus::Queued.as_str(), "queued");
        assert_eq!(UpdateStatus::Pending.as_str(), "pending");
        assert_eq!(UpdateStatus::InProgress.as_str(), "in_progress");
        assert_eq!(UpdateStatus::AwaitingRestart.as_str(), "awaiting_restart");
        assert_eq!(UpdateStatus::Completed.as_str(), "completed");
        assert_eq!(UpdateStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn test_awaiting_restart_round_trip() {
        let s = UpdateStatus::AwaitingRestart;
        assert_eq!(s.as_str(), "awaiting_restart");
        let parsed: UpdateStatus = "awaiting_restart".parse().unwrap();
        assert_eq!(parsed, UpdateStatus::AwaitingRestart);
    }

    #[test]
    fn parse_error_display() {
        let err = ParseUpdateStatusError;
        assert_eq!(err.to_string(), "invalid update status value");
    }
}
