#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use std::sync::Arc;

#[cfg(any(feature = "embedded-scheduler", feature = "embedded-mqtt"))]
use std::collections::BTreeSet;
#[cfg(any(feature = "embedded-scheduler", feature = "embedded-mqtt"))]
use uptrakit_wire::Capability;
#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use uuid::Uuid;

#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use crate::tasks::BackgroundTasks;

#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use super::embedded_host::BuiltinServiceHost;
#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use super::yielding::matches_yield_policy;

#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
pub(crate) struct BuiltinRegistration {
    pub label: &'static str,
    pub app_name: &'static str,
    pub yield_policy: uptrakit_service_platform::YieldPolicy,
}

#[cfg(feature = "embedded-agent")]
const AGENT: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded Agent",
    app_name: "uptrakit-agent",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceSameHost,
};

#[cfg(feature = "embedded-ssh-agent")]
const AGENT_SSH: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded SSH Agent",
    app_name: "uptrakit-agent-ssh",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
};

#[cfg(feature = "embedded-scheduler")]
const SCHEDULER: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded Scheduler",
    app_name: "uptrakit-scheduler",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
};

#[cfg(feature = "embedded-mqtt")]
const MQTT: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded MQTT",
    app_name: uptrakit_mqtt_runtime::bootstrap::MQTT_SERVICE_APP_NAME,
    yield_policy: uptrakit_mqtt_runtime::bootstrap::YIELD_POLICY,
};

#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
fn map_yield_policy(
    registration: &BuiltinRegistration,
    local_machine_id: Option<String>,
) -> crate::embedded::types::CoexistencePolicy {
    match registration.yield_policy {
        uptrakit_service_platform::YieldPolicy::SameServiceAnywhere => {
            crate::embedded::types::CoexistencePolicy::YieldOnSameAppName
        }
        uptrakit_service_platform::YieldPolicy::SameServiceSameHost => {
            let app_name = registration.app_name;
            crate::embedded::types::CoexistencePolicy::Custom(Box::new(move |info| {
                matches_yield_policy(
                    uptrakit_service_platform::YieldPolicy::SameServiceSameHost,
                    app_name,
                    local_machine_id.as_deref(),
                    info,
                )
            }))
        }
        uptrakit_service_platform::YieldPolicy::Never => {
            crate::embedded::types::CoexistencePolicy::NeverYield
        }
    }
}

#[cfg(any(feature = "embedded-scheduler", feature = "embedded-mqtt"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddedBridgeMode {
    System,
}

#[cfg(any(feature = "embedded-scheduler", feature = "embedded-mqtt"))]
struct EmbeddedBridgeRegistration {
    service_id: Uuid,
    connection_id: Uuid,
    capabilities: BTreeSet<Capability>,
    app_name: String,
    service_rx: tokio::sync::mpsc::Receiver<uptrakit_wire::ServiceMessage>,
    mode: EmbeddedBridgeMode,
}

#[cfg(feature = "embedded-scheduler")]
fn scheduler_bridge_registration(
    scheduler_caps: BTreeSet<Capability>,
    add_result: crate::embedded::AddResult,
) -> EmbeddedBridgeRegistration {
    EmbeddedBridgeRegistration {
        service_id: add_result.service_id,
        connection_id: add_result.connection_id,
        capabilities: scheduler_caps,
        app_name: SCHEDULER.app_name.to_string(),
        service_rx: add_result.service_rx,
        mode: EmbeddedBridgeMode::System,
    }
}

/// Spawn the server-side bridge task for an untenanted system service
/// (scheduler, MQTT). System bridges are untenanted by construction: the
/// inner session is always built with `service_tenant_id: None`.
#[cfg(any(feature = "embedded-scheduler", feature = "embedded-mqtt"))]
fn spawn_system_bridge(
    label: &'static str,
    app_state: &Arc<uptrakit_web_api::AppState>,
    bg: &mut BackgroundTasks,
    bridge: EmbeddedBridgeRegistration,
) {
    let bridge_cancel = bg.child_token();
    let EmbeddedBridgeRegistration {
        service_id,
        connection_id,
        capabilities,
        app_name,
        service_rx,
        mode,
    } = bridge;
    let bridge_handle = match mode {
        EmbeddedBridgeMode::System => tokio::spawn(
            uptrakit_web_api::embedded_support::run_embedded_system_message_handler(
                uptrakit_web_api::embedded_support::EmbeddedHandlerParams::new(
                    Arc::clone(app_state),
                    service_id,
                    connection_id,
                    capabilities,
                    app_name,
                    service_rx,
                    bridge_cancel,
                ),
            ),
        ),
    };
    bg.track(label, bridge_handle);
}

