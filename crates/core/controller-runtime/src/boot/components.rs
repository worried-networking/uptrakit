//! Web-API component construction — the tail of the controller boot sequence.
//!
//! [`build`] constructs every value that [`uptrakit_web_api::AppState`] needs
//! beyond what the PKI/identity and settings phases already produce.  All NATS
//! wiring is delegated to [`super::nats`] (feature-gated) so this module stays
//! `#[cfg]`-free.
//!
//! The return type, [`Components`], groups fields into cohesive sub-bundles to
//! keep the top-level field count at nine.

use std::sync::Arc;

use rootcause::prelude::*;
use tokio_util::sync::CancellationToken;
use uptrakit_plugin_infrastructure_registry::{PluginHttpClientConfig, build_plugin_http_client};

use crate::AppError;
use crate::boot::config::BootConfig;
use crate::boot::crypto::MasterKey;
use crate::boot::persistence::Persistence;
use crate::boot::settings::SettingsBundle;

// ---------------------------------------------------------------------------
// Sub-bundles
// ---------------------------------------------------------------------------

/// Audit log infrastructure.
pub(crate) struct AuditBits {
    pub dispatcher: uptrakit_audit_log::AuditLogDispatcher,
    pub emitter: uptrakit_audit_log::AuditEmitter,
}

/// Notification service and associated broadcasters.
pub(crate) struct NotificationBits {
    pub service: uptrakit_web_api::notification_service::NotificationService,
    pub dispatcher: uptrakit_web_api::notifications::dispatcher::NotificationDispatcher,
    pub event_broadcaster: uptrakit_web_api::event_broadcaster::EventBroadcaster,
    pub batch_progress_broadcaster:
        uptrakit_web_api::batch_progress_broadcaster::BatchProgressBroadcaster,
}

/// Plugin catalog and surface infrastructure.
pub(crate) struct PluginBits {
    pub plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps>,
    pub instance_snapshot_handle: Arc<
        arc_swap::ArcSwap<
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
        >,
    >,
    pub surface_registry: Arc<uptrakit_web_api::surface_registry::SurfaceRegistry>,
    pub surface_proxy: Arc<uptrakit_web_api::surface_proxy::SurfaceProxy>,
    pub embedded_host: Arc<crate::embedded::EmbeddedServiceHost>,
}

/// Authentication stores and denylist.
pub(crate) struct AuthStores {
    pub device_flow: uptrakit_web_api::auth::device_flow::DeviceFlowStore,
    pub rate_limit: uptrakit_web_api::auth::rate_limit::RateLimitStore,
    pub token_denylist: Arc<uptrakit_web_api::auth::token_denylist::TokenDenylist>,
    pub global_providers: Arc<uptrakit_web_api::global_providers::GlobalProviders>,
    #[cfg(feature = "oidc")]
    pub oidc_flow_store: uptrakit_web_api::auth::oidc_state::OidcFlowStore,
    #[cfg(feature = "oidc")]
    pub account_link_store: uptrakit_web_api::auth::oidc_state::AccountLinkStore,
    #[cfg(feature = "oidc")]
    pub oidc_token_exchange_store: uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore,
    #[cfg(feature = "oidc")]
    pub oidc_registration_store: uptrakit_web_api::auth::oidc_state::OidcRegistrationStore,
}

// ---------------------------------------------------------------------------
// Top-level Components
// ---------------------------------------------------------------------------

