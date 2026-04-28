// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
/// Status of a pending device authorization flow.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeviceAuthStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "authorized"))]
    Authorized,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "expired"))]
    Expired,
}
impl DeviceAuthStatus {
    /// Returns the string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Authorized => "authorized",
            Self::Expired => "expired",
        }
    }
}
impl fmt::Display for DeviceAuthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Error returned when parsing an invalid device auth status string.
#[derive(Debug)]
pub struct ParseDeviceAuthStatusError;
impl fmt::Display for ParseDeviceAuthStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid device auth status value")
    }
}
impl std::error::Error for ParseDeviceAuthStatusError {}
impl FromStr for DeviceAuthStatus {
    type Err = ParseDeviceAuthStatusError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "authorized" => Ok(Self::Authorized),
            "expired" => Ok(Self::Expired),
            _ => Err(ParseDeviceAuthStatusError),
        }
    }
}
