//! Controller boot sequence.
//!
//! This module owns the top-level `run_server` entry point that drives
//! every phase of controller startup, from config loading through to the
//! main event loop.  Helper functions (`build_audit_logger`,
//! `spawn_background_tasks`, etc.) remain in the crate root so that later
//! per-phase extraction tasks can move them independently.

pub(crate) mod config;
pub(crate) mod crypto;
pub(crate) mod directories;
pub(crate) mod identity;
pub(crate) mod listeners;
pub(crate) mod persistence;
pub(crate) mod settings;

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use tokio_util::sync::CancellationToken;
use uptrakit_build_info::BuildInfo;
use uptrakit_plugin_infrastructure_registry::{PluginHttpClientConfig, build_plugin_http_client};
use uptrakit_web_api::AppState;
use uptrakit_web_api::oauth::boot::deregister_oauth_instance;

use crate::{AppError, ReloadBridgeChannels};

pub(crate) async fn run_server(args: crate::cli::Args, info: BuildInfo) -> crate::Result<()> {
    // Phase 0: Load TOML config, parse bootstrap env args, initialise tracing.
    let cfg = config::load(args, &info).await?;

    // Phase 1: Master key initialization — reads from --master-key-from or TOML
    // master_key as a fallback. The TOML value already carries the full source
    // string (file:, env:, or inline hex) so no prefix injection is needed.
    let crypto = crypto::init(&cfg)?;
    let master_key_hex = crypto.hex;

    let config_path_for_coord = cfg.config_path.clone();

    // Phase 2: Application directories — use platform defaults (no CLI overrides).
    let layout = directories::resolve().await?;
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    let controller_installation_id = layout.installation_id;

    // Phase 3: Database — URL and pool size from TOML [db].
    let db = persistence::open(&cfg, &layout).await?;

    // Phases 4/4b/4c/4d: master key verify, column AAD mappings, data key ring, ENC:v3 migration
    crypto::verify_and_migrate(&db.db).await?;

    // Phases 5, 6, 7, 7b, 7c, 8: load settings, reconcile, seed, validate
    let settings_bundle = settings::load_and_seed(&cfg, &db).await?;

    // Destructure cfg and db after Phases 3–8 so earlier phases can borrow them.
    let booted = cfg.booted;
    let args = cfg.args;
    let runtime = &booted.runtime;
    let persistence::Persistence {
        db: db_conn,
        url: db_url,
        default_tenant_id,
    } = db;

    // Phase 8b: Claim inherited TCP sockets and pre-bind listeners (FD-atomic).
    //
    // This must happen before the coordinator block so that `listener_count` and
    // `https_std`/`pki_std_for_spawn` are in scope when the reexec hook is
    // constructed and when the server task is spawned.
    let listeners::Listeners {
        https_std,
        pki_std_for_spawn,
        listener_count,
        first_listener_fd,
    } = listeners::claim(&settings_bundle)?;

    // Destructure the settings bundle now that listener FDs are claimed.
    let settings::SettingsBundle {
        settings,
        reconciled,
        validated,
    } = settings_bundle;

    // Phases 7d, 9, 10: OAuth boot, PKI/TLS init, cert_signer construction, JWT init.
    let identity::Identity {
        pki:
            identity::PkiFields {
                ca_managed,
                pki_path,
                ca_tx,
                ca_rx,
                ca_key_store,
                rustls_config,
                server_cert_resolver,
                revocation_notify,
                ca_rotation_trigger,
                crl_pem_cache,
                crl_manager,
                initial_ca_version,
                has_external_tls_cert,
            },
        jwt_manager,
        cert_signer,
        oauth_state,
        oauth_instance_for_shutdown,
    } = identity::init(
        runtime,
        &db_conn,
        layout.app_dirs.config_dir(),
        layout.app_dirs.state_dir(),
        &reconciled,
    )
    .await?;

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
    #[cfg_attr(
        not(feature = "nats"),
        expect(
            unused_mut,
            reason = "only mutated inside the #[cfg(feature = \"nats\")] block below"
        )
    )]
    let mut notification_service =
        uptrakit_web_api::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        )
        .with_claim_registry(Arc::clone(&workload_claim_registry));

    // NATS transport (optional, feature-gated)
    // Uses the reconciled NATS URL (DB value wins over TOML; TOML seeds DB on first run).
    #[cfg(feature = "nats")]
    let nats_transport = if let Some(ref url) = reconciled.nats_url {
        let nats = uptrakit_web_api::nats_transport::NatsTransport::connect(url, controller_id)
            .await
            .context_transform(|e| {
                use uptrakit_web_api::nats_transport::NatsTransportError;
                match e {
                    NatsTransportError::Connection(msg) => {
                        AppError::Config(format!("NATS connection failed: {msg}"))
                    }
                    NatsTransportError::JetStream(msg) => AppError::Config(format!(
                        "NATS JetStream setup failed: {msg}\n\
                         Ensure JetStream is enabled on the NATS server: start with the \
                         -js flag, or add `jetstream: {{enabled: true}}` to nats-server.conf"
                    )),
                    _ => AppError::Config("NATS initialization failed".to_string()),
                }
            })?;
        notification_service = notification_service.with_nats(Arc::new(nats.clone()));
        Some(nats)
    } else {
        None
    };

    // Build the batch progress broadcaster with NATS for cross-instance SSE.
    // When NATS is not configured the broadcaster operates in single-instance mode.
    let batch_progress_broadcaster =
        uptrakit_web_api::batch_progress_broadcaster::BatchProgressBroadcaster::new();
    #[cfg(feature = "nats")]
    let batch_progress_broadcaster = if let Some(ref nats) = nats_transport {
        batch_progress_broadcaster.with_nats(nats.nats_client())
    } else {
        batch_progress_broadcaster
    };

    // Build the admin event broadcaster with NATS for cross-instance SSE fan-out.
    // When NATS is not configured the broadcaster operates in single-instance mode.
    #[cfg_attr(
        not(feature = "nats"),
        expect(
            unused_mut,
            reason = "only mutated inside the #[cfg(feature = \"nats\")] block below"
        )
    )]
    let mut event_broadcaster = uptrakit_web_api::event_broadcaster::EventBroadcaster::new();
    #[cfg(feature = "nats")]
    if let Some(ref nats) = nats_transport {
        event_broadcaster = event_broadcaster.with_nats(Arc::new(nats.clone()), controller_id);
    }

    let token_denylist = Arc::new(
        uptrakit_web_api::auth::token_denylist::TokenDenylist::new_with_db(db_conn.clone()),
    );
    let global_providers = Arc::new(uptrakit_web_api::global_providers::GlobalProviders::new(
        db_conn.clone(),
    ));

    // Shared cancellation token: cancelled by BackgroundTasks::shutdown(), which
    // also signals open SSE streams in the web API to terminate cleanly.
    let shutdown_token = CancellationToken::new();

    // Load instance-scoped plugin state from DB before catalog construction so
    // that instance-gated plugins reflect their persisted enabled/disabled state
    // from first request rather than requiring a restart after toggling.
    let instance_plugin_snapshot =
        uptrakit_web_api_queries::instance_plugin_settings::load_at_boot(&db_conn)
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

    // Build InstancePluginStates by intersecting the snapshot with all
    // compiled-in instance-scoped descriptors.
    let all_descriptors = uptrakit_plugin_infrastructure_registry::all_descriptors();
    let instance_states = uptrakit_plugin_infrastructure_registry::InstancePluginStates::from_pairs(
        all_descriptors
            .iter()
            .filter(|d| d.scope == uptrakit_plugin_infrastructure_registry::PluginScope::Instance)
            .map(|d| (d.type_id, instance_plugin_snapshot.enabled(d.type_id))),
    );

    // Wrap the snapshot in Arc<ArcSwap<>> so AppState can serve lock-free reads
    // on the hot path and routes can atomically publish upserts.
    let instance_plugin_snapshot_handle =
        std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(instance_plugin_snapshot));

    // Build the plugin catalog from all compiled-in descriptors.
    // The catalog replaces the old PluginRegistry and provides PluginOps.
    // allow_private_urls defaults to false (SSRF-safe by default).
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

    // Build credential sources for external services that need direct infrastructure access.
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
            master_key_hex,
        );
        #[cfg(feature = "nats")]
        if let Some(ref url) = reconciled.nats_url {
            sources.nats_url = Some(url.clone());
        }
        sources
    };

    // Audit log backend and filter wiring.
    let audit_dispatcher = crate::build_audit_logger(runtime, &db_conn).await?;

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

    // Create the embedded service host before AppState so it can be stored
    // in the state. The host's `add()` is called later for embedded service registration.
    let embedded_host = Arc::new(crate::embedded::EmbeddedServiceHost::new());
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    let builtin_host = crate::service_host::BuiltinServiceHost::new(Arc::clone(&embedded_host));

    let builder = AppState::builder()
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .db(db_conn.clone())
        .settings(settings)
        .cert_signer(cert_signer)
        .service_connections(service_connections.clone())
        .revocation_notify(revocation_notify)
        .embedded_service_notifier(
            Arc::clone(&embedded_host) as Arc<dyn uptrakit_web_api::EmbeddedServiceNotifier>
        )
        .jwt(Arc::new(jwt_manager))
        .device_flow_store(device_flow_store)
        .rate_limit_store(rate_limit_store)
        .pki_path(pki_path)
        .rustls_config(rustls_config.clone())
        .server_cert_resolver(std::sync::Arc::clone(&server_cert_resolver)
            as std::sync::Arc<dyn uptrakit_web_api::server_cert_swap::ServerCertSwap>)
        .crl_pem_cache(crl_pem_cache)
        .ca_rotation_trigger(ca_rotation_trigger)
        .default_tenant_id(default_tenant_id)
        .controller_id(controller_id)
        .notification_service(notification_service)
        .notification_dispatcher(notification_dispatcher)
        .token_denylist(token_denylist)
        .credential_sources(credential_sources)
        .global_providers(global_providers)
        .event_broadcaster(event_broadcaster.clone())
        .batch_progress_broadcaster(batch_progress_broadcaster)
        .shutdown_token(shutdown_token.clone())
        .audit_log_dispatcher(audit_dispatcher.clone())
        .audit_emitter(audit_emitter.clone())
        .plugin_ops(plugin_ops)
        .surface_registry(surface_registry)
        .surface_proxy(surface_proxy)
        .workload_claim_registry(workload_claim_registry)
        .instance_plugin_snapshot(std::sync::Arc::clone(&instance_plugin_snapshot_handle))
        .reject_dangerous_commands(true)
        .oauth(oauth_state);

    // Wire config-reload coordinator and receivers.
    //
    // Build each Reloadable from the loaded RuntimeConfig + available subsystem
    // handles, extend the coordinator (which was not yet spawned), extract a
    // handle, then spawn coordinator + reconciler.
    let (
        coordinator_handle_opt,
        settings_version_cache_opt,
        receivers_opt,
        reload_file_state_rx_opt,
        reload_last_reload_rx_opt,
        reload_recent_events_rx_opt,
        audit_log_filter_rx_opt,
    ) = {
        let mut b = booted;
        // DB → TLS → Listeners → NATS → Audit → Zeroconf → Plugins → Embedded
        let db_reloadable =
            crate::reload::db_pool::DbPoolReloadable::new(db_conn.clone(), db_url.clone());
        let db_rx = db_reloadable.subscribe();
        let (tls_reloadable, _tls_rx) =
            crate::reload::tls_snapshot::TlsSnapshotReloadable::new(b.runtime.tls.clone());
        let (https_reloadable, _https_rx) =
            crate::reload::https_listener::HttpsListenerReloadable::new(
                b.runtime.network.https.clone(),
            );
        let (pki_reloadable, _pki_rx) = crate::reload::pki_listener::PkiListenerReloadable::new(
            b.runtime.network.pki_addr.clone(),
        );
        let (audit_reloadable, audit_log_filter_rx) =
            crate::reload::audit::AuditDispatcherReloadable::new(
                audit_dispatcher.clone(),
                b.runtime.audit.clone(),
            );
        let (zeroconf_reloadable, _zeroconf_rx) =
            crate::reload::zeroconf::ZeroconfReloadable::new(b.runtime.zeroconf.clone());
        let (plugin_reloadable, _plugin_rx) =
            uptrakit_web_api_queries::reload::plugin_registry::PluginCatalogReloadable::new(
                uptrakit_config_reload::config::PluginsConfig::default(),
            );
        let (embedded_reloadable, _embedded_rx) =
            crate::reload::embedded::EmbeddedServicesReloadable::new(
                b.runtime.embedded_services.clone(),
            );

        #[cfg_attr(
            not(feature = "nats"),
            expect(
                unused_mut,
                reason = "only pushed inside the #[cfg(feature = \"nats\")] block below"
            )
        )]
        let mut reloadables: Vec<
            std::sync::Arc<dyn uptrakit_config_reload::ReloadableErased>,
        > = vec![
            Arc::new(db_reloadable),
            Arc::new(tls_reloadable),
            Arc::new(https_reloadable),
            Arc::new(pki_reloadable),
            Arc::new(audit_reloadable),
            Arc::new(zeroconf_reloadable),
            Arc::new(plugin_reloadable),
            Arc::new(embedded_reloadable),
        ];

        #[cfg(feature = "nats")]
        if let (Some(nats), Some(url)) = (&nats_transport, &reconciled.nats_url) {
            reloadables.push(Arc::new(crate::reload::nats::NatsReloadable::new(
                nats.nats_client(),
                url.clone(),
            )));
        }

        b.coordinator.extend_reloadables(reloadables);
        b.coordinator.set_alert_writer(std::sync::Arc::new(
            crate::reload::audit::AuditAlertWriter::new(audit_emitter.clone()),
        ));

        let current_exe = std::env::current_exe()
            .map_err(|e| report!(AppError::Config(format!("resolve current_exe: {e}"))))?;
        b.coordinator.set_config_path(config_path_for_coord.clone());
        b.coordinator
            .set_current_config(Arc::new(b.runtime.clone()));
        b.coordinator
            .set_reexec_hook(Box::new(crate::ControllerReexecHook {
                current_exe,
                config_path: config_path_for_coord.clone(),
                master_key_from: args.master_key_from.clone(),
                generation: crate::reexec::listenfd::current_generation(),
                listener_count,
                first_listener_fd,
                oauth_instance: oauth_instance_for_shutdown.clone(),
            }));

        let coordinator_handle = b.coordinator.handle();

        let _reconciler = crate::reload::reconciler::spawn_config_reconciler(
            db_rx,
            coordinator_handle.sender(),
            b.settings_version_cache.clone(),
            shutdown_token.clone(),
        );

        tokio::spawn(b.coordinator.run());

        let audit_rx = b.audit_rx;
        let reload_file_state_tx = b.reload_file_state_tx;
        let reload_file_state_rx = b.reload_file_state_rx;
        let reload_last_reload_tx = b.reload_last_reload_tx;
        let reload_last_reload_rx = b.reload_last_reload_rx;
        let reload_recent_events_tx = b.reload_recent_events_tx;
        let reload_recent_events_rx = b.reload_recent_events_rx;
        tokio::spawn(crate::reload_audit_bridge(
            audit_rx,
            audit_emitter,
            ReloadBridgeChannels {
                file_state_tx: reload_file_state_tx,
                last_reload_tx: reload_last_reload_tx,
                recent_events_tx: reload_recent_events_tx,
                config_path: config_path_for_coord.clone(),
            },
        ));

        (
            Some(coordinator_handle),
            Some(b.settings_version_cache),
            Some(b.receivers),
            Some(reload_file_state_rx),
            Some(reload_last_reload_rx),
            Some(reload_recent_events_rx),
            Some(audit_log_filter_rx),
        )
    };

    let builder = match (
        coordinator_handle_opt,
        settings_version_cache_opt,
        receivers_opt,
        reload_file_state_rx_opt,
        reload_last_reload_rx_opt,
        reload_recent_events_rx_opt,
        audit_log_filter_rx_opt,
    ) {
        (
            Some(handle),
            Some(cache),
            Some(receivers),
            Some(fs_rx),
            Some(lr_rx),
            Some(re_rx),
            Some(audit_filter_rx),
        ) => builder
            .coordinator_handle(handle)
            .settings_version_cache(cache)
            .config_receivers(receivers)
            .config_reload_status_receivers(fs_rx, lr_rx, re_rx)
            .audit_log_filter_rx(audit_filter_rx),
        _ => builder,
    };

    #[cfg(feature = "oidc")]
    let builder = builder
        .oidc_flow_store(oidc_flow_store)
        .account_link_store(account_link_store)
        .oidc_token_exchange_store(oidc_token_exchange_store)
        .oidc_registration_store(oidc_registration_store);

    let app_state = Arc::new(
        builder
            .build()
            .map_err(|e| report!(AppError::Config(format!("failed to build AppState: {e}"))))?,
    );

    #[cfg(feature = "test-utils")]
    if let Some(notify) = app_state.test_reexec_notify() {
        // current_exe was moved into ControllerReexecHook at set_reexec_hook() above.
        // Call current_exe() a second time — the OS call is cheap at startup.
        let current_exe = std::env::current_exe().map_err(|e| {
            report!(AppError::Config(format!(
                "resolve current_exe (test-utils): {e}"
            )))
        })?;
        let plan = crate::reexec::ReexecPlan {
            current_exe,
            config_path: config_path_for_coord.clone(),
            master_key_from: args.master_key_from.clone(),
            listener_count,
            generation: crate::reexec::listenfd::current_generation(),
            first_listener_fd,
        };
        tokio::spawn(async move {
            notify.notified().await;
            tracing::warn!(
                "test-utils force_reexec: triggering unconditional reexec at generation {}; \
                 a concurrent coordinator-driven reexec at this moment would produce an \
                 unexpected generation number in the integration test",
                plan.generation
            );
            // Brief pause to allow the 202 ACCEPTED response to be flushed by the HTTP
            // layer before exec() replaces the process image. Without this, the response
            // can be dropped by the kernel mid-send on multi-threaded runtimes.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            // perform_reexec returns Result<Infallible, _>; the Ok branch is unreachable.
            // On Err, exec() itself failed (binary not at path) — process stays alive
            // and the integration test times out rather than hanging forever.
            match crate::reexec::perform_reexec(&plan) {
                Ok(infallible) => match infallible {},
                Err(e) => tracing::error!(error = %e, "test-utils force_reexec: exec failed"),
            }
        });
    }

    uptrakit_web_api::global_providers::github::emit_global_github_provider_diagnostic_if_needed(
        app_state.db(),
        &app_state.notification.event_broadcaster,
    )
    .await;

    let recovered =
        uptrakit_web_api::queries::update_batches::mark_all_in_progress_as_failed_for_rollout(
            app_state.db(),
        )
        .await
        .map_err(|e| {
            report!(AppError::Config(format!(
                "failed to run owner-aware rollout cleanup: {e}"
            )))
        })?;

    if !recovered.is_empty() {
        tracing::warn!(
            count = recovered.len(),
            "marked pre-existing in-progress updates as failed during owner-aware rollout cleanup"
        );

        for record in &recovered {
            #[cfg(feature = "plugin-ops")]
            if let Err(error) =
                uptrakit_web_api::queries::update_dispatch::finalize_post_update_hook(
                    app_state.db(),
                    app_state.controller_update_hook(),
                    app_state.plugin.plugin_ops.as_ref(),
                    record,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    update_id = %record.id,
                    "post-update hook (resource restore) failed during startup cleanup"
                );
            }

            if let Err(error) =
                uptrakit_web_api::queries::update_dispatch::finalize_post_update_with_timeout(
                    app_state.db(),
                    app_state.controller_update_protection(),
                    record,
                    Duration::from_secs(2),
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    update_id = %record.id,
                    "post-update finalization failed during startup cleanup"
                );
            }

            if let Some(batch_id) = record.batch_id {
                match uptrakit_web_api::queries::update_batches::dispatch_next_in_batch(
                    app_state.db(),
                    uptrakit_web_api::queries::update_dispatch::DispatchContext {
                        notifier: &app_state.notification.notification_service,
                        protection: app_state.controller_update_protection(),
                        #[cfg(feature = "plugin-ops")]
                        hook: app_state.controller_update_hook(),
                        #[cfg(feature = "plugin-ops")]
                        notification_ops: Some(app_state.plugin.plugin_ops.as_ref()),
                    },
                    batch_id,
                    record.host_id,
                    record.tenant_id,
                )
                .await
                {
                    Ok(Some(completion)) => {
                        tracing::debug!(
                            %batch_id,
                            status = %completion.status.as_str(),
                            completed = completion.completed_count,
                            failed = completion.failed_count,
                            "startup rollout cleanup intentionally does not replay retroactive batch-completion notifications"
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            %batch_id,
                            host_id = %record.host_id,
                            "failed to promote next queued batch item after rollout cleanup"
                        );
                    }
                }
            } else if let Err(error) =
                uptrakit_web_api::queries::update_batches::dispatch_next_queued_for_host(
                    app_state.db(),
                    uptrakit_web_api::queries::update_dispatch::DispatchContext {
                        notifier: &app_state.notification.notification_service,
                        protection: app_state.controller_update_protection(),
                        #[cfg(feature = "plugin-ops")]
                        hook: app_state.controller_update_hook(),
                        #[cfg(feature = "plugin-ops")]
                        notification_ops: Some(app_state.plugin.plugin_ops.as_ref()),
                    },
                    record.host_id,
                    record.tenant_id,
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    host_id = %record.host_id,
                    "failed to dispatch next queued update after rollout cleanup"
                );
            }
        }
    }

    // Seed the in-memory token denylist from DB before accepting traffic.
    // This ensures revocations made before a controller restart are honoured.
    app_state
        .auth
        .token_denylist
        .load_from_db()
        .await
        .map_err(|e| {
            report!(AppError::Config(format!(
                "failed to seed token denylist: {e}"
            )))
        })?;

    // Spawn background tasks
    let mut bg = crate::tasks::BackgroundTasks::new(shutdown_token);
    crate::spawn_background_tasks(
        &mut bg,
        &app_state,
        &crl_manager,
        ca_managed,
        &ca_tx,
        initial_ca_version,
        has_external_tls_cert,
        #[cfg(feature = "nats")]
        &service_connections,
        #[cfg(feature = "nats")]
        &nats_transport,
    )
    .await;

    // Embedded service registration. Failures are fatal — a broken embedded
    // service should not leave the deployment in an indeterminate state.
    #[cfg(feature = "embedded-scheduler")]
    crate::service_host::builtins::register_scheduler(
        &builtin_host,
        &app_state,
        &mut bg,
        controller_id,
        controller_installation_id,
        ca_managed,
        &ca_tx,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded scheduler: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-agent")]
    crate::service_host::builtins::register_agent(
        &builtin_host,
        &app_state,
        &mut bg,
        controller_installation_id,
        layout.app_dirs.state_dir().to_path_buf(),
        None,
        &info,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded agent: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-ssh-agent")]
    crate::service_host::builtins::register_agent_ssh(
        &builtin_host,
        &app_state,
        &mut bg,
        controller_installation_id,
        layout.app_dirs.state_dir().to_path_buf(),
        &info,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded SSH agent: {e}"
        )))
    })?;

    #[cfg(feature = "embedded-mqtt")]
    crate::service_host::builtins::register_mqtt(
        &builtin_host,
        &app_state,
        &mut bg,
        controller_installation_id,
    )
    .await
    .map_err(|e| {
        report!(AppError::Config(format!(
            "failed to start embedded MQTT service: {e}"
        )))
    })?;

    // Set up signal handlers
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_transform(|e| {
            AppError::Config(format!("failed to set up SIGTERM handler: {e}"))
        })?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context_transform(|e| AppError::Config(format!("failed to set up SIGINT handler: {e}")))?;

    // Spawn HTTPS server
    let server_handle = axum_server::Handle::new();
    let server_options = crate::server::ServerOptions {
        https_addr: reconciled.https_addr,
        rustls_config,
        app_state: Arc::clone(&app_state),
        static_dir: validated.static_dir,
        handle: server_handle.clone(),
        inherited_listener: Some(https_std),
    };
    let server_task = tokio::spawn(crate::server::run(server_options));

    // Spawn zeroconf mDNS advertiser if enabled
    #[cfg(feature = "zeroconf")]
    crate::spawn_zeroconf(&mut bg, &app_state, reconciled.https_addr);

    // Spawn PKI HTTP server if needed
    crate::spawn_pki_http(
        &mut bg,
        &app_state,
        validated.pki_http_port,
        pki_std_for_spawn,
    );

    // Notify the service manager (and stdout-based supervisors) that all
    // servers are bound and the controller is ready to accept connections.
    crate::reexec::sd_notify::signal_ready();

    // Main event loop — wait for shutdown signal or server exit
    let mut server_task = server_task;
    let shutdown_reason = tokio::select! {
        result = &mut server_task => {
            match result {
                Ok(Ok(())) => {
                    tracing::info!("server task exited normally");
                    "server exit"
                }
                Ok(Err(e)) => {
                    tracing::error!(error = ?e, "server error");
                    return Err(e).context(AppError::Server)?;
                }
                Err(e) => {
                    tracing::error!(error = %e, "server task panicked");
                    "server panic"
                }
            }
        }
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM, initiating graceful shutdown");
            "SIGTERM"
        }
        _ = sigint.recv() => {
            tracing::info!("received SIGINT, initiating graceful shutdown");
            "SIGINT"
        }
    };

    // Graceful shutdown — 30 second default timeout
    tracing::info!(reason = shutdown_reason, "shutdown signal received");
    let shutdown_timeout = Duration::from_secs(30);
    bg.shutdown(server_handle, service_connections, shutdown_timeout)
        .await;

    if let Some((instance_id, ref db)) = oauth_instance_for_shutdown {
        deregister_oauth_instance(db, instance_id).await;
    }

    Ok(())
}
