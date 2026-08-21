//! Pure decision types returned by the controller's `AccessEngine` (M1.3).
//!
//! No DB dependencies — consumable by tests and by the engine's decision-time
//! target resolution, which resolves host tags at call time
//! (`authorize_target`, M2.1). Design record:
//! `docs/superpowers/specs/2026-07-28-access-engine-design.md`.

use std::collections::BTreeSet;

use uuid::Uuid;

/// A concrete object an action is evaluated against.
///
/// Carries `Host` and, since M2.1, `HostSoftwareItem`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetRef {
    /// A managed host, by id.
    Host(Uuid),
    /// A `(host, software item)` link row, by its `host_software_items` ids.
    HostSoftwareItem {
        /// The `host_software_items.id` link-row id.
        id: Uuid,
        /// The owning host's id.
        host_id: Uuid,
        /// The linked software item's id.
        software_item_id: Uuid,
    },
}

impl TargetRef {
    /// The host every target ultimately belongs to.
    #[must_use]
    pub fn host_id(&self) -> Uuid {
        match self {
            Self::Host(host_id) | Self::HostSoftwareItem { host_id, .. } => *host_id,
        }
    }
}

/// Outcome of an `authorize()` call.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The principal may perform the action.
    Allow,
    /// The principal may not perform the action.
    Deny(DenyReason),
}

/// Why an `authorize()` call denied.
///
/// **Diagnostic-internal — normative:** feeds traces/metrics and (M1.6b) deny
/// audit Events, never response bodies. HTTP surfaces return a generic 403
/// regardless of the reason (the D3 generic-403 rule).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(strum::EnumIter))]
pub enum DenyReason {
    /// No grant pattern matched the action.
    NoGrant,
    /// A grant matched but the credential's scope ceiling excludes the action.
    OutOfScope,
    /// A grant matched but its selector does not cover the target.
    /// Unreachable until M2.1 — every M1 grant carries `Selector::All`
    /// (types-complete, behavior-restricted).
    OutsideSelector,
    /// Dynamic action (`plugin.*` / `surface.*`) not registered with the
    /// engine's `DynamicActionRegistry` (fail-closed).
    UnknownAction,
}

impl DenyReason {
    /// Stable lowercase label for logs and the deny counter metric.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NoGrant => "no_grant",
            Self::OutOfScope => "out_of_scope",
            Self::OutsideSelector => "outside_selector",
            Self::UnknownAction => "unknown_action",
        }
    }
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn deny_reason_labels_are_stable_and_display_matches() {
        for reason in DenyReason::iter() {
            let label = match reason {
                DenyReason::NoGrant => "no_grant",
                DenyReason::OutOfScope => "out_of_scope",
                DenyReason::OutsideSelector => "outside_selector",
                DenyReason::UnknownAction => "unknown_action",
            };
            assert_eq!(reason.as_str(), label);
            assert_eq!(reason.to_string(), label);
        }
    }

    #[test]
    fn host_id_resolves_for_both_variants() {
        let host = Uuid::from_u128(1);
        assert_eq!(TargetRef::Host(host).host_id(), host);
        assert_eq!(
            TargetRef::HostSoftwareItem {
                id: Uuid::from_u128(2),
                host_id: host,
                software_item_id: Uuid::from_u128(3),
            }
            .host_id(),
            host
        );
    }
}

/// Visibility verdict for list/read filtering.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    /// Every row is visible.
    Full,
    /// Only rows covered by the union of matching grant selectors are
    /// visible. Never produced until M2.3 (types-complete,
    /// behavior-restricted — every M1 selector is `All`).
    Filter {
        /// Visible tag ids.
        tags: BTreeSet<Uuid>,
        /// Visible host ids.
        hosts: BTreeSet<Uuid>,
        /// Visible software-item ids.
        software: BTreeSet<Uuid>,
        /// Visible host-software-item ids.
        items: BTreeSet<Uuid>,
    },
    /// No rows are visible.
    None,
}
