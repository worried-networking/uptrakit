use std::time::Duration;

use uptrakit_internal_wire::surfaces;

use crate::service_connections::ServiceConnectionRegistry;
use crate::surface_registry::SurfaceRegistry;

use super::idempotency::{build_idempotency_key, fingerprint_request};
use super::validation::{
    caller_origin_for_request, implicit_target_provider_for_request, map_lookup_error,
    resolve_timeout, validate_input_schema, validate_sensitive_fields,
};
use super::{IdempotencyKey, SurfaceInvokeRequest, SurfaceProxyError};

pub(super) struct PreparedInvocation {
    pub(super) resolved: crate::surface_registry::ResolvedSurfaceAction,
    pub(super) caller_origin: surfaces::CallerOrigin,
    pub(super) timeout: Duration,
    pub(super) idem_key: IdempotencyKey,
    pub(super) request_fingerprint: u64,
}

pub(super) async fn prepare_invocation(
    service_connections: &ServiceConnectionRegistry,
    registry: &SurfaceRegistry,
    request: &SurfaceInvokeRequest,
    timeout_override: Option<Duration>,
) -> Result<PreparedInvocation, SurfaceProxyError> {
    let target_provider_id =
        implicit_target_provider_for_request(service_connections, registry, request).await?;
    let resolved = registry
        .resolve_surface_action(
            request.tenant_id,
            &request.surface_id,
            &request.interaction_id,
            target_provider_id.as_deref(),
        )
        .map_err(map_lookup_error)?;

    let caller_origin =
        caller_origin_for_request(registry, &request.caller_origin, &resolved, request)?;

    validate_input_schema(&resolved.interaction, &request.params)?;
    validate_sensitive_fields(
        &resolved.interaction,
        &request.params,
        request.encrypted_sensitive_params.as_ref(),
    )?;

    let timeout = resolve_timeout(timeout_override, &resolved.interaction)?;
    let request_fingerprint =
        fingerprint_request(&request.params, request.encrypted_sensitive_params.as_ref());
    let idem_key = build_idempotency_key(request, &caller_origin);

    if matches!(&caller_origin, surfaces::CallerOrigin::Provider { .. })
        && resolved.interaction.required_permission.is_some()
    {
        return Err(SurfaceProxyError::PermissionDenied(
            "provider-initiated requests cannot satisfy user permission gates".to_string(),
        ));
    }

    Ok(PreparedInvocation {
        resolved,
        caller_origin,
        timeout,
        idem_key,
        request_fingerprint,
    })
}
