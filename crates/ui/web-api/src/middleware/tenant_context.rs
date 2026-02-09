use std::sync::Arc;

use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use uptrakit_web_api_types::error::ErrorResponse;
use uuid::Uuid;

use crate::AppState;

/// Resolved tenant for the current request.
///
/// Always returns the default tenant. Multi-tenancy is future work.
///
/// TODO: When multi-tenancy is enabled, re-add X-Tenant-Id header processing
/// with these requirements:
/// 1. Only accept X-Tenant-Id from authenticated users
/// 2. Verify the user has access to the requested tenant via a user_tenant
///    mapping table
/// 3. Reject with 403 if the user doesn't have access
/// 4. Strip X-Tenant-Id header from non-proxy requests (already done in
///    resolve_proxy_headers.rs)
#[derive(Clone, Debug)]
pub struct TenantContext {
    pub tenant_id: Uuid,
}

impl FromRequestParts<Arc<AppState>> for TenantContext {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        Ok(TenantContext {
            tenant_id: state.default_tenant_id,
        })
    }
}
