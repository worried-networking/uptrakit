use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ExprTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, sea_query::Expr,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::enrollment_token;
use uptrakit_web_api_types::enrollment_tokens::EnrollmentTokenResponse;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uuid::Uuid;

use crate::tenant_db::TenantDb;

/// Parameters for creating a new enrollment token.
pub struct CreateTokenParams<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub token_hash: &'a str,
    pub allowed_capabilities: Option<&'a [String]>,
    pub max_uses: Option<u32>,
    pub expires_at: Option<OffsetDateTime>,
    pub created_by_user_id: Option<Uuid>,
}

fn model_to_response(m: enrollment_token::Model) -> EnrollmentTokenResponse {
    let allowed_capabilities: Option<Vec<String>> = m
        .allowed_capabilities
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    EnrollmentTokenResponse {
        id: m.id,
        name: m.name,
        allowed_capabilities,
        max_uses: m.max_uses.map(|v| v as u32),
        current_uses: m.current_uses as u32,
        expires_at: m.expires_at,
        created_at: m.created_at,
        revoked_at: m.revoked_at,
        created_by_user_id: m.created_by_user_id,
    }
}

/// Insert a new enrollment token and return the model.
#[tracing::instrument(skip_all)]
pub async fn create_enrollment_token(
    tenant_db: &TenantDb,
    params: CreateTokenParams<'_>,
) -> Result<enrollment_token::Model, sea_orm::DbErr> {
    let caps_json = params
        .allowed_capabilities
        .map(|caps| {
            serde_json::to_string(caps).map_err(|e| {
                sea_orm::DbErr::Custom(format!("capability list serialization failed: {e}"))
            })
        })
        .transpose()?;

    let now = OffsetDateTime::now_utc();
    let model = enrollment_token::ActiveModel {
        id: Set(params.id),
        tenant_id: Set(tenant_db.tenant_id()),
        name: Set(params.name.to_string()),
        token_hash: Set(params.token_hash.to_string()),
        allowed_capabilities: Set(caps_json),
        max_uses: Set(params.max_uses.map(|v| v as i32)),
        current_uses: Set(0),
        expires_at: Set(params.expires_at),
        created_at: Set(now),
        revoked_at: Set(None),
        created_by_user_id: Set(params.created_by_user_id),
    };

    model.insert(tenant_db.db()).await
}

/// List enrollment tokens for this tenant, ordered by `created_at` desc.
#[tracing::instrument(skip_all)]
pub async fn list_enrollment_tokens(
    tenant_db: &TenantDb,
    pagination: &uptrakit_web_api_types::pagination::PaginationParams,
) -> Result<PaginatedResponse<EnrollmentTokenResponse>, sea_orm::DbErr> {
    let pagination = pagination.resolve();

    let base_query = tenant_db
        .find::<enrollment_token::Entity>()
        .order_by_desc(enrollment_token::Column::CreatedAt);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let tokens = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    let items: Vec<EnrollmentTokenResponse> = tokens.into_iter().map(model_to_response).collect();
    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Get a single enrollment token by ID.
#[tracing::instrument(skip_all)]
pub async fn get_enrollment_token(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<EnrollmentTokenResponse>, sea_orm::DbErr> {
    let token = tenant_db
        .find_by_id::<enrollment_token::Entity, _>(id)
        .one(tenant_db.db())
        .await?;

    Ok(token.map(model_to_response))
}

/// Soft-revoke a token by setting `revoked_at = now()`.
/// Returns `true` if a token was found and revoked.
#[tracing::instrument(skip_all)]
pub async fn revoke_enrollment_token(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let result = tenant_db
        .update_many::<enrollment_token::Entity>()
        .col_expr(
            enrollment_token::Column::RevokedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .filter(enrollment_token::Column::Id.eq(id))
        .filter(enrollment_token::Column::RevokedAt.is_null())
        .exec(tenant_db.db())
        .await?;

    Ok(result.rows_affected > 0)
}

/// Find all active tokens for a tenant: not revoked, not expired, uses remaining.
/// Used during enrollment to verify a provided token against all active tokens.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_active_tokens(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<enrollment_token::Model>, sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();

    enrollment_token::Entity::find()
        .filter(enrollment_token::Column::TenantId.eq(tenant_id))
        .filter(enrollment_token::Column::RevokedAt.is_null())
        .filter(
            enrollment_token::Column::ExpiresAt
                .is_null()
                .or(enrollment_token::Column::ExpiresAt.gt(now)),
        )
        // Only tokens that have uses remaining (max_uses is NULL = unlimited, or current_uses < max_uses)
        .filter(
            enrollment_token::Column::MaxUses
                .is_null()
                .or(Expr::col(enrollment_token::Column::CurrentUses)
                    .lt(Expr::col(enrollment_token::Column::MaxUses))),
        )
        .all(db)
        .await
}

/// Atomically increment `current_uses` on a token.
#[tracing::instrument(skip_all, fields(%token_id))]
pub async fn increment_token_uses(
    db: &sea_orm::DatabaseConnection,
    token_id: Uuid,
) -> Result<(), sea_orm::DbErr> {
    enrollment_token::Entity::update_many()
        .col_expr(
            enrollment_token::Column::CurrentUses,
            Expr::col(enrollment_token::Column::CurrentUses).add(1),
        )
        .filter(enrollment_token::Column::Id.eq(token_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Count active tokens for a tenant (for the combined settings summary).
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn count_active_tokens(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<u32, sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();

    let count = enrollment_token::Entity::find()
        .filter(enrollment_token::Column::TenantId.eq(tenant_id))
        .filter(enrollment_token::Column::RevokedAt.is_null())
        .filter(
            enrollment_token::Column::ExpiresAt
                .is_null()
                .or(enrollment_token::Column::ExpiresAt.gt(now)),
        )
        .filter(
            enrollment_token::Column::MaxUses
                .is_null()
                .or(Expr::col(enrollment_token::Column::CurrentUses)
                    .lt(Expr::col(enrollment_token::Column::MaxUses))),
        )
        .count(db)
        .await?;

    Ok(count as u32)
}
