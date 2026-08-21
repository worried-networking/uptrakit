use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::bounds;
use super::decision::TargetRef;
use super::pattern::ActionPattern;

/// Resource selector on a grant: which hosts / software items a
/// selector-capable action may target. `All` is the M1 default; write-path
/// validation (rules 3–5) lives in this module plus
/// `uptrakit-shared-db::access_grants`. Non-`All` selectors validate fully
/// but stay write-gated until M2.3 lifts `SelectorPhaseGate`.
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

/// The narrowing axis a non-`All` selector filters on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorAxis {
    Tags,
    Hosts,
    Software,
    Items,
}

impl SelectorAxis {
    /// The axis of `selector`, or `None` for `Selector::All`.
    #[must_use]
    pub fn of(selector: &Selector) -> Option<Self> {
        match selector {
            Selector::All => None,
            Selector::Tags { .. } => Some(Self::Tags),
            Selector::Hosts { .. } => Some(Self::Hosts),
            Selector::Software { .. } => Some(Self::Software),
            Selector::Items { .. } => Some(Self::Items),
        }
    }

    /// Stable lowercase label, used in error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tags => "tags",
            Self::Hosts => "hosts",
            Self::Software => "software",
            Self::Items => "items",
        }
    }
}

impl std::fmt::Display for SelectorAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Rule-3 (capability-level) rejection for a grant's selector.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectorLevelError {
    /// A concrete catalog action matched by `pattern` does not admit this
    /// selector axis (per its `SelectorSupport`).
    #[error("pattern `{pattern}` matches `{action}`, which does not admit a {axis} selector")]
    NotAdmitted {
        pattern: String,
        action: &'static str,
        axis: SelectorAxis,
    },
    /// The pattern reaches dynamically-registered actions, which never
    /// admit non-`All` selectors.
    #[error("pattern `{pattern}` reaches dynamic actions, which never admit non-All selectors")]
    DynamicPattern { pattern: String },
}

