//! Guard: first-party surface / interaction / data-source IDs follow the
//! kebab-case + method convention (ADR-0031, docs/development/surfaces.md).
//! Full-catalog run:
//! `cargo test -p uptrakit-plugin-infrastructure-registry --features notifications-email,notifications-telegram,notifications-webhook`
//!
//! Guarantee scope: naming rules are checked for every registration present
//! in the compiled catalog. Presence itself (including proxmox's, which is
//! feature-invariant since ADR-0032) is guarded by
//! `tests/contribution_monotonicity_guard.rs` — the single assertion site;
//! do not add presence checks here.

use std::collections::BTreeSet;

use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, InstancePluginStates, PluginSurfaceOps, all_descriptors, build_catalog,
};
use uptrakit_surfaces as surfaces;

fn is_kebab(id: &str) -> bool {
    id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.ends_with('-')
        && !id.contains("--")
}

fn is_dotted_kebab(id: &str) -> bool {
    !id.is_empty() && id.split('.').all(is_kebab)
}

#[test]
fn first_party_surface_ids_follow_naming_convention() {
    let mut saw_notifications_email = false;
    for desc in all_descriptors() {
        let Some(ops) = desc.surfaces else { continue };
        for registration in (ops.registrations)() {
            for surface in &registration.surfaces {
                let surface_id = surface.descriptor.surface_id.as_str();
                saw_notifications_email |= surface_id == "notifications.email";
                assert!(
                    is_dotted_kebab(surface_id),
                    "surface id `{surface_id}` violates the convention"
                );
                for interaction in &surface.interactions {
                    let id = interaction.descriptor().interaction_id.as_str();
                    assert!(
                        is_kebab(id),
                        "interaction id `{id}` on `{surface_id}` violates the convention"
                    );
                }
                for ds in &surface.data_sources {
                    let ds_id = ds.data_source_id.as_str();
                    assert!(
                        is_kebab(ds_id),
                        "data-source id `{ds_id}` on `{surface_id}` violates the convention"
                    );
                    if let surfaces::DataSourceKind::ProviderQuery { operation_id } = &ds.kind {
                        assert_eq!(
                            ds_id,
                            operation_id.as_str(),
                            "data-source `{ds_id}` on `{surface_id}` must reuse its ProviderQuery operation_id"
                        );
                        let has_get_pair = surface.interactions.iter().any(|i| {
                            let d = i.descriptor();
                            d.interaction_id.as_str() == operation_id.as_str()
                                && d.effective_http_method() == surfaces::InteractionHttpMethod::Get
                        });
                        assert!(
                            has_get_pair,
                            "ProviderQuery op `{operation_id}` on `{surface_id}` has no GET interaction"
                        );
                    }
                }
            }
        }
    }
    if cfg!(feature = "notifications-email") {
        assert!(
            saw_notifications_email,
            "notifications.email absent from catalog despite feature"
        );
    }
}

/// ADR-0034: a Plugin-kind provider id IS the owning descriptor's type id.
/// Consumes the PRODUCTION aggregation (`PluginCatalog::surface_registrations()`,
/// the accessor controller boot feeds into `SurfaceRegistry`) — never a
/// test-side `to_wire()` re-derivation, which would supply the very value it
/// asserts. Per-family presence stays in `contribution_monotonicity_guard.rs`
/// (single assertion site); the non-emptiness check below only prevents this
/// guard from passing green on an empty catalog. `all_disabled` boot states
/// are fine: no instance-scoped plugin declares surfaces today, and the
/// subset assertion is filter-agnostic.
#[test]
fn plugin_provider_ids_equal_descriptor_type_ids() {
    let catalog = build_catalog(
        &CatalogConfig::default(),
        InstancePluginStates::all_disabled(),
    )
    .expect("catalog builds from compiled-in descriptors");
    let type_ids: BTreeSet<&str> = all_descriptors()
        .iter()
        .filter(|desc| desc.surfaces.is_some())
        .map(|desc| desc.type_id)
        .collect();

    let registrations = catalog.surface_registrations();
    assert!(
        !registrations.is_empty(),
        "compiled-in catalog emitted zero surface registrations — guard is vacuous"
    );
    for registration in &registrations {
        let provider_id = registration.provider.provider_id.as_str();
        assert!(
            type_ids.contains(provider_id),
            "provider id `{provider_id}` is not a surface-bearing descriptor type id"
        );
        assert_eq!(
            registration.provider.provider_kind,
            surfaces::ProviderKind::Plugin,
            "plugin registration `{provider_id}` must carry ProviderKind::Plugin"
        );
        assert_eq!(
            registration.provider.provider_namespace, "plugin",
            "plugin registration `{provider_id}` must carry the `plugin` namespace"
        );
    }
}

/// ADR-0034: the admission roots are reserved at the authoring boundary for
/// every plugin type id, not just surface-bearing ones — a plugin named
/// `service.foo` would pass the grammar yet be rejected at `SurfaceRegistry`
/// admission, silently dropping its surfaces at controller startup.
#[test]
fn no_descriptor_type_id_uses_a_reserved_admission_root() {
    for desc in all_descriptors() {
        let type_id = desc.type_id;
        assert!(
            !type_id.starts_with("service.") && !type_id.starts_with("builtin."),
            "plugin type id `{type_id}` uses a reserved provider-id admission root"
        );
    }
}

/// ADR-0034 admission roots (`service.`, `builtin.`) are deliberately absent
/// from this allowlist and must never be added: a plugin type id in those
/// namespaces would pass the type-id grammar yet be rejected at
/// `SurfaceRegistry` admission, silently dropping its surfaces at controller
/// startup, far from the authoring site.
const KNOWN_TYPE_ID_CATEGORIES: &[&str] = &[
    "package-manager",
    "releases",
    "hook",
    "infrastructure",
    "generic",
    "discovery",
    "enhancement",
    "notifications",
    "test",
];

/// ADR-0031 (plugin-type amendment): plugin type IDs are dot-separated kebab
/// segments with a known category first segment.
#[test]
fn all_descriptor_type_ids_follow_dotted_kebab_grammar() {
    let mut saw_generic_shell = false;
    for desc in all_descriptors() {
        let type_id = desc.type_id;
        saw_generic_shell |= type_id == "generic.shell";
        assert!(
            type_id.split('.').all(is_kebab),
            "plugin type id `{type_id}` violates the dotted-kebab grammar"
        );
        assert!(
            type_id.split('.').count() >= 2,
            "plugin type id `{type_id}` lacks a category segment"
        );
        let category = type_id.split('.').next().unwrap_or_default();
        assert!(
            KNOWN_TYPE_ID_CATEGORIES.contains(&category),
            "plugin type id `{type_id}` has unknown category `{category}`"
        );
    }
    assert!(
        saw_generic_shell,
        "always-on generic.shell missing — catalog empty or stripped"
    );
}
