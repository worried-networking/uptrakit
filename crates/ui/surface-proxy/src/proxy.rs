#![expect(
    clippy::indexing_slicing,
    reason = "index is computed or validated to be in bounds"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget send intentionally drops the result"
)]

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

mod controller_local;
mod local_executor;
pub use controller_local::map_surface_action_error;
pub use local_executor::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, SurfaceLocalActionExecutor,
};

#[cfg(test)]
mod tests;

pub use controller_local::AppStateSurfaceActionController;
pub mod entity_enrichment;
use uuid::Uuid;

use uptrakit_wire::{ControllerMessage, surfaces};

use crate::registry::{SurfaceRegistry, SurfaceRegistryLookupError};
use uptrakit_service_connections::ServiceConnectionRegistry;

const DEFAULT_TIMEOUT_SECONDS: u16 = 30;
const MIN_TIMEOUT_SECONDS: u16 = 1;
const MAX_TIMEOUT_SECONDS: u16 = 300;
const MAX_IN_FLIGHT_PER_PROVIDER: usize = 32;
const MAX_IN_FLIGHT_PER_TENANT: usize = 128;
const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MAX_RESULT_ROWS: usize = 200;
const IDEMPOTENCY_RETENTION: Duration = Duration::from_secs(20 * 60);
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const FAILURE_LIMIT: usize = 5;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

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
        reason = "constructor mirrors the eight fields of #[non_exhaustive] SurfaceInvokeRequest"
    )]
    pub fn new(
        tenant_id: Uuid,
        surface_id: String,
        interaction_id: String,
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
            idempotency_key,
            target_provider_id,
            caller_origin,
            params,
            encrypted_sensitive_params,
        }
    }
}

#[derive(Default)]
struct PendingState {
    pending: HashMap<Uuid, PendingRequest>,
    in_flight_per_provider: HashMap<String, usize>,
    in_flight_per_tenant: HashMap<Uuid, usize>,
    in_flight_idempotency: HashMap<IdempotencyKey, IdempotencyInFlight>,
    idempotency_cache: HashMap<IdempotencyKey, CachedIdempotent>,
    provider_failures: HashMap<String, ProviderFailureState>,
}

#[derive(Debug)]
struct PendingRequest {
    provider_id: String,
    tenant_id: Uuid,
    idempotency_key: IdempotencyKey,
    sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdempotencyKey {
    tenant_id: Uuid,
    surface_id: String,
    interaction_id: String,
    caller_key: String,
    idempotency_key: String,
}

#[derive(Debug, Clone)]
struct IdempotencyInFlight {
    request_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct CachedIdempotent {
    request_fingerprint: u64,
    response: surfaces::SurfaceActionResponse,
    stored_at: std::time::Instant,
}

#[derive(Debug, Default)]
struct ProviderFailureState {
    failures: VecDeque<std::time::Instant>,
    blocked_until: Option<std::time::Instant>,
}

#[non_exhaustive]
pub struct SurfaceProxy {
    pending: Arc<Mutex<PendingState>>,
    local_executor: Arc<dyn SurfaceLocalActionExecutor>,
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
        }
    }

