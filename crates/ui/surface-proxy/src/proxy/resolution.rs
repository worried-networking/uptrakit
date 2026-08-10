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
            SurfaceProxyError::InvalidProvider(format!(
                "no surface provider '{provider_id}' is registered for the requested surface"
            ))
        }
        SurfaceRegistryLookupError::NoTenantCompatibleProvider => SurfaceProxyError::NoProvider,
        SurfaceRegistryLookupError::MethodNotAllowed { allowed, .. } => {
            SurfaceProxyError::SchemaValidationFailed(format!(
                "method not allowed for this interaction; allowed methods: [{}]",
                crate::registry::format_allowed_methods(&allowed)
            ))
        }
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
    visibility: &dyn crate::registry::SurfaceProviderVisibility,
) -> Result<Option<String>, SurfaceProxyError> {
    if request.target_provider_id.is_some() {
        return Ok(request.target_provider_id.clone());
    }

    // Enumerate the requested surface's providers once, then share the list across
    // the self-target check and the availability-aware fallthrough. The fallthrough
    // runs on every nested provider→plugin call, so re-enumerating (and re-locking the
    // registry) per branch would be wasteful.
    let providers = registry.list_targeted_providers_for_surface(
        request.surface_id.as_str(),
        request.tenant_id,
        visibility,
    );

    // A provider-origin call targets the requested surface's provider, which is not
    // necessarily the caller's own: nested provider→controller-plugin calls (agent-ssh
    // → the controller-side `proxmox.hosts` plugin) target a different provider. Prefer
    // self-target only when the caller itself provides the requested surface — a service
    // invoking its own surface names the provider unambiguously, so this branch returns
    // it even for a `Targeted` surface, where the fallthrough would raise
    // `TargetProviderRequired`. Non-self provider-origin callers fall through to the same
    // guard as user/builtin origins.
    if let SurfaceCallerOrigin::Provider { service_id } = &request.caller_origin
        && let Some(own) = registry.provider_id_for_service(service_id)
        && providers.iter().any(|provider| provider.provider_id == own)
    {
        return Ok(Some(own));
    }

    select_available_provider_for_surface(service_connections, providers).await
}

async fn select_available_provider_for_surface(
    service_connections: &ServiceConnectionRegistry,
    providers: Vec<crate::registry::SurfaceProviderSummary>,
) -> Result<Option<String>, SurfaceProxyError> {
    let mut available = Vec::new();
    for provider in providers {
        if !provider.tenant_compatible {
            continue;
        }
        if provider_is_available(service_connections, &provider).await {
            available.push(provider);
        }
    }

    if available.is_empty() {
        return Ok(None);
    }

    if available
        .iter()
        .any(|provider| provider.targeting == surfaces::Targeting::Targeted)
    {
        return Err(SurfaceProxyError::TargetProviderRequired);
    }

    Ok(available
        .first()
        .map(|provider| provider.provider_id.clone()))
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
