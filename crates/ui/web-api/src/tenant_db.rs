use crate::AppState;
use crate::middleware::tenant_context::TenantContext;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use uuid::Uuid;

/// Combined database connection + tenant identity extractor.
///
/// Route handlers that deal with tenant-scoped data should use this extractor
/// instead of accessing `state.db` directly. Tenant-agnostic routes (e.g. PKI,
/// system health) should continue to use `State(state)` and call `state.db()`.
pub struct TenantDb {
    db: DatabaseConnection,
    pub tenant_id: Uuid,
}

impl TenantDb {
    /// Returns a reference to the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Construct a `TenantDb` directly from its components, for use in unit tests
    /// that cannot go through the Axum extractor machinery.
    #[cfg(test)]
    pub fn new_for_test(db: DatabaseConnection, tenant_id: Uuid) -> Self {
        Self { db, tenant_id }
    }
}

impl FromRequestParts<Arc<AppState>> for TenantDb {
    type Rejection = axum::response::Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let tenant = TenantContext::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self {
            db: state.db().clone(),
            tenant_id: tenant.tenant_id,
        })
    }
}
