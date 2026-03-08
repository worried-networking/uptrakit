use std::sync::Arc;

use uptrakit_shared_db::entity::scheduled_task;

use crate::{SchedulerNotifier, executor::TaskExecutor};

/// Triggers a CRL rebuild on all controller instances via the notifier.
///
/// Runs on a configurable cron schedule (default `0 */4 * * *`). The
/// notifier fires `revocation_notify` locally (embedded scheduler) and
/// publishes `RequestCrlRenewal` to NATS so that all remote controller
/// instances rebuild and hot-reload their TLS configurations.
pub struct CrlRenewalExecutor {
    notifier: Arc<dyn SchedulerNotifier>,
}

impl CrlRenewalExecutor {
    pub fn new(notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { notifier }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for CrlRenewalExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        tracing::info!("CRL renewal triggered by scheduler");
        self.notifier.signal_crl_renewal().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use uptrakit_internal_wire::{ControllerMessage, MqttSoftwareStatesPayload};
    use uuid::Uuid;

    struct MockNotifier {
        called: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl SchedulerNotifier for MockNotifier {
        async fn send_to_service(&self, _service_id: &Uuid, _msg: ControllerMessage) {}
        async fn broadcast(&self, _msg: ControllerMessage) {}
        async fn send_by_capability(&self, _capability: &str, _msg: ControllerMessage) {}
        async fn signal_ca_rotation(&self, _reason: &str) {}
        async fn push_software_states_for_tenant(&self, _payload: MqttSoftwareStatesPayload) {}
        async fn signal_crl_renewal(&self) {
            self.called.store(true, Ordering::Release);
        }
    }

    fn dummy_task() -> scheduled_task::Model {
        use time::OffsetDateTime;
        use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
        scheduled_task::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            task_type: ScheduledTaskType::CrlRenewal,
            interval_seconds: 14400,
            jitter_seconds: 120,
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: OffsetDateTime::now_utc(),
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[tokio::test]
    async fn execute_calls_signal_crl_renewal() {
        let called = Arc::new(AtomicBool::new(false));
        let notifier = Arc::new(MockNotifier {
            called: Arc::clone(&called),
        });

        let executor = CrlRenewalExecutor::new(notifier);
        executor.execute(&dummy_task()).await.expect("execute");

        assert!(
            called.load(Ordering::Acquire),
            "signal_crl_renewal should have been called"
        );
    }
}
