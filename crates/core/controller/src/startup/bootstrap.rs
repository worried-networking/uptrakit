//! Phase 7: OIDC and enrollment token bootstrap.

use rootcause::prelude::*;

use crate::AppError;

/// Bootstrap an OIDC provider from CLI flags if all required flags are present.
pub(crate) async fn bootstrap_oidc(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    args: &crate::cli::Args,
) -> crate::Result<()> {
    let oidc = &args.oidc_bootstrap;
    let any_set = oidc.oidc_issuer_url.is_some()
        || oidc.oidc_client_id.is_some()
        || oidc.oidc_client_secret.is_some();

    if !any_set {
        return Ok(());
    }

    let issuer_url = oidc.oidc_issuer_url.as_deref().ok_or_else(|| {
        report!(AppError::Config(
            "--oidc-issuer-url is required when any OIDC bootstrap flag is set".into()
        ))
    })?;
    let client_id = oidc.oidc_client_id.as_deref().ok_or_else(|| {
        report!(AppError::Config(
            "--oidc-client-id is required with --oidc-issuer-url".into()
        ))
    })?;
    let client_secret = oidc.oidc_client_secret.as_deref().ok_or_else(|| {
        report!(AppError::Config(
            "--oidc-client-secret is required with --oidc-issuer-url".into()
        ))
    })?;

    let slug = oidc.oidc_provider_slug.as_deref().unwrap_or("sso");
    let name = oidc.oidc_provider_name.as_deref().unwrap_or("SSO");
    let scopes = oidc
        .oidc_scopes
        .as_deref()
        .unwrap_or("openid email profile groups");

    let force = args.force_settings_override;

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::{oidc_provider, prelude::OidcProvider};

    let existing = OidcProvider::find()
        .filter(oidc_provider::Column::Slug.eq(slug))
        .filter(oidc_provider::Column::TenantId.eq(tenant_id))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context(AppError::Database)?;

    match existing {
        None => {
            use sea_orm::ActiveModelTrait;
            use sea_orm::Set;
            use time::OffsetDateTime;

            let now = OffsetDateTime::now_utc();
            let encrypted_secret = uptrakit_crypto::EncryptedString::new(
                client_secret.to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .context_to()?;
            let provider = oidc_provider::ActiveModel {
                id: Set(uuid::Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                name: Set(name.to_string()),
                slug: Set(slug.to_string()),
                logo_url: Set(None),
                issuer_url: Set(issuer_url.to_string()),
                client_id: Set(client_id.to_string()),
                client_secret: Set(encrypted_secret),
                scopes: Set(scopes.to_string()),
                auto_create_users: Set(true),
                role_claim_path: Set(None),
                role_mapping: Set(
                    uptrakit_shared_db::entity::oidc_provider::RoleMapping::default(),
                ),
                is_active: Set(true),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
            };
            provider.insert(db).await.context(AppError::Database)?;
            tracing::info!(slug = slug, name = name, "bootstrapped OIDC provider");
        }
        Some(existing_provider) if force => {
            use sea_orm::Set;
            use sea_orm::{ActiveModelTrait, IntoActiveModel};
            use time::OffsetDateTime;

            let encrypted_secret = uptrakit_crypto::EncryptedString::new(
                client_secret.to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .context_to()?;
            let mut model = existing_provider.into_active_model();
            model.issuer_url = Set(issuer_url.to_string());
            model.client_id = Set(client_id.to_string());
            model.client_secret = Set(encrypted_secret);
            model.is_active = Set(true);
            model.updated_at = Set(OffsetDateTime::now_utc());
            model.update(db).await.context(AppError::Database)?;
            tracing::info!(
                slug = slug,
                name = name,
                "force-updated bootstrapped OIDC provider"
            );
        }
        Some(_) => {
            tracing::info!(
                slug = slug,
                "OIDC provider already exists, skipping bootstrap \
                 (pass --force-settings-override to overwrite)"
            );
        }
    }
    Ok(())
}

/// Bootstrap enrollment tokens from CLI flags / environment variables.
///
/// Creates pre-hashed enrollment tokens named "bootstrap" at startup so that
/// services can auto-enroll using a shared secret (e.g. in docker-compose).
/// Idempotent: skips creation if an active token named "bootstrap" already
/// exists (not revoked, not expired, uses remaining).
pub(crate) async fn bootstrap_enrollment_tokens(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    args: &crate::cli::Args,
) -> crate::Result<()> {
    let eb = &args.enrollment_bootstrap;

    // Tenant enrollment token
    if let Some(ref token_value) = eb.bootstrap_enrollment_token {
        bootstrap_tenant_enrollment_token(db, tenant_id, token_value, eb).await?;
    }

    // System enrollment token
    if let Some(ref token_value) = eb.bootstrap_system_enrollment_token {
        bootstrap_system_enrollment_token(db, token_value, eb).await?;
    }

    Ok(())
}

/// Create a tenant enrollment token named "bootstrap" if none exists.
async fn bootstrap_tenant_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    token_value: &str,
    eb: &crate::cli::EnrollmentBootstrapArgs,
) -> crate::Result<()> {
    use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, QueryFilter, sea_query::Expr};
    use uptrakit_shared_db::entity::enrollment_token;

    let now = time::OffsetDateTime::now_utc();

    // Check for an existing active token named "bootstrap"
    let existing = enrollment_token::Entity::find()
        .filter(enrollment_token::Column::TenantId.eq(tenant_id))
        .filter(enrollment_token::Column::Name.eq("bootstrap"))
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
        .one(db)
        .await
        .context(AppError::Database)?;

    if existing.is_some() {
        tracing::info!("bootstrap enrollment token already exists, skipping");
        return Ok(());
    }

    let hash = uptrakit_web_api::auth::password::hash_password(token_value).map_err(|e| {
        report!(AppError::Config(format!(
            "failed to hash bootstrap enrollment token: {e}"
        )))
    })?;

    let expires_at = now + time::Duration::seconds(eb.bootstrap_enrollment_token_ttl as i64);

    use sea_orm::{ActiveModelTrait, Set};

    let model = enrollment_token::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        name: Set("bootstrap".to_string()),
        token_hash: Set(hash.expose_secret().to_string()),
        allowed_capabilities: Set(None),
        max_uses: Set(Some(eb.bootstrap_enrollment_token_max_uses as i32)),
        current_uses: Set(0),
        expires_at: Set(Some(expires_at)),
        created_at: Set(now),
        revoked_at: Set(None),
        created_by_user_id: Set(None),
    };
    model.insert(db).await.context(AppError::Database)?;

    tracing::info!(
        max_uses = eb.bootstrap_enrollment_token_max_uses,
        ttl_secs = eb.bootstrap_enrollment_token_ttl,
        "bootstrapped tenant enrollment token"
    );
    Ok(())
}

