//! HTTP handlers for `GET /api/v1/global-settings/nats` and `PUT /api/v1/global-settings/nats`.
//!
//! The NATS URL is stored as a global setting (encrypted at rest). It is used to
//! connect to NATS at startup; **hot-reload is not supported** — changes take
//! effect after the controller is restarted.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::global_settings::GlobalSettingView;

use uptrakit_web_api_types::MaskedUrl;
pub use uptrakit_web_api_types::settings_nats::{NatsSettingsResponse, UpdateNatsSettingsRequest};

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::action::CanManageSystemSettings;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::settings_store::{load_global_setting_raw, upsert_global_setting_raw};

fn snapshot_to_response(nats_url: Option<MaskedUrl>) -> NatsSettingsResponse {
    let has_url = nats_url.is_some();
    NatsSettingsResponse {
        url: nats_url,
        has_url,
    }
}

/// Emit a failed NATS settings audit event (no DB write — uses the dispatcher).
fn emit_nats_failed_event(
    state: &AppState,
    actor_type: uptrakit_audit_log::AuditActorType,
    actor_id: Option<uuid::Uuid>,
    details: serde_json::Value,
) {
    if let Ok(entry) = AuditEntry::<Event>::builder_event(
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
    )
    .system_scope()
    .actor(actor_type, actor_id)
    .target(
        "global_setting",
        "nats.url".to_string(),
        Some("nats.url".to_string()),
    )
    .outcome(AuditOutcome::Failed)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }
}

