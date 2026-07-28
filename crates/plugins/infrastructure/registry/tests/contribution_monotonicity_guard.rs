//! Guard: descriptor contributions are feature-monotonic (ADR-0032,
//! docs/development/coding-standards.md#feature-flags).
//!
//! Layer C: business-critical surfaces are present in the compiled catalog
//! unconditionally. Layer A: any descriptor that declares surface ops must
//! yield non-empty registrations.
//!
//! Lane honesty: both assertions are *valid* in every feature configuration,
//! but they only *catch* feature-predicate suppression in an `agent-infra`-ON
//! build (in the default lane nothing is suppressed). The catching lane is the
//! canonical `cargo test --all-features` workspace gate. Feature-scoped run:
//! `cargo test -p uptrakit-plugin-infrastructure-registry --features agent-infra --test contribution_monotonicity_guard`

use uptrakit_plugin_infrastructure_registry::all_descriptors;

/// Layer C: `proxmox.hosts` must be present in every feature configuration.
/// Proxmox is a mandatory (non-optional, non-cfg-gated) registry dependency,
/// so this assertion is unconditional. Both historical suppression shapes
/// (whole-descriptor `None`, 2026-04; empty registrations, 2026-07) manifest
/// as this surface being absent. Single assertion site — do not duplicate
/// this check in other guard files.
#[test]
fn critical_surfaces_present_in_every_feature_configuration() {
    let mut saw_proxmox_hosts = false;
    for desc in all_descriptors() {
        let Some(ops) = desc.surfaces else { continue };
        for registration in (ops.registrations)() {
            for surface in &registration.surfaces {
                if surface.descriptor.surface_id.as_str() == "proxmox.hosts" {
                    saw_proxmox_hosts = true;
                }
            }
        }
    }
    assert!(
        saw_proxmox_hosts,
        "`proxmox.hosts` is absent from the compiled catalog — a descriptor \
         contribution was suppressed by a feature predicate; see ADR-0032 \
         (contribution monotonicity)"
    );
}

/// Layer A: declaring the `surfaces` ops block while yielding nothing is
/// always a defect — a plugin with no surfaces omits the block entirely.
#[test]
fn declaring_surface_ops_implies_non_empty_registrations() {
    for desc in all_descriptors() {
        let Some(ops) = desc.surfaces else { continue };
        let type_id = desc.type_id;
        let registrations = (ops.registrations)();
        assert!(
            !registrations.is_empty(),
            "{type_id}: declares the surfaces ops block but yields zero registrations \
             (feature-predicate suppression? see ADR-0032)"
        );
        for registration in &registrations {
            assert!(
                !registration.surfaces.is_empty(),
                "{type_id}: a PluginSurfaceRegistration carries zero surfaces"
            );
        }
    }
}
