use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::bounds;

/// Resource selector on a grant: which hosts / software items a
/// selector-capable action may target. `All` is the M1 default; write-path
/// acceptance of the narrowing variants lands in M2 (grant validation, not
/// this type).
///
/// Serialized form is the `access_grants.selector` storage JSON
/// (`06-grant-model.md` §Storage schema): `{"type":"all"}`,
/// `{"type":"tags","ids":[…]}`, … — uniform `ids` field. Unknown extra
/// keys are ignored on deserialize (serde internally-tagged enums cannot
/// `deny_unknown_fields`); that is safe because an ignored key can never
/// broaden authority and a missing/mistyped `ids` still fails as a
/// missing field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Selector {
    /// Whole tenant scope (today's behavior).
    All,
    /// Host axis: hosts carrying ANY of these tags (`host_tags.id`).
    Tags { ids: Vec<Uuid> },
    /// Host axis: explicit host set (`hosts.id`).
    Hosts { ids: Vec<Uuid> },
    /// Software axis: these software items on any host (`software_items.id`).
    Software { ids: Vec<Uuid> },
    /// Exact (host, software item) pairs (`host_software_items.id`).
    Items { ids: Vec<Uuid> },
}

/// Selector-capability level of an action — first-class catalog metadata
/// (`05-action-model.md` §Selector-capable actions). Each level admits
/// the previous levels' selector kinds.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SelectorSupport {
    /// Only `Selector::All` is valid.
    None,
    /// Host-axis selectors (`Tags`/`Hosts`) are valid.
    Host,
    /// Host-axis plus software-axis selectors (`Software`/`Items`) are valid.
    HostAndSoftware,
}

impl SelectorSupport {
    /// Whether an action at this support level admits `selector`.
    pub fn admits(&self, selector: &Selector) -> bool {
        match selector {
            Selector::All => true,
            Selector::Tags { .. } | Selector::Hosts { .. } => {
                matches!(self, Self::Host | Self::HostAndSoftware)
            }
            Selector::Software { .. } | Selector::Items { .. } => {
                matches!(self, Self::HostAndSoftware)
            }
        }
    }
}

/// Error returned when a [`Selector`] exceeds its bounded sizes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectorValidationError {
    /// A selector variant carries more IDs than its bound allows.
    #[error("too many {kind} ids: {actual} exceeds the maximum of {max}")]
    TooManyIds {
        kind: &'static str,
        max: usize,
        actual: usize,
    },
}

