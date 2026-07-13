//! Boot phase: wire the config-reload coordinator and audit bridge.
//!
//! [`wire`] constructs every [`Reloadable`] subsystem, wires them into the
//! coordinator, spawns the reconciler and the `reload_audit_bridge`, and
//! returns [`ReloadWiring`] whose fields are consumed directly — without any
//! `Option` wrapping — by [`super::app_state::assemble`].

use std::sync::Arc;

use rootcause::prelude::*;
use tokio::sync::watch;
use uptrakit_config_reload::SettingsVersionCache;

use crate::AppError;
use crate::ReloadBridgeChannels;
use crate::boot::components::Components;
use crate::boot::identity::Identity;

// ---------------------------------------------------------------------------
// Output type
// ---------------------------------------------------------------------------

/// All values produced by the reload-wiring phase.
///
/// Fields are non-optional — the coordinator always runs, so there is no
/// "Option" fallback path.  The old 7-tuple of `Option`s and the trailing
/// `match (Some(…), …)` arm have been deleted.
pub(crate) struct ReloadWiring {
    pub coordinator_handle: uptrakit_config_reload::ReloadCoordinatorHandle,
    pub settings_version_cache: SettingsVersionCache,
    pub receivers: uptrakit_config_reload::RuntimeConfigReceivers,
    pub reload_file_state_rx: watch::Receiver<uptrakit_config_reload::ConfigFileState>,
    pub reload_last_reload_rx: watch::Receiver<Option<uptrakit_config_reload::LastReloadInfo>>,
    pub reload_recent_events_rx: watch::Receiver<Vec<serde_json::Value>>,
    pub audit_log_filter_rx: watch::Receiver<Arc<uptrakit_config_reload::config::AuditConfig>>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Wire the config-reload coordinator and spawn supporting tasks.
///
/// # Order of operations
///
/// 1. Build all [`Reloadable`] subsystems (DB → TLS → Listeners → NATS →
///    Audit → Zeroconf → Plugins → Embedded).
/// 2. Feed them into `booted.coordinator`; set alert writer, current config,
///    config path, and reexec hook.
/// 3. Extract `coordinator_handle`; spawn `spawn_config_reconciler` (detached
///    — the `JoinHandle` is intentionally dropped inside this function) and
///    `coordinator.run()`.
/// 4. Spawn the `reload_audit_bridge` task (clones `audit_emitter` — it is
///    `Clone`).
/// 5. Return [`ReloadWiring`] with all non-optional fields.
///
/// # Note on `_reconciler` drop
///
/// `spawn_config_reconciler` returns a `JoinHandle`.  Dropping it detaches the
/// task (Tokio drop ≠ cancel).  `ReloadWiring` intentionally does **not** store
/// the handle — the `let _ = …` keeps the semantics of the original code.
#[expect(
    clippy::too_many_arguments,
    reason = "wires coordinator from all phase outputs; each parameter drives a distinct subsystem or lifecycle hook"
)]
pub(crate) async fn wire(
    mut booted: crate::boot::init::BootedConfig,
    config_path_for_coord: std::path::PathBuf,
    args_master_key_from: Option<String>,
    components: &Components,
    listener_count: usize,
    first_listener_fd: std::os::unix::io::RawFd,
    identity: &Identity,
    db_conn: sea_orm::DatabaseConnection,
    db_url: String,
    #[cfg(feature = "nats")] reconciled: &crate::boot::init::ReconciledSettings,
) -> crate::Result<ReloadWiring> {
    // DB → TLS → Listeners → NATS → Audit → Zeroconf → Plugins → Embedded
    let db_reloadable =
        crate::reload::db_pool::DbPoolReloadable::new(db_conn.clone(), db_url.clone());
    let db_rx = db_reloadable.subscribe();
    let tls_reloadable = crate::reload::tls_snapshot::TlsSnapshotReloadable::new(
        booted.runtime.tls.clone(),
        Arc::clone(&identity.pki.server_cert_resolver),
    );
    let (https_reloadable, _https_rx) = crate::reload::https_listener::HttpsListenerReloadable::new(
        booted.runtime.network.https.clone(),
    );
    let (pki_reloadable, _pki_rx) = crate::reload::pki_listener::PkiListenerReloadable::new(
        booted.runtime.network.pki_addr.clone(),
    );
    let (audit_reloadable, audit_log_filter_rx) =
        crate::reload::audit::AuditDispatcherReloadable::new(
            components.audit.dispatcher.clone(),
            booted.runtime.audit.clone(),
        );
    let (zeroconf_reloadable, _zeroconf_rx) =
        crate::reload::zeroconf::ZeroconfReloadable::new(booted.runtime.zeroconf.clone());
    let (plugin_reloadable, _plugin_rx) =
        uptrakit_web_api_queries::reload::plugin_registry::PluginCatalogReloadable::new(
            uptrakit_config_reload::config::PluginsConfig::default(),
        );
    let (embedded_reloadable, _embedded_rx) =
        crate::reload::embedded::EmbeddedServicesReloadable::new(
            booted.runtime.embedded_services.clone(),
        );

    #[cfg_attr(
        not(feature = "nats"),
        expect(
            unused_mut,
            reason = "only pushed inside the #[cfg(feature = \"nats\")] block below"
        )
    )]
    let mut reloadables: Vec<Arc<dyn uptrakit_config_reload::ReloadableErased>> = vec![
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
    reloadables.push(Arc::new(crate::reload::nats::NatsReloadable::new(
        reconciled.nats_url.clone(),
        booted.runtime.nats.clone(),
    )));

    booted.coordinator.extend_reloadables(reloadables);
    booted
        .coordinator
        .set_alert_writer(Arc::new(crate::reload::audit::AuditAlertWriter::new(
            components.audit.emitter.clone(),
        )));

    let current_exe = std::env::current_exe()
        .map_err(|e| report!(AppError::Config(format!("resolve current_exe: {e}"))))?;
    booted
        .coordinator
        .set_config_path(config_path_for_coord.clone());
    booted
        .coordinator
        .set_current_config(Arc::new(booted.runtime.clone()));
    booted
        .coordinator
        .set_reexec_hook(Box::new(crate::ControllerReexecHook {
            current_exe,
            config_path: config_path_for_coord.clone(),
            master_key_from: args_master_key_from,
            generation: crate::reexec::listenfd::current_generation(),
            listener_count,
            first_listener_fd,
            oauth_instance: identity.oauth_instance_for_shutdown.clone(),
        }));

    let coordinator_handle = booted.coordinator.handle();

    let _reconciler = crate::reload::reconciler::spawn_config_reconciler(
        db_rx,
        coordinator_handle.sender(),
        booted.settings_version_cache.clone(),
        components.shutdown_token.clone(),
    );

    tokio::spawn(booted.coordinator.run());

    // Spawn the reload-audit bridge, cloning `audit_emitter` (it is Clone).
    let audit_rx = booted.audit_rx;
    let reload_file_state_tx = booted.reload_file_state_tx;
    let reload_file_state_rx = booted.reload_file_state_rx;
    let reload_last_reload_tx = booted.reload_last_reload_tx;
    let reload_last_reload_rx = booted.reload_last_reload_rx;
    let reload_recent_events_tx = booted.reload_recent_events_tx;
    let reload_recent_events_rx = booted.reload_recent_events_rx;
    tokio::spawn(reload_audit_bridge(
        audit_rx,
        components.audit.emitter.clone(),
        ReloadBridgeChannels {
            file_state_tx: reload_file_state_tx,
            last_reload_tx: reload_last_reload_tx,
            recent_events_tx: reload_recent_events_tx,
            config_path: config_path_for_coord,
        },
    ));

    Ok(ReloadWiring {
        coordinator_handle,
        settings_version_cache: booted.settings_version_cache,
        receivers: booted.receivers,
        reload_file_state_rx,
        reload_last_reload_rx,
        reload_recent_events_rx,
        audit_log_filter_rx,
    })
}

