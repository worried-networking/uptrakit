//! Guard: first-party surface / interaction / data-source IDs follow the
//! kebab-case + method convention (ADR-0031, docs/development/surfaces.md).
//! Full-catalog run:
//! `cargo test -p uptrakit-plugin-infrastructure-registry --features notifications-email,notifications-telegram,notifications-webhook`
//!
//! Guarantee scope (feature-dependent, stated honestly): proxmox rows are
//! checked only when its registrations are populated in the compiled
//! feature set — presence is observed from the catalog, never from `cfg!`
//! on a foreign crate's feature (its `agent-infra` can be enabled by other
//! workspace crates through feature unification).

use uptrakit_plugin_infrastructure_registry::all_descriptors;
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
