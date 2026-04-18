use super::validation::{validate_result_limits, validate_result_schema};
use super::*;

impl SurfaceProxy {
    pub(super) async fn execute_local_invocation(
        &self,
        resolved: &crate::surface_registry::ResolvedSurfaceAction,
        request: &SurfaceInvokeRequest,
        caller_origin: surfaces::CallerOrigin,
        idem_key: IdempotencyKey,
        request_fingerprint: u64,
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

        {
            let mut state = self.pending.lock();
            state.cleanup_expired();
            state.ensure_idempotency_available(&idem_key, request_fingerprint)?;
            state.reserve_idempotency(idem_key.clone(), request_fingerprint);
        }

        let local_request = surfaces::SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: request.tenant_id.to_string(),
            surface_id: resolved.descriptor.surface_id.clone(),
            interaction_id: resolved.interaction.interaction_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            target_provider_id: Some(resolved.provider_id.clone()),
            caller_origin,
            params: request.params.clone(),
            encrypted_sensitive_params: request.encrypted_sensitive_params.clone(),
        };
        let local_result = self.local_executor.execute(resolved, &local_request).await;

        {
            let mut state = self.pending.lock();
            state.release_idempotency(&idem_key);
        }

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

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_proxied_invocation(
        &self,
        service_connections: &ServiceConnectionRegistry,
        resolved: &crate::surface_registry::ResolvedSurfaceAction,
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
            state.register_pending(
                request_id,
                &resolved.provider_id,
                request.tenant_id,
                idem_key.clone(),
                request_fingerprint,
                tx,
            );
        }

        let outbound = surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: request.tenant_id.to_string(),
            surface_id: resolved.descriptor.surface_id.clone(),
            interaction_id: resolved.interaction.interaction_id.clone(),
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

    pub(super) async fn timeout_pending_request(
        &self,
        service_connections: &ServiceConnectionRegistry,
        service_id: Uuid,
        provider_id: &str,
        request_id: Uuid,
    ) {
        let removed = {
            let mut state = self.pending.lock();
            state.remove_pending(&request_id)
        };
        if removed {
            let _ = service_connections
                .send(
                    &service_id,
                    ControllerMessage::SurfaceActionCancel(surfaces::SurfaceActionCancel {
                        request_id,
                        target_provider_id: provider_id.to_string(),
                        reason: surfaces::SurfaceActionCancelReason::Timeout,
                    }),
                )
                .await;
            self.record_provider_failure(provider_id);
        }
    }

    pub(super) fn fail_pending_request(&self, provider_id: &str, request_id: Uuid) {
        let removed = {
            let mut state = self.pending.lock();
            state.remove_pending(&request_id)
        };
        if removed {
            self.record_provider_failure(provider_id);
        }
    }

    pub(super) fn record_provider_failure(&self, provider_id: &str) {
        let mut state = self.pending.lock();
        state.record_provider_failure(provider_id);
    }

    pub(super) fn try_get_cached_response(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Option<surfaces::SurfaceActionResponse> {
        let mut state = self.pending.lock();
        state.cleanup_expired();
        state.cached_response(key, request_fingerprint)
    }

    pub(super) fn store_cached_response(
        &self,
        key: IdempotencyKey,
        request_fingerprint: u64,
        response: surfaces::SurfaceActionResponse,
    ) {
        let mut state = self.pending.lock();
        state.cleanup_expired();
        state.store_cached_response(key, request_fingerprint, response);
    }
}
