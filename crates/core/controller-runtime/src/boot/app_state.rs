//! Boot phase: assemble [`AppState`] from phase structs.
//!
//! [`assemble`] consumes the outputs of all prior boot phases and calls
//! `AppState::builder()` with every field populated directly — no `Option`
//! wrapping, no unreachable `match` arms.  The old 7-`Some` tuple and its
//! trailing `_ => builder` arm no longer exist.
//!
//! This module also defines [`ServeDeps`], a bundle of cloned / moved handles
//! that the server-tail code (signal loop, shutdown, embedded registration)
//! needs after `assemble` has consumed `identity` and `components` by move.

use std::sync::Arc;

use rootcause::prelude::*;

use crate::AppError;
use crate::boot::components::Components;
use crate::boot::identity::Identity;
use crate::boot::reload::ReloadWiring;

// ---------------------------------------------------------------------------
// ServeDeps — handles separated before assemble consumes identity/components
// ---------------------------------------------------------------------------

/// Handles that the server-tail code (background tasks, embedded registration,
/// signal loop, graceful shutdown, OAuth deregister) needs after [`assemble`]
/// has consumed `identity` and `components` by move.
///
/// Build this struct *before* calling `assemble`, by cloning `Arc`/`Copy`
/// fields from `identity.pki` and `components`, and by moving
/// `oauth_instance_for_shutdown` out of `identity` (the SIGTERM path owns it).
pub(crate) struct ServeDeps {
    // ---- PKI / TLS ----
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    /// Cloned watch sender — `ca_tx.clone()` is cheap (Arc-backed).
    pub ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    pub ca_managed: bool,
    pub initial_ca_version: i64,
    pub has_external_tls_cert: bool,

    // ---- Service connections, NATS, and shutdown ----
    /// Cloned unconditionally — `bg.shutdown` needs it on every build.
    pub service_connections: uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    /// Cloned (Arc-cheap) when the `nats` feature is enabled.
    #[cfg(feature = "nats")]
    pub nats_transport: Option<uptrakit_web_api::nats_transport::NatsTransport>,
    /// Cloned for `BackgroundTasks::new`; components are moved into assemble.
    pub shutdown_token: tokio_util::sync::CancellationToken,

    // ---- OAuth shutdown ----
    /// Moved (not cloned) from `identity.oauth_instance_for_shutdown`.
    /// The SIGTERM/SIGINT deregister path is the single owner.
    pub oauth_instance_for_shutdown: Option<(uuid::Uuid, sea_orm::DatabaseConnection)>,

    // ---- Controller identity ----
    /// Copied before components is moved into assemble.
    /// Used by `register_scheduler` under `#[cfg(feature = "embedded-scheduler")]`.
    #[cfg_attr(
        not(feature = "embedded-scheduler"),
        expect(
            dead_code,
            reason = "controller_id is only passed to register_scheduler when the embedded-scheduler feature is enabled"
        )
    )]
    pub controller_id: uuid::Uuid,

    // ---- Embedded-service fields ----
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    pub builtin_host: crate::service_host::BuiltinServiceHost,
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    pub controller_installation_id: uuid::Uuid,
    #[cfg(any(feature = "embedded-agent", feature = "embedded-ssh-agent"))]
    pub state_dir: std::path::PathBuf,
}

// ---------------------------------------------------------------------------
// Assemble
// ---------------------------------------------------------------------------

/// Assemble [`AppState`] from the outputs of all prior boot phases.
///
/// # Parameters
///
/// - `settings` — consumed; fed into `.settings()` on the builder.
/// - `identity` — consumed; `oauth_instance_for_shutdown` has already been
///   moved into `ServeDeps` by the caller via `Option::take` (its field is
///   `None` at this point — the builder does not use it).
/// - `components` — consumed.
/// - `reload` — non-optional wiring produced by [`super::reload::wire`].
/// - `db_conn` — cloned into the builder.
/// - `default_tenant_id` — passed through to the builder.
///
/// The remaining parameters are needed only by the `#[cfg(feature = "test-utils")]`
/// force-reexec block, which is moved here verbatim.
///
/// # Infallible-match note
///
/// The `#[cfg(feature = "test-utils")]` block ends with
/// `match infallible {}` — an intentional infallible exhaustive match over
/// `Infallible`.  This pattern is preserved verbatim from the original code.
#[cfg_attr(
    feature = "nats",
    expect(
        clippy::too_many_arguments,
        reason = "assembles AppState from all boot phase structs; each parameter is a distinct phase output"
    )
)]
pub(crate) async fn assemble(
    settings: uptrakit_web_api::settings::Settings,
    identity: Identity,
    components: Components,
    reload: ReloadWiring,
    db_conn: sea_orm::DatabaseConnection,
    default_tenant_id: uuid::Uuid,
    #[cfg(feature = "test-utils")] config_path: std::path::PathBuf,
    #[cfg(feature = "test-utils")] args_master_key_from: Option<String>,
    #[cfg(feature = "test-utils")] listener_count: usize,
    #[cfg(feature = "test-utils")] first_listener_fd: std::os::unix::io::RawFd,
) -> crate::Result<Arc<uptrakit_web_api::AppState>> {
    let Identity {
        pki:
            crate::boot::identity::PkiFields {
                ca_managed: _ca_managed,
                pki_path,
                ca_tx: _ca_tx,
                ca_rx,
                ca_key_store,
                rustls_config,
                server_cert_resolver,
                revocation_notify,
                ca_rotation_trigger,
                crl_pem_cache,
                crl_manager: _crl_manager,
                initial_ca_version: _initial_ca_version,
                has_external_tls_cert: _has_external_tls_cert,
            },
        jwt_manager,
        cert_signer,
        oauth_state,
        // oauth_instance_for_shutdown was moved into ServeDeps by the caller;
        // it is None here and not used by the builder.
        oauth_instance_for_shutdown: _,
    } = identity;

    let builder = uptrakit_web_api::AppState::builder()
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
        .oauth(oauth_state)
        // Apply ReloadWiring fields directly and unconditionally.
        // (Replaces the old `match (Some(handle), Some(cache), …)` block.)
        .coordinator_handle(reload.coordinator_handle)
        .settings_version_cache(reload.settings_version_cache)
        .config_receivers(reload.receivers)
        .config_reload_status_receivers(
            reload.reload_file_state_rx,
            reload.reload_last_reload_rx,
            reload.reload_recent_events_rx,
        )
        .audit_log_filter_rx(reload.audit_log_filter_rx);

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
            config_path,
            master_key_from: args_master_key_from,
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

    Ok(app_state)
}
