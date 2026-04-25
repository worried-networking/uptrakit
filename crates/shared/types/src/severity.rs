use std::fmt;

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_low_lt_medium_lt_high() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::Low < Severity::High);
    }

    #[test]
    fn max_across_iterator() {
        let severities = [Severity::Low, Severity::High, Severity::Medium];
        assert_eq!(severities.iter().copied().max(), Some(Severity::High));
    }

    #[test]
    fn serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Severity::Low).unwrap(), r#""low""#);
        assert_eq!(
            serde_json::to_string(&Severity::Medium).unwrap(),
            r#""medium""#
        );
        assert_eq!(serde_json::to_string(&Severity::High).unwrap(), r#""high""#);
    }

    #[test]
    fn display_matches_serde() {
        for s in [Severity::Low, Severity::Medium, Severity::High] {
            let displayed = s.to_string();
            let serialized = serde_json::to_string(&s).unwrap();
            assert_eq!(format!("\"{displayed}\""), serialized);
        }
    }
}