/// All values produced by the web-API component construction phase.
///
/// Nine top-level fields; complex groups are factored into sub-bundles above.
pub(crate) struct Components {
    pub controller_id: uuid::Uuid,
    pub workload_claim_registry: Arc<uptrakit_web_api::workload_claims::WorkloadClaimRegistry>,
    pub shutdown_token: CancellationToken,
    pub credential_sources: uptrakit_web_api::ServiceCredentialSources,
    pub service_connections: uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    pub audit: AuditBits,
    pub notification: NotificationBits,
    pub plugins: PluginBits,
    pub auth: AuthStores,
    /// NATS transport, present only when the `nats` feature is enabled and a
    /// NATS URL is configured.
    #[cfg(feature = "nats")]
    pub nats_transport: Option<uptrakit_web_api::nats_transport::NatsTransport>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build all web-API components from the outputs of earlier boot phases.
///
/// Corresponds to the block in `boot::run_server` that starts after the
/// cert_signer/oauth/jwt are available and ends just before `AppState::builder()`.
pub(crate) async fn build(
    cfg: &BootConfig,
    db: &Persistence,
    settings: &SettingsBundle,
    crypto: &MasterKey,
) -> crate::Result<Components> {
    let db_conn = &db.db;
    let db_url = &db.url;
    let reconciled = &settings.reconciled;
    let runtime = &cfg.booted.runtime;

    // OIDC stores (feature-gated)
    #[cfg(feature = "oidc")]
    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new(db_conn.clone());
    #[cfg(feature = "oidc")]
    let account_link_store =
        uptrakit_web_api::auth::oidc_state::AccountLinkStore::new(db_conn.clone());
    #[cfg(feature = "oidc")]
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new(db_conn.clone());
    #[cfg(feature = "oidc")]
    let oidc_registration_store =
        uptrakit_web_api::auth::oidc_state::OidcRegistrationStore::new(db_conn.clone());

    let device_flow_store =
        uptrakit_web_api::auth::device_flow::DeviceFlowStore::new(db_conn.clone());
    let rate_limit_store = uptrakit_web_api::auth::rate_limit::RateLimitStore::new(db_conn.clone());

    let service_connections =
        uptrakit_web_api::service_connections::ServiceConnectionRegistry::new();
    let controller_id = uuid::Uuid::now_v7();
    let workload_claim_registry =
        Arc::new(uptrakit_web_api::workload_claims::WorkloadClaimRegistry::new());

    let notification_service_base =
        uptrakit_web_api::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        )
        .with_claim_registry(Arc::clone(&workload_claim_registry));

    // Build the batch progress broadcaster (NATS wiring applied below).
    let batch_progress_broadcaster_base =
        uptrakit_web_api::batch_progress_broadcaster::BatchProgressBroadcaster::new();

    // Build the admin event broadcaster (NATS wiring applied below).
    let event_broadcaster_base = uptrakit_web_api::event_broadcaster::EventBroadcaster::new();

    // NATS wiring: connects to the server and augments the three objects above.
    // The entire block is feature-gated; without nats, we keep the base objects.
    #[cfg(feature = "nats")]
    let (nats_transport, notification_service, event_broadcaster, batch_progress_broadcaster) = {
        let bits = super::nats::wire(
            reconciled.nats_url.as_deref(),
            controller_id,
            notification_service_base,
            event_broadcaster_base,
            batch_progress_broadcaster_base,
        )
        .await?;
        (
            bits.transport,
            bits.notification_service,
            bits.event_broadcaster,
            bits.batch_progress_broadcaster,
        )
    };
    #[cfg(not(feature = "nats"))]
    let (notification_service, event_broadcaster, batch_progress_broadcaster) = (
        notification_service_base,
        event_broadcaster_base,
        batch_progress_broadcaster_base,
    );

    let token_denylist = Arc::new(
        uptrakit_web_api::auth::token_denylist::TokenDenylist::new_with_db(db_conn.clone()),
    );
    let global_providers = Arc::new(uptrakit_web_api::global_providers::GlobalProviders::new(
        db_conn.clone(),
    ));

    // Shared cancellation token.
    let shutdown_token = CancellationToken::new();

    // Instance-scoped plugin state.
    let instance_plugin_snapshot =
        uptrakit_web_api_queries::instance_plugin_settings::load_at_boot(db_conn)
            .await
            .map_err(|e| {
                report!(AppError::Config(format!(
                    "failed to load instance plugin snapshot: {e}"
                )))
            })?;
    tracing::info!(
        plugin_count = instance_plugin_snapshot.iter().count(),
        "instance plugin snapshot loaded"
    );

    let all_descriptors = uptrakit_plugin_infrastructure_registry::all_descriptors();
    let instance_states = uptrakit_plugin_infrastructure_registry::InstancePluginStates::from_pairs(
        all_descriptors
            .iter()
            .filter(|d| d.scope == uptrakit_plugin_infrastructure_registry::PluginScope::Instance)
            .map(|d| (d.type_id, instance_plugin_snapshot.enabled(d.type_id))),
    );

