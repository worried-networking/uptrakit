use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// Registration mode controlling how new users can sign up.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(test, derive(strum::EnumIter))]
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

impl std::fmt::Display for RegistrationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    // ── Serde round-trip ─────────────────────────────────────────────

    #[test]
    fn serde_round_trip_all_variants() {
        for mode in RegistrationMode::iter() {
            let json = serde_json::to_string(&mode).expect("serialization should succeed");
            let deserialized: RegistrationMode =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(deserialized, mode);
        }
    }

    #[test]
    fn serde_serializes_as_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&RegistrationMode::Open).expect("serialization should succeed"),
            r#""open""#
        );
        assert_eq!(
            serde_json::to_string(&RegistrationMode::Invite).expect("serialization should succeed"),
            r#""invite""#
        );
        assert_eq!(
            serde_json::to_string(&RegistrationMode::Closed).expect("serialization should succeed"),
            r#""closed""#
        );
    }

    #[test]
    fn serde_rejects_unknown_variant() {
        let result = serde_json::from_str::<RegistrationMode>(r#""disabled""#);
        assert!(result.is_err());
    }

    #[test]
    fn serde_rejects_uppercase_variant() {
        let result = serde_json::from_str::<RegistrationMode>(r#""Open""#);
        assert!(result.is_err());
    }

    // ── Display output ───────────────────────────────────────────────

    #[test]
    fn display_open() {
        assert_eq!(format!("{}", RegistrationMode::Open), "open");
    }

    #[test]
    fn display_invite() {
        assert_eq!(format!("{}", RegistrationMode::Invite), "invite");
    }

    #[test]
    fn display_closed() {
        assert_eq!(format!("{}", RegistrationMode::Closed), "closed");
    }

    #[test]
    fn display_matches_as_str_for_all_variants() {
        for mode in RegistrationMode::iter() {
            assert_eq!(format!("{mode}"), mode.as_str());
        }
    }

    // ── FromStr valid inputs ─────────────────────────────────────────

    #[test]
    fn from_str_open() {
        let parsed: RegistrationMode = "open".parse().expect("should parse 'open'");
        assert_eq!(parsed, RegistrationMode::Open);
    }

    #[test]
    fn from_str_invite() {
        let parsed: RegistrationMode = "invite".parse().expect("should parse 'invite'");
        assert_eq!(parsed, RegistrationMode::Invite);
    }

    #[test]
    fn from_str_closed() {
        let parsed: RegistrationMode = "closed".parse().expect("should parse 'closed'");
        assert_eq!(parsed, RegistrationMode::Closed);
    }

    #[test]
    fn from_str_round_trips_through_as_str() {
        for mode in RegistrationMode::iter() {
            let s = mode.as_str();
            let parsed: RegistrationMode = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(parsed, mode);
        }
    }

    // ── FromStr invalid inputs ───────────────────────────────────────

    #[test]
    fn from_str_empty_string_fails() {
        assert!("".parse::<RegistrationMode>().is_err());
    }

    #[test]
    fn from_str_uppercase_fails() {
        assert!("OPEN".parse::<RegistrationMode>().is_err());
        assert!("Open".parse::<RegistrationMode>().is_err());
    }

    #[test]
    fn from_str_unknown_value_fails() {
        assert!("disabled".parse::<RegistrationMode>().is_err());
        assert!("public".parse::<RegistrationMode>().is_err());
    }

    #[test]
    fn from_str_whitespace_fails() {
        assert!(" open".parse::<RegistrationMode>().is_err());
        assert!("open ".parse::<RegistrationMode>().is_err());
    }

    // ── ParseRegistrationModeError ───────────────────────────────────

    #[test]
    fn parse_error_display_message() {
        let err = ParseRegistrationModeError;
        assert_eq!(err.to_string(), "invalid registration mode value");
    }
}