// ---------------------------------------------------------------------------
// Moved functions (previously in crate root lib.rs)
// ---------------------------------------------------------------------------

/// Bridge task: receive [`ReloadAuditEvent`]s from the coordinator and emit them as
/// system-scoped [`AuditEntry`] rows via [`AuditEmitter::emit_event`].
///
/// Also maintains the three status watch channels consumed by the
/// `GET /api/v1/instance/config-state` endpoint.
async fn reload_audit_bridge(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<uptrakit_config_reload::ReloadAuditEvent>,
    emitter: uptrakit_audit_log::AuditEmitter,
    channels: ReloadBridgeChannels,
) {
    use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome, Event};
    use uptrakit_config_reload::ReloadAuditEvent;

    while let Some(event) = rx.recv().await {
        // Update status watch channels.
        match &event {
            ReloadAuditEvent::FileChanged { path } => {
                let pending = match uptrakit_config_reload::file_digest(path) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "pending digest unavailable");
                        None
                    }
                };
                channels.file_state_tx.send_modify(|s| {
                    s.pending_digest = pending;
                    s.pending_detected_at = Some(time::OffsetDateTime::now_utc());
                });
            }
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source,
            } => {
                let info = uptrakit_config_reload::LastReloadInfo::new(
                    time::OffsetDateTime::now_utc(),
                    sections.clone(),
                    per_subsystem_ms.clone(),
                );
                // Receivers may have been dropped (e.g. tests); ignore send errors.
                drop(channels.last_reload_tx.send(Some(info)));

                match source {
                    uptrakit_config_reload::ReloadSource::Sighup
                    | uptrakit_config_reload::ReloadSource::FileWatch { .. } => {
                        let applied = uptrakit_config_reload::file_digest(&channels.config_path)
                            .inspect_err(|e| tracing::warn!(error = %e, "applied digest re-read failed; keeping last digest"))
                            .ok();
                        channels.file_state_tx.send_modify(|s| {
                            if let Some(d) = applied {
                                s.digest = d;
                            }
                            s.loaded_at = time::OffsetDateTime::now_utc();
                            s.pending_digest = None;
                            s.pending_detected_at = None;
                        });
                    }
                    _ => {
                        tracing::debug!(
                            ?source,
                            "reload_audit_bridge: non-file source applied, no file-state update"
                        );
                    }
                }

                let event_json = serde_json::json!({
                    "type": "applied",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "sections": sections,
                });
                channels.recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            ReloadAuditEvent::Failed {
                phase,
                subsystem,
                error,
            } => {
                channels.file_state_tx.send_modify(|s| {
                    s.pending_digest = None;
                    s.pending_detected_at = None;
                });
                let event_json = serde_json::json!({
                    "type": "failed",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "phase": phase.as_str(),
                    "subsystem": subsystem,
                    "error": error,
                });
                channels.recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            ReloadAuditEvent::Reverted { subsystem, reason } => {
                let event_json = serde_json::json!({
                    "type": "reverted",
                    "at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| String::new()),
                    "subsystem": subsystem,
                    "reason": reason,
                });
                channels.recent_events_tx.send_modify(|v| {
                    v.push(event_json);
                    if v.len() > 20 {
                        v.remove(0);
                    }
                });
            }
            _ => {
                tracing::warn!(
                    "reload_audit_bridge: unhandled ReloadAuditEvent variant in status-watch; not updating any channel"
                );
            }
        }

        let (action, outcome, details) = match &event {
            ReloadAuditEvent::Requested { source } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_REQUESTED,
                AuditOutcome::Success,
                serde_json::json!({ "source": source }),
            ),
            ReloadAuditEvent::Refused { source, reason } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_REFUSED,
                AuditOutcome::Failed,
                serde_json::json!({ "source": source, "reason": reason }),
            ),
            ReloadAuditEvent::Applied {
                sections,
                per_subsystem_ms,
                source: _,
            } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_APPLIED,
                AuditOutcome::Success,
                serde_json::json!({ "sections": sections, "per_subsystem_ms": per_subsystem_ms }),
            ),
            ReloadAuditEvent::Failed {
                phase,
                subsystem,
                error,
            } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_FAILED,
                AuditOutcome::Failed,
                serde_json::json!({ "phase": phase, "subsystem": subsystem, "error": error }),
            ),
            ReloadAuditEvent::Reverted { subsystem, reason } => (
                AuditActionType::SYSTEM_CONFIG_RELOAD_REVERTED,
                AuditOutcome::Failed,
                serde_json::json!({ "subsystem": subsystem, "reason": reason }),
            ),
            ReloadAuditEvent::FileChanged { .. } => continue, // not audit-logged; handled in status watch
            _ => {
                tracing::warn!(
                    "reload_audit_bridge: unhandled ReloadAuditEvent variant (skipping audit emit)"
                );
                continue;
            }
        };
        if let Ok(entry) = AuditEntry::<Event>::builder_event(action)
            .system_scope()
            .outcome(outcome)
            .details(details)
            .build()
        {
            emitter.emit_event(entry);
        }
    }
}
