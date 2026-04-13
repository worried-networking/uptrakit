//! Descriptor assembly and plugin creation functions.
//!
//! This module provides:
//! - [`all_descriptors()`] — the authoritative list of compiled-in plugin descriptors
//! - Descriptor-based plugin creation for agent-side use
//! - Sudo command collection from descriptors

use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::{
    PluginDescriptor, PluginFamily, PluginTypeId, SudoCommandEntry,
};
use uptrakit_shared_types::plugin_ids;

/// Returns the authoritative list of all compiled-in plugin descriptors.
///
/// This is the single source of truth — no hardcoded lists elsewhere.
/// Feature-gated plugins are included only when the corresponding feature is enabled.
pub fn all_descriptors() -> Vec<&'static PluginDescriptor> {
    #[allow(unused_mut)]
    let mut descs: Vec<&'static PluginDescriptor> = vec![
        // Software — Release plugins
        &uptrakit_plugin_releases_github::DESCRIPTOR,
        &uptrakit_plugin_releases_gitlab::DESCRIPTOR,
        &uptrakit_plugin_releases_forgejo::DESCRIPTOR,
        &uptrakit_plugin_releases_docker::DESCRIPTOR,
        // Software — Discovery
        &uptrakit_plugin_discovery_proxmox_helper_scripts::DESCRIPTOR,
        // Software — Package managers
        &uptrakit_plugin_package_manager_homebrew::DESCRIPTOR,
        &uptrakit_plugin_package_manager_apt::DESCRIPTOR,
        &uptrakit_plugin_package_manager_dnf::DESCRIPTOR,
        &uptrakit_plugin_package_manager_npm::DESCRIPTOR,
        &uptrakit_plugin_package_manager_mas::DESCRIPTOR,
        &uptrakit_plugin_package_manager_pacman::DESCRIPTOR,
        &uptrakit_plugin_package_manager_pkg::DESCRIPTOR,
        &uptrakit_plugin_package_manager_apk::DESCRIPTOR,
        &uptrakit_plugin_package_manager_snap::DESCRIPTOR,
        &uptrakit_plugin_package_manager_cargo::DESCRIPTOR,
        // Software — Generic
        &uptrakit_plugin_generic_shell::DESCRIPTOR,
        // Hooks
        &uptrakit_plugin_hook_shell::DESCRIPTOR,
        &uptrakit_plugin_hook_systemd::DESCRIPTOR,
        // Infrastructure
        &uptrakit_plugin_infrastructure_proxmox::DESCRIPTOR,
    ];
    // Notifications (feature-gated)
    #[cfg(feature = "notifications-webhook")]
    descs.push(&uptrakit_notification_plugin_webhook::DESCRIPTOR);
    #[cfg(feature = "notifications-telegram")]
    descs.push(&uptrakit_notification_plugin_telegram::DESCRIPTOR);
    #[cfg(feature = "notifications-email")]
    descs.push(&uptrakit_notification_plugin_email::DESCRIPTOR);
    // Enhancements (feature-gated)
    #[cfg(feature = "dashboard-icons")]
    descs.push(&uptrakit_plugin_enhancement_dashboard_icons::DESCRIPTOR);
    #[cfg(feature = "test-support")]
    {
        descs.push(&crate::test_support::DESCRIPTOR);
        descs.push(&crate::test_support::PER_ITEM_FAIL_DESCRIPTOR);
    }
    descs
}

/// Look up a descriptor by type ID string.
pub fn get_descriptor(type_id: &str) -> Option<&'static PluginDescriptor> {
    all_descriptors().into_iter().find(|d| d.type_id == type_id)
}

/// Returns the descriptor family for a known plugin type.
pub fn plugin_family(plugin_type_id: &PluginTypeId) -> Option<PluginFamily> {
    get_descriptor(plugin_type_id.as_str()).map(|d| d.family)
}

