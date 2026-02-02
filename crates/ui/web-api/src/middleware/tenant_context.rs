use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::AppState;

/// Resolved tenant for the current request.
///
/// In single-tenant mode the default tenant is always used. When multi-tenancy
/// is enabled the `X-Tenant-Id` header selects the tenant (future work).
#[derive(Clone, Debug)]
pub struct TenantContext {
    pub tenant_id: Uuid,
}

impl FromRequestParts<Arc<AppState>> for TenantContext {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        // Check for X-Tenant-Id header
        if let Some(header_val) = parts.headers.get("x-tenant-id") {
            let header_str = header_val.to_str().map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "X-Tenant-Id header is not valid UTF-8",
                )
            })?;

            if !header_str.is_empty() {
                let tenant_id = header_str.parse::<Uuid>().map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "X-Tenant-Id header is not a valid UUID",
                    )
                })?;
                return Ok(TenantContext { tenant_id });
            }
        }

        // Fallback to default tenant
        Ok(TenantContext {
            tenant_id: state.default_tenant_id,
        })
    }
}
