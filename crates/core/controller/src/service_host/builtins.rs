use std::collections::BTreeSet;
use std::sync::Arc;

use uptrakit_internal_wire::Capability;
use uuid::Uuid;

use crate::tasks::BackgroundTasks;

use super::embedded_host::BuiltinServiceHost;
use super::yielding::matches_yield_policy;

pub(crate) struct BuiltinRegistration {
    pub label: &'static str,
    pub app_name: &'static str,
    pub yield_policy: uptrakit_service_platform::YieldPolicy,
}

const AGENT: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded Agent",
    app_name: "uptrakit-agent",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceSameHost,
};

const AGENT_SSH: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded SSH Agent",
    app_name: "uptrakit-agent-ssh",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
};

const SCHEDULER: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded Scheduler",
    app_name: "uptrakit-scheduler",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
};

const MQTT: BuiltinRegistration = BuiltinRegistration {
    label: "Embedded MQTT",
    app_name: "uptrakit-mqtt",
    yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
};

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
    use crate::scheduler::ControllerSchedulerNotifier;
    use uptrakit_internal_wire::ServiceTransport;
    use uptrakit_scheduler_engine::executors::*;
    use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;

    let scheduler_caps: BTreeSet<Capability> = [
        Capability::Scheduler,
        Capability::SystemService,
        Capability::GracefulShutdown,
    ]
    .into();

    let db = app_state.db().clone();
    let notification_service = app_state.notification_service.clone();
    let ca_rotation_trigger = Arc::clone(&app_state.cert.ca_rotation_trigger);
    let revocation_notify = Arc::clone(&app_state.cert.revocation_notify);
    let embedded_notifier_ref = app_state.embedded_service_notifier.clone();
    let ca_tx_sub = ca_tx.subscribe();

    host.add(
        &SCHEDULER,
        scheduler_caps,
        true,
        None,
        controller_installation_id,
        map_yield_policy(&SCHEDULER, None),
        move |transport, tokens| {
            Box::pin(async move {
                let notifier: Arc<dyn uptrakit_scheduler_engine::SchedulerNotifier> =
                    Arc::new(ControllerSchedulerNotifier::new(
                        notification_service,
                        db.clone(),
                        Arc::clone(&ca_rotation_trigger),
                        Arc::clone(&revocation_notify),
                    ));

                let yield_check: Box<dyn Fn() -> bool + Send + Sync> = if let Some(notifier_arc) =
                    embedded_notifier_ref
                {
                    Box::new(move || {
                        notifier_arc
                            .is_capability_yielded(&uptrakit_internal_wire::Capability::Scheduler)
                    })
                } else {
                    Box::new(move || transport.is_yielded())
                };

                let mut sched = uptrakit_scheduler_engine::Scheduler::new(
                    db.clone(),
                    uptrakit_scheduler_engine::SchedulerConfig::new(controller_id),
                    yield_check,
                );

                sched.register(
                    ScheduledTaskType::AuthCleanup,
                    Box::new(auth_cleanup::AuthCleanupExecutor::new(db.clone())),
                );
                sched.register(
                    ScheduledTaskType::StaleLeaseCleanup,
                    Box::new(stale_lease_cleanup::StaleLeaseCleanupExecutor::new(
                        db.clone(),
                    )),
                );
                if ca_managed {
                    sched.register(
                        ScheduledTaskType::CaRotationCheck,
                        Box::new(crate::scheduler::CaRotationCheckExecutor::new(
                            ca_tx_sub,
                            Arc::clone(&ca_rotation_trigger),
                        )),
                    );
                }
                sched.register(
                    ScheduledTaskType::FetchReleases,
                    Box::new(fetch_releases::FetchReleasesExecutor::new(
                        db.clone(),
                        Arc::clone(&notifier),
                    )),
                );
                sched.register(
                    ScheduledTaskType::DetectVersion,
                    Box::new(detect_version::DetectVersionExecutor::new(
                        db.clone(),
                        Arc::clone(&notifier),
                    )),
                );
                sched.register(
                    ScheduledTaskType::ServiceCertCheck,
                    Box::new(service_cert_check::ServiceCertCheckExecutor::new(
                        db.clone(),
                        Arc::clone(&notifier),
                    )),
                );
                sched.register(
                    ScheduledTaskType::CrlRenewal,
                    Box::new(crl_renewal::CrlRenewalExecutor::new(Arc::clone(&notifier))),
                );
                sched.register(
                    ScheduledTaskType::AuditLogCleanup,
                    Box::new(audit_log_cleanup::AuditLogCleanupExecutor::new(db.clone())),
                );
                sched.register(
                    ScheduledTaskType::DiscoverSoftware,
                    Box::new(discover_software::DiscoverSoftwareExecutor::new(
                        db,
                        Arc::clone(&notifier),
                    )),
                );

                sched.run(tokens.drain, tokens.abort).await;
            })
        },
        app_state,
        bg,
    )
    .await?;

    Ok(())
}

#[cfg(feature = "embedded-agent")]
pub(crate) async fn register_agent(
    host: &BuiltinServiceHost,
    app_state: &Arc<uptrakit_web_api::AppState>,
    bg: &mut BackgroundTasks,
    controller_installation_id: Uuid,
    state_dir: std::path::PathBuf,
) -> rootcause::Result<()> {
    let agent_caps = crate::agent::agent_capabilities();
    let default_tenant_id = app_state.default_tenant_id;
    let local_machine_id = uptrakit_agent_core::host_info::read_machine_id();

    let add_result = host
        .add(
            &AGENT,
            agent_caps.clone(),
            false,
            Some(default_tenant_id),
            controller_installation_id,
            map_yield_policy(&AGENT, Some(local_machine_id)),
            move |transport, tokens| {
                Box::pin(crate::agent::run_embedded_agent(
                    transport,
                    tokens.abort,
                    state_dir,
                ))
            },
            app_state,
            bg,
        )
        .await?;

    let bridge_cancel = bg.child_token();
    let bridge_handle = tokio::spawn(
        uptrakit_web_api::embedded_support::run_embedded_message_handler(
            Arc::clone(app_state),
            add_result.service_id,
            default_tenant_id,
            agent_caps,
            AGENT.app_name.to_string(),
            add_result.service_rx,
            bridge_cancel,
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
) -> rootcause::Result<()> {
    let ssh_caps = crate::ssh_agent::ssh_agent_capabilities();
    let default_tenant_id = app_state.default_tenant_id;
    let db_for_ssh = app_state.db().clone();

    let add_result = host
        .add(
            &AGENT_SSH,
            ssh_caps.clone(),
            false,
            Some(default_tenant_id),
            controller_installation_id,
            map_yield_policy(&AGENT_SSH, None),
            move |transport, tokens| {
                Box::pin(crate::ssh_agent::run_embedded_ssh_agent(
                    transport, tokens, state_dir, db_for_ssh,
                ))
            },
            app_state,
            bg,
        )
        .await?;

    let bridge_cancel = bg.child_token();
    let bridge_handle = tokio::spawn(
        uptrakit_web_api::embedded_support::run_embedded_message_handler(
            Arc::clone(app_state),
            add_result.service_id,
            default_tenant_id,
            ssh_caps,
            AGENT_SSH.app_name.to_string(),
            add_result.service_rx,
            bridge_cancel,
        ),
    );
    bg.track("Embedded SSH Agent (bridge)", bridge_handle);

    Ok(())
}

pub(crate) async fn register_mqtt(host: &BuiltinServiceHost) -> rootcause::Result<()> {
    host.register_deferred(&MQTT);
    Ok(())
}
