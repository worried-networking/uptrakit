// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
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
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseUpdateStatusError),
        }
    }
}
