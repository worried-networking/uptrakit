use std::sync::Arc;

use tokio::sync::Notify;
use uptrakit_shared_db::entity::scheduled_task;

use crate::pki;
use crate::scheduler::executor::TaskExecutor;

/// Checks whether the managed CA is within its rotation window and fires the
/// rotation trigger if so.
///
/// The actual rotation is still handled by the per-controller CA rotation loop
/// (which listens on the `Notify`). This executor only triggers the check on a
/// schedule instead of using a 24h `tokio::time::interval`.
pub struct CaRotationCheckExecutor {
    ca_snapshot: tokio::sync::watch::Receiver<crate::pki::CaSnapshot>,
    ca_rotation_trigger: Arc<Notify>,
}

impl CaRotationCheckExecutor {
    pub fn new(
        ca_snapshot: tokio::sync::watch::Receiver<pki::CaSnapshot>,
        ca_rotation_trigger: Arc<Notify>,
    ) -> Self {
        Self {
            ca_snapshot,
            ca_rotation_trigger,
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for CaRotationCheckExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let snapshot = self.ca_snapshot.borrow().clone();
        if pki::should_rotate_ca(&snapshot.active_cert_pem) {
            tracing::info!("CA certificate is within rotation window, triggering rotation");
            self.ca_rotation_trigger.notify_one();
        } else {
            tracing::debug!("CA certificate does not need rotation");
        }
        Ok(())
    }
}
