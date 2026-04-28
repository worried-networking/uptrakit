use serde::{Deserialize, Serialize};
use std::fmt;
/// Security impact level for an operation shown to the user before execution.
///
/// Ordered `Low < Medium < High` so that `Iterator::max()` yields the worst-case
/// severity across a set of operations.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum Severity {
    #[default]
    Low,
    Medium,
    High,
}
impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}
