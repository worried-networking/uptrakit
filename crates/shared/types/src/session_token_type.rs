use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        let variant = SessionTokenType::RefreshToken;
        let json = serde_json::to_string(&variant).unwrap();
        let deserialized: SessionTokenType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, variant);
    }

    #[test]
    fn display_matches_as_str() {
        let variant = SessionTokenType::RefreshToken;
        assert_eq!(format!("{variant}"), variant.as_str());
    }

    #[test]
    fn from_str_round_trip() {
        let variant = SessionTokenType::RefreshToken;
        let s = variant.as_str();
        let parsed: SessionTokenType = s.parse().unwrap();
        assert_eq!(parsed, variant);
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<SessionTokenType>().is_err());
        assert!("".parse::<SessionTokenType>().is_err());
    }

    #[test]
    fn serde_value() {
        assert_eq!(
            serde_json::to_string(&SessionTokenType::RefreshToken).unwrap(),
            r#""refresh_token""#
        );
    }
}