/// Returns true when the plugin type is one of the known package-manager plugins.
pub fn is_package_manager_plugin(plugin_type_id: &PluginTypeId) -> bool {
    const PACKAGE_MANAGER_IDS: &[PluginTypeId] = &[
        plugin_ids::PACKAGE_MANAGER_APT,
        plugin_ids::PACKAGE_MANAGER_HOMEBREW,
        plugin_ids::PACKAGE_MANAGER_DNF,
        plugin_ids::PACKAGE_MANAGER_NPM,
        plugin_ids::PACKAGE_MANAGER_MAS,
        plugin_ids::PACKAGE_MANAGER_PACMAN,
        plugin_ids::PACKAGE_MANAGER_PKG,
        plugin_ids::PACKAGE_MANAGER_APK,
        plugin_ids::PACKAGE_MANAGER_SNAP,
        plugin_ids::PACKAGE_MANAGER_CARGO,
    ];

    PACKAGE_MANAGER_IDS
        .iter()
        .any(|known| known == plugin_type_id)
}

/// Create a per-instance role plugin from descriptor + config + runtime.
///
/// This is the descriptor-based replacement for `PluginRegistry::create_plugin()`.
/// The caller specifies which role they need by accessing the appropriate slot
/// on the descriptor's `roles` field.
///
/// For agent-side use, the typical pattern is:
/// ```ignore
/// let desc = get_descriptor(plugin_type_str).ok_or("unknown")?;
/// let slot = desc.roles.version_detector.as_ref().ok_or("unsupported")?;
/// let detector = (slot.create)(&config, runtime)?;
/// ```
/// Collect sudo command entries from all plugins that declare them.
///
/// Calls each descriptor's `sudo` function pointer with an empty config.
pub fn all_required_sudo_commands() -> Vec<(PluginTypeId, Vec<SudoCommandEntry>)> {
    let empty = serde_json::json!({});
    let mut result = Vec::new();
    for desc in all_descriptors() {
        if let Some(sudo_fn) = desc.sudo {
            let entries = sudo_fn(&empty);
            if !entries.is_empty() {
                result.push((PluginTypeId::from_static(desc.type_id), entries));
            }
        }
    }
    result
}