#[cfg(feature = "embedded-scheduler")]
pub(crate) async fn register_scheduler(
    host: &BuiltinServiceHost,
    app_state: &Arc<uptrakit_web_api::AppState>,
    bg: &mut BackgroundTasks,
    controller_id: Uuid,
    controller_installation_id: Uuid,
    ca_managed: bool,
    ca_tx: &tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
) -> rootcause::Result<()> {
    use uptrakit_wire::ServiceTransport;

    let scheduler_caps: BTreeSet<Capability> = [
        Capability::Scheduler,
        Capability::SystemService,
        Capability::GracefulShutdown,
    ]
    .into();

    let db = app_state.db().clone();
    let notification_service = app_state.notification.notification_service.clone();
    let ca_rotation_trigger = Arc::clone(&app_state.cert.ca_rotation_trigger);
    let revocation_notify = Arc::clone(&app_state.cert.revocation_notify);
    let embedded_notifier_ref = app_state.embedded_service_notifier.clone();
    let ca_tx_sub = ca_tx.subscribe();
    let global_providers = app_state.global_providers();
    #[cfg(feature = "plugin-ops")]
    let controller_update_hook = app_state.controller_update_hook();

    let add_result = host
        .add(
            &SCHEDULER,
            scheduler_caps.clone(),
            true,
            None,
            controller_installation_id,
            map_yield_policy(&SCHEDULER, None),
            move |service_id, transport, tokens| {
                let _ = service_id;
                Box::pin(async move {
                    let yield_check: Box<dyn Fn() -> bool + Send + Sync> =
                        if let Some(notifier_arc) = embedded_notifier_ref {
                            Box::new(move || {
                                notifier_arc
                                    .is_capability_yielded(&uptrakit_wire::Capability::Scheduler)
                            })
                        } else {
                            Box::new(move || transport.is_yielded())
                        };

                    crate::scheduler::run_embedded_scheduler(
                        crate::scheduler::EmbeddedSchedulerConfig {
                            db,
                            notification_service,
                            controller_id,
                            should_yield: yield_check,
                            ca_managed,
                            ca_snapshot: ca_tx_sub,
                            ca_rotation_trigger,
                            revocation_notify,
                            #[cfg(feature = "plugin-ops")]
                            controller_update_hook,
                            global_provider_lookup: Some(global_providers),
                        },
                        tokens.drain,
                        tokens.abort,
                    )
                    .await;
                })
            },
            app_state,
            bg,
            None,
        )
        .await?;

    spawn_system_bridge(
        "Embedded Scheduler (bridge)",
        app_state,
        bg,
        scheduler_bridge_registration(scheduler_caps, add_result),
    );

    Ok(())
}

#[cfg(feature = "embedded-agent")]
pub(crate) async fn register_agent(
    host: &BuiltinServiceHost,
    app_state: &Arc<uptrakit_web_api::AppState>,
    bg: &mut BackgroundTasks,
    controller_installation_id: Uuid,
    state_dir: std::path::PathBuf,
    pid_file: Option<std::path::PathBuf>,
    info: &uptrakit_build_info::BuildInfo,
) -> rootcause::Result<()> {
    let agent_caps = crate::agent::agent_capabilities();
    let default_tenant_id = app_state.default_tenant_id;
    let local_machine_id = uptrakit_agent_core::host_info::read_machine_id();
    let app_name = info.binary.clone();
    let app_version = info.version.clone();

    let add_result = host
        .add(
            &AGENT,
            agent_caps.clone(),
            false,
            Some(default_tenant_id),
            controller_installation_id,
            map_yield_policy(&AGENT, Some(local_machine_id)),
            move |service_id, transport, tokens| {
                let _ = service_id;
                Box::pin(crate::agent::run_embedded_agent(
                    app_name,
                    app_version,
                    transport,
                    tokens.abort,
                    state_dir,
                    pid_file,
                ))
            },
            app_state,
            bg,
            None,
        )
        .await?;

    let bridge_cancel = bg.child_token();
    let bridge_handle = tokio::spawn(
        uptrakit_web_api::embedded_support::run_embedded_message_handler(
            uptrakit_web_api::embedded_support::EmbeddedHandlerParams::new(
                Arc::clone(app_state),
                add_result.service_id,
                add_result.connection_id,
                agent_caps,
                AGENT.app_name.to_string(),
                add_result.service_rx,
                bridge_cancel,
            ),
            default_tenant_id,
        ),
    );
    bg.track("Embedded Agent (bridge)", bridge_handle);

    Ok(())
}

