use std::collections::BTreeSet;
use std::convert::Infallible;
use std::future::pending;
use std::time::Duration;

use async_trait::async_trait;
use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_service_sdk::{
    ControllerConnection, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, cli::CommonServiceArgs,
};

use crate::runtime::ServiceRuntime;

pub async fn run_standalone<R>(binary_name: &str, args: &CommonServiceArgs, runtime: &mut R)
where
    R: ServiceRuntime + Send,
{
    let mut handler = RuntimeHandlerAdapter::new(runtime);
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(binary_name, args, &mut handler).await;
}

struct RuntimeHandlerAdapter<'a, R> {
    runtime: &'a mut R,
}

impl<'a, R> RuntimeHandlerAdapter<'a, R> {
    fn new(runtime: &'a mut R) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<R> ServiceHandler for RuntimeHandlerAdapter<'_, R>
where
    R: ServiceRuntime + Send,
{
    // Temporary placeholder metadata until standalone host definitions are
    // threaded through the service-platform runtime layer.
    const DIR_NAME: &'static str = "standalone";
    const SERVICE_LABEL: &'static str = "uptrakit standalone service";
    const SERVICE_APP_NAME: &'static str = "uptrakit-standalone";

    type ServiceEvent = Infallible;

    async fn on_connected(
        &mut self,
        _conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let _ = &mut self.runtime;
        Ok(())
    }

    async fn on_message(
        &mut self,
        _msg: ControllerMessage,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        let _ = &mut self.runtime;
        Ok(None)
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        let _ = &self.runtime;
        BTreeSet::new()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        let _ = &mut self.runtime;
        pending().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {}
    }

    async fn on_shutdown(
        &mut self,
        _conn: &mut ControllerConnection,
        cause: ShutdownCause,
        _shutdown_timeout: Duration,
    ) -> LoopOutcome {
        let _ = &mut self.runtime;
        uptrakit_service_sdk::default_resolve_shutdown(cause).1
    }
}
