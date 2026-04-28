use async_trait::async_trait;

use uptrakit_plugin_infrastructure_registry::agent_infra::{
    InfraActionInvokeError, InfraActionInvoker,
};
use uptrakit_wire::{
    ServiceMessage,
    surfaces::{self, SurfaceActionResponse},
};

/// [`InfraActionInvoker`] that routes calls through the `ServiceSurfaceProxy`.
///
/// Wraps `invoke_proxy_surface_action` so that infrastructure plugins can invoke
/// controller-side surface actions without depending on `uptrakit-service-sdk`.
pub struct InfraActionInvokerImpl<'a> {
    proxy: &'a uptrakit_service_sdk::ServiceSurfaceProxy,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    tenant_id: Option<uuid::Uuid>,
}

impl<'a> InfraActionInvokerImpl<'a> {
    pub fn new(
        proxy: &'a uptrakit_service_sdk::ServiceSurfaceProxy,
        bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
        tenant_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            proxy,
            bg_tx,
            tenant_id,
        }
    }
}

#[async_trait]
impl InfraActionInvoker for InfraActionInvokerImpl<'_> {
    async fn invoke(
        &self,
        surface_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<SurfaceActionResponse, InfraActionInvokeError> {
        invoke_proxy_surface_action(
            self.proxy,
            self.bg_tx,
            self.tenant_id,
            surface_id,
            action_id,
            params,
        )
        .await
        .map_err(|e| InfraActionInvokeError::from(e.to_string()))
    }
}

/// Invoke a surface action on the controller via the proxy.
///
/// Sends the request via `bg_tx` (which flows through the event loop to
/// `conn.send()`), then waits for the controller's response via the proxy's
/// oneshot channel.
pub(crate) async fn invoke_proxy_surface_action(
    proxy: &uptrakit_service_sdk::ServiceSurfaceProxy,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    tenant_id: Option<uuid::Uuid>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Result<SurfaceActionResponse, uptrakit_service_sdk::ServiceSurfaceProxyError> {
    let Some(tenant_id) = tenant_id else {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    };
    let Ok(surface_id) = surfaces::SurfaceId::new(surface_id.to_string()) else {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    };
    let Ok(interaction_id) = surfaces::InteractionId::new(action_id.to_string()) else {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    };
    let params_map = params.as_object().cloned().unwrap_or_default();
    let pending = proxy.invoke(
        tenant_id,
        surface_id,
        interaction_id,
        &uuid::Uuid::now_v7().to_string(),
        surfaces::CallerOrigin::Provider {
            provider_id: "service.uptrakit-agent-ssh".to_string(),
        },
        params_map,
        None,
        None,
    );

    // Send the request to the controller via bg_tx.
    if bg_tx.send(pending.message.clone()).await.is_err() {
        return Err(uptrakit_service_sdk::ServiceSurfaceProxyError::SendFailed);
    }

    // Wait for the response (15s timeout for proxy calls).
    let response = pending
        .wait(proxy, std::time::Duration::from_secs(15))
        .await?;
    Ok(response)
}
