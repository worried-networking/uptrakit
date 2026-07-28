//! Bidirectional executor guard (spec D5, ADR-0028): the controller-local
//! executor const table and the catalog's unified registrations must agree.
//! Scope: proves interaction EXISTENCE and DELIVERY KIND per pair — not
//! audit-tier value (Tiers 2 and 3 both map to PluginHandled).

use uptrakit_plugin_infrastructure_registry as registry;
use uptrakit_surface_proxy::{CONTROLLER_LOCAL_EXECUTOR_TABLE, ExecutorTier};

// Returns the Result alias `build_catalog` actually returns:
// `uptrakit_plugin_infrastructure_core::Result` (registry/src/lib.rs:108) —
// NOT `registry::Result`; they are different aliases and no cross-boundary
// conversion exists. `.expect()` lives only inside the `#[test]` fns below:
// clippy's `allow-expect-in-tests` covers ONLY `#[test]`-annotated fns, not
// plain helpers in integration tests (coding-standards.md test-mode
// exemptions).
fn catalog_deliveries() -> uptrakit_plugin_infrastructure_core::Result<
    Vec<(
        String,
        String,
        registry::InteractionHttpMethod,
        registry::InteractionDeliveryKind,
    )>,
> {
    let catalog = registry::build_catalog(
        &registry::CatalogConfig::default(),
        registry::InstancePluginStates::all_disabled(),
    )?;
    Ok(catalog.interaction_deliveries())
}

#[test]
fn every_executor_table_pair_is_registered_with_matching_delivery() {
    let deliveries = catalog_deliveries().expect("catalog builds");
    assert!(
        !deliveries.is_empty(),
        "green-on-empty: no unified registrations found"
    );
    for (surface, interaction, method, tier) in CONTROLLER_LOCAL_EXECUTOR_TABLE {
        // Feature-gated plugins: their rows only assert when compiled in.
        let compiled = match *surface {
            s if s.starts_with("notifications.telegram") => {
                cfg!(feature = "notifications-telegram")
            }
            s if s.starts_with("notifications.email") => cfg!(feature = "notifications-email"),
            _ => true,
        };
        if !compiled {
            continue;
        }
        let expected_kind = match tier {
            ExecutorTier::ControllerExecutes => {
                registry::InteractionDeliveryKind::ControllerExecutor
            }
            ExecutorTier::PluginWithAudit => registry::InteractionDeliveryKind::PluginHandled,
            _ => panic!("unmapped executor tier for ({surface}, {interaction})"),
        };
        assert!(
            deliveries.iter().any(|(s, i, m, k)| s == surface
                && i == interaction
                && m == method
                && *k == expected_kind),
            "executor table row ({surface}, {interaction}, {method}, {tier:?}) has no matching \
             registration"
        );
    }
}

#[test]
fn every_controller_executor_registration_has_an_executor_table_row() {
    let deliveries = catalog_deliveries().expect("catalog builds");
    let controller_executed: Vec<_> = deliveries
        .iter()
        .filter(|(_, _, _, kind)| *kind == registry::InteractionDeliveryKind::ControllerExecutor)
        .collect();
    // Green-on-empty protection: webhook is unconditionally in web-api's
    // catalog and registers four ControllerExecutor interactions.
    assert!(
        controller_executed
            .iter()
            .any(|(s, i, _, _)| s == "notifications.webhook" && i == "channels"),
        "known member (notifications.webhook, channels) missing — catalog wiring broken"
    );
    // Expression-position cfg!() per coding-standards.md "Additive patterns
    // in tests" — matches the `compiled` idiom in the sibling test above.
    if cfg!(feature = "notifications-telegram") {
        assert!(
            controller_executed
                .iter()
                .any(|(s, i, _, _)| s == "notifications.telegram" && i == "channels"),
            "known member (notifications.telegram, channels) missing"
        );
    }
    if cfg!(feature = "notifications-email") {
        assert!(
            controller_executed
                .iter()
                .any(|(s, i, _, _)| s == "notifications.email" && i == "channels"),
            "known member (notifications.email, channels) missing"
        );
    }
    for (surface, interaction, method, _) in &controller_executed {
        assert!(
            CONTROLLER_LOCAL_EXECUTOR_TABLE
                .iter()
                .any(|(s, i, m, tier)| s == surface
                    && i == interaction
                    && m == method
                    && *tier == ExecutorTier::ControllerExecutes),
            "registered ControllerExecutor interaction ({surface}, {interaction}, {method}) has \
             no Tier-1 executor table row — it is registered but unexecutable"
        );
    }
}

#[test]
fn known_plugin_handled_members_are_registered() {
    let deliveries = catalog_deliveries().expect("catalog builds");
    let expected = vec![
        ("docker.item-host-actions", "switch-tag"),
        ("proxmox.hosts", "discover"),
    ];
    for (surface, interaction) in expected {
        assert!(
            deliveries.iter().any(|(s, i, _, k)| s == surface
                && i == interaction
                && *k == registry::InteractionDeliveryKind::PluginHandled),
            "known member ({surface}, {interaction}) missing"
        );
    }
}
