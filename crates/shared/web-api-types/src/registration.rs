use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// Registration mode controlling how new users can sign up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    /// Anyone can register without a token.
    Open,
    /// Registration requires a valid token.
    Invite,
    /// Registration is disabled.
    Closed,
}

impl RegistrationMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Invite => "invite",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid registration mode value")]
pub struct ParseRegistrationModeError;

impl FromStr for RegistrationMode {
    type Err = ParseRegistrationModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "invite" => Ok(Self::Invite),
            "closed" => Ok(Self::Closed),
            _ => Err(ParseRegistrationModeError),
        }
    }
}