    pub fn with_local_executor(
        mut self,
        local_executor: Arc<dyn SurfaceLocalActionExecutor>,
    ) -> Self {
        self.local_executor = local_executor;
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
        let target_provider_id =
            implicit_target_provider_for_request(service_connections, registry, &request).await?;
        let resolved = registry
            .resolve_surface_action(
                request.tenant_id,
                &request.surface_id,
                &request.interaction_id,
                target_provider_id.as_deref(),
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
            && resolved.interaction.required_permission.is_some()
        {
            return Err(SurfaceProxyError::PermissionDenied(
                "provider-initiated requests cannot satisfy user permission gates".to_string(),
            ));
        }

        if let Some(cached) = self.try_get_cached_response(&idem_key, request_fingerprint) {
            return Ok(cached);
        }

        match &resolved.interaction.transport {
            surfaces::InteractionTransport::ControllerLocal
            | surfaces::InteractionTransport::DirectBuiltInApi { .. } => {
                if matches!(
                    resolved.interaction.transport,
                    surfaces::InteractionTransport::ControllerLocal
                ) && resolved.provider_kind != surfaces::ProviderKind::Plugin
                {
                    return Err(SurfaceProxyError::SchemaValidationFailed(
                        "controller_local transport is only supported for plugin providers"
                            .to_string(),
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
                let local_result = self.local_executor.execute(&resolved, &local_request).await;
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
            surfaces::InteractionTransport::ProviderProxied => {
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

impl PendingState {
    fn register_pending(
        &mut self,
        request_id: Uuid,
        provider_id: &str,
        tenant_id: Uuid,
        idempotency_key: IdempotencyKey,
        request_fingerprint: u64,
        sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
    ) {
        self.pending.insert(
            request_id,
            PendingRequest {
                provider_id: provider_id.to_string(),
                tenant_id,
                idempotency_key: idempotency_key.clone(),
                sender,
            },
        );
        *self
            .in_flight_per_provider
            .entry(provider_id.to_string())
            .or_default() += 1;
        *self.in_flight_per_tenant.entry(tenant_id).or_default() += 1;
        self.reserve_idempotency(idempotency_key, request_fingerprint);
    }

    fn take_pending(
        &mut self,
        request_id: &Uuid,
    ) -> Option<tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>> {
        let pending = self.pending.remove(request_id)?;
        decrement_counter(&mut self.in_flight_per_provider, &pending.provider_id);
        decrement_counter(&mut self.in_flight_per_tenant, &pending.tenant_id);
        self.in_flight_idempotency.remove(&pending.idempotency_key);
        Some(pending.sender)
    }

    fn remove_pending(&mut self, request_id: &Uuid) -> bool {
        self.take_pending(request_id).is_some()
    }

    fn reserve_idempotency(&mut self, key: IdempotencyKey, request_fingerprint: u64) {
        self.in_flight_idempotency.insert(
            key,
            IdempotencyInFlight {
                request_fingerprint,
            },
        );
    }

    fn release_idempotency(&mut self, key: &IdempotencyKey) {
        self.in_flight_idempotency.remove(key);
    }

    fn ensure_budget(&self, provider_id: &str, tenant_id: Uuid) -> Result<(), SurfaceProxyError> {
        if self
            .in_flight_per_provider
            .get(provider_id)
            .copied()
            .unwrap_or(0)
            >= MAX_IN_FLIGHT_PER_PROVIDER
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        if self
            .in_flight_per_tenant
            .get(&tenant_id)
            .copied()
            .unwrap_or(0)
            >= MAX_IN_FLIGHT_PER_TENANT
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        Ok(())
    }

    fn ensure_idempotency_available(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Result<(), SurfaceProxyError> {
        if let Some(in_flight) = self.in_flight_idempotency.get(key) {
            if in_flight.request_fingerprint == request_fingerprint {
                return Err(SurfaceProxyError::DuplicateRequest);
            }
            return Err(SurfaceProxyError::DuplicateRequest);
        }
        if let Some(cached) = self.idempotency_cache.get(key)
            && cached.request_fingerprint != request_fingerprint
        {
            return Err(SurfaceProxyError::DuplicateRequest);
        }
        Ok(())
    }

    fn ensure_provider_not_rate_limited(&self, provider_id: &str) -> Result<(), SurfaceProxyError> {
        let now = std::time::Instant::now();
        if let Some(state) = self.provider_failures.get(provider_id)
            && let Some(blocked_until) = state.blocked_until
            && blocked_until > now
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        Ok(())
    }

    fn record_provider_failure(&mut self, provider_id: &str) {
        let now = std::time::Instant::now();
        let tracker = self
            .provider_failures
            .entry(provider_id.to_string())
            .or_default();
        tracker.failures.push_back(now);
        while let Some(oldest) = tracker.failures.front().copied() {
            if now.duration_since(oldest) <= FAILURE_WINDOW {
                break;
            }
            tracker.failures.pop_front();
        }
        if tracker.failures.len() >= FAILURE_LIMIT {
            tracker.blocked_until = Some(now + FAILURE_COOLDOWN);
            tracker.failures.clear();
        }
    }

    fn cleanup_expired(&mut self) {
        let now = std::time::Instant::now();
        self.idempotency_cache
            .retain(|_, cached| now.duration_since(cached.stored_at) <= IDEMPOTENCY_RETENTION);
        for tracker in self.provider_failures.values_mut() {
            while let Some(oldest) = tracker.failures.front().copied() {
                if now.duration_since(oldest) <= FAILURE_WINDOW {
                    break;
                }
                tracker.failures.pop_front();
            }
            if tracker
                .blocked_until
                .is_some_and(|blocked_until| blocked_until <= now)
            {
                tracker.blocked_until = None;
            }
        }
    }
}

fn decrement_counter<K: Eq + Hash + Clone>(map: &mut HashMap<K, usize>, key: &K) {
    if let Some(counter) = map.get_mut(key) {
        *counter = counter.saturating_sub(1);
        if *counter == 0 {
            map.remove(key);
        }
    }
}

fn map_lookup_error(error: SurfaceRegistryLookupError) -> SurfaceProxyError {
    match error {
        SurfaceRegistryLookupError::SurfaceNotFound => SurfaceProxyError::NoProvider,
        SurfaceRegistryLookupError::InteractionNotFound => SurfaceProxyError::InteractionNotFound,
        SurfaceRegistryLookupError::TargetProviderRequired => {
            SurfaceProxyError::TargetProviderRequired
        }
        SurfaceRegistryLookupError::InvalidProvider(provider_id) => {
            SurfaceProxyError::InvalidProvider(provider_id)
        }
        SurfaceRegistryLookupError::NoTenantCompatibleProvider => SurfaceProxyError::NoProvider,
    }
}

fn caller_origin_for_request(
    registry: &SurfaceRegistry,
    caller: &SurfaceCallerOrigin,
    _resolved: &crate::registry::ResolvedSurfaceAction,
    _request: &SurfaceInvokeRequest,
) -> Result<surfaces::CallerOrigin, SurfaceProxyError> {
    match caller {
        SurfaceCallerOrigin::UserSession {
            user_id,
            session_id,
        } => Ok(surfaces::CallerOrigin::UserSession {
            user_id: user_id.to_string(),
            session_id: session_id.clone(),
        }),
        SurfaceCallerOrigin::BuiltInSystem { principal } => {
            Ok(surfaces::CallerOrigin::BuiltInSystem {
                principal: principal.clone(),
            })
        }
        SurfaceCallerOrigin::Provider { service_id } => {
            let provider_id = registry
                .provider_id_for_service(service_id)
                .ok_or_else(|| {
                    SurfaceProxyError::InvalidProvider(format!(
                        "service {service_id} has no registered surface provider"
                    ))
                })?;
            Ok(surfaces::CallerOrigin::Provider { provider_id })
        }
    }
}

async fn implicit_target_provider_for_request(
    service_connections: &ServiceConnectionRegistry,
    registry: &SurfaceRegistry,
    request: &SurfaceInvokeRequest,
) -> Result<Option<String>, SurfaceProxyError> {
    if request.target_provider_id.is_some() {
        return Ok(request.target_provider_id.clone());
    }

    match &request.caller_origin {
        SurfaceCallerOrigin::Provider { service_id } => registry
            .provider_id_for_service(service_id)
            .map(Some)
            .ok_or_else(|| {
                SurfaceProxyError::InvalidProvider(format!(
                    "service {service_id} has no registered surface provider"
                ))
            }),
        SurfaceCallerOrigin::UserSession { .. } | SurfaceCallerOrigin::BuiltInSystem { .. } => {
            let mut available_candidates = Vec::new();
            for provider in registry
                .list_targeted_providers_for_surface(request.surface_id.as_str(), request.tenant_id)
            {
                if !provider.tenant_compatible {
                    continue;
                }
                if provider_is_available(service_connections, &provider).await {
                    available_candidates.push(provider);
                }
            }

            if available_candidates.is_empty() {
                return Ok(None);
            }

            if available_candidates
                .iter()
                .any(|provider| provider.targeting == surfaces::Targeting::Targeted)
            {
                return Err(SurfaceProxyError::TargetProviderRequired);
            }

            Ok(available_candidates
                .first()
                .map(|provider| provider.provider_id.clone()))
        }
    }
}

async fn provider_is_available(
    service_connections: &ServiceConnectionRegistry,
    provider: &crate::registry::SurfaceProviderSummary,
) -> bool {
    match provider.service_id {
        Some(service_id) => {
            service_connections.is_connected(&service_id).await
                && !service_connections.is_yielded(&service_id)
        }
        None => true,
    }
}

fn validate_input_schema(
    interaction: &surfaces::InteractionDescriptor,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), SurfaceProxyError> {
    if let Some(schema) = &interaction.input_schema {
        let value = serde_json::Value::Object(params.clone());
        if !schema_matches(schema, &value) {
            return Err(SurfaceProxyError::SchemaValidationFailed(
                "input schema validation failed".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_sensitive_fields(
    interaction: &surfaces::InteractionDescriptor,
    params: &serde_json::Map<String, serde_json::Value>,
    encrypted_sensitive_params: Option<&surfaces::EncryptedSensitiveParams>,
) -> Result<(), SurfaceProxyError> {
    if interaction.sensitive_fields.is_empty() {
        return Ok(());
    }

    if matches!(
        interaction.transport,
        surfaces::InteractionTransport::ControllerLocal
    ) {
        return Ok(());
    }

    for key in &interaction.sensitive_fields {
        if params.contains_key(key) {
            return Err(SurfaceProxyError::SensitiveFieldRejected(format!(
                "sensitive field `{key}` must not be sent in cleartext params"
            )));
        }
    }

    if encrypted_sensitive_params.is_none() {
        return Ok(());
    }

    Ok(())
}

fn resolve_timeout(
    timeout_override: Option<Duration>,
    interaction: &surfaces::InteractionDescriptor,
) -> Result<Duration, SurfaceProxyError> {
    let timeout = timeout_override.unwrap_or_else(|| {
        Duration::from_secs(u64::from(
            interaction
                .timeout_seconds
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        ))
    });
    let secs = timeout.as_secs();
    if !(u64::from(MIN_TIMEOUT_SECONDS)..=u64::from(MAX_TIMEOUT_SECONDS)).contains(&secs) {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "timeout must be between {MIN_TIMEOUT_SECONDS}s and {MAX_TIMEOUT_SECONDS}s"
        )));
    }
    Ok(timeout)
}

fn validate_result_schema(
    interaction: &surfaces::InteractionDescriptor,
    result: Option<&serde_json::Value>,
) -> Result<(), SurfaceProxyError> {
    if let Some(schema) = &interaction.result_schema {
        let result = result.unwrap_or(&serde_json::Value::Null);
        if !schema_matches(schema, result) {
            return Err(SurfaceProxyError::SchemaValidationFailed(
                "result schema validation failed".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_result_limits(result: &serde_json::Value) -> Result<(), SurfaceProxyError> {
    let bytes = serde_json::to_vec(result)
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?
        .len();
    if bytes > MAX_RESULT_BYTES {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result payload is {bytes} bytes, max {MAX_RESULT_BYTES}"
        )));
    }

    if let Some(array) = result.as_array()
        && array.len() > MAX_RESULT_ROWS
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result array has {} rows, max {MAX_RESULT_ROWS}",
            array.len()
        )));
    }

    if let Some(rows) = result.get("rows").and_then(|rows| rows.as_array())
        && rows.len() > MAX_RESULT_ROWS
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result rows has {} rows, max {MAX_RESULT_ROWS}",
            rows.len()
        )));
    }

    Ok(())
}

fn build_idempotency_key(
    request: &SurfaceInvokeRequest,
    caller_origin: &surfaces::CallerOrigin,
) -> IdempotencyKey {
    IdempotencyKey {
        tenant_id: request.tenant_id,
        surface_id: request.surface_id.clone(),
        interaction_id: request.interaction_id.clone(),
        caller_key: match caller_origin {
            surfaces::CallerOrigin::UserSession {
                user_id,
                session_id,
            } => format!("user:{user_id}:{session_id}"),
            surfaces::CallerOrigin::BuiltInSystem { principal } => {
                format!("system:{principal}")
            }
            surfaces::CallerOrigin::Provider { provider_id } => {
                format!("provider:{provider_id}")
            }
        },
        idempotency_key: request.idempotency_key.clone(),
    }
}

fn fingerprint_request(
    params: &serde_json::Map<String, serde_json::Value>,
    encrypted: Option<&surfaces::EncryptedSensitiveParams>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::Value::Object(params.clone())
        .to_string()
        .hash(&mut hasher);
    encrypted
        .map(|value| (&value.key_id, &value.ciphertext_b64))
        .hash(&mut hasher);
    hasher.finish()
}

fn schema_matches(schema: &surfaces::SchemaContract, value: &serde_json::Value) -> bool {
    match schema {
        surfaces::SchemaContract::Any => true,
        surfaces::SchemaContract::Object => value.is_object(),
        surfaces::SchemaContract::Array => value.is_array(),
        surfaces::SchemaContract::String => value.is_string(),
        surfaces::SchemaContract::Integer => value.as_i64().is_some(),
        surfaces::SchemaContract::Number => value.as_f64().is_some(),
        surfaces::SchemaContract::Boolean => value.is_boolean(),
        surfaces::SchemaContract::Null => value.is_null(),
    }
}
