use std::str::FromStr;

/// A verb from the closed access-model verb set.
///
/// The set is deliberately closed and this enum is deliberately
/// **exhaustive** (no `#[non_exhaustive]`): adding a verb is an
/// architecture decision (`05-action-model.md`) and must break every
/// match site at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(strum::EnumIter))]
pub enum Verb {
    Read,
    Create,
    Update,
    Delete,
    Trigger,
    Approve,
    Reject,
    Manage,
    Use,
}

impl Verb {
    /// Every verb, for production code needing the closed set (e.g. the
    /// allowed-verb set of dynamic resources). Guarded against silent
    /// drift by the `all_matches_iter` test below.
    pub const ALL: &'static [Verb] = &[
        Verb::Read,
        Verb::Create,
        Verb::Update,
        Verb::Delete,
        Verb::Trigger,
        Verb::Approve,
        Verb::Reject,
        Verb::Manage,
        Verb::Use,
    ];

    /// Canonical wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Verb::Read => "read",
            Verb::Create => "create",
            Verb::Update => "update",
            Verb::Delete => "delete",
            Verb::Trigger => "trigger",
            Verb::Approve => "approve",
            Verb::Reject => "reject",
            Verb::Manage => "manage",
            Verb::Use => "use",
        }
    }
}

impl std::fmt::Display for Verb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`Verb`] string.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown verb")]
pub struct ParseVerbError;

impl FromStr for Verb {
    type Err = ParseVerbError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read" => Ok(Self::Read),
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            "trigger" => Ok(Self::Trigger),
            "approve" => Ok(Self::Approve),
            "reject" => Ok(Self::Reject),
            "manage" => Ok(Self::Manage),
            "use" => Ok(Self::Use),
            _ => Err(ParseVerbError),
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn as_str_from_str_round_trip_exhaustive() {
        for verb in Verb::iter() {
            let parsed: Verb = verb.as_str().parse().expect("round-trip");
            assert_eq!(parsed, verb);
            assert_eq!(verb.to_string(), verb.as_str());
        }
    }

    #[test]
    fn all_matches_iter() {
        let from_iter: Vec<Verb> = Verb::iter().collect();
        assert_eq!(Verb::ALL, from_iter.as_slice());
    }

    #[test]
    fn unknown_verbs_rejected() {
        for bad in ["write", "list", "READ", "read ", "", "use2"] {
            assert!(bad.parse::<Verb>().is_err(), "should reject {bad:?}");
        }
    }
}
