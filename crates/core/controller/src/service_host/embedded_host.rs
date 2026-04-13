use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_internal_wire::Capability;
use uuid::Uuid;

use crate::tasks::BackgroundTasks;

use super::builtins::BuiltinRegistration;

pub(crate) struct BuiltinServiceHost {
    embedded: Arc<crate::embedded::EmbeddedServiceHost>,
}

impl BuiltinServiceHost {
    pub(crate) fn new(embedded: Arc<crate::embedded::EmbeddedServiceHost>) -> Self {
        Self { embedded }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn add(
        &self,
        registration: &BuiltinRegistration,
        capabilities: BTreeSet<Capability>,
        is_system_service: bool,
        tenant_id: Option<Uuid>,
        embedded_owner_key: Uuid,
        coexistence_policy: crate::embedded::types::CoexistencePolicy,
        run_fn: impl FnOnce(
            crate::embedded::types::EmbeddedTransport,
            crate::embedded::EmbeddedShutdownTokens,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
        state: &Arc<uptrakit_web_api::AppState>,
        bg: &mut BackgroundTasks,
    ) -> rootcause::Result<crate::embedded::AddResult> {
        self.embedded
            .add(
                registration.label,
                registration.app_name,
                capabilities,
                is_system_service,
                tenant_id,
                embedded_owner_key,
                coexistence_policy,
                run_fn,
                state,
                bg,
            )
            .await
    }

    pub(crate) fn register_deferred(&self, registration: &BuiltinRegistration) {
        let _ = &self.embedded;
        tracing::debug!(
            label = registration.label,
            app_name = registration.app_name,
            yield_policy = ?registration.yield_policy,
            "built-in service registration is present but runtime wiring is deferred"
        );
    }
}
