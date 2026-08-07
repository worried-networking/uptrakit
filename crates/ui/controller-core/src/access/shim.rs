//! Temporary `Permission` → `Action` bridge for the M1.4–M1.7 transition.
//!
//! Deleted in M1.8 together with `Permission` itself. Mapping source:
//! `.superpowers/authn-and-authz-refactoring/05-action-model.md` §Built-in
//! catalog and mapping from the old enum.

use uptrakit_shared_types::Permission;
use uptrakit_shared_types::access::{Action, actions};

/// Map a legacy [`Permission`] to the [`Action`]s it confers.
///
/// Temporary M1 bridge: no non-test consumers remain now that M1.7 deleted
/// `permission_extractor!` and every enforcement site now gates via
/// `action_extractor!`. This function survives only until `Permission`
/// itself is deleted in M1.8, at which point it is removed with it. **No
/// site may gate `access:manage` through this shim** — grant-admin authorization uses the
/// `access:manage` extractor directly. `ManageUsers` maps to `users:manage`
/// only: mapping both would re-merge the `users:manage`/`access:manage`
/// split for every transitional consumer during the transition window;
/// durable both-ness for real users is carried by the M1.2
/// `settings_manager` seed. Unknown legacy strings (`Other`) and any future
/// variant confer nothing (fail-closed; the shim's strum guard test fails
/// loudly if a new variant lands unmapped).
pub fn actions_for_permission(permission: &Permission) -> &'static [Action] {
    match permission {
        Permission::ViewServices => const { &[actions::SERVICES_READ] },
        Permission::ApproveServices => const { &[actions::SERVICES_APPROVE] },
        Permission::RejectServices => const { &[actions::SERVICES_REJECT] },
        Permission::RemoveServices => const { &[actions::SERVICES_DELETE] },
        Permission::UpdateServices => const { &[actions::SERVICES_UPDATE] },
        Permission::ViewSystemServices => const { &[actions::SYSTEM_SERVICES_READ] },
        Permission::ApproveSystemServices => const { &[actions::SYSTEM_SERVICES_APPROVE] },
        Permission::RejectSystemServices => const { &[actions::SYSTEM_SERVICES_REJECT] },
        Permission::RemoveSystemServices => const { &[actions::SYSTEM_SERVICES_DELETE] },
        Permission::UpdateSystemServices => const { &[actions::SYSTEM_SERVICES_UPDATE] },
        Permission::ViewSoftware => const { &[actions::SOFTWARE_READ] },
        Permission::CreateSoftware => const { &[actions::SOFTWARE_CREATE] },
        Permission::UpdateSoftware => const { &[actions::SOFTWARE_UPDATE] },
        Permission::DeleteSoftware => const { &[actions::SOFTWARE_DELETE] },
        Permission::TriggerChecks => const { &[actions::CHECKS_TRIGGER] },
        Permission::TriggerUpdates => const { &[actions::UPDATES_TRIGGER] },
        Permission::ManageScheduler => const { &[actions::SCHEDULER_MANAGE] },
        Permission::ViewHosts => const { &[actions::HOSTS_READ] },
        Permission::UpdateHosts => const { &[actions::HOSTS_UPDATE] },
        Permission::DeactivateHosts => const { &[actions::HOSTS_DELETE] },
        Permission::ViewSettings => const { &[actions::SETTINGS_READ] },
        Permission::ManageAuthSettings => const { &[actions::SETTINGS_AUTH_MANAGE] },
        Permission::ManageEnrollmentTokens => {
            const { &[actions::SETTINGS_ENROLLMENT_TOKENS_MANAGE] }
        }
        Permission::ManageAgentCerts => const { &[actions::SETTINGS_CERTIFICATES_MANAGE] },
        Permission::ManageGlobalSettings => const { &[actions::SYSTEM_SETTINGS_MANAGE] },
        Permission::ManageCommands => const { &[actions::COMMANDS_MANAGE] },
        Permission::ViewNotifications => const { &[actions::NOTIFICATIONS_READ] },
        Permission::ManageNotifications => const { &[actions::NOTIFICATIONS_MANAGE] },
        Permission::ViewAuditLogs => const { &[actions::AUDIT_READ] },
        Permission::ViewSystemAuditLogs => const { &[actions::SYSTEM_AUDIT_READ] },
        Permission::ManageUsers => const { &[actions::USERS_MANAGE] },
        Permission::ManageIgnores => const { &[actions::DISCOVERY_IGNORES_MANAGE] },
        Permission::TestPluginConfigs => const { &[actions::PLUGIN_CONFIGS_TRIGGER] },
        Permission::AccessMcp => const { &[actions::MCP_USE] },
        Permission::ViewInstanceConfigState => const { &[actions::SYSTEM_CONFIG_STATE_READ] },
        Permission::ManageInstanceConfigState => const { &[actions::SYSTEM_CONFIG_STATE_MANAGE] },
        // Other(_) and any future variant: fail-closed, confers nothing.
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;
    use uptrakit_shared_types::access::CATALOG;

    use super::*;

    fn in_catalog(action: &Action) -> bool {
        CATALOG.iter().any(|entry| {
            entry.resource == *action.resource()
                && entry.verbs.iter().any(|v| v.verb == action.verb())
        })
    }

    #[test]
    fn every_permission_variant_maps_to_catalog_actions() {
        assert!(!CATALOG.is_empty(), "catalog must be non-empty");
        for permission in Permission::iter() {
            let mapped = actions_for_permission(&permission);
            assert!(
                !mapped.is_empty(),
                "permission {permission:?} maps to no action — extend the shim per 05-action-model.md"
            );
            for action in mapped {
                assert!(
                    in_catalog(action),
                    "shim emitted non-catalog action for {permission:?}"
                );
            }
        }
    }

    #[test]
    fn manage_users_maps_to_users_manage_only() {
        assert_eq!(
            actions_for_permission(&Permission::ManageUsers),
            [actions::USERS_MANAGE].as_slice()
        );
    }

    #[test]
    fn shim_never_emits_access_manage() {
        for permission in Permission::iter() {
            assert!(
                !actions_for_permission(&permission).contains(&actions::ACCESS_MANAGE),
                "the shim must never confer access:manage (contrarian decision 3)"
            );
        }
    }

    #[test]
    fn unknown_legacy_string_confers_nothing() {
        assert!(actions_for_permission(&Permission::Other("anything".into())).is_empty());
    }
}
