use crate::AppState;
use crate::middleware::tenant_context::TenantContext;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use std::sync::Arc;
use uptrakit_web_api_queries::TenantDb as TenantDbInner;

/// Axum extractor wrapping [`TenantDbInner`] with `FromRequestParts`.
///
/// Route handlers keep `tenant_db: TenantDb` in signatures — auto-deref
/// handles method calls (`tenant_db.find::<E>()`), field access
/// (`tenant_db.tenant_id`), and passing `&tenant_db` to query functions
/// expecting `&TenantDbInner`.
pub struct TenantDb(pub TenantDbInner);

impl TenantDb {
    /// Construct a `TenantDb` directly from its components, for use in unit tests
    /// that cannot go through the Axum extractor machinery.
    #[cfg(test)]
    pub fn new_for_test(db: sea_orm::DatabaseConnection, tenant_id: uuid::Uuid) -> Self {
        Self(TenantDbInner::new(db, tenant_id))
    }
}

impl std::ops::Deref for TenantDb {
    type Target = TenantDbInner;

    fn deref(&self) -> &Self::Target {
        &self.0
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
        Ok(Self(TenantDbInner::new(
            state.db().clone(),
            tenant.tenant_id,
        )))
    }
}
