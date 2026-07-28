use std::str::FromStr;

use super::action::Action;
use super::bounds;
use super::catalog::{CATALOG, CatalogEntry, Resource, VerbEntry};
use super::is_valid_segment_path;
use super::verb::{ParseVerbError, Verb};

/// A grant pattern matching a set of concrete actions
/// (`06-grant-model.md` §Grant patterns).
///
/// Fields are `pub(crate)`; construct via [`ActionPattern::new`] or
/// `FromStr`. Parse is grammar-only on the resource side — catalog
/// membership is the write-time [`ActionPattern::can_match_any`] check,
/// never folded into `FromStr` (stored patterns outlive catalog changes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPattern {
    pub(crate) resource: ResourcePattern,
    pub(crate) verb: VerbPattern,
}

/// Resource side of a pattern.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePattern {
    /// `*` — every tenant-plane resource (built-in and dynamic), never
    /// the `system.` plane.
    Any,
    /// Exact resource string (grammar-validated; catalog membership not
    /// required at parse time).
    Exact(String),
    /// `<stem>.*` — strict descendants of `stem`, never the stem itself.
    Subtree(String),
}

/// Verb side of a pattern.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbPattern {
    /// `*`.
    Any,
    /// A single closed-set verb.
    Exact(Verb),
}

/// Error returned when parsing an invalid [`ActionPattern`] string.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParsePatternError {
    /// Longer than [`bounds::MAX_PATTERN_LEN`] bytes.
    #[error("pattern string exceeds {max} bytes")]
    TooLong { max: usize },
    /// Not exactly `resource-pattern:verb-pattern` with non-empty sides.
    #[error("pattern must be `resource:verb`")]
    Structure,
    /// Verb side neither `*` nor a closed-set verb (source via `#[from]`,
    /// keeping `FromStr` free of `clippy::map_err_ignore`).
    #[error("unknown verb in pattern")]
    UnknownVerb(#[from] ParseVerbError),
    /// Resource side violates the pattern grammar (`*` is legal only as
    /// the whole resource or a trailing `.*`).
    #[error("invalid resource pattern grammar")]
    InvalidResourcePattern,
}

impl ActionPattern {
    /// Infallible: per-pattern validity (matchability) is the separate
    /// write-time [`Self::can_match_any`] check.
    pub fn new(resource: ResourcePattern, verb: VerbPattern) -> Self {
        Self { resource, verb }
    }

    pub fn resource(&self) -> &ResourcePattern {
        &self.resource
    }

    pub fn verb(&self) -> &VerbPattern {
        &self.verb
    }

    /// Decision-time truth: does this pattern match the concrete action?
    pub fn matches(&self, action: &Action) -> bool {
        self.resource_matches(&action.resource) && self.verb_matches(action.verb)
    }

    /// Every built-in catalog action this pattern matches. Ranges over
    /// the built-in catalog ONLY — dynamic reachability is a separate
    /// dimension (see [`Self::can_match_any`]); an empty iterator is NOT
    /// "matches nothing" for a dynamic-reaching pattern, and validation
    /// rule 3 (M1.2) must combine both (spec §Grant patterns, rule-3
    /// reuse contract).
    pub fn matched_catalog_actions(
        &self,
    ) -> impl Iterator<Item = (&'static CatalogEntry, &'static VerbEntry)> + '_ {
        CATALOG
            .iter()
            .flat_map(|e| e.verbs.iter().map(move |ve| (e, ve)))
            .filter(|(e, ve)| self.resource_matches(&e.resource) && self.verb_matches(ve.verb))
    }

    /// Write-time validity (rule 1): rejected iff provably unmatchable —
    /// matches no catalog action AND cannot reach a dynamic namespace.
    pub fn can_match_any(&self) -> bool {
        self.matched_catalog_actions().next().is_some() || self.reaches_dynamic()
    }

    /// Whether the resource side can reach `plugin.*` / `surface.*`
    /// resources (whose verbs are registry-declared, so any closed-set
    /// verb is accepted at write time).
    pub fn reaches_dynamic(&self) -> bool {
        match &self.resource {
            ResourcePattern::Any => true,
            ResourcePattern::Exact(s) => s.starts_with("plugin.") || s.starts_with("surface."),
            ResourcePattern::Subtree(stem) => {
                stem == "plugin"
                    || stem == "surface"
                    || stem.starts_with("plugin.")
                    || stem.starts_with("surface.")
            }
        }
    }

    fn resource_matches(&self, resource: &Resource) -> bool {
        match &self.resource {
            ResourcePattern::Any => !resource.is_system(),
            ResourcePattern::Exact(s) => resource.as_str() == s,
            ResourcePattern::Subtree(stem) => resource
                .as_str()
                .strip_prefix(stem.as_str())
                .is_some_and(|rest| rest.len() > 1 && rest.starts_with('.')),
        }
    }

    fn verb_matches(&self, verb: Verb) -> bool {
        match &self.verb {
            VerbPattern::Any => true,
            VerbPattern::Exact(v) => *v == verb,
        }
    }
}

