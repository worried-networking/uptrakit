#![expect(
    clippy::indexing_slicing,
    reason = "index is computed or validated to be in bounds"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget send intentionally drops the result"
)]

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

mod bookkeeping;
mod controller_local;
mod dispatch;
mod idempotency;
mod local_executor;
mod resolution;
mod validation;
use bookkeeping::{CachedIdempotent, IdempotencyKey, PendingState};
pub use controller_local::map_surface_action_error;
pub use controller_local::{CONTROLLER_LOCAL_EXECUTOR_TABLE, ExecutorTier};
use idempotency::{build_idempotency_key, fingerprint_request};
pub use local_executor::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, SurfaceInvokerContext,
    SurfaceLocalActionExecutor,
};
use resolution::{
    caller_origin_for_request, implicit_target_provider_for_request, map_lookup_error,
};
use validation::{resolve_timeout, validate_input_schema, validate_sensitive_fields};

#[cfg(test)]
mod tests;

pub use controller_local::AppStateSurfaceActionController;
pub mod entity_enrichment;
use uuid::Uuid;

use uptrakit_wire::{ControllerMessage, surfaces};

use crate::registry::SurfaceRegistry;
use uptrakit_service_connections::ServiceConnectionRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceProxyError {
    NoProvider,
    TargetProviderRequired,
    InvalidProvider(String),
    InteractionNotFound,
    PermissionDenied(String),
    Conflict { message: String, code: &'static str },
    SchemaValidationFailed(String),
    SensitiveFieldRejected(String),
    DuplicateRequest,
    RateLimited,
    ServiceDisconnected,
    SendFailed,
    Timeout,
}

impl std::fmt::Display for SurfaceProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProvider => write!(f, "no provider available for surface interaction"),
            Self::TargetProviderRequired => {
                write!(
                    f,
                    "target_provider_id is required for targeted surface interactions"
                )
            }
            Self::InvalidProvider(provider_id) => {
                write!(
                    f,
                    "provider `{provider_id}` is not valid for this surface interaction"
                )
            }
            Self::InteractionNotFound => write!(f, "surface interaction was not found"),
            Self::PermissionDenied(message) => write!(f, "{message}"),
            Self::Conflict { message, .. } => write!(f, "{message}"),
            Self::SchemaValidationFailed(message) => write!(f, "{message}"),
            Self::SensitiveFieldRejected(message) => write!(f, "{message}"),
            Self::DuplicateRequest => write!(f, "duplicate idempotency key"),
            Self::RateLimited => write!(f, "surface provider is temporarily rate-limited"),
            Self::ServiceDisconnected => write!(f, "target service disconnected"),
            Self::SendFailed => write!(f, "failed to send proxied surface request"),
            Self::Timeout => write!(f, "surface provider timed out"),
        }
    }
}

impl std::error::Error for SurfaceProxyError {}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SurfaceCallerOrigin {
    UserSession { user_id: Uuid, session_id: String },
    BuiltInSystem { principal: String },
    Provider { service_id: Uuid },
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SurfaceInvokeRequest {
    pub tenant_id: Uuid,
    pub surface_id: String,
    pub interaction_id: String,
    /// HTTP method the caller resolved this invocation against, threaded
    /// through to [`SurfaceRegistry::resolve_surface_action_for_method`].
    /// `None` preserves id-only resolution (single registered method
    /// required); callers with a concrete method (e.g. a REST route) should
    /// pass `Some`. The provider-origin wire path (a service-forwarded
    /// `SurfaceActionRequest`) always resolves by id only and must keep
    /// passing `None` here — its wire `method` field is not a trustworthy
    /// resolution key (see `message_processor.rs`'s handling of
    /// `SurfaceActionRequest.method`); method-aware resolution is
    /// HTTP-route-origin only.
    pub method: Option<surfaces::InteractionHttpMethod>,
    pub idempotency_key: String,
    pub target_provider_id: Option<String>,
    pub caller_origin: SurfaceCallerOrigin,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub encrypted_sensitive_params: Option<surfaces::EncryptedSensitiveParams>,
}

impl SurfaceInvokeRequest {
    /// Constructs a new [`SurfaceInvokeRequest`].
    ///
    /// External crates must use this constructor rather than a struct literal
    /// because the type is `#[non_exhaustive]`.
    #[expect(
        clippy::too_many_arguments,
        reason = "constructor mirrors the nine fields of #[non_exhaustive] SurfaceInvokeRequest"
    )]
    pub fn new(
        tenant_id: Uuid,
        surface_id: String,
        interaction_id: String,
        method: Option<surfaces::InteractionHttpMethod>,
        idempotency_key: String,
        target_provider_id: Option<String>,
        caller_origin: SurfaceCallerOrigin,
        params: serde_json::Map<String, serde_json::Value>,
        encrypted_sensitive_params: Option<surfaces::EncryptedSensitiveParams>,
    ) -> Self {
        Self {
            tenant_id,
            surface_id,
            interaction_id,
            method,
            idempotency_key,
            target_provider_id,
            caller_origin,
            params,
            encrypted_sensitive_params,
        }
    }
}

#[non_exhaustive]
pub struct SurfaceProxy {
    pending: Arc<Mutex<PendingState>>,
    local_executor: Arc<dyn SurfaceLocalActionExecutor>,
    provider_visibility: Arc<dyn crate::registry::SurfaceProviderVisibility>,
}

