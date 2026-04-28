// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
/// Output stream source for update execution output lines.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputStreamType {
    #[default]
    Stdout,
    Stderr,
    PreHook,
    PostHook,
    System,
}
impl OutputStreamType {
    /// Returns the string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::PreHook => "pre_hook",
            Self::PostHook => "post_hook",
            Self::System => "system",
        }
    }
}
impl fmt::Display for OutputStreamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Error returned when parsing an invalid [`OutputStreamType`] string.
#[derive(Debug)]
pub struct ParseOutputStreamTypeError;
impl fmt::Display for ParseOutputStreamTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid output stream type value")
    }
}
impl std::error::Error for ParseOutputStreamTypeError {}
impl FromStr for OutputStreamType {
    type Err = ParseOutputStreamTypeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "pre_hook" => Ok(Self::PreHook),
            "post_hook" => Ok(Self::PostHook),
            "system" => Ok(Self::System),
            _ => Err(ParseOutputStreamTypeError),
        }
    }
}
