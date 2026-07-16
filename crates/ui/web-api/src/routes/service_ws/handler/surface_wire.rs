//! Surface↔wire conversion helpers.
//!
//! Pure functions that convert between the in-memory surface/proxy domain types
//! and the wire-level types sent to clients over WebSocket.

pub(super) fn register_surface_provider(
    surface_registry: &crate::surface_registry::SurfaceRegistry,
    surface_proxy: &crate::surface_proxy::SurfaceProxy,
    service_id: uuid::Uuid,
    app_name: &str,
    service_tenant_id: Option<uuid::Uuid>,
    payload: uptrakit_wire::surfaces::SurfaceRegistration,
) -> Result<(), crate::surface_registry::SurfaceRegistryError> {
    let previous_provider_id = surface_registry.provider_id_for_service(&service_id);
    let incoming_provider_id = payload.provider.provider_id.clone();

    surface_registry.register_service(service_id, app_name, service_tenant_id, payload)?;

    if let Some(previous_provider_id) = previous_provider_id
        && previous_provider_id != incoming_provider_id
    {
        surface_proxy.fail_in_flight_for_provider(&previous_provider_id);
    }

    Ok(())
}

pub(super) fn surface_registration_error_message(
    error: &crate::surface_registry::SurfaceRegistryError,
) -> String {
    match error {
        crate::surface_registry::SurfaceRegistryError::ProviderRejected(rejection) => {
            serde_json::to_string(rejection).unwrap_or_else(|_| error.to_string())
        }
        crate::surface_registry::SurfaceRegistryError::ProviderConflict(_) => error.to_string(),
        _ => {
            tracing::warn!(
                ?error,
                "unhandled SurfaceRegistryError variant in error message"
            );
            error.to_string()
        }
    }
}

pub(super) fn surface_proxy_error_to_wire(
    error: crate::surface_proxy::SurfaceProxyError,
) -> uptrakit_wire::surfaces::SurfaceActionError {
    let (code, message) = match error {
        crate::surface_proxy::SurfaceProxyError::NoProvider => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "no provider available for requested surface interaction".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::TargetProviderRequired => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "target_provider_id is required for targeted surface interactions".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::InvalidProvider(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::PermissionDenied(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::Conflict { message, .. } => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::SchemaValidationFailed(message)
        | crate::surface_proxy::SurfaceProxyError::SensitiveFieldRejected(message) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            message,
        ),
        crate::surface_proxy::SurfaceProxyError::InteractionNotFound => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "interaction not found".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::DuplicateRequest => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::DuplicateRequest,
            "duplicate idempotency key".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::RateLimited => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "surface provider is temporarily rate-limited".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::ServiceDisconnected
        | crate::surface_proxy::SurfaceProxyError::SendFailed => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "surface provider is disconnected".to_string(),
        ),
        crate::surface_proxy::SurfaceProxyError::Timeout => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::Timeout,
            "surface action timed out".to_string(),
        ),
        unknown => {
            tracing::warn!(
                ?unknown,
                "unhandled SurfaceProxyError variant in wire conversion"
            );
            (
                uptrakit_wire::surfaces::SurfaceActionErrorCode::InternalError,
                "surface proxy error".to_string(),
            )
        }
    };

    uptrakit_wire::surfaces::SurfaceActionError {
        code,
        message,
        details: None,
    }
}

pub(super) fn surface_registry_lookup_error_to_wire(
    error: crate::surface_registry::SurfaceRegistryLookupError,
) -> uptrakit_wire::surfaces::SurfaceActionError {
    let (code, message) = match error {
        crate::surface_registry::SurfaceRegistryLookupError::SurfaceNotFound => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "surface not found".to_string(),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::InteractionNotFound => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "interaction not found".to_string(),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::TargetProviderRequired => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            "target_provider_id is required for targeted surface interactions".to_string(),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::InvalidProvider(provider_id) => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
            format!("no surface provider '{provider_id}' is registered for the requested surface"),
        ),
        crate::surface_registry::SurfaceRegistryLookupError::NoTenantCompatibleProvider => (
            uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable,
            "no provider available for requested surface interaction".to_string(),
        ),
        unknown => {
            tracing::warn!(
                ?unknown,
                "unhandled SurfaceRegistryLookupError variant in wire conversion"
            );
            (
                uptrakit_wire::surfaces::SurfaceActionErrorCode::InternalError,
                "surface lookup error".to_string(),
            )
        }
    };

    uptrakit_wire::surfaces::SurfaceActionError {
        code,
        message,
        details: None,
    }
}

pub(super) fn action_error_code(
    code: &uptrakit_wire::surfaces::SurfaceActionErrorCode,
) -> &'static str {
    match code {
        uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied => "permission_denied",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest => "invalid_request",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::SchemaValidationFailed => {
            "schema_validation_failed"
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::UnsupportedCapability => {
            "unsupported_capability"
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::ProviderUnavailable => {
            "provider_unavailable"
        }
        uptrakit_wire::surfaces::SurfaceActionErrorCode::Timeout => "timeout",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::DuplicateRequest => "duplicate_request",
        uptrakit_wire::surfaces::SurfaceActionErrorCode::InternalError => "internal_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_registration_error_message_serializes_structured_rejection_reasons() {
        let message =
            surface_registration_error_message(
                &crate::surface_registry::SurfaceRegistryError::ProviderRejected(
                    crate::surface_registry::SurfaceProviderRejection::new(
                        "provider-a".to_string(),
                        vec![crate::surface_registry::SurfaceProviderRejectionReason::new(
                        crate::surface_registry::SurfaceProviderRejectionCode::InvalidTransport,
                        "invalid transport".to_string(),
                        Some("ssh.guest.panel".to_string()),
                    )],
                    ),
                ),
            );

        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("expected JSON rejection payload");
        assert_eq!(parsed["provider_id"], "provider-a");
        assert_eq!(parsed["reasons"][0]["message"], "invalid transport");
    }
}
