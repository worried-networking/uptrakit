use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use uptrakit_service_connections::ServiceConnectionRegistry;
use uptrakit_wire::{ControllerMessage, surfaces};

use crate::registry::ResolvedSurfaceAction;

use super::bookkeeping::{IdempotencyGuard, IdempotencyKey, PendingGuard, PendingRegistration};
use super::validation::{validate_result_limits, validate_result_schema};
use super::{SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError};

impl SurfaceProxy {
    pub(super) async fn execute_local_invocation(
        &self,
        resolved: &ResolvedSurfaceAction,
        request: &SurfaceInvokeRequest,
        caller_origin: surfaces::CallerOrigin,
        idem_key: IdempotencyKey,
        request_fingerprint: u64,
        timeout: Duration,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        if matches!(
            resolved.interaction.transport,
            surfaces::InteractionTransport::ControllerLocal
        ) && resolved.provider_kind != surfaces::ProviderKind::Plugin
        {
            return Err(SurfaceProxyError::SchemaValidationFailed(
                "controller_local transport is only supported for plugin providers".to_string(),
            ));
        }

        let request_id = Uuid::now_v7();
        {
            let mut state = self.pending.lock();
            state.cleanup_expired();
            state.ensure_idempotency_available(&idem_key, request_fingerprint)?;
            state.reserve_idempotency(
                idem_key.clone(),
                request_fingerprint,
                request_id,
                std::time::Instant::now() + timeout,
            );
        }

        let _idem_guard =
            IdempotencyGuard::new(Arc::clone(&self.pending), idem_key.clone(), request_id);

        let local_request = surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: request.tenant_id.to_string(),
            surface_id: resolved.descriptor.surface_id.clone(),
            interaction_id: resolved.interaction.interaction_id.clone(),
            method: resolved.interaction.effective_http_method(),
            idempotency_key: request.idempotency_key.clone(),
            target_provider_id: Some(resolved.provider_id.clone()),
            caller_origin,
            params: request.params.clone(),
            encrypted_sensitive_params: request.encrypted_sensitive_params.clone(),
        };
        let local_result = self.local_executor.execute(resolved, &local_request).await;
        let result = local_result?;
        validate_result_schema(&resolved.interaction, Some(&result))?;
        validate_result_limits(&result)?;
        let response = surfaces::SurfaceActionResponse {
            request_id: local_request.request_id,
            success: true,
            result: Some(result),
            error: None,
        };
        self.store_cached_response(idem_key, request_fingerprint, response.clone());
        Ok(response)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "parameters mirror invoke_inner's resolved invocation state; \
                  bundling them would author new types in a mechanical move"
    )]
    pub(super) async fn execute_proxied_invocation(
        &self,
        service_connections: &ServiceConnectionRegistry,
        resolved: &ResolvedSurfaceAction,
        request: &SurfaceInvokeRequest,
        caller_origin: surfaces::CallerOrigin,
        idem_key: IdempotencyKey,
        request_fingerprint: u64,
        timeout: Duration,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        let Some(service_id) = resolved.service_id else {
            return Err(SurfaceProxyError::NoProvider);
        };
        if !service_connections.is_connected(&service_id).await
            || service_connections.is_yielded(&service_id)
        {
            return Err(SurfaceProxyError::NoProvider);
        }

        let request_id = Uuid::now_v7();
        let (tx, rx) = tokio::sync::oneshot::channel();

        {
            let mut state = self.pending.lock();
            state.cleanup_expired();
            state.ensure_provider_not_rate_limited(&resolved.provider_id)?;
            state.ensure_budget(&resolved.provider_id, request.tenant_id)?;
            state.ensure_idempotency_available(&idem_key, request_fingerprint)?;
            state.register_pending(PendingRegistration {
                request_id,
                provider_id: &resolved.provider_id,
                tenant_id: request.tenant_id,
                idempotency_key: idem_key.clone(),
                request_fingerprint,
                deadline: std::time::Instant::now() + timeout,
                sender: tx,
            });
        }

        let _cleanup_guard = PendingGuard::new(Arc::clone(&self.pending), request_id);

        let outbound = surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: request.tenant_id.to_string(),
            surface_id: resolved.descriptor.surface_id.clone(),
            interaction_id: resolved.interaction.interaction_id.clone(),
            method: resolved.interaction.effective_http_method(),
            idempotency_key: request.idempotency_key.clone(),
            target_provider_id: Some(resolved.provider_id.clone()),
            caller_origin,
            params: request.params.clone(),
            encrypted_sensitive_params: request.encrypted_sensitive_params.clone(),
        };

        let sent = service_connections
            .send(
                &service_id,
                ControllerMessage::SurfaceActionRequest(outbound),
            )
            .await;
        if !sent {
            self.fail_pending_request(&resolved.provider_id, request_id);
            return Err(SurfaceProxyError::SendFailed);
        }

        let response = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                self.record_provider_failure(&resolved.provider_id);
                return Err(SurfaceProxyError::ServiceDisconnected);
            }
            Err(_) => {
                self.timeout_pending_request(
                    service_connections,
                    service_id,
                    &resolved.provider_id,
                    request_id,
                )
                .await;
                return Err(SurfaceProxyError::Timeout);
            }
        };

        if response.success {
            validate_result_schema(&resolved.interaction, response.result.as_ref())?;
            if let Some(result) = response.result.as_ref() {
                validate_result_limits(result)?;
            }
        }

        self.store_cached_response(idem_key, request_fingerprint, response.clone());
        Ok(response)
    }
}