impl std::fmt::Display for ActionPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.resource {
            ResourcePattern::Any => f.write_str("*")?,
            ResourcePattern::Exact(s) => f.write_str(s)?,
            ResourcePattern::Subtree(stem) => write!(f, "{stem}.*")?,
        }
        f.write_str(":")?;
        match &self.verb {
            VerbPattern::Any => f.write_str("*"),
            VerbPattern::Exact(v) => f.write_str(v.as_str()),
        }
    }
}

impl FromStr for ActionPattern {
    type Err = ParsePatternError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > bounds::MAX_PATTERN_LEN {
            return Err(ParsePatternError::TooLong {
                max: bounds::MAX_PATTERN_LEN,
            });
        }
        let Some((resource_s, verb_s)) = s.split_once(':') else {
            return Err(ParsePatternError::Structure);
        };
        if resource_s.is_empty() || verb_s.is_empty() || verb_s.contains(':') {
            return Err(ParsePatternError::Structure);
        }
        let verb = if verb_s == "*" {
            VerbPattern::Any
        } else {
            VerbPattern::Exact(verb_s.parse::<Verb>()?)
        };
        let resource = if resource_s == "*" {
            ResourcePattern::Any
        } else if let Some(stem) = resource_s.strip_suffix(".*") {
            if !is_valid_segment_path(stem) {
                return Err(ParsePatternError::InvalidResourcePattern);
            }
            ResourcePattern::Subtree(stem.to_string())
        } else {
            if !is_valid_segment_path(resource_s) {
                return Err(ParsePatternError::InvalidResourcePattern);
            }
            ResourcePattern::Exact(resource_s.to_string())
        };
        Ok(Self { resource, verb })
    }
}

impl serde::Serialize for ActionPattern {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ActionPattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::actions;

    fn pat(s: &str) -> ActionPattern {
        s.parse().expect("pattern parses")
    }

    fn action(s: &str) -> Action {
        s.parse().expect("action parses")
    }

    #[test]
    fn a10_pattern_positives_parse_and_round_trip() {
        for good in [
            "hosts:read",
            "settings.*:manage",
            "*:read",
            "hosts:*",
            "*:*",
            "plugin.package-manager.*:manage",
            "surface.*:use",
            "system.*:*",
            "unknownthing:read",
        ] {
            assert_eq!(pat(good).to_string(), good);
        }
    }

    #[test]
    fn pattern_grammar_negatives() {
        use ParsePatternError as E;
        type Check = fn(&E) -> bool;
        let cases: &[(&str, Check)] = &[
            ("*.foo:read", |e| matches!(e, E::InvalidResourcePattern)),
            ("a.*.b:read", |e| matches!(e, E::InvalidResourcePattern)),
            ("*x:read", |e| matches!(e, E::InvalidResourcePattern)),
            ("x*:read", |e| matches!(e, E::InvalidResourcePattern)),
            ("**:read", |e| matches!(e, E::InvalidResourcePattern)),
            (".*:read", |e| matches!(e, E::InvalidResourcePattern)),
            ("settings.*.auth:read", |e| {
                matches!(e, E::InvalidResourcePattern)
            }),
            ("Set.*:read", |e| matches!(e, E::InvalidResourcePattern)),
            ("hosts:frobnicate", |e| matches!(e, E::UnknownVerb(_))),
            ("hosts", |e| matches!(e, E::Structure)),
            (":*", |e| matches!(e, E::Structure)),
            ("hosts:", |e| matches!(e, E::Structure)),
            ("a:*:b", |e| matches!(e, E::Structure)),
        ];
        for (input, check) in cases {
            let err = input.parse::<ActionPattern>().expect_err("must reject");
            assert!(check(&err), "wrong error {err:?} for {input:?}");
        }
    }

