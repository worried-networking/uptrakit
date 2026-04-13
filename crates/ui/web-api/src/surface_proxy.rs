use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::{ExtensionActionContext, PluginOps};
use uuid::Uuid;

use uptrakit_internal_wire::{ControllerMessage, surfaces};

use crate::service_connections::ServiceConnectionRegistry;
use crate::surface_registry::{SurfaceRegistry, SurfaceRegistryLookupError};

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
pub enum SurfaceProxyError {
    NoProvider,
    TargetProviderRequired,
    InvalidProvider(String),
    InteractionNotFound,
    PermissionDenied(String),
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
pub enum SurfaceCallerOrigin {
    UserSession { user_id: Uuid, session_id: String },
    BuiltInSystem { principal: String },
    Provider { service_id: Uuid },
}

#[derive(Debug, Clone)]
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

#[async_trait]
pub trait SurfaceLocalActionExecutor: Send + Sync {
    async fn execute(
        &self,
        _resolved: &crate::surface_registry::ResolvedSurfaceAction,
        _request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError>;
}

#[async_trait]
pub trait PluginSurfaceActionInvoker: Send + Sync {
    async fn invoke(
        &self,
        db: &(dyn std::any::Any + Send + Sync),
        tenant_id: Option<Uuid>,
        caller_user_id: Option<Uuid>,
        surface_id: &str,
        interaction_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String>;
}

pub struct PluginOpsSurfaceActionInvoker {
    plugin_ops: Arc<dyn PluginOps>,
}

impl PluginOpsSurfaceActionInvoker {
    pub fn new(plugin_ops: Arc<dyn PluginOps>) -> Self {
        Self { plugin_ops }
    }
}

#[async_trait]
impl PluginSurfaceActionInvoker for PluginOpsSurfaceActionInvoker {
    async fn invoke(
        &self,
        db: &(dyn std::any::Any + Send + Sync),
        tenant_id: Option<Uuid>,
        caller_user_id: Option<Uuid>,
        surface_id: &str,
        interaction_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let ctx = ExtensionActionContext {
            db,
            tenant_id,
            caller_user_id,
        };
        self.plugin_ops
            .handle_extension_action(&ctx, surface_id, interaction_id, params)
            .await
    }
}

pub struct PluginSurfaceLocalExecutor {
    action_context_db: Arc<dyn std::any::Any + Send + Sync>,
    plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>,
}

impl PluginSurfaceLocalExecutor {
    pub fn new(
        action_context_db: Arc<dyn std::any::Any + Send + Sync>,
        plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>,
    ) -> Self {
        Self {
            action_context_db,
            plugin_invoker,
        }
    }
}

#[async_trait]
impl SurfaceLocalActionExecutor for PluginSurfaceLocalExecutor {
    async fn execute(
        &self,
        resolved: &crate::surface_registry::ResolvedSurfaceAction,
        request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError> {
        if resolved.provider_kind != surfaces::ProviderKind::Plugin {
            return Err(SurfaceProxyError::SchemaValidationFailed(format!(
                "local surface transport is only implemented for plugin providers (got `{}`)",
                resolved.provider_id
            )));
        }

        if resolved.interaction.transport != surfaces::InteractionTransport::ControllerLocal {
            return Err(SurfaceProxyError::SchemaValidationFailed(format!(
                "plugin local executor only supports controller_local transport for interaction `{}`",
                resolved.interaction.interaction_id
            )));
        }

        let tenant_id = Uuid::parse_str(request.tenant_id.as_str()).map_err(|error| {
            SurfaceProxyError::SchemaValidationFailed(format!(
                "invalid tenant_id in surface action request: {error}"
            ))
        })?;
        let caller_user_id = match &request.caller_origin {
            surfaces::CallerOrigin::UserSession { user_id, .. } => {
                Some(Uuid::parse_str(user_id.as_str()).map_err(|error| {
                    SurfaceProxyError::SchemaValidationFailed(format!(
                        "invalid caller user_id in surface action request: {error}"
                    ))
                })?)
            }
            _ => None,
        };

        self.plugin_invoker
            .invoke(
                self.action_context_db.as_ref(),
                Some(tenant_id),
                caller_user_id,
                request.surface_id.as_str(),
                request.interaction_id.as_str(),
                serde_json::Value::Object(request.params.clone()),
            )
            .await
            .map_err(SurfaceProxyError::SchemaValidationFailed)
    }
}

struct NoopSurfaceLocalExecutor;

#[async_trait]
impl SurfaceLocalActionExecutor for NoopSurfaceLocalExecutor {
    async fn execute(
        &self,
        resolved: &crate::surface_registry::ResolvedSurfaceAction,
        _request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError> {
        Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "local surface transport is not implemented for provider `{}`",
            resolved.provider_id
        )))
    }
}

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
            local_executor: Arc::new(NoopSurfaceLocalExecutor),
        }
    }

    pub fn with_local_executor(
        mut self,
        local_executor: Arc<dyn SurfaceLocalActionExecutor>,
    ) -> Self {
        self.local_executor = local_executor;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke(
        &self,
        service_connections: &ServiceConnectionRegistry,
        registry: &SurfaceRegistry,
        request: SurfaceInvokeRequest,
        timeout_override: Option<Duration>,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        let resolved = registry
            .resolve_surface_action(
                request.tenant_id,
                &request.surface_id,
                &request.interaction_id,
                request.target_provider_id.as_deref(),
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

        if let Some(cached) = self.try_get_cached_response(&idem_key, request_fingerprint) {
            return Ok(cached);
        }

        if let surfaces::CallerOrigin::Provider { .. } = &caller_origin {
            if resolved.interaction.required_permission.is_some() {
                return Err(SurfaceProxyError::PermissionDenied(
                    "provider-initiated requests cannot satisfy user permission gates".to_string(),
                ));
            }
        }

        match &resolved.interaction.transport {
            surfaces::InteractionTransport::ControllerLocal
            | surfaces::InteractionTransport::DirectBuiltInApi { .. } => {
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
    _resolved: &crate::surface_registry::ResolvedSurfaceAction,
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
        return Err(SurfaceProxyError::SensitiveFieldRejected(
            "encrypted_sensitive_params is required for sensitive fields".to_string(),
        ));
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc as StdArc;

    use async_trait::async_trait;
    use uptrakit_internal_wire::ControllerMessage;

    use super::*;
    use crate::surface_registry::{SurfaceRegistry, SurfaceRegistryConfig};

    fn tenant_id() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn user_id() -> Uuid {
        Uuid::parse_str("aaaaaaaa-1111-1111-1111-111111111111").unwrap()
    }

    fn registration(provider_id: &str, service_tenant: Uuid) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Service,
                provider_namespace: "service".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::TargetedTargeting,
                surfaces::Capability::MutationAction,
                surfaces::Capability::SensitiveFields,
                surfaces::Capability::ProviderInitiatedActions,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(service_tenant.to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                    label: "SSH".to_string(),
                    priority: 100,
                    slot: "software.tabs".to_string(),
                    scope: surfaces::Scope::Tenant,
                    targeting: surfaces::Targeting::Targeted,
                    required_permission: Some("view_software".to_string()),
                    provider_kind: surfaces::ProviderKind::Service,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::TargetedTargeting,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec!["token".to_string()],
                    timeout_seconds: Some(2),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ProviderProxied,
                    workflow_steps: vec![],
                }],
                data_sources: vec![],
            }],
            encryption_metadata: Some(surfaces::ProviderEncryptionMetadata {
                key_id: "key-1".to_string(),
                algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
                public_key: "pubkey".to_string(),
            }),
        }
    }

    fn plugin_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("notifications.email.global_smtp")
                        .unwrap(),
                    label: "SMTP Defaults".to_string(),
                    priority: 100,
                    slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                    scope: surfaces::Scope::Global,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: None,
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::UniversalTargeting,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("save_global_smtp").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    required_permission: None,
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec![],
                    timeout_seconds: Some(30),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    fn plugin_registration_with_local_sensitive(
        provider_id: &str,
    ) -> surfaces::SurfaceRegistration {
        let mut registration = plugin_registration(provider_id);
        registration
            .capabilities
            .0
            .insert(surfaces::Capability::SensitiveFields);
        registration.surfaces[0].interactions[0].sensitive_fields =
            vec!["smtp_password".to_string()];
        registration
    }

    fn registry() -> SurfaceRegistry {
        SurfaceRegistry::new(SurfaceRegistryConfig {
            allowed_controller_queries: HashSet::new(),
            allowed_sse_topics: HashSet::new(),
            allowed_direct_builtin_operations: HashSet::new(),
            ..SurfaceRegistryConfig::default()
        })
    }

    fn request_with_idem(idempotency_key: &str) -> SurfaceInvokeRequest {
        SurfaceInvokeRequest {
            tenant_id: tenant_id(),
            surface_id: "ssh.guest.panel".to_string(),
            interaction_id: "refresh".to_string(),
            idempotency_key: idempotency_key.to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: SurfaceCallerOrigin::UserSession {
                user_id: user_id(),
                session_id: "session-1".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: Some(surfaces::EncryptedSensitiveParams {
                key_id: "key-1".to_string(),
                algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
                ciphertext_b64: "AAAA".to_string(),
            }),
        }
    }

    struct TestPluginInvoker {
        response: serde_json::Value,
        seen: StdArc<Mutex<Vec<(String, String, Option<Uuid>, Option<Uuid>)>>>,
    }

    #[async_trait]
    impl PluginSurfaceActionInvoker for TestPluginInvoker {
        async fn invoke(
            &self,
            _db: &(dyn std::any::Any + Send + Sync),
            tenant_id: Option<Uuid>,
            caller_user_id: Option<Uuid>,
            surface_id: &str,
            interaction_id: &str,
            _params: serde_json::Value,
        ) -> std::result::Result<serde_json::Value, String> {
            self.seen.lock().push((
                surface_id.to_string(),
                interaction_id.to_string(),
                tenant_id,
                caller_user_id,
            ));
            Ok(self.response.clone())
        }
    }

    struct BlockingPluginInvoker {
        started: StdArc<tokio::sync::Notify>,
        release: StdArc<tokio::sync::Notify>,
        calls: StdArc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl PluginSurfaceActionInvoker for BlockingPluginInvoker {
        async fn invoke(
            &self,
            _db: &(dyn std::any::Any + Send + Sync),
            _tenant_id: Option<Uuid>,
            _caller_user_id: Option<Uuid>,
            _surface_id: &str,
            _interaction_id: &str,
            _params: serde_json::Value,
        ) -> std::result::Result<serde_json::Value, String> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.started.notify_waiters();
            self.release.notified().await;
            Ok(serde_json::json!({"ok": true}))
        }
    }

    async fn register_service_for_proxy(
        registry: &SurfaceRegistry,
        service_connections: &ServiceConnectionRegistry,
    ) -> (Uuid, tokio::sync::mpsc::Receiver<ControllerMessage>) {
        let service_id = Uuid::now_v7();
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id()),
                registration("provider-a", tenant_id()),
            )
            .expect("registration should succeed");

        let (rx, _cancel) = service_connections
            .register(
                service_id,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        (service_id, rx)
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_correlates_request_and_response() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());

        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;
        let proxy_clone = Arc::clone(&proxy);

        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy_clone.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"ok": true})),
                        error: None,
                    },
                );
            }
        });

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-1"),
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("invoke should succeed");

        assert!(response.success);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_executes_plugin_controller_local_interaction() {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
            .expect("plugin registration should succeed");

        let seen = StdArc::new(Mutex::new(Vec::new()));
        let invoker = TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::clone(&seen),
        };
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(StdArc::new(()), Arc::new(invoker)),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-plugin-local".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("plugin-backed local interaction should succeed");

        assert!(response.success);
        assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "notifications.email.global_smtp");
        assert_eq!(seen[0].1, "save_global_smtp");
        assert_eq!(seen[0].2, Some(tenant_id()));
        assert_eq!(seen[0].3, Some(user_id()));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_rejects_duplicate_idempotency_deterministically() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let proxy_clone = Arc::clone(&proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy_clone.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"cached": true})),
                        error: None,
                    },
                );
            }
        });

        let first = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-dup"),
                Some(Duration::from_secs(5)),
            )
            .await;
        assert!(first.is_ok(), "first invocation should succeed");

        let second = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-dup"),
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("second invocation should be deterministic");
        assert!(
            second.success,
            "duplicate should return deterministic previous result"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_controller_local_rejects_concurrent_duplicate_idempotency() {
        let registry = Arc::new(SurfaceRegistry::new(SurfaceRegistryConfig::default()));
        registry
            .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
            .expect("plugin registration should succeed");

        let started = StdArc::new(tokio::sync::Notify::new());
        let release = StdArc::new(tokio::sync::Notify::new());
        let calls = StdArc::new(std::sync::atomic::AtomicUsize::new(0));
        let proxy = Arc::new(SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                StdArc::new(()),
                Arc::new(BlockingPluginInvoker {
                    started: StdArc::clone(&started),
                    release: StdArc::clone(&release),
                    calls: StdArc::clone(&calls),
                }),
            ),
        )));
        let service_connections = Arc::new(ServiceConnectionRegistry::new());

        let proxy_first = Arc::clone(&proxy);
        let registry_first = Arc::clone(&registry);
        let service_connections_first = Arc::clone(&service_connections);
        let first_invoke = tokio::spawn(async move {
            proxy_first
                .invoke(
                    &service_connections_first,
                    &registry_first,
                    SurfaceInvokeRequest {
                        tenant_id: tenant_id(),
                        surface_id: "notifications.email.global_smtp".to_string(),
                        interaction_id: "save_global_smtp".to_string(),
                        idempotency_key: "idem-local-dup".to_string(),
                        target_provider_id: None,
                        caller_origin: SurfaceCallerOrigin::UserSession {
                            user_id: user_id(),
                            session_id: "session-1".to_string(),
                        },
                        params: serde_json::Map::new(),
                        encrypted_sensitive_params: None,
                    },
                    Some(Duration::from_secs(5)),
                )
                .await
        });

        started.notified().await;

        let second = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-local-dup".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await;

        assert!(
            matches!(second, Err(SurfaceProxyError::DuplicateRequest)),
            "concurrent duplicate local invocation must be rejected"
        );

        release.notify_waiters();
        let first = first_invoke
            .await
            .expect("first invoke task should complete")
            .expect("first local invocation should succeed");
        assert!(first.success);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_rejects_cleartext_sensitive_fields() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let (_service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

        let mut request = request_with_idem("idem-cleartext");
        request
            .params
            .insert("token".to_string(), serde_json::json!("clear"));

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect_err("cleartext sensitive field should be rejected");
        assert!(matches!(err, SurfaceProxyError::SensitiveFieldRejected(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_controller_local_allows_cleartext_sensitive_fields() {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(plugin_registration_with_local_sensitive(
                "plugin.notifications_email",
            ))
            .expect("plugin registration should succeed");

        let invoker = TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::new(Mutex::new(Vec::new())),
        };
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(StdArc::new(()), Arc::new(invoker)),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let mut params = serde_json::Map::new();
        params.insert("smtp_password".to_string(), serde_json::json!("clear"));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-plugin-local-sensitive".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params,
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("controller-local sensitive fields should be accepted in cleartext");

        assert!(response.success);
        assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_times_out_and_emits_cancellation() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let invoke_task = tokio::spawn({
            let request = request_with_idem("idem-timeout");
            let proxy = proxy;
            async move {
                proxy
                    .invoke(
                        &service_connections,
                        &registry,
                        request,
                        Some(Duration::from_secs(2)),
                    )
                    .await
            }
        });

        let first = rx
            .recv()
            .await
            .expect("first message should be action request");
        assert!(matches!(first, ControllerMessage::SurfaceActionRequest(_)));
        let second = rx.recv().await.expect("second message should be cancel");
        assert!(matches!(second, ControllerMessage::SurfaceActionCancel(_)));

        let result = invoke_task.await.expect("invoke task should finish");
        assert!(matches!(result, Err(SurfaceProxyError::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_ignores_late_response_after_timeout() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let proxy_clone = Arc::clone(&proxy);
        tokio::spawn(async move {
            let request = match rx.recv().await {
                Some(ControllerMessage::SurfaceActionRequest(request)) => request,
                other => panic!("expected surface action request, got {other:?}"),
            };
            tokio::time::advance(Duration::from_secs(3)).await;
            proxy_clone.complete(
                request.request_id,
                surfaces::SurfaceActionResponse {
                    request_id: request.request_id,
                    success: true,
                    result: Some(serde_json::json!({"late": true})),
                    error: None,
                },
            );
        });

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-late"),
                Some(Duration::from_secs(2)),
            )
            .await;
        assert!(matches!(result, Err(SurfaceProxyError::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_validates_input_schema_before_dispatch() {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let service_id = Uuid::now_v7();
        let mut custom_registration = registration("provider-a", tenant_id());
        custom_registration.surfaces[0].interactions[0].input_schema =
            Some(surfaces::SchemaContract::Integer);
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id()),
                custom_registration,
            )
            .expect("registration should succeed");
        let (_rx, _cancel) = service_connections
            .register(
                service_id,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let mut request = request_with_idem("idem-schema");
        request
            .params
            .insert("value".to_string(), serde_json::json!("not-integer"));

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await;
        assert!(
            matches!(result, Err(SurfaceProxyError::SchemaValidationFailed(_))),
            "expected schema validation failure, got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_targeted_surface_requires_explicit_target_provider() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let (_service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

        let mut request = request_with_idem("idem-missing-target");
        request.target_provider_id = None;

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect_err("targeted invocation must require explicit target provider");
        assert!(matches!(err, SurfaceProxyError::TargetProviderRequired));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_provider_origin_can_route_to_another_provider() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());

        let service_a = Uuid::now_v7();
        let mut reg_a = registration("provider-a", tenant_id());
        reg_a.surfaces[0].interactions[0].required_permission = None;
        registry
            .register_service(service_a, "uptrakit-agent-ssh", Some(tenant_id()), reg_a)
            .expect("provider-a registration should succeed");

        let service_b = Uuid::now_v7();
        let mut reg_b = registration("provider-b", tenant_id());
        reg_b.surfaces[0].interactions[0].required_permission = None;
        registry
            .register_service(service_b, "uptrakit-agent-ssh", Some(tenant_id()), reg_b)
            .expect("provider-b registration should succeed");

        let (_rx_a, _cancel_a) = service_connections
            .register(
                service_a,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        let (mut rx_b, _cancel_b) = service_connections
            .register(
                service_b,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let proxy_clone = Arc::clone(&proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx_b.recv().await {
                proxy_clone.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"routed": true})),
                        error: None,
                    },
                );
            }
        });

        let mut request = request_with_idem("idem-cross-provider");
        request.target_provider_id = Some("provider-b".to_string());
        request.caller_origin = SurfaceCallerOrigin::Provider {
            service_id: service_a,
        };

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("controller-authorized cross-provider invoke should succeed");
        assert!(response.success);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_fails_immediately_when_provider_disconnects() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let proxy_invoke = Arc::clone(&proxy);
        let invoke_task = tokio::spawn(async move {
            proxy_invoke
                .invoke(
                    &service_connections,
                    &registry,
                    request_with_idem("idem-disconnect"),
                    Some(Duration::from_secs(60)),
                )
                .await
        });

        let outbound = rx
            .recv()
            .await
            .expect("first message should be action request");
        assert!(matches!(
            outbound,
            ControllerMessage::SurfaceActionRequest(_)
        ));

        proxy.fail_in_flight_for_provider("provider-a");

        let result = invoke_task.await.expect("invoke task should finish");
        assert!(matches!(
            result,
            Err(SurfaceProxyError::ServiceDisconnected)
        ));
    }
}