/// Get NATS settings
///
/// Returns the current NATS URL configuration. The password component of the
/// URL is always redacted in the response. Changes take effect after the
/// controller is restarted.
#[utoipa::path(
    get,
    path = "/api/v1/global-settings/nats",
    responses(
        (status = 200, description = "NATS settings", body = NatsSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_nats_settings(
    State(state): State<Arc<AppState>>,
    CanManageSystemSettings(_user): CanManageSystemSettings,
) -> Response {
    let nats_url = state.settings.nats_url();
    (StatusCode::OK, Json(snapshot_to_response(nats_url))).into_response()
}

/// Update NATS settings
///
/// Update the NATS server URL. The URL is encrypted at rest. Changes take
/// effect **after the controller is restarted** — the live NATS connection is
/// not replaced while the controller is running.
///
/// Send `"url": null` to clear the stored URL (NATS will be disabled after
/// the next restart).
#[utoipa::path(
    put,
    path = "/api/v1/global-settings/nats",
    request_body = UpdateNatsSettingsRequest,
    responses(
        (status = 200, description = "NATS settings updated", body = NatsSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_nats_settings(
    State(state): State<Arc<AppState>>,
    CanManageSystemSettings(user): CanManageSystemSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpdateNatsSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let mut nats_url = state.settings.nats_url();

    if let Some(ref val) = req.url {
        if val.is_null() {
            // Clear stored URL
            let changed = nats_url.is_some();
            if changed {
                // Read the before value for the audit snapshot.
                let before_value = load_global_setting_raw(state.db(), "nats.url")
                    .await
                    .unwrap_or(None)
                    .unwrap_or(serde_json::Value::Null);

                let tx = match state
                    .db()
                    .begin_with_options(TransactionOptions {
                        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                        ..Default::default()
                    })
                    .await
                {
                    Ok(tx) => tx,
                    Err(e) => {
                        tracing::error!("Failed to begin tx for nats url clear: {e}");
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                };

                if let Err(e) =
                    upsert_global_setting_raw(&tx, "nats.url", serde_json::json!("")).await
                {
                    tracing::error!("Failed to clear nats.url: {e:?}");
                    drop(tx);
                    emit_nats_failed_event(
                        &state,
                        actor_type,
                        actor_id,
                        serde_json::json!({
                            "setting_key": "nats.url",
                            "operation": "clear",
                            "reason_code": "nats_url_clear_failed",
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }

                let key_str = "nats.url".to_string();
                let before_view = GlobalSettingView {
                    key: key_str.clone(),
                    value: before_value,
                };
                let after_view = GlobalSettingView {
                    key: key_str,
                    value: serde_json::json!(""),
                };
                let hook = state.audit_emitter.commit_hook();
                let audit_entry =
                    match AuditEntry::<Stateful>::global_setting_update(&before_view, &after_view)
                        .system_scope()
                        .actor(actor_type, actor_id)
                        .outcome(AuditOutcome::Success)
                        .details(serde_json::json!({
                            "setting_key": "nats.url",
                            "changed": true,
                            "operation": "clear",
                        }))
                        .build()
                    {
                        Ok(entry) => entry,
                        Err(e) => {
                            tracing::error!("Failed to build audit entry for nats url clear: {e}");
                            drop(tx);
                            return error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            );
                        }
                    };

                if let Err(e) = state
                    .audit_emitter
                    .emit_stateful(&tx, &hook, audit_entry)
                    .await
                {
                    tracing::error!("Failed to emit stateful audit for nats url clear: {e}");
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }

                if let Err(e) = tx.commit().await {
                    tracing::error!("Failed to commit nats url clear: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
                hook.flush_after_commit().await;

                nats_url = None;
            }
        } else if let Some(s) = val.as_str() {
            let changed = nats_url.as_ref().map(|value| value.as_raw_str()) != Some(s);
            if !changed {
                state.settings.set_nats_url(nats_url.clone()).await;
                return (StatusCode::OK, Json(snapshot_to_response(nats_url))).into_response();
            }

            // Read the before value for the audit snapshot.
            let before_value = load_global_setting_raw(state.db(), "nats.url")
                .await
                .unwrap_or(None);

            let stored_value = match uptrakit_crypto::encrypt_str(s, "uptrakit:settings:nats_url") {
                Ok(encrypted) => serde_json::json!(encrypted),
                Err(e) => {
                    tracing::error!("Failed to encrypt NATS URL: {e:?}");
                    emit_nats_failed_event(
                        &state,
                        actor_type,
                        actor_id,
                        serde_json::json!({
                            "setting_key": "nats.url",
                            "operation": "save",
                            "reason_code": "nats_url_encrypt_failed",
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            let tx = match state
                .db()
                .begin_with_options(TransactionOptions {
                    sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                    ..Default::default()
                })
                .await
            {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::error!("Failed to begin tx for nats url save: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if let Err(e) = upsert_global_setting_raw(&tx, "nats.url", stored_value.clone()).await {
                tracing::error!("Failed to save nats.url: {e:?}");
                drop(tx);
                emit_nats_failed_event(
                    &state,
                    actor_type,
                    actor_id,
                    serde_json::json!({
                        "setting_key": "nats.url",
                        "operation": "save",
                        "reason_code": "nats_url_save_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }

            let key_str = "nats.url".to_string();
            let after_view = GlobalSettingView {
                key: key_str.clone(),
                value: stored_value,
            };
            let hook = state.audit_emitter.commit_hook();
            let audit_entry = match before_value {
                Some(bv) => {
                    let before_view = GlobalSettingView {
                        key: key_str,
                        value: bv,
                    };
                    AuditEntry::<Stateful>::global_setting_update(&before_view, &after_view)
                        .system_scope()
                        .actor(actor_type, actor_id)
                        .outcome(AuditOutcome::Success)
                        .details(serde_json::json!({
                            "setting_key": "nats.url",
                            "changed": true,
                            "operation": "save",
                        }))
                        .build()
                }
                None => AuditEntry::<Stateful>::global_setting_update(
                    &AbsentView(&after_view),
                    &after_view,
                )
                .system_scope()
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Success)
                .details(serde_json::json!({
                    "setting_key": "nats.url",
                    "changed": true,
                    "operation": "save",
                }))
                .build(),
            };
            let audit_entry = match audit_entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::error!("Failed to build audit entry for nats url save: {e}");
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if let Err(e) = state
                .audit_emitter
                .emit_stateful(&tx, &hook, audit_entry)
                .await
            {
                tracing::error!("Failed to emit stateful audit for nats url save: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }

            if let Err(e) = tx.commit().await {
                tracing::error!("Failed to commit nats url save: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            hook.flush_after_commit().await;

            nats_url = Some(MaskedUrl::new(s));
        }
    }

    state.settings.set_nats_url(nats_url.clone()).await;

    (StatusCode::OK, Json(snapshot_to_response(nats_url))).into_response()
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "fire-and-forget sends in tests drop results intentionally"
    )]

    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::AuthMethod;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
        EntityTrait, PaginatorTrait, QueryOrder, Set,
    };
    use uptrakit_shared_db::entity::{system_audit_log, tenant, user};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn test_state(db: DatabaseConnection, tenant_id: uuid::Uuid) -> Arc<AppState> {
        use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
        use crate::settings::Settings;

        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::report!(CertSignerError::Signing(
                    "noop signer".to_string(),
                )))
            }

            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        uptrakit_crypto::enable_plaintext_mode();

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let notification_service = crate::notification_service::NotificationService::new(
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
                uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
            )
            .expect("default catalog should build"),
        );

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );
        let db_backend = Arc::new(uptrakit_audit_log::DatabaseBackend::new(db.clone()))
            as Arc<dyn uptrakit_audit_log::AuditLogBackend>;
        let dispatcher = uptrakit_audit_log::AuditLogDispatcher::new(Arc::clone(&db_backend));

        let (_, config_rx_for_settings_nats) =
            uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
                &uptrakit_config_reload::RuntimeConfig::default(),
            );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            access_engine: Arc::new(uptrakit_controller_core::access::AccessEngine::new(
                db.clone(),
            )),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState::new(
                Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-for-settings-nats-tests",
                )),
                crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            ),
            notification: crate::app_state::NotificationState::new(
                notification_service,
                notification_dispatcher,
                crate::event_broadcaster::EventBroadcaster::new(),
            ),
            broadcast: crate::app_state::BroadcastState {
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            default_tenant_id: tenant_id,
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            plugin: crate::app_state::PluginState::new(
                plugin_ops,
                Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
            ),
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter_rx: tokio::sync::watch::channel(std::sync::Arc::new(
                uptrakit_config_reload::config::AuditConfig::default(),
            ))
            .1,
            audit_log_dispatcher: dispatcher.clone(),
            audit_emitter: uptrakit_audit_log::AuditEmitter::with_backends(
                dispatcher,
                Arc::clone(&db_backend),
                Arc::new(uptrakit_audit_log::NoopBackend),
            ),
            surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
                Arc::new(crate::surface_registry::SurfaceRegistry::new(
                    crate::surface_registry::SurfaceRegistryConfig::default(),
                )),
                Arc::new(crate::surface_proxy::SurfaceProxy::new()),
                Arc::new(crate::surface_proxy::AllProvidersVisible),
            ),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            server: crate::app_state::ServerState::new(
                std::path::PathBuf::from("/tmp/test-pki"),
                rustls_cfg,
            ),
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
            #[cfg(feature = "test-utils")]
            test_reexec_notify: None,
            update_dispatcher: Arc::new(uptrakit_controller_core::update::NoopUpdateDispatcher),
            instance_plugin_snapshot: Arc::new(arc_swap::ArcSwap::from_pointee(
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
            )),
            coordinator_handle: {
                let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                uptrakit_config_reload::ReloadCoordinator::new(
                    vec![],
                    tx,
                    std::sync::Arc::new(uptrakit_config_reload::NoopAlertWriter),
                )
                .1
            },
            settings_version_cache: uptrakit_config_reload::SettingsVersionCache::new(),
            db_config_rx: config_rx_for_settings_nats.db,
            network_config_rx: config_rx_for_settings_nats.network,
            nats_config_rx: config_rx_for_settings_nats.nats,
            tls_config_rx: config_rx_for_settings_nats.tls,
            audit_config_rx: config_rx_for_settings_nats.audit,
            log_config_rx: config_rx_for_settings_nats.log,
            master_key_config_rx: config_rx_for_settings_nats.master_key,
            embedded_services_config_rx: config_rx_for_settings_nats.embedded_services,
            zeroconf_config_rx: config_rx_for_settings_nats.zeroconf,
            oauth: crate::oauth::OAuthState::disabled(),
            config_file_state: tokio::sync::watch::channel(
                uptrakit_config_reload::ConfigFileState::default(),
            )
            .1,
            last_reload: tokio::sync::watch::channel(None).1,
            recent_reload_events: tokio::sync::watch::channel(Vec::new()).1,
        })
    }

    async fn latest_system_audit_row(db: &sea_orm::DatabaseConnection) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row");
    }

    async fn count_system_audit_rows(db: &sea_orm::DatabaseConnection) -> u64 {
        system_audit_log::Entity::find()
            .count(db)
            .await
            .expect("count system audit rows")
    }

    async fn wait_for_system_audit_rows(db: &sea_orm::DatabaseConnection, expected: u64) {
        for _ in 0..50 {
            if count_system_audit_rows(db).await == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected {expected} system audit rows");
    }

    #[tokio::test]
    async fn update_nats_settings_writes_global_setting_update_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant")
            .id;
        let state = test_state(db.clone(), tenant_id).await;

        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(uptrakit_shared_types::MaskedEmail::new("admin@example.com")),
            first_name: Set("Admin".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(
                crate::auth::password::hash_password("test-password").expect("password hash"),
            )),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        let response = update_nats_settings(
            State(state),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::Password,
                None,
            )),
            None,
            Validated(UpdateNatsSettingsRequest {
                url: Some(serde_json::json!("nats://demo:secret@localhost:4222")),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["setting_key"], "nats.url");
        assert_eq!(details["changed"], serde_json::json!(true));
        assert!(details.get("value").is_none());
        assert!(details.get("url").is_none());
    }

    #[tokio::test]
    async fn update_nats_settings_without_url_does_not_write_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant")
            .id;
        let state = test_state(db.clone(), tenant_id).await;

        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(uptrakit_shared_types::MaskedEmail::new("admin@example.com")),
            first_name: Set("Admin".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(
                crate::auth::password::hash_password("test-password").expect("password hash"),
            )),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        let response = update_nats_settings(
            State(state),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::Password,
                None,
            )),
            None,
            Validated(UpdateNatsSettingsRequest { url: None }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(count_system_audit_rows(&db).await, 0);
    }

    #[tokio::test]
    async fn update_nats_settings_with_same_url_does_not_write_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant")
            .id;
        let state = test_state(db.clone(), tenant_id).await;

        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(uptrakit_shared_types::MaskedEmail::new("admin@example.com")),
            first_name: Set("Admin".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(
                crate::auth::password::hash_password("test-password").expect("password hash"),
            )),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        let request = UpdateNatsSettingsRequest {
            url: Some(serde_json::json!("nats://demo:secret@localhost:4222")),
        };

        let first_response = update_nats_settings(
            State(Arc::clone(&state)),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::Password,
                None,
            )),
            None,
            Validated(request.clone()),
        )
        .await;
        assert_eq!(first_response.status(), StatusCode::OK);
        wait_for_system_audit_rows(&db, 1).await;
        let baseline_rows = count_system_audit_rows(&db).await;
        assert_eq!(baseline_rows, 1);

        let second_response = update_nats_settings(
            State(state),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::Password,
                None,
            )),
            None,
            Validated(request),
        )
        .await;
        assert_eq!(second_response.status(), StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(count_system_audit_rows(&db).await, baseline_rows);
    }

    #[tokio::test]
    async fn update_nats_settings_with_api_token_uses_api_token_actor_type() {
        let db = setup_test_db().await;
        let tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant")
            .id;
        let state = test_state(db.clone(), tenant_id).await;

        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(uptrakit_shared_types::MaskedEmail::new(
                "token-user@example.com",
            )),
            first_name: Set("Token".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(
                crate::auth::password::hash_password("test-password").expect("password hash"),
            )),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");
        let token_id = uuid::Uuid::now_v7();

        let response = update_nats_settings(
            State(state),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::ApiToken,
                None,
            )),
            Some(Extension(AuthenticatedApiTokenId(token_id))),
            Validated(UpdateNatsSettingsRequest {
                url: Some(serde_json::json!("nats://demo:secret@localhost:4222")),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(token_id));
    }

    #[tokio::test]
    async fn clear_nats_settings_persistence_failure_writes_failed_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant")
            .id;
        let state = test_state(db.clone(), tenant_id).await;
        state
            .settings
            .set_nats_url(Some(MaskedUrl::new("nats://demo:secret@localhost:4222")))
            .await;

        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(uptrakit_shared_types::MaskedEmail::new("admin@example.com")),
            first_name: Set("Admin".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(
                crate::auth::password::hash_password("test-password").expect("password hash"),
            )),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        db.execute_unprepared("DROP TABLE global_settings")
            .await
            .expect("drop global_settings table");

        let response = update_nats_settings(
            State(state),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::Password,
                None,
            )),
            None,
            Validated(UpdateNatsSettingsRequest {
                url: Some(serde_json::Value::Null),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("nats_url_clear_failed")
        );
        assert_eq!(details["operation"], serde_json::json!("clear"));
    }

    #[tokio::test]
    async fn save_nats_settings_persistence_failure_writes_failed_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant")
            .id;
        let state = test_state(db.clone(), tenant_id).await;

        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(uptrakit_shared_types::MaskedEmail::new("admin@example.com")),
            first_name: Set("Admin".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(
                crate::auth::password::hash_password("test-password").expect("password hash"),
            )),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        db.execute_unprepared("DROP TABLE global_settings")
            .await
            .expect("drop global_settings table");

        let response = update_nats_settings(
            State(state),
            CanManageSystemSettings::new(AuthenticatedUser::new(
                user.id,
                AuthMethod::Password,
                None,
            )),
            None,
            Validated(UpdateNatsSettingsRequest {
                url: Some(serde_json::json!("nats://demo:secret@localhost:4222")),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let row = latest_system_audit_row(&db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("nats_url_save_failed")
        );
        assert_eq!(details["operation"], serde_json::json!("save"));
    }
}