    #[test]
    fn a11_subtree_never_matches_its_root() {
        assert!(!pat("settings.*:read").matches(&action("settings:read")));
        assert!(pat("settings.*:manage").matches(&action("settings.auth:manage")));
        assert!(pat("hosts.*:manage").matches(&action("hosts.tags:manage")));
    }

    #[test]
    fn a12_system_exclusion() {
        let system_action = action("system.settings:manage");
        assert!(!pat("*:*").matches(&system_action));
        assert!(!pat("*:manage").matches(&system_action));
        assert!(pat("system.*:*").matches(&system_action));
        assert!(pat("system.settings:manage").matches(&system_action));
        // system.* matches ONLY the system plane.
        assert!(!pat("system.*:read").matches(&action("hosts:read")));
    }

    #[test]
    fn a13_wildcard_reaches_dynamic() {
        assert!(pat("*:manage").matches(&action("plugin.package-manager.apt:manage")));
        assert!(pat("*:use").matches(&action("surface.proxmox.hosts:use")));
        assert!(pat("plugin.*:manage").matches(&action("plugin.package-manager.apt:manage")));
    }

    #[test]
    fn segment_boundary_matching() {
        let p = pat("plugin.package-manager.*:manage");
        assert!(p.matches(&action("plugin.package-manager.apt:manage")));
        assert!(!p.matches(&action("plugin.package-managerx.apt:manage")));
    }

    #[test]
    fn a14_matchability() {
        // Rejected: provably unmatchable against the matrix.
        assert!(!pat("hosts:approve").can_match_any());
        assert!(!pat("settings.*:trigger").can_match_any());
        assert!(!pat("unknownthing:read").can_match_any());
        // Accepted: catalog matches or dynamic-reachable.
        assert!(pat("hosts:read").can_match_any());
        assert!(pat("hosts.*:manage").can_match_any());
        assert!(pat("*:approve").can_match_any());
        assert!(pat("plugin.foo:approve").can_match_any());
        assert!(pat("surface.*:use").can_match_any());
    }

    #[test]
    fn parse_validate_split_pin() {
        // Grammar-valid non-catalog Exact PARSES (FromStr never consults
        // the catalog on the resource side), then write-time rejects.
        let p = pat("unknownthing:read");
        assert!(!p.can_match_any());
    }

    #[test]
    fn matched_catalog_actions_feed_rule_three() {
        let matched: Vec<&str> = pat("hosts.*:manage")
            .matched_catalog_actions()
            .map(|(_, ve)| ve.action_str)
            .collect();
        assert_eq!(matched, ["hosts.tags:manage"]);
        // Dynamic-only pattern: empty iterator but reaches_dynamic —
        // rule 3 must NOT treat this as a vacuous pass.
        let dynamic_only = pat("plugin.*:trigger");
        assert_eq!(dynamic_only.matched_catalog_actions().count(), 0);
        assert!(dynamic_only.reaches_dynamic());
    }

    #[test]
    fn matches_typed_constants() {
        assert!(pat("hosts:*").matches(&actions::HOSTS_READ));
        assert!(pat("*:read").matches(&actions::HOSTS_READ));
        assert!(!pat("*:read").matches(&actions::SYSTEM_AUDIT_READ));
    }

    #[test]
    fn serde_string_form() {
        let p = pat("settings.*:manage");
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(json, r#""settings.*:manage""#);
        let back: ActionPattern = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, p);
        assert!(
            serde_json::from_str::<ActionPattern>(r#""a.*.b:read""#).is_err(),
            "malformed pattern must fail deserialization"
        );
    }

    #[test]
    fn validate_patterns_bounds_and_matchability() {
        use crate::access::bounds::{MAX_PATTERNS_PER_GRANT, PatternSetError, validate_patterns};
        let ok: Vec<ActionPattern> = (0..MAX_PATTERNS_PER_GRANT)
            .map(|_| pat("hosts:read"))
            .collect();
        assert!(validate_patterns(&ok).is_ok(), "at-bound count passes");
        let too_many: Vec<ActionPattern> = (0..=MAX_PATTERNS_PER_GRANT)
            .map(|_| pat("hosts:read"))
            .collect();
        assert!(matches!(
            validate_patterns(&too_many).expect_err("over bound"),
            PatternSetError::TooMany { .. }
        ));
        let unmatchable = [pat("hosts:read"), pat("hosts:approve")];
        assert!(matches!(
            validate_patterns(&unmatchable).expect_err("unmatchable"),
            PatternSetError::Unmatchable { index: 1, .. }
        ));
    }
}
