use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl AlertSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

impl fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
#[error("invalid alert severity value")]
pub struct ParseAlertSeverityError;

impl FromStr for AlertSeverity {
    type Err = ParseAlertSeverityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            "critical" => Ok(Self::Critical),
            _ => Err(ParseAlertSeverityError),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemAlert {
    pub id: String,
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
    pub action: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemAlertsResponse {
    pub alerts: Vec<SystemAlert>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let alert = SystemAlert {
            id: "test".to_string(),
            severity: AlertSeverity::Warning,
            title: "Test".to_string(),
            message: "Test message".to_string(),
            action: None,
        };
        let json = serde_json::to_string(&alert).expect("serialize");
        assert!(json.contains(r#""severity":"warning""#));
        let parsed: SystemAlert = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.severity, AlertSeverity::Warning);
    }

    #[test]
    fn display_all_variants() {
        assert_eq!(AlertSeverity::Info.to_string(), "info");
        assert_eq!(AlertSeverity::Warning.to_string(), "warning");
        assert_eq!(AlertSeverity::Error.to_string(), "error");
        assert_eq!(AlertSeverity::Critical.to_string(), "critical");
    }

    #[test]
    fn from_str_valid() {
        assert_eq!(
            "info".parse::<AlertSeverity>().expect("info"),
            AlertSeverity::Info
        );
        assert_eq!(
            "warning".parse::<AlertSeverity>().expect("warning"),
            AlertSeverity::Warning
        );
        assert_eq!(
            "error".parse::<AlertSeverity>().expect("error"),
            AlertSeverity::Error
        );
        assert_eq!(
            "critical".parse::<AlertSeverity>().expect("critical"),
            AlertSeverity::Critical
        );
    }

    #[test]
    fn from_str_invalid() {
        assert!("unknown".parse::<AlertSeverity>().is_err());
        assert!("WARNING".parse::<AlertSeverity>().is_err());
        assert!("".parse::<AlertSeverity>().is_err());
    }

    #[test]
    fn as_str_matches_display() {
        for severity in [
            AlertSeverity::Info,
            AlertSeverity::Warning,
            AlertSeverity::Error,
            AlertSeverity::Critical,
        ] {
            assert_eq!(severity.as_str(), severity.to_string());
        }
    }
}