/// Create a system enrollment token named "bootstrap" if none exists.
async fn bootstrap_system_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    token_value: &str,
    eb: &crate::cli::EnrollmentBootstrapArgs,
) -> crate::Result<()> {
    use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, QueryFilter, sea_query::Expr};
    use uptrakit_shared_db::entity::system_enrollment_token;

    let now = time::OffsetDateTime::now_utc();

    // Check for an existing active token named "bootstrap"
    let existing = system_enrollment_token::Entity::find()
        .filter(system_enrollment_token::Column::Name.eq("bootstrap"))
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
        .one(db)
        .await
        .context(AppError::Database)?;

    if existing.is_some() {
        tracing::info!("bootstrap system enrollment token already exists, skipping");
        return Ok(());
    }

    let hash = uptrakit_web_api::auth::password::hash_password(token_value).map_err(|e| {
        report!(AppError::Config(format!(
            "failed to hash bootstrap system enrollment token: {e}"
        )))
    })?;

    let expires_at = now + time::Duration::seconds(eb.bootstrap_system_enrollment_token_ttl as i64);

    use sea_orm::{ActiveModelTrait, Set};

    let model = system_enrollment_token::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        name: Set("bootstrap".to_string()),
        token_hash: Set(hash.expose_secret().to_string()),
        max_uses: Set(Some(eb.bootstrap_system_enrollment_token_max_uses as i32)),
        current_uses: Set(0),
        expires_at: Set(Some(expires_at)),
        created_at: Set(now),
        revoked_at: Set(None),
        created_by_user_id: Set(None),
    };
    model.insert(db).await.context(AppError::Database)?;

    tracing::info!(
        max_uses = eb.bootstrap_system_enrollment_token_max_uses,
        ttl_secs = eb.bootstrap_system_enrollment_token_ttl,
        "bootstrapped system enrollment token"
    );
    Ok(())
}