impl Selector {
    /// Enforces the per-variant ID count bounds ([`bounds`]).
    pub fn validate(&self) -> Result<(), SelectorValidationError> {
        let (len, max, kind) = match self {
            Selector::All => return Ok(()),
            Selector::Tags { ids } => (ids.len(), bounds::MAX_SELECTOR_TAG_IDS, "tag"),
            Selector::Hosts { ids } => (ids.len(), bounds::MAX_SELECTOR_HOST_IDS, "host"),
            Selector::Software { ids } => (
                ids.len(),
                bounds::MAX_SELECTOR_SOFTWARE_IDS,
                "software item",
            ),
            Selector::Items { ids } => (
                ids.len(),
                bounds::MAX_SELECTOR_ITEM_IDS,
                "host software item",
            ),
        };
        if len > max {
            return Err(SelectorValidationError::TooManyIds {
                kind,
                max,
                actual: len,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuids(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::nil()).collect()
    }

    #[test]
    fn golden_json_shapes() {
        let id = Uuid::nil();
        let cases: &[(Selector, &str)] = &[
            (Selector::All, r#"{"type":"all"}"#),
            (
                Selector::Tags { ids: vec![id] },
                r#"{"type":"tags","ids":["00000000-0000-0000-0000-000000000000"]}"#,
            ),
            (
                Selector::Hosts { ids: vec![id] },
                r#"{"type":"hosts","ids":["00000000-0000-0000-0000-000000000000"]}"#,
            ),
            (
                Selector::Software { ids: vec![id] },
                r#"{"type":"software","ids":["00000000-0000-0000-0000-000000000000"]}"#,
            ),
            (
                Selector::Items { ids: vec![id] },
                r#"{"type":"items","ids":["00000000-0000-0000-0000-000000000000"]}"#,
            ),
        ];
        for (selector, json) in cases {
            assert_eq!(&serde_json::to_string(selector).expect("serialize"), json);
            let back: Selector = serde_json::from_str(json).expect("deserialize");
            assert_eq!(&back, selector);
        }
    }

    #[test]
    fn unknown_type_tag_rejected() {
        assert!(
            serde_json::from_str::<Selector>(r#"{"type":"everything"}"#).is_err(),
            "unknown type tag must be rejected"
        );
    }

    #[test]
    fn mistyped_ids_field_rejected() {
        assert!(
            serde_json::from_str::<Selector>(r#"{"type":"tags","idz":[]}"#).is_err(),
            "mistyped ids field must fail as missing field"
        );
    }

    #[test]
    fn admits_full_matrix() {
        let all = Selector::All;
        let tags = Selector::Tags { ids: vec![] };
        let hosts = Selector::Hosts { ids: vec![] };
        let software = Selector::Software { ids: vec![] };
        let items = Selector::Items { ids: vec![] };
        // (support, selector, expected)
        let cases: &[(SelectorSupport, &Selector, bool)] = &[
            (SelectorSupport::None, &all, true),
            (SelectorSupport::None, &tags, false),
            (SelectorSupport::None, &hosts, false),
            (SelectorSupport::None, &software, false),
            (SelectorSupport::None, &items, false),
            (SelectorSupport::Host, &all, true),
            (SelectorSupport::Host, &tags, true),
            (SelectorSupport::Host, &hosts, true),
            (SelectorSupport::Host, &software, false),
            (SelectorSupport::Host, &items, false),
            (SelectorSupport::HostAndSoftware, &all, true),
            (SelectorSupport::HostAndSoftware, &tags, true),
            (SelectorSupport::HostAndSoftware, &hosts, true),
            (SelectorSupport::HostAndSoftware, &software, true),
            (SelectorSupport::HostAndSoftware, &items, true),
        ];
        for (support, selector, expected) in cases {
            assert_eq!(
                support.admits(selector),
                *expected,
                "{support:?} admits {selector:?}"
            );
        }
    }

    #[test]
    fn validate_bounds_edges() {
        use crate::access::bounds::*;
        // (selector at bound, selector over bound)
        let cases: &[(Selector, Selector)] = &[
            (
                Selector::Tags {
                    ids: uuids(MAX_SELECTOR_TAG_IDS),
                },
                Selector::Tags {
                    ids: uuids(MAX_SELECTOR_TAG_IDS + 1),
                },
            ),
            (
                Selector::Hosts {
                    ids: uuids(MAX_SELECTOR_HOST_IDS),
                },
                Selector::Hosts {
                    ids: uuids(MAX_SELECTOR_HOST_IDS + 1),
                },
            ),
            (
                Selector::Software {
                    ids: uuids(MAX_SELECTOR_SOFTWARE_IDS),
                },
                Selector::Software {
                    ids: uuids(MAX_SELECTOR_SOFTWARE_IDS + 1),
                },
            ),
            (
                Selector::Items {
                    ids: uuids(MAX_SELECTOR_ITEM_IDS),
                },
                Selector::Items {
                    ids: uuids(MAX_SELECTOR_ITEM_IDS + 1),
                },
            ),
        ];
        assert!(Selector::All.validate().is_ok(), "All has no bounds");
        for (at_bound, over_bound) in cases {
            assert!(at_bound.validate().is_ok(), "{at_bound:?} at bound");
            assert!(over_bound.validate().is_err(), "{over_bound:?} over bound");
        }
    }
}