    let instance_snapshot_handle =
        Arc::new(arc_swap::ArcSwap::from_pointee(instance_plugin_snapshot));

    // Plugin catalog.
    let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig {
        allow_private_urls: false,
        http_client: Some(
            build_plugin_http_client(PluginHttpClientConfig {
                user_agent: "uptrakit-controller",
                redirect_policy: reqwest::redirect::Policy::limited(5),
                ..Default::default()
            })
            .map_err(|e| report!(AppError::Config(format!("plugin catalog HTTP client: {e}"))))?,
        ),
        cancellation_token: Some(shutdown_token.clone()),
        global_provider_lookup: Some(global_providers.clone()),
    };
    let catalog =
        uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config, instance_states)
            .context_transform(|_| {
                AppError::Config("failed to build plugin catalog".to_string())
            })?;

    let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(catalog);

    tracing::info!(
        update_protection = plugin_ops.controller_update_protection().is_some(),
        "plugin catalog ready"
    );

    let callback_base_url = format!("https://{}", reconciled.https_addr);
    let notification_dispatcher =
        uptrakit_web_api::notifications::dispatcher::NotificationDispatcher::new(
            db_conn.clone(),
            Arc::clone(&plugin_ops),
            callback_base_url,
        );

    // Credential sources.
    let credential_sources = {
        #[cfg_attr(
            not(feature = "nats"),
            expect(
                unused_mut,
                reason = "only mutated inside the #[cfg(feature = \"nats\")] block below"
            )
        )]
        let mut sources = uptrakit_web_api::ServiceCredentialSources::new(
            Some(db_url.clone()),
            None,
            crypto.hex.clone(),
        );
        #[cfg(feature = "nats")]
        if let Some(ref url) = reconciled.nats_url {
            sources.nats_url = Some(url.clone());
        }
        sources
    };

    // Audit log backend and filter wiring.
    let audit_dispatcher = crate::build_audit_logger(runtime, db_conn).await?;

    let surface_registry = Arc::new(uptrakit_web_api::surface_registry::SurfaceRegistry::new(
        uptrakit_web_api::surface_registry::SurfaceRegistryConfig::default(),
    ));
    for registration in plugin_ops.surface_registrations() {
        let provider_id = registration.provider.provider_id.clone();
        surface_registry
            .bootstrap_plugin(registration)
            .map_err(|error| {
                report!(AppError::Config(format!(
                    "failed to bootstrap plugin surfaces for provider {provider_id}: {error}"
                )))
            })?;
    }

    let audit_emitter = uptrakit_audit_log::AuditEmitter::new(audit_dispatcher.clone());
    let surface_proxy = Arc::new(
        uptrakit_web_api::surface_proxy::SurfaceProxy::new().with_local_executor(Arc::new(
            uptrakit_web_api::surface_proxy::PluginSurfaceLocalExecutor::new(
                Arc::new(db_conn.clone()),
                Arc::clone(&plugin_ops),
            )
            .with_audit_emitter(audit_emitter.clone()),
        )),
    );

    // Embedded service host.
    let embedded_host = Arc::new(crate::embedded::EmbeddedServiceHost::new());

    Ok(Components {
        controller_id,
        workload_claim_registry,
        shutdown_token,
        credential_sources,
        service_connections,
        audit: AuditBits {
            dispatcher: audit_dispatcher,
            emitter: audit_emitter,
        },
        notification: NotificationBits {
            service: notification_service,
            dispatcher: notification_dispatcher,
            event_broadcaster,
            batch_progress_broadcaster,
        },
        plugins: PluginBits {
            plugin_ops,
            instance_snapshot_handle,
            surface_registry,
            surface_proxy,
            embedded_host,
        },
        auth: AuthStores {
            device_flow: device_flow_store,
            rate_limit: rate_limit_store,
            token_denylist,
            global_providers,
            #[cfg(feature = "oidc")]
            oidc_flow_store,
            #[cfg(feature = "oidc")]
            account_link_store,
            #[cfg(feature = "oidc")]
            oidc_token_exchange_store,
            #[cfg(feature = "oidc")]
            oidc_registration_store,
        },
        #[cfg(feature = "nats")]
        nats_transport,
    })
}
