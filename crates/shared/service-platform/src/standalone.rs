use std::collections::BTreeSet;
use std::convert::Infallible;
use std::future::pending;
use std::time::Duration;

use async_trait::async_trait;
use uptrakit_service_sdk::{
    ControllerConnection, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, cli::CommonServiceArgs,
};
use uptrakit_wire::{Capability, ControllerMessage};

use crate::runtime::ServiceRuntime;

/// Per-service metadata required by the temporary standalone bridge.
///
/// The standalone lifecycle in `uptrakit-service-sdk` is keyed off these
/// constants, so the runtime side must provide real values even while the
/// callback bridge remains skeletal.
pub trait StandaloneMetadata {
    const DIR_NAME: &'static str;
    const SERVICE_LABEL: &'static str;
    const SERVICE_APP_NAME: &'static str;
}

/// Temporary standalone adapter seam from `ServiceRuntime` into
/// `uptrakit-service-sdk`'s standalone lifecycle.
///
/// This slice only establishes the dependency and metadata boundary. The SDK
/// still owns websocket enrollment, reconnect, and lifecycle plumbing.
///
/// Metadata is intentionally real and comes from [`StandaloneMetadata`] on the
/// runtime type. By contrast, lifecycle callback bridging is intentionally
/// incomplete here: later slices will translate runtime/session behavior into
/// these callbacks instead of the current no-op placeholders.
pub async fn run_standalone<R>(binary_name: &str, args: &CommonServiceArgs, runtime: &mut R)
where
    R: ServiceRuntime + StandaloneMetadata + Send,
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
    R: ServiceRuntime + StandaloneMetadata + Send,
{
    const DIR_NAME: &'static str = R::DIR_NAME;
    const SERVICE_LABEL: &'static str = R::SERVICE_LABEL;
    const SERVICE_APP_NAME: &'static str = R::SERVICE_APP_NAME;

    type ServiceEvent = Infallible;

    // Placeholder: later slices will map runtime activation/session state into
    // lifecycle callbacks instead of acknowledging the connection as a no-op.
    async fn on_connected(
        &mut self,
        _conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let _ = &mut self.runtime;
        Ok(())
    }

    // Placeholder: controller messages are not yet bridged into
    // `ServiceRuntime`; this seam only proves the standalone linkage.
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

    // Placeholder: service-platform does not yet expose a runtime event stream
    // that can drive the service-sdk event loop.
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

    // Minimal honest behavior: shutdown still follows the SDK's default
    // mapping, but runtime drain/abort coordination is deferred to later work.
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