/// Collect sudo command entries for plugins compatible with the given host.
///
/// Runs host compatibility probes concurrently and filters out incompatible plugins.
pub async fn compatible_sudo_commands_for_host(
    executor: Arc<dyn uptrakit_command::CommandExecutor>,
) -> Vec<(PluginTypeId, Vec<SudoCommandEntry>)> {
    use uptrakit_plugin_infrastructure_core::{HostCapabilities, construct_host_runtime};

    let caps = HostCapabilities::default();
    let runtime = construct_host_runtime(executor.clone(), caps);
    let empty = serde_json::json!({});
    let mut result = Vec::new();

    for desc in all_descriptors() {
        let Some(sudo_fn) = desc.sudo else {
            continue;
        };

        // Check host compatibility if plugin has a discoverer role
        if let Some(slot) = &desc.roles.discoverer
            && let Ok(discoverer) = (slot.create)(&empty, Arc::clone(&runtime))
        {
            match discoverer.detect_host_compatibility().await {
                Ok(uptrakit_plugin_infrastructure_core::HostCompatibility::Compatible) => {}
                Ok(uptrakit_plugin_infrastructure_core::HostCompatibility::Incompatible(
                    reason,
                )) => {
                    tracing::debug!(
                        plugin = desc.type_id,
                        reason = %reason,
                        "plugin not compatible with host; skipping sudo commands"
                    );
                    continue;
                }
                Ok(_) => {
                    tracing::warn!(
                        plugin = desc.type_id,
                        "unknown HostCompatibility variant; assuming compatible"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        plugin = desc.type_id,
                        error = %e,
                        "host compatibility check failed; assuming compatible"
                    );
                }
            }
        }

        let entries = sudo_fn(&empty);
        if !entries.is_empty() {
            result.push((PluginTypeId::from_static(desc.type_id), entries));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every descriptor has a unique type_id.
    #[test]
    fn all_descriptors_unique_type_ids() {
        let descs = all_descriptors();
        let mut seen = BTreeSet::new();
        for d in &descs {
            assert!(seen.insert(d.type_id), "duplicate type_id: {}", d.type_id);
        }
    }

    /// Every descriptor has a corresponding plugin_ids constant.
    #[test]
    fn descriptors_subset_of_known_ids() {
        let descs = all_descriptors();
        #[allow(unused_mut)]
        let mut known: BTreeSet<String> = plugin_ids::ALL
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        #[cfg(feature = "test-support")]
        {
            known.insert(plugin_ids::TEST_FETCH_FAIL.as_str().to_owned());
            known.insert(plugin_ids::TEST_PER_ITEM_FAIL.as_str().to_owned());
        }
        for d in &descs {
            assert!(
                known.contains(d.type_id),
                "descriptor '{}' has no corresponding plugin_ids constant — \
                 add it to plugin_ids::ALL",
                d.type_id
            );
        }
    }

    /// Every always-on plugin_ids entry must be in all_descriptors().
    #[test]
    fn always_on_ids_have_descriptors() {
        let descs = all_descriptors();
        let desc_ids: BTreeSet<&str> = descs.iter().map(|d| d.type_id).collect();
        let always_on = [
            &plugin_ids::RELEASES_GITHUB,
            &plugin_ids::RELEASES_GITLAB,
            &plugin_ids::RELEASES_FORGEJO,
            &plugin_ids::RELEASES_DOCKER,
            &plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS,
            &plugin_ids::PACKAGE_MANAGER_APT,
            &plugin_ids::PACKAGE_MANAGER_HOMEBREW,
            &plugin_ids::PACKAGE_MANAGER_DNF,
            &plugin_ids::PACKAGE_MANAGER_NPM,
            &plugin_ids::PACKAGE_MANAGER_MAS,
            &plugin_ids::PACKAGE_MANAGER_PACMAN,
            &plugin_ids::PACKAGE_MANAGER_PKG,
            &plugin_ids::PACKAGE_MANAGER_APK,
            &plugin_ids::PACKAGE_MANAGER_SNAP,
            &plugin_ids::PACKAGE_MANAGER_CARGO,
            &plugin_ids::GENERIC_SHELL,
            &plugin_ids::HOOK_SHELL,
            &plugin_ids::HOOK_SYSTEMD,
            &plugin_ids::INFRASTRUCTURE_PROXMOX,
        ];
        for id in &always_on {
            assert!(
                desc_ids.contains(id.as_str()),
                "plugin_ids::{} has no descriptor in all_descriptors()",
                id.as_str()
            );
        }
        // Feature-gated checks
        #[cfg(feature = "notifications-webhook")]
        assert!(desc_ids.contains(plugin_ids::WEBHOOK.as_str()));
        #[cfg(feature = "notifications-telegram")]
        assert!(desc_ids.contains(plugin_ids::TELEGRAM.as_str()));
        #[cfg(feature = "notifications-email")]
        assert!(desc_ids.contains(plugin_ids::EMAIL.as_str()));
        #[cfg(feature = "dashboard-icons")]
        assert!(desc_ids.contains(plugin_ids::ENHANCEMENT_DASHBOARD_ICONS.as_str()));
    }

    #[test]
    fn package_manager_lookup_covers_all_current_package_managers() {
        let package_managers = [
            plugin_ids::PACKAGE_MANAGER_APT,
            plugin_ids::PACKAGE_MANAGER_HOMEBREW,
            plugin_ids::PACKAGE_MANAGER_DNF,
            plugin_ids::PACKAGE_MANAGER_NPM,
            plugin_ids::PACKAGE_MANAGER_MAS,
            plugin_ids::PACKAGE_MANAGER_PACMAN,
            plugin_ids::PACKAGE_MANAGER_PKG,
            plugin_ids::PACKAGE_MANAGER_APK,
            plugin_ids::PACKAGE_MANAGER_SNAP,
            plugin_ids::PACKAGE_MANAGER_CARGO,
        ];
        let github = PluginTypeId::from_static("releases_github");

        for plugin_type in package_managers {
            assert!(is_package_manager_plugin(&plugin_type));
        }
        assert!(!is_package_manager_plugin(&github));
    }

    #[test]
    fn plugin_family_lookup_returns_descriptor_family() {
        let apt = PluginTypeId::from_static("package_manager_apt");
        let proxmox = PluginTypeId::from_static("infrastructure_proxmox");
        let missing = PluginTypeId::new("missing_plugin");

        assert_eq!(plugin_family(&apt), Some(PluginFamily::Software));
        assert_eq!(plugin_family(&proxmox), Some(PluginFamily::Infrastructure));
        assert_eq!(plugin_family(&missing), None);
    }
}
