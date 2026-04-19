use uptrakit_plugin_infrastructure_registry::{
    FormFieldDescriptor as PluginFormFieldDescriptor, FormFieldType as PluginFormFieldType,
    FormSelectOptionDescriptor as PluginFormSelectOptionDescriptor, PluginFamily,
    SurfaceActionDescriptor, SurfaceActionUi, SurfaceFormDescriptor as PluginSurfaceFormDescriptor,
    SurfaceWorkflowStep as PluginSurfaceWorkflowStep, all_descriptors,
};
use uptrakit_shared_types::Permission;

use super::super::SSH_HOSTS_PRIMARY_ACTION_ID;

pub fn build_actions() -> Vec<SurfaceActionDescriptor> {
    let mut actions = vec![
        SurfaceActionDescriptor::new("remove-host", "Remove Host")
            .with_permission(Permission::UpdateHosts)
            .destructive()
            .with_confirm_entity_field("name")
            .with_timeout(30)
            .batch(),
        sync_host_action(),
        bootstrap_action(),
        // Internal wizard-step actions (not shown in UI directly).
        SurfaceActionDescriptor::new("bootstrap-connect", "Bootstrap Connect")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(60),
        SurfaceActionDescriptor::new("bootstrap-execute", "Bootstrap Execute")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(120),
        SurfaceActionDescriptor::new("sync-connect", "Sync Connect")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(60),
        SurfaceActionDescriptor::new("sync-execute", "Sync Execute")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(120),
    ];
    let infra_actions: Vec<SurfaceActionDescriptor> = all_descriptors()
        .iter()
        .filter(|d| d.family == PluginFamily::Infrastructure)
        .filter_map(|d| d.surface_actions)
        .flat_map(|surface_actions| (surface_actions.actions)())
        .collect();
    actions.extend(infra_actions);
    actions
}

pub(super) fn collect_infra_primary_actions() -> Vec<String> {
    all_descriptors()
        .iter()
        .filter(|descriptor| descriptor.family == PluginFamily::Infrastructure)
        .filter_map(|descriptor| descriptor.surface_actions)
        .flat_map(|surface_actions| (surface_actions.actions)())
        .filter(|action| action.action_id == "bootstrap-proxmox-guest")
        .map(|action| action.action_id)
        .collect()
}

pub(super) fn build_primary_actions(infra_primary_actions: &[String]) -> Vec<String> {
    let mut primary_actions = vec![SSH_HOSTS_PRIMARY_ACTION_ID.to_string()];
    primary_actions.extend(infra_primary_actions.iter().cloned());
    primary_actions
}

