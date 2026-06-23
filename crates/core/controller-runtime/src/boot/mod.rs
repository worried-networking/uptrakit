//! Controller boot sequence.
//!
//! This module owns the top-level `run_server` entry point that drives
//! every phase of controller startup, from config loading through to the
//! main event loop.  Helper functions (`build_audit_logger`,
//! `spawn_background_tasks`, etc.) remain in the crate root so that later
//! per-phase extraction tasks can move them independently.

pub(crate) mod components;
pub(crate) mod config;
pub(crate) mod crypto;
pub(crate) mod directories;
pub(crate) mod identity;
pub(crate) mod listeners;
#[cfg(feature = "nats")]
pub(crate) mod nats;
pub(crate) mod persistence;
pub(crate) mod settings;

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_build_info::BuildInfo;
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

    // Phases 7d, 9, 10: OAuth boot, PKI/TLS init, cert_signer construction, JWT init.
    // identity::init borrows cfg (via runtime), db.db, and settings_bundle.reconciled,
    // so it runs before any of those are destructured.
    let identity = identity::init(
        &cfg.booted.runtime,
        &db.db,
        layout.app_dirs.config_dir(),
        layout.app_dirs.state_dir(),
        &settings_bundle.reconciled,
    )
    .await?;

    // Build all web-API components (stores, plugin catalog, audit, broadcasters, etc.).
    // Borrows cfg, db, settings_bundle, and crypto by reference so none of them
    // need to be destructured yet.
    let components = components::build(&cfg, &db, &settings_bundle, &crypto).await?;

    // Destructure cfg and db now that components::build has finished borrowing them.
    let booted = cfg.booted;
    let args = cfg.args;
    let persistence::Persistence {
        db: db_conn,
        url: db_url,
        default_tenant_id,
    } = db;

    // Destructure the settings bundle now that listener FDs are claimed and
    // components have been built.
    let settings::SettingsBundle {
        settings,
        reconciled,
        validated,
    } = settings_bundle;

    // Destructure identity into flat locals for the AppState builder and
    // background-task spawner.
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
    } = identity;

    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    let builtin_host =
        crate::service_host::BuiltinServiceHost::new(Arc::clone(&components.plugins.embedded_host));

    let builder = AppState::builder()
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .db(db_conn.clone())
        .settings(settings)
        .cert_signer(cert_signer)
        .service_connections(components.service_connections.clone())
        .revocation_notify(revocation_notify)
        .embedded_service_notifier(Arc::clone(&components.plugins.embedded_host)
            as Arc<dyn uptrakit_web_api::EmbeddedServiceNotifier>)
        .jwt(Arc::new(jwt_manager))
        .device_flow_store(components.auth.device_flow)
        .rate_limit_store(components.auth.rate_limit)
        .pki_path(pki_path)
        .rustls_config(rustls_config.clone())
        .server_cert_resolver(std::sync::Arc::clone(&server_cert_resolver)
            as std::sync::Arc<dyn uptrakit_web_api::server_cert_swap::ServerCertSwap>)
        .crl_pem_cache(crl_pem_cache)
        .ca_rotation_trigger(ca_rotation_trigger)
        .default_tenant_id(default_tenant_id)
        .controller_id(components.controller_id)
        .notification_service(components.notification.service)
        .notification_dispatcher(components.notification.dispatcher)
        .token_denylist(components.auth.token_denylist)
        .credential_sources(components.credential_sources)
        .global_providers(components.auth.global_providers)
        .event_broadcaster(components.notification.event_broadcaster.clone())
        .batch_progress_broadcaster(components.notification.batch_progress_broadcaster)
        .shutdown_token(components.shutdown_token.clone())
        .audit_log_dispatcher(components.audit.dispatcher.clone())
        .audit_emitter(components.audit.emitter.clone())
        .plugin_ops(components.plugins.plugin_ops)
        .surface_registry(components.plugins.surface_registry)
        .surface_proxy(components.plugins.surface_proxy)
        .workload_claim_registry(components.workload_claim_registry)
        .instance_plugin_snapshot(Arc::clone(&components.plugins.instance_snapshot_handle))
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
                components.audit.dispatcher.clone(),
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
        if let (Some(nats), Some(url)) = (&components.nats_transport, &reconciled.nats_url) {
            reloadables.push(Arc::new(crate::reload::nats::NatsReloadable::new(
                nats.nats_client(),
                url.clone(),
            )));
        }

        b.coordinator.extend_reloadables(reloadables);
        b.coordinator.set_alert_writer(std::sync::Arc::new(
            crate::reload::audit::AuditAlertWriter::new(components.audit.emitter.clone()),
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
            components.shutdown_token.clone(),
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
            components.audit.emitter.clone(),
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
        .oidc_flow_store(components.auth.oidc_flow_store)
        .account_link_store(components.auth.account_link_store)
        .oidc_token_exchange_store(components.auth.oidc_token_exchange_store)
        .oidc_registration_store(components.auth.oidc_registration_store);

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

    // Extract the service_connections and shutdown_token from components before
    // consuming components for background tasks (CancellationToken is Clone;
    // ServiceConnectionRegistry is Clone).
    let service_connections = components.service_connections.clone();
    #[cfg(feature = "nats")]
    let nats_transport = components.nats_transport;
    let shutdown_token = components.shutdown_token;

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
        components.controller_id,
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
