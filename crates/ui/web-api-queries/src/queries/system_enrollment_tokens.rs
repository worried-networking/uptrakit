use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ExprTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, sea_query::Expr,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::system_enrollment_token;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::system_enrollment_tokens::SystemEnrollmentTokenResponse;
use uuid::Uuid;

/// Parameters for creating a new system enrollment token.
pub struct CreateSystemTokenParams<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub token_hash: &'a str,
    pub max_uses: Option<u32>,
    pub expires_at: Option<OffsetDateTime>,
    pub created_by_user_id: Option<Uuid>,
}

fn model_to_response(m: system_enrollment_token::Model) -> SystemEnrollmentTokenResponse {
    SystemEnrollmentTokenResponse {
        id: m.id,
        name: m.name,
        max_uses: m.max_uses.map(|v| v as u32),
        current_uses: m.current_uses as u32,
        expires_at: m.expires_at,
        created_at: m.created_at,
        revoked_at: m.revoked_at,
        created_by_user_id: m.created_by_user_id,
    }
}

/// Insert a new system enrollment token and return the model.
#[tracing::instrument(skip_all)]
pub async fn create_system_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    params: CreateSystemTokenParams<'_>,
) -> Result<system_enrollment_token::Model, sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();
    let model = system_enrollment_token::ActiveModel {
        id: Set(params.id),
        name: Set(params.name.to_string()),
        token_hash: Set(params.token_hash.to_string()),
        max_uses: Set(params.max_uses.map(|v| v as i32)),
        current_uses: Set(0),
        expires_at: Set(params.expires_at),
        created_at: Set(now),
        revoked_at: Set(None),
        created_by_user_id: Set(params.created_by_user_id),
    };

    model.insert(db).await
}

/// List all system enrollment tokens, ordered by `created_at` desc.
#[tracing::instrument(skip_all)]
pub async fn list_system_enrollment_tokens(
    db: &sea_orm::DatabaseConnection,
    pagination: &uptrakit_web_api_types::pagination::PaginationParams,
) -> Result<PaginatedResponse<SystemEnrollmentTokenResponse>, sea_orm::DbErr> {
    let pagination = pagination.resolve();

    let base_query = system_enrollment_token::Entity::find()
        .order_by_desc(system_enrollment_token::Column::CreatedAt);

    let total = base_query.clone().count(db).await?;

    let tokens = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(db)
        .await?;

    let items: Vec<SystemEnrollmentTokenResponse> =
        tokens.into_iter().map(model_to_response).collect();
    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Get a single system enrollment token by ID.
#[tracing::instrument(skip_all)]
pub async fn get_system_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    id: Uuid,
) -> Result<Option<SystemEnrollmentTokenResponse>, sea_orm::DbErr> {
    let token = system_enrollment_token::Entity::find_by_id(id)
        .one(db)
        .await?;

    Ok(token.map(model_to_response))
}

/// Soft-revoke a system enrollment token by setting `revoked_at = now()`.
/// Returns `true` if a token was found and revoked.
#[tracing::instrument(skip_all)]
pub async fn revoke_system_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let result = system_enrollment_token::Entity::update_many()
        .col_expr(
            system_enrollment_token::Column::RevokedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .filter(system_enrollment_token::Column::Id.eq(id))
        .filter(system_enrollment_token::Column::RevokedAt.is_null())
        .exec(db)
        .await?;

    Ok(result.rows_affected > 0)
}

/// Find all active system enrollment tokens: not revoked, not expired, uses remaining.
/// Used during system service enrollment to verify a provided token.
#[tracing::instrument(skip_all)]
pub async fn find_active_system_tokens(
    db: &sea_orm::DatabaseConnection,
) -> Result<Vec<system_enrollment_token::Model>, sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();

    system_enrollment_token::Entity::find()
        .filter(system_enrollment_token::Column::RevokedAt.is_null())
        .filter(
            system_enrollment_token::Column::ExpiresAt
                .is_null()
                .or(system_enrollment_token::Column::ExpiresAt.gt(now)),
        )
        // Only tokens with uses remaining (max_uses NULL = unlimited, or current_uses < max_uses)
        .filter(
            system_enrollment_token::Column::MaxUses
                .is_null()
                .or(Expr::col(system_enrollment_token::Column::CurrentUses)
                    .lt(Expr::col(system_enrollment_token::Column::MaxUses))),
        )
        .all(db)
        .await
}

/// Atomically increment `current_uses` on a system enrollment token.
#[tracing::instrument(skip_all, fields(%token_id))]
pub async fn increment_system_token_uses(
    db: &sea_orm::DatabaseConnection,
    token_id: Uuid,
) -> Result<(), sea_orm::DbErr> {
    system_enrollment_token::Entity::update_many()
        .col_expr(
            system_enrollment_token::Column::CurrentUses,
            Expr::col(system_enrollment_token::Column::CurrentUses).add(1),
        )
        .filter(system_enrollment_token::Column::Id.eq(token_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Count active system enrollment tokens (for the global settings summary).
#[tracing::instrument(skip_all)]
pub async fn count_active_system_tokens(
    db: &sea_orm::DatabaseConnection,
) -> Result<u32, sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();

    let count = system_enrollment_token::Entity::find()
        .filter(system_enrollment_token::Column::RevokedAt.is_null())
        .filter(
            system_enrollment_token::Column::ExpiresAt
                .is_null()
                .or(system_enrollment_token::Column::ExpiresAt.gt(now)),
        )
        .filter(
            system_enrollment_token::Column::MaxUses
                .is_null()
                .or(Expr::col(system_enrollment_token::Column::CurrentUses)
                    .lt(Expr::col(system_enrollment_token::Column::MaxUses))),
        )
        .count(db)
        .await?;

    Ok(count as u32)
}
