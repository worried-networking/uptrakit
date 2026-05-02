use std::sync::Arc;

#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use std::collections::BTreeSet;
#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use std::future::Future;
#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
use std::pin::Pin;
#[cfg(any(
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
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
use super::builtins::BuiltinRegistration;

pub(crate) struct BuiltinServiceHost {
    #[cfg_attr(
        not(any(
            feature = "embedded-scheduler",
            feature = "embedded-agent",
            feature = "embedded-ssh-agent",
            feature = "embedded-mqtt"
        )),
        allow(dead_code)
    )]
    embedded: Arc<crate::embedded::EmbeddedServiceHost>,
}

impl BuiltinServiceHost {
    pub(crate) fn new(embedded: Arc<crate::embedded::EmbeddedServiceHost>) -> Self {
        Self { embedded }
    }

    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    #[expect(
        clippy::too_many_arguments,
        reason = "each parameter drives a distinct aspect of builtin service registration"
    )]
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
}
