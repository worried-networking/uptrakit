//! Guard: every declared `required_action` string (surface-level and
//! interaction-level) parses against the catalog `Action` type
//! (`crates/shared/types/src/access/catalog.rs`). Catalog validity is
//! enforced by the parse itself — the matrix rejects invalid
//! (resource, verb) pairs — so this guard's job is the field-write string
//! seams (`Some(actions::X.to_string())` refactors gone wrong) and future
//! regressions, not presence: presence/monotonicity stays the single
//! assertion site of `tests/contribution_monotonicity_guard.rs`.
//!
//! Iteration mirrors `surface_id_naming_guard.rs`: walks compiled-in
//! descriptors directly, not `PluginCatalog::surface_registrations()`.

use uptrakit_plugin_infrastructure_registry::all_descriptors;
use uptrakit_shared_types::access::Action;

#[test]
fn declared_required_action_values_parse_against_the_catalog() {
    let mut checked = 0usize;
    for desc in all_descriptors() {
        let Some(ops) = desc.surfaces else { continue };
        for registration in (ops.registrations)() {
            for surface in &registration.surfaces {
                let surface_id = surface.descriptor.surface_id.as_str();
                if let Some(required_action) = surface.descriptor.required_action.as_deref() {
                    checked += 1;
                    assert!(
                        required_action.parse::<Action>().is_ok(),
                        "surface `{surface_id}` declares invalid required_action `{required_action}`"
                    );
                }
                for interaction in &surface.interactions {
                    let descriptor = interaction.descriptor();
                    if let Some(required_action) = descriptor.required_action.as_deref() {
                        checked += 1;
                        let interaction_id = descriptor.interaction_id.as_str();
                        assert!(
                            required_action.parse::<Action>().is_ok(),
                            "interaction `{interaction_id}` on `{surface_id}` declares invalid required_action `{required_action}`"
                        );
                    }
                }
            }
        }
    }
    assert!(
        checked > 0,
        "no required_action values found in the compiled catalog — guard is vacuous"
    );
}
