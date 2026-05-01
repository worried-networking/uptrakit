use std::time::Duration;

use uptrakit_wire::surfaces;

use crate::registry::{SurfaceRegistry, SurfaceRegistryLookupError};
use uptrakit_service_connections::ServiceConnectionRegistry;

use super::{SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxyError};

pub(super) fn map_lookup_error(error: SurfaceRegistryLookupError) -> SurfaceProxyError {
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

pub(super) fn caller_origin_for_request(
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

pub(super) async fn implicit_target_provider_for_request(
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

pub(super) async fn provider_is_available(
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

pub(super) fn validate_input_schema(
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

pub(super) fn validate_sensitive_fields(
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

pub(super) fn resolve_timeout(
    timeout_override: Option<Duration>,
    interaction: &surfaces::InteractionDescriptor,
) -> Result<Duration, SurfaceProxyError> {
    let timeout = timeout_override.unwrap_or_else(|| {
        Duration::from_secs(u64::from(
            interaction
                .timeout_seconds
                .unwrap_or(super::DEFAULT_TIMEOUT_SECONDS),
        ))
    });
    let secs = timeout.as_secs();
    if !(u64::from(super::MIN_TIMEOUT_SECONDS)..=u64::from(super::MAX_TIMEOUT_SECONDS))
        .contains(&secs)
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "timeout must be between {MIN}s and {MAX}s",
            MIN = super::MIN_TIMEOUT_SECONDS,
            MAX = super::MAX_TIMEOUT_SECONDS
        )));
    }
    Ok(timeout)
}

pub(super) fn validate_result_schema(
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

pub(super) fn validate_result_limits(result: &serde_json::Value) -> Result<(), SurfaceProxyError> {
    let bytes = serde_json::to_vec(result)
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?
        .len();
    if bytes > super::MAX_RESULT_BYTES {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result payload is {bytes} bytes, max {max}",
            max = super::MAX_RESULT_BYTES
        )));
    }

    if let Some(array) = result.as_array()
        && array.len() > super::MAX_RESULT_ROWS
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result array has {} rows, max {max}",
            array.len(),
            max = super::MAX_RESULT_ROWS
        )));
    }

    if let Some(rows) = result.get("rows").and_then(|rows| rows.as_array())
        && rows.len() > super::MAX_RESULT_ROWS
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result rows has {} rows, max {max}",
            rows.len(),
            max = super::MAX_RESULT_ROWS
        )));
    }

    Ok(())
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
