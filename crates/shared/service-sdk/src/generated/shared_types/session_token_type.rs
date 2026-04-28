// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
/// Type of session token stored in the `sessions` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionTokenType {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "refresh_token"))]
    RefreshToken,
}
impl SessionTokenType {
    /// Returns the string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::RefreshToken => "refresh_token",
        }
    }
}
impl fmt::Display for SessionTokenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Error returned when parsing an invalid session token type string.
#[derive(Debug)]
pub struct ParseSessionTokenTypeError;
impl fmt::Display for ParseSessionTokenTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid session token type value")
    }
}
impl std::error::Error for ParseSessionTokenTypeError {}
impl FromStr for SessionTokenType {
    type Err = ParseSessionTokenTypeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "refresh_token" => Ok(Self::RefreshToken),
            _ => Err(ParseSessionTokenTypeError),
        }
    }
}
