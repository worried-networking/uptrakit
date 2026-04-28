// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
pub struct SystemAlert {
    pub id: String,
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
    pub action: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SystemAlertsResponse {
    pub alerts: Vec<SystemAlert>,
}