/// Rule 3: every action reachable by every pattern must admit the
/// selector's axis. `Selector::All` always passes.
pub fn validate_selector_level(
    patterns: &[ActionPattern],
    selector: &Selector,
) -> Result<(), SelectorLevelError> {
    let Some(axis) = SelectorAxis::of(selector) else {
        return Ok(());
    };
    for pattern in patterns {
        if pattern.reaches_dynamic() {
            return Err(SelectorLevelError::DynamicPattern {
                pattern: pattern.to_string(),
            });
        }
        for (_, verb_entry) in pattern.matched_catalog_actions() {
            if !verb_entry.selector_support.admits(selector) {
                return Err(SelectorLevelError::NotAdmitted {
                    pattern: pattern.to_string(),
                    action: verb_entry.action_str,
                    axis,
                });
            }
        }
    }
    Ok(())
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
    /// A narrowing selector with an empty id list matches nothing — reject
    /// at write time rather than storing a dead grant.
    #[error("empty {kind} id list — a narrowing selector must name at least one id")]
    EmptyIds { kind: &'static str },
}

impl Selector {
    /// Canonicalize in place: sort and dedup each id list. The write path
    /// calls this before `validate()` so bounds apply to the deduped list.
    pub fn canonicalize(&mut self) {
        match self {
            Self::All => {}
            Self::Tags { ids }
            | Self::Hosts { ids }
            | Self::Software { ids }
            | Self::Items { ids } => {
                ids.sort_unstable();
                ids.dedup();
            }
        }
    }

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
        if len == 0 {
            return Err(SelectorValidationError::EmptyIds { kind });
        }
        if len > max {
            return Err(SelectorValidationError::TooManyIds {
                kind,
                max,
                actual: len,
            });
        }
        Ok(())
    }

    /// Decision-time matcher: does this selector cover `target`?
    ///
    /// Pure — `host_tags` is the caller-resolved set of tag ids assigned to
    /// the target's host (`Selector::Tags` is the only variant that reads
    /// it). Fail-closed: axes that do not apply to the target's shape deny
    /// (`Software`/`Items` never cover a bare `TargetRef::Host`), and both
    /// matches are exhaustive so a future variant of either enum forces a
    /// reviewed decision here instead of a silent allow.
    #[must_use]
    pub fn covers(&self, target: &TargetRef, host_tags: &BTreeSet<Uuid>) -> bool {
        match self {
            Self::All => true,
            Self::Tags { ids } => ids.iter().any(|id| host_tags.contains(id)),
            Self::Hosts { ids } => ids.contains(&target.host_id()),
            Self::Software { ids } => match target {
                TargetRef::Host(_) => false,
                TargetRef::HostSoftwareItem {
                    software_item_id, ..
                } => ids.contains(software_item_id),
            },
            Self::Items { ids } => match target {
                TargetRef::Host(_) => false,
                TargetRef::HostSoftwareItem { id, .. } => ids.contains(id),
            },
        }
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

    #[test]
    fn canonicalize_sorts_and_dedups() {
        let mut selector = Selector::Hosts {
            ids: vec![
                Uuid::from_u128(3),
                Uuid::from_u128(1),
                Uuid::from_u128(3),
                Uuid::from_u128(2),
            ],
        };
        selector.canonicalize();
        assert_eq!(
            selector,
            Selector::Hosts {
                ids: vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
            }
        );
        let mut all = Selector::All;
        all.canonicalize();
        assert_eq!(all, Selector::All);
    }

    #[test]
    fn validate_rejects_empty_id_lists() {
        let cases: &[(Selector, &'static str)] = &[
            (Selector::Tags { ids: vec![] }, "tag"),
            (Selector::Hosts { ids: vec![] }, "host"),
            (Selector::Software { ids: vec![] }, "software item"),
            (Selector::Items { ids: vec![] }, "host software item"),
        ];
        for (selector, kind) in cases {
            assert_eq!(
                selector.validate(),
                Err(SelectorValidationError::EmptyIds { kind }),
                "{selector:?}"
            );
        }
    }

    #[test]
    fn canonicalization_applies_before_bounds() {
        // MAX distinct ids plus one duplicate: over the bound raw, inside
        // it after canonicalize().
        let mut ids: Vec<Uuid> = (0..bounds::MAX_SELECTOR_TAG_IDS)
            .map(|i| Uuid::from_u128(i as u128))
            .collect();
        ids.push(Uuid::from_u128(0));
        let mut selector = Selector::Tags { ids };
        selector.canonicalize();
        assert_eq!(selector.validate(), Ok(()));

        let over: Vec<Uuid> = (0..=bounds::MAX_SELECTOR_TAG_IDS)
            .map(|i| Uuid::from_u128(i as u128))
            .collect();
        let mut selector = Selector::Tags { ids: over };
        selector.canonicalize();
        assert!(matches!(
            selector.validate(),
            Err(SelectorValidationError::TooManyIds { .. })
        ));
    }

    #[test]
    fn covers_full_matrix() {
        let tag = Uuid::from_u128(1);
        let host_a = Uuid::from_u128(2);
        let host_b = Uuid::from_u128(3);
        let sw_x = Uuid::from_u128(4);
        let sw_y = Uuid::from_u128(5);
        let t_host_a = TargetRef::Host(host_a);
        let t_host_b = TargetRef::Host(host_b);
        let t_ax = TargetRef::HostSoftwareItem {
            id: Uuid::from_u128(6),
            host_id: host_a,
            software_item_id: sw_x,
        };
        let t_ay = TargetRef::HostSoftwareItem {
            id: Uuid::from_u128(7),
            host_id: host_a,
            software_item_id: sw_y,
        };
        let t_bx = TargetRef::HostSoftwareItem {
            id: Uuid::from_u128(8),
            host_id: host_b,
            software_item_id: sw_x,
        };
        let link_ax = Uuid::from_u128(6);
        let tagged = BTreeSet::from([tag]);
        let untagged = BTreeSet::new();

        let cases: &[(Selector, &TargetRef, &BTreeSet<Uuid>, bool)] = &[
            (Selector::All, &t_host_a, &untagged, true),
            (Selector::All, &t_ax, &untagged, true),
            (Selector::Tags { ids: vec![tag] }, &t_host_a, &tagged, true),
            (
                Selector::Tags { ids: vec![tag] },
                &t_host_a,
                &untagged,
                false,
            ),
            (Selector::Tags { ids: vec![tag] }, &t_ax, &tagged, true),
            (Selector::Tags { ids: vec![tag] }, &t_bx, &untagged, false),
            (Selector::Tags { ids: vec![] }, &t_host_a, &tagged, false),
            (
                Selector::Hosts { ids: vec![host_a] },
                &t_host_a,
                &untagged,
                true,
            ),
            (
                Selector::Hosts { ids: vec![host_a] },
                &t_host_b,
                &untagged,
                false,
            ),
            (
                Selector::Hosts { ids: vec![host_a] },
                &t_ax,
                &untagged,
                true,
            ),
            (
                Selector::Hosts { ids: vec![host_a] },
                &t_bx,
                &untagged,
                false,
            ),
            (
                Selector::Software { ids: vec![sw_x] },
                &t_ax,
                &untagged,
                true,
            ),
            (
                Selector::Software { ids: vec![sw_x] },
                &t_ay,
                &untagged,
                false,
            ),
            (
                Selector::Software { ids: vec![sw_x] },
                &t_bx,
                &untagged,
                true,
            ),
            (
                Selector::Software { ids: vec![sw_x] },
                &t_host_a,
                &untagged,
                false,
            ),
            (
                Selector::Items { ids: vec![link_ax] },
                &t_ax,
                &untagged,
                true,
            ),
            (
                Selector::Items { ids: vec![link_ax] },
                &t_ay,
                &untagged,
                false,
            ),
            (
                Selector::Items { ids: vec![link_ax] },
                &t_bx,
                &untagged,
                false,
            ),
            (
                Selector::Items { ids: vec![link_ax] },
                &t_host_a,
                &untagged,
                false,
            ),
        ];
        for (selector, target, host_tags, expected) in cases {
            assert_eq!(
                selector.covers(target, host_tags),
                *expected,
                "{selector:?} covering {target:?}"
            );
        }
    }

    #[test]
    fn selector_level_matrix() {
        let id = Uuid::from_u128(1);
        let tags = Selector::Tags { ids: vec![id] };
        let hosts = Selector::Hosts { ids: vec![id] };
        let software = Selector::Software { ids: vec![id] };
        let items = Selector::Items { ids: vec![id] };
        let cases: &[(&str, &Selector, bool)] = &[
            ("hosts:read", &tags, true),
            ("hosts:read", &hosts, true),
            ("hosts:read", &software, false),
            ("hosts:read", &items, false),
            ("hosts:update", &tags, true),
            ("hosts:delete", &hosts, true),
            ("checks:trigger", &items, true),
            ("updates:trigger", &items, true),
            ("access:manage", &tags, false),
            ("*:trigger", &items, false),
            ("plugin.package-manager.*:manage", &tags, false),
            ("plugin.package-manager.*:manage", &Selector::All, true),
            ("checks:trigger", &Selector::All, true),
        ];
        for (pattern_str, selector, expected_ok) in cases {
            let patterns = vec![
                pattern_str
                    .parse::<ActionPattern>()
                    .expect("valid test pattern"),
            ];
            assert_eq!(
                validate_selector_level(&patterns, selector).is_ok(),
                *expected_ok,
                "{pattern_str} with {selector:?}"
            );
        }
    }

    #[test]
    fn selector_level_error_names_pattern_and_action() {
        let patterns = vec![
            "hosts:read"
                .parse::<ActionPattern>()
                .expect("valid pattern"),
        ];
        let err = validate_selector_level(
            &patterns,
            &Selector::Items {
                ids: vec![Uuid::from_u128(1)],
            },
        )
        .expect_err("Items not admitted on hosts:read");
        assert_eq!(
            err,
            SelectorLevelError::NotAdmitted {
                pattern: "hosts:read".to_string(),
                action: "hosts:read",
                axis: SelectorAxis::Items,
            }
        );

        let dynamic = vec![
            "plugin.package-manager.*:manage"
                .parse::<ActionPattern>()
                .expect("valid pattern"),
        ];
        let err = validate_selector_level(
            &dynamic,
            &Selector::Tags {
                ids: vec![Uuid::from_u128(1)],
            },
        )
        .expect_err("dynamic pattern never admits non-All");
        assert_eq!(
            err,
            SelectorLevelError::DynamicPattern {
                pattern: "plugin.package-manager.*:manage".to_string(),
            }
        );
    }
}
