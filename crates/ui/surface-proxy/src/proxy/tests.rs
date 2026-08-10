#![expect(
    clippy::let_underscore_must_use,
    reason = "test discards: an aborted task's join outcome, idempotent master-key init, and \
              invoke() calls asserted on their side effects rather than their own result"
)]

use std::sync::Arc as StdArc;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::SurfaceActionError;
use uuid::Uuid;

use super::{PluginSurfaceActionInvoker, SurfaceInvokerContext};

mod bookkeeping;
mod controller_local;
mod controller_owned;
mod provider_proxied;

fn tenant_id() -> Uuid {
    Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
}

fn user_id() -> Uuid {
    Uuid::parse_str("aaaaaaaa-1111-1111-1111-111111111111").unwrap()
}

pub(super) type SeenInvocation = (String, String, Option<Uuid>, Option<Uuid>);

pub(super) struct TestPluginInvoker {
    pub(super) response: serde_json::Value,
    pub(super) seen: StdArc<Mutex<Vec<SeenInvocation>>>,
}

#[async_trait]
impl PluginSurfaceActionInvoker for TestPluginInvoker {
    async fn invoke(
        &self,
        ctx: SurfaceInvokerContext<'_>,
        surface_id: &str,
        interaction_id: &str,
        _method: uptrakit_wire::surfaces::InteractionHttpMethod,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        self.seen.lock().push((
            surface_id.to_string(),
            interaction_id.to_string(),
            ctx.tenant_id,
            ctx.caller_user_id,
        ));
        Ok(self.response.clone())
    }
}
