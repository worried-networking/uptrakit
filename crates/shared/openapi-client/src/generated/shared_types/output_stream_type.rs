use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
/// Output stream source for update execution output lines.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OutputStreamType {
    #[default]
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "stdout"))]
    Stdout,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "stderr"))]
    Stderr,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pre_hook"))]
    PreHook,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "post_hook"))]
    PostHook,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "system"))]
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