/// Build the sync-host action definition as a 3-step wizard.
fn sync_host_action() -> SurfaceActionDescriptor {
    let connect_step = PluginSurfaceWorkflowStep::new(
        "connect",
        "Connection & Authentication",
        PluginSurfaceFormDescriptor::new(vec![
            PluginFormFieldDescriptor::new("auth_method", "Auth Method")
                .with_type(PluginFormFieldType::Select)
                .with_default_value("stored")
                .with_options(vec![
                    PluginFormSelectOptionDescriptor::new("stored", "Stored Credentials"),
                    PluginFormSelectOptionDescriptor::new("password", "Password"),
                    PluginFormSelectOptionDescriptor::new("private_key", "Private Key"),
                ]),
            PluginFormFieldDescriptor::new("username", "SSH Username")
                .with_default_value("root")
                .with_help_text("User to connect as (e.g. root). Only used with custom auth.")
                .with_visible_when(
                    "auth_method",
                    vec!["password".to_string(), "private_key".to_string()],
                ),
            PluginFormFieldDescriptor::new("auth_password", "SSH Password")
                .with_type(PluginFormFieldType::Password)
                .with_help_text("Required when auth method is 'password'.")
                .sensitive()
                .with_visible_when("auth_method", vec!["password".to_string()]),
            PluginFormFieldDescriptor::new("auth_private_key", "SSH Private Key")
                .with_type(PluginFormFieldType::SshPrivateKey)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            PluginFormFieldDescriptor::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            PluginFormFieldDescriptor::new("auto", "Auto")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Skip review and execute immediately."),
        ]),
    )
    .with_submit_action("sync-connect");

    let review_step = PluginSurfaceWorkflowStep::new(
        "review",
        "Review Plan",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_render_previous_response();

    let execute_step = PluginSurfaceWorkflowStep::new(
        "execute",
        "Execute",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_submit_action("sync-execute");

    SurfaceActionDescriptor::new("sync-host", "Sync Host")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(SurfaceActionUi::Wizard {
            steps: vec![connect_step, review_step, execute_step],
        })
        .batch()
}

/// Build the bootstrap host action definition as a 3-step wizard.
fn bootstrap_action() -> SurfaceActionDescriptor {
    let connect_step = PluginSurfaceWorkflowStep::new(
        "connect",
        "Connection & Authentication",
        PluginSurfaceFormDescriptor::new(vec![
            PluginFormFieldDescriptor::new("target", "SSH Target")
                .required()
                .with_placeholder("[user@]host[:port]")
                .with_help_text(
                    "SSH target in [user@]host[:port] format. Default user: root, port: 22.",
                ),
            PluginFormFieldDescriptor::new("name", "Host Name")
                .with_placeholder("my-server")
                .with_help_text("Optional. Defaults to the hostname from the SSH target."),
            PluginFormFieldDescriptor::new("auth_method", "Auth Method")
                .with_type(PluginFormFieldType::Select)
                .required()
                .with_default_value("password")
                .with_options(vec![
                    PluginFormSelectOptionDescriptor::new("password", "Password"),
                    PluginFormSelectOptionDescriptor::new("private_key", "Private Key"),
                ]),
            PluginFormFieldDescriptor::new("auth_password", "SSH Password")
                .with_type(PluginFormFieldType::Password)
                .with_help_text("Required when auth method is 'password'.")
                .sensitive()
                .with_visible_when("auth_method", vec!["password".to_string()]),
            PluginFormFieldDescriptor::new("auth_private_key", "SSH Private Key")
                .with_type(PluginFormFieldType::SshPrivateKey)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            PluginFormFieldDescriptor::new("target_username", "Target Username")
                .with_help_text("User to create/use on the remote host.")
                .with_default_value("uptrakit"),
            PluginFormFieldDescriptor::new("host_key_fingerprint", "Host Key Fingerprint")
                .with_placeholder("SHA256:...")
                .with_help_text("Expected SHA-256 fingerprint of the host key."),
            PluginFormFieldDescriptor::new("strict_host_key_checking", "Strict Host Key Checking")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Require fingerprint match (disables TOFU)."),
            PluginFormFieldDescriptor::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            PluginFormFieldDescriptor::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Remove existing Uptrakit-managed keys before writing new ones."),
            PluginFormFieldDescriptor::new("auto", "Auto")
                .with_type(PluginFormFieldType::Toggle)
                .with_help_text("Skip review and execute immediately."),
        ]),
    )
    .with_submit_action("bootstrap-connect");

    let review_step = PluginSurfaceWorkflowStep::new(
        "review",
        "Review Plan",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_render_previous_response();

    let execute_step = PluginSurfaceWorkflowStep::new(
        "execute",
        "Execute",
        PluginSurfaceFormDescriptor::new(vec![]),
    )
    .with_submit_action("bootstrap-execute");

    SurfaceActionDescriptor::new("bootstrap", "Bootstrap Host")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(SurfaceActionUi::Wizard {
            steps: vec![connect_step, review_step, execute_step],
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_three_step_wizard(
        action: &SurfaceActionDescriptor,
        connect_submit_action: &str,
        execute_submit_action: &str,
    ) {
        let Some(SurfaceActionUi::Wizard { steps }) = action.ui.as_ref() else {
            panic!("expected wizard UI");
        };
        assert_eq!(steps.len(), 3, "wizard should keep 3 steps");
        assert_eq!(
            steps[0].submit_action.as_deref(),
            Some(connect_submit_action),
            "connect step submit action changed"
        );
        assert!(
            steps[1].render_previous_response,
            "review step should still render previous response"
        );
        assert_eq!(
            steps[2].submit_action.as_deref(),
            Some(execute_submit_action),
            "execute step submit action changed"
        );
    }

    #[test]
    fn build_primary_actions_preserves_ssh_primary_and_appends_infra_primary_actions() {
        let infra_primary_actions = vec![
            "bootstrap-proxmox-guest".to_string(),
            "another-infra-action".to_string(),
        ];

        let primary_actions = build_primary_actions(&infra_primary_actions);

        assert_eq!(primary_actions[0], SSH_HOSTS_PRIMARY_ACTION_ID);
        assert_eq!(&primary_actions[1..], infra_primary_actions.as_slice());
    }

    #[test]
    fn sync_host_action_remains_three_step_wizard() {
        let action = sync_host_action();

        assert_eq!(action.action_id, "sync-host");
        assert_eq!(action.permission, Permission::UpdateHosts.as_str());
        assert_eq!(action.timeout_seconds, Some(120));
        assert!(action.batch_action, "sync-host should remain batch-capable");
        assert_three_step_wizard(&action, "sync-connect", "sync-execute");
    }

    #[test]
    fn bootstrap_action_remains_three_step_wizard() {
        let action = bootstrap_action();

        assert_eq!(action.action_id, "bootstrap");
        assert_eq!(action.permission, Permission::UpdateHosts.as_str());
        assert_eq!(action.timeout_seconds, Some(120));
        assert_three_step_wizard(&action, "bootstrap-connect", "bootstrap-execute");
    }

    #[test]
    fn build_actions_includes_sync_and_bootstrap_wizards() {
        let actions = build_actions();
        let action_ids: Vec<&str> = actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect();

        assert!(action_ids.contains(&"sync-host"));
        assert!(action_ids.contains(&"bootstrap"));
        assert!(action_ids.contains(&"bootstrap-connect"));
        assert!(action_ids.contains(&"sync-connect"));
    }

    #[test]
    fn build_actions_includes_infrastructure_primary_action_when_available() {
        let actions = build_actions();
        assert!(
            actions
                .iter()
                .any(|action| action.action_id == "bootstrap-proxmox-guest"),
            "expected infra action bootstrap-proxmox-guest to be present"
        );
    }

    #[test]
    fn collect_infra_primary_actions_is_subset_of_action_library() {
        let actions = build_actions();
        let action_ids: std::collections::BTreeSet<&str> = actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect();

        let primary_actions = collect_infra_primary_actions();
        for action_id in primary_actions {
            assert!(
                action_ids.contains(action_id.as_str()),
                "infra primary action {action_id} must exist in action library"
            );
            let _ = uptrakit_internal_wire::surfaces::InteractionId::new(action_id)
                .expect("infra primary action should remain a valid interaction id");
        }
    }
}