impl Default for SurfaceProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceProxy {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(PendingState::default())),
            local_executor: Arc::new(local_executor::NoopSurfaceLocalExecutor),
            provider_visibility: Arc::new(crate::registry::DenyAllPluginProviders),
        }
    }

    pub fn with_local_executor(
        mut self,
        local_executor: Arc<dyn SurfaceLocalActionExecutor>,
    ) -> Self {
        self.local_executor = local_executor;
        self
    }

    /// Stores the plugin-provider visibility filter used by [`Self::invoke`]'s
    /// internal resolution — the only gate on the provider-origin leg.
    /// Defaults to [`crate::registry::DenyAllPluginProviders`] (fail-closed):
    /// a proxy constructed without the production wiring hides plugin
    /// surfaces rather than serving them ungated.
    pub fn with_provider_visibility(
        mut self,
        provider_visibility: Arc<dyn crate::registry::SurfaceProviderVisibility>,
    ) -> Self {
        self.provider_visibility = provider_visibility;
        self
    }

    pub async fn invoke(
        &self,
        service_connections: &ServiceConnectionRegistry,
        registry: &SurfaceRegistry,
        request: SurfaceInvokeRequest,
        timeout_override: Option<Duration>,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        self.invoke_inner(service_connections, registry, request, timeout_override)
            .await
    }

    async fn invoke_inner(
        &self,
        service_connections: &ServiceConnectionRegistry,
        registry: &SurfaceRegistry,
        request: SurfaceInvokeRequest,
        timeout_override: Option<Duration>,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        let target_provider_id = implicit_target_provider_for_request(
            service_connections,
            registry,
            &request,
            self.provider_visibility.as_ref(),
        )
        .await?;
        let resolved = registry
            .resolve_surface_action_for_method(
                request.tenant_id,
                &request.surface_id,
                &request.interaction_id,
                request.method.as_ref(),
                target_provider_id.as_deref(),
                self.provider_visibility.as_ref(),
            )
            .map_err(map_lookup_error)?;

        let caller_origin =
            caller_origin_for_request(registry, &request.caller_origin, &resolved, &request)?;

        validate_input_schema(&resolved.interaction, &request.params)?;
        validate_sensitive_fields(
            &resolved.interaction,
            &request.params,
            request.encrypted_sensitive_params.as_ref(),
        )?;

        let timeout = resolve_timeout(timeout_override, &resolved.interaction)?;
        let request_fingerprint =
            fingerprint_request(&request.params, request.encrypted_sensitive_params.as_ref());
        let idem_key = build_idempotency_key(&request, &caller_origin);

        if matches!(&caller_origin, surfaces::CallerOrigin::Provider { .. })
            && (resolved.descriptor_required_action.is_some()
                || resolved.interaction_required_action.is_some())
            && !resolved.interaction.provider_invocable
        {
            return Err(SurfaceProxyError::PermissionDenied(
                "provider-initiated requests cannot satisfy user permission gates".to_string(),
            ));
        }

        if let Some(cached) = self.try_get_cached_response(&idem_key, request_fingerprint) {
            return Ok(cached);
        }

        match &resolved.interaction.transport {
            surfaces::InteractionTransport::ControllerLocal => {
                self.execute_local_invocation(
                    &resolved,
                    &request,
                    caller_origin,
                    idem_key,
                    request_fingerprint,
                    timeout,
                )
                .await
            }
            surfaces::InteractionTransport::ProviderProxied => {
                self.execute_proxied_invocation(
                    service_connections,
                    &resolved,
                    &request,
                    caller_origin,
                    idem_key,
                    request_fingerprint,
                    timeout,
                )
                .await
            }
            &_ => {
                tracing::warn!("unknown interaction transport — update match arm");
                Err(SurfaceProxyError::SchemaValidationFailed(
                    "unsupported interaction transport".to_string(),
                ))
            }
        }
    }

    pub fn complete(&self, request_id: Uuid, response: surfaces::SurfaceActionResponse) {
        let sender = self.pending.lock().take_pending(&request_id);
        if let Some(sender) = sender {
            let _ = sender.send(response);
        }
    }

    pub fn fail_in_flight_for_provider(&self, provider_id: &str) {
        let mut state = self.pending.lock();
        let request_ids: Vec<Uuid> = state
            .pending
            .iter()
            .filter_map(|(request_id, pending)| {
                if pending.provider_id == provider_id {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .collect();

        for request_id in &request_ids {
            let _ = state.remove_pending(request_id);
        }
        if !request_ids.is_empty() {
            state.record_provider_failure(provider_id);
        }
    }

    async fn timeout_pending_request(
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

    fn fail_pending_request(&self, provider_id: &str, request_id: Uuid) {
        let removed = {
            let mut state = self.pending.lock();
            state.remove_pending(&request_id)
        };
        if removed {
            self.record_provider_failure(provider_id);
        }
    }

    fn record_provider_failure(&self, provider_id: &str) {
        let mut state = self.pending.lock();
        state.record_provider_failure(provider_id);
    }

    fn try_get_cached_response(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Option<surfaces::SurfaceActionResponse> {
        let mut state = self.pending.lock();
        state.cleanup_expired();
        let cached = state.idempotency_cache.get(key)?;
        if cached.request_fingerprint == request_fingerprint {
            return Some(cached.response.clone());
        }
        None
    }

    fn store_cached_response(
        &self,
        key: IdempotencyKey,
        request_fingerprint: u64,
        response: surfaces::SurfaceActionResponse,
    ) {
        let mut state = self.pending.lock();
        state.cleanup_expired();
        state.idempotency_cache.insert(
            key,
            CachedIdempotent {
                request_fingerprint,
                response,
                stored_at: std::time::Instant::now(),
            },
        );
    }
}
