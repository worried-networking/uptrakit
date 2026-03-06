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

#[cfg(test)]
mod tests {
    use sea_orm::{Database, DatabaseConnection};
    use time::OffsetDateTime;
    use uuid::Uuid;

    use super::*;

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    fn make_token_params<'a>(id: Uuid, name: &'a str) -> CreateSystemTokenParams<'a> {
        CreateSystemTokenParams {
            id,
            name,
            token_hash: "testhash",
            max_uses: None,
            expires_at: None,
            created_by_user_id: None,
        }
    }

    #[tokio::test]
    async fn expired_token_excluded_from_active() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        let past = OffsetDateTime::now_utc() - time::Duration::hours(1);
        create_system_enrollment_token(
            &db,
            CreateSystemTokenParams {
                id,
                name: "expired",
                token_hash: "h1",
                max_uses: None,
                expires_at: Some(past),
                created_by_user_id: None,
            },
        )
        .await
        .unwrap();

        let active = find_active_system_tokens(&db).await.unwrap();
        assert!(
            active.iter().all(|t| t.id != id),
            "expired token must not appear in active list"
        );
    }

    #[tokio::test]
    async fn revoked_token_excluded_from_active() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        create_system_enrollment_token(&db, make_token_params(id, "revokable"))
            .await
            .unwrap();

        let revoked = revoke_system_enrollment_token(&db, id).await.unwrap();
        assert!(revoked, "first revocation must return true");

        let active = find_active_system_tokens(&db).await.unwrap();
        assert!(
            active.iter().all(|t| t.id != id),
            "revoked token must not appear in active list"
        );
    }

    #[tokio::test]
    async fn exhausted_token_excluded_from_active() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        create_system_enrollment_token(
            &db,
            CreateSystemTokenParams {
                id,
                name: "limited",
                token_hash: "h2",
                max_uses: Some(1),
                expires_at: None,
                created_by_user_id: None,
            },
        )
        .await
        .unwrap();

        // Increment uses to reach max_uses.
        increment_system_token_uses(&db, id).await.unwrap();

        let active = find_active_system_tokens(&db).await.unwrap();
        assert!(
            active.iter().all(|t| t.id != id),
            "exhausted token must not appear in active list"
        );
    }

    #[tokio::test]
    async fn unlimited_token_always_included() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        create_system_enrollment_token(&db, make_token_params(id, "unlimited"))
            .await
            .unwrap();

        let active = find_active_system_tokens(&db).await.unwrap();
        assert!(
            active.iter().any(|t| t.id == id),
            "unlimited token must appear in active list"
        );
    }

    #[tokio::test]
    async fn token_below_max_uses_is_included() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        create_system_enrollment_token(
            &db,
            CreateSystemTokenParams {
                id,
                name: "partially-used",
                token_hash: "h3",
                max_uses: Some(3),
                expires_at: None,
                created_by_user_id: None,
            },
        )
        .await
        .unwrap();

        // Use it once — still has 2 remaining.
        increment_system_token_uses(&db, id).await.unwrap();

        let active = find_active_system_tokens(&db).await.unwrap();
        assert!(
            active.iter().any(|t| t.id == id),
            "token with uses remaining must appear in active list"
        );
    }

    #[tokio::test]
    async fn revoke_is_idempotent() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        create_system_enrollment_token(&db, make_token_params(id, "idempotent"))
            .await
            .unwrap();

        let first = revoke_system_enrollment_token(&db, id).await.unwrap();
        assert!(first, "first revocation must return true");

        let second = revoke_system_enrollment_token(&db, id).await.unwrap();
        assert!(
            !second,
            "second revocation of already-revoked token must return false"
        );
    }

    #[tokio::test]
    async fn non_expiring_token_included_when_expires_at_is_null() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        // expires_at = None → never expires
        create_system_enrollment_token(
            &db,
            CreateSystemTokenParams {
                id,
                name: "no-expiry",
                token_hash: "h4",
                max_uses: None,
                expires_at: None,
                created_by_user_id: None,
            },
        )
        .await
        .unwrap();

        let active = find_active_system_tokens(&db).await.unwrap();
        assert!(
            active.iter().any(|t| t.id == id),
            "token with no expiry must appear in active list"
        );
    }
}