#[cfg(feature = "embedded-ssh-agent")]
pub(crate) async fn register_agent_ssh(
    host: &BuiltinServiceHost,
    app_state: &Arc<uptrakit_web_api::AppState>,
    bg: &mut BackgroundTasks,
    controller_installation_id: Uuid,
    state_dir: std::path::PathBuf,
    info: &uptrakit_build_info::BuildInfo,
) -> rootcause::Result<()> {
    // Warn about legacy standalone DB file — no longer used in embedded mode.
    let ssh_db_path = state_dir.join("agent-ssh.db");
    if let Ok(meta) = tokio::fs::metadata(&ssh_db_path).await
        && meta.len() > 0
    {
        tracing::warn!(
            path = %ssh_db_path.display(),
            "legacy agent-ssh.db found in state directory; \
             this file is no longer used in embedded mode — \
             SSH host data must be migrated manually if needed \
             (see agent-ssh-runtime/src/db/entity/ for table schemas)"
        );
    }

    // Column AAD for ssh_hosts.private_key is registered during Phase 4b via
    // register_column_aad_mappings() + AgentSshHandler::column_aad_entries().
    uptrakit_agent_ssh_runtime::init_ssh_data_key_ring(app_state.db()).await;
    uptrakit_agent_ssh_runtime::reencrypt_ssh_to_v3(app_state.db()).await;

    let ssh_caps = crate::ssh_agent::ssh_agent_capabilities();
    let default_tenant_id = app_state.default_tenant_id;
    let db_for_ssh = app_state.db().clone();

    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(
        db_for_ssh,
        state_dir,
        info.version.clone(),
    );

    let add_result = host
        .add(
            &AGENT_SSH,
            ssh_caps.clone(),
            false,
            Some(default_tenant_id),
            controller_installation_id,
            map_yield_policy(&AGENT_SSH, None),
            move |service_id, transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    service_id,
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
            app_state,
            bg,
            None,
        )
        .await?;

    let bridge_cancel = bg.child_token();
    let bridge_handle = tokio::spawn(
        uptrakit_web_api::embedded_support::run_embedded_message_handler(
            uptrakit_web_api::embedded_support::EmbeddedHandlerParams::new(
                Arc::clone(app_state),
                add_result.service_id,
                add_result.connection_id,
                ssh_caps,
                AGENT_SSH.app_name.to_string(),
                add_result.service_rx,
                bridge_cancel,
            ),
            default_tenant_id,
        ),
    );
    bg.track("Embedded SSH Agent (bridge)", bridge_handle);

    Ok(())
}

#[cfg(feature = "embedded-mqtt")]
pub(crate) async fn register_mqtt(
    host: &BuiltinServiceHost,
    app_state: &Arc<uptrakit_web_api::AppState>,
    bg: &mut BackgroundTasks,
    controller_installation_id: Uuid,
) -> rootcause::Result<()> {
    let mqtt_caps = uptrakit_mqtt_runtime::bootstrap::capabilities();

    let add_result = host
        .add(
            &MQTT,
            mqtt_caps.clone(),
            true,
            None,
            controller_installation_id,
            map_yield_policy(&MQTT, None),
            move |service_id, transport, tokens| {
                Box::pin(uptrakit_mqtt_runtime::bootstrap::run_embedded(
                    service_id,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
            app_state,
            bg,
            Some(uptrakit_mqtt_runtime::bootstrap::EMBEDDED_SHUTDOWN_TIMEOUT),
        )
        .await?;

    spawn_system_bridge(
        "Embedded MQTT (bridge)",
        app_state,
        bg,
        EmbeddedBridgeRegistration {
            service_id: add_result.service_id,
            connection_id: add_result.connection_id,
            capabilities: mqtt_caps,
            app_name: MQTT.app_name.to_string(),
            service_rx: add_result.service_rx,
            mode: EmbeddedBridgeMode::System,
        },
    );

    crate::mqtt::send_initial_service_config(app_state, add_result.service_id).await;

    Ok(())
}

#[cfg(all(test, feature = "embedded-scheduler"))]
mod tests {
    use super::*;
    use uptrakit_wire::ServiceMessage;

    #[tokio::test]
    async fn scheduler_bridge_registration_uses_system_handler_and_service_receiver() {
        let scheduler_caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        let service_id = Uuid::now_v7();
        let (service_tx, service_rx) = tokio::sync::mpsc::channel(4);

        let mut bridge = scheduler_bridge_registration(
            scheduler_caps.clone(),
            crate::embedded::AddResult {
                service_id,
                connection_id: uuid::Uuid::now_v7(),
                service_rx,
            },
        );

        assert_eq!(bridge.service_id, service_id);
        assert_eq!(bridge.capabilities, scheduler_caps);
        assert_eq!(bridge.app_name, SCHEDULER.app_name.to_string());
        assert_eq!(bridge.mode, EmbeddedBridgeMode::System);

        service_tx
            .send(ServiceMessage::Unknown)
            .await
            .expect("send test message");
        let received = bridge
            .service_rx
            .recv()
            .await
            .expect("receive bridge message");
        assert!(matches!(received, ServiceMessage::Unknown));
    }
}
