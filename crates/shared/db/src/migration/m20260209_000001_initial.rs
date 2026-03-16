use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use uuid::Uuid;

use super::helpers::{timestamp, timestamp_null};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();

        // ============================================================
        // 1. Foundational tables (no FK dependencies)
        // ============================================================

        // --- tenants ---
        manager
            .create_table(
                Table::create()
                    .table(Tenants::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Tenants::Id).uuid().not_null().primary_key())
                    .col(string(Tenants::Name))
                    .col(string_uniq(Tenants::Slug))
                    .col(boolean(Tenants::IsDefault).default(false))
                    .col(timestamp(Tenants::CreatedAt))
                    .col(timestamp(Tenants::UpdatedAt))
                    .col(timestamp_null(Tenants::DeactivatedAt))
                    .to_owned(),
            )
            .await?;

        // Note: idx_tenants_slug is intentionally absent — string_uniq() already
        // creates a UNIQUE constraint which implies an index on all supported backends.

        manager
            .create_index(
                Index::create()
                    .name("idx_tenants_deactivated_at")
                    .table(Tenants::Table)
                    .col(Tenants::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // Seed default tenant
        manager
            .exec_stmt(
                Query::insert()
                    .into_table(Tenants::Table)
                    .columns([
                        Tenants::Id,
                        Tenants::Name,
                        Tenants::Slug,
                        Tenants::IsDefault,
                        Tenants::CreatedAt,
                        Tenants::UpdatedAt,
                    ])
                    .values_panic([
                        Uuid::now_v7().into(),
                        "Default".into(),
                        "default".into(),
                        true.into(),
                        now.into(),
                        now.into(),
                    ])
                    .to_owned(),
            )
            .await?;

        // --- users ---
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Users::Id).uuid().not_null().primary_key())
                    .col(string_uniq(Users::Email))
                    .col(string(Users::FirstName))
                    .col(string(Users::LastName))
                    .col(string_null(Users::PasswordHash))
                    .col(boolean(Users::IsActive).default(true))
                    .col(timestamp_null(Users::DeactivatedAt))
                    .col(timestamp(Users::CreatedAt))
                    .col(timestamp(Users::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        // Note: idx_users_email is intentionally absent — string_uniq() already
        // creates a UNIQUE constraint which implies an index on all supported backends.

        // --- ca_certificates ---
        manager
            .create_table(
                Table::create()
                    .table(CaCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CaCertificates::Fingerprint)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(CaCertificates::CertPem).text().not_null())
                    .col(ColumnDef::new(CaCertificates::KeyPem).text().not_null())
                    .col(
                        ColumnDef::new(CaCertificates::NotBefore)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CaCertificates::NotAfter)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CaCertificates::ActivatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(CaCertificates::DeactivatedAt).timestamp_with_time_zone())
                    .col(
                        ColumnDef::new(CaCertificates::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_ca_certificates_not_after")
                    .table(CaCertificates::Table)
                    .col(CaCertificates::NotAfter)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_ca_certificates_deactivated_at")
                    .table(CaCertificates::Table)
                    .col(CaCertificates::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 2. RBAC tables
        // ============================================================

        // --- roles ---
        manager
            .create_table(
                Table::create()
                    .table(Roles::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Roles::Id).uuid().not_null().primary_key())
                    .col(string_uniq(Roles::Name))
                    .col(string_null(Roles::Description))
                    .col(timestamp(Roles::CreatedAt))
                    .to_owned(),
            )
            .await?;

        // --- permissions ---
        manager
            .create_table(
                Table::create()
                    .table(Permissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Permissions::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(string_uniq(Permissions::Name))
                    .col(string_null(Permissions::Description))
                    .col(timestamp(Permissions::CreatedAt))
                    .to_owned(),
            )
            .await?;

        // --- role_permissions ---
        manager
            .create_table(
                Table::create()
                    .table(RolePermissions::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(RolePermissions::RoleId).uuid().not_null())
                    .col(
                        ColumnDef::new(RolePermissions::PermissionId)
                            .uuid()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(RolePermissions::RoleId)
                            .col(RolePermissions::PermissionId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(RolePermissions::Table, RolePermissions::RoleId)
                            .to(Roles::Table, Roles::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(RolePermissions::Table, RolePermissions::PermissionId)
                            .to(Permissions::Table, Permissions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // --- user_roles ---
        manager
            .create_table(
                Table::create()
                    .table(UserRoles::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(UserRoles::TenantId).uuid().not_null())
                    .col(ColumnDef::new(UserRoles::UserId).uuid().not_null())
                    .col(ColumnDef::new(UserRoles::RoleId).uuid().not_null())
                    .col(timestamp(UserRoles::AssignedAt))
                    .primary_key(
                        Index::create()
                            .col(UserRoles::TenantId)
                            .col(UserRoles::UserId)
                            .col(UserRoles::RoleId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_roles_tenant")
                            .from(UserRoles::Table, UserRoles::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserRoles::Table, UserRoles::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserRoles::Table, UserRoles::RoleId)
                            .to(Roles::Table, Roles::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Seed roles and permissions (final state)
        seed_rbac(manager, now).await?;

        // ============================================================
        // 3. OIDC tables
        // ============================================================

        // --- oidc_providers ---
        manager
            .create_table(
                Table::create()
                    .table(OidcProviders::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OidcProviders::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(OidcProviders::TenantId).uuid().not_null())
                    .col(string(OidcProviders::Name))
                    .col(string(OidcProviders::Slug))
                    .col(string_null(OidcProviders::LogoUrl))
                    .col(string(OidcProviders::IssuerUrl))
                    .col(string(OidcProviders::ClientId))
                    .col(string(OidcProviders::ClientSecret))
                    .col(string(OidcProviders::Scopes))
                    .col(boolean(OidcProviders::AutoCreateUsers))
                    .col(string_null(OidcProviders::RoleClaimPath))
                    .col(ColumnDef::new(OidcProviders::RoleMapping).json().not_null())
                    .col(boolean(OidcProviders::IsActive))
                    .col(timestamp(OidcProviders::CreatedAt))
                    .col(timestamp(OidcProviders::UpdatedAt))
                    .col(timestamp_null(OidcProviders::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oidc_providers_tenant")
                            .from(OidcProviders::Table, OidcProviders::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oidc_providers_tenant_id")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_oidc_providers_tenant_slug")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::TenantId)
                    .col(OidcProviders::Slug)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oidc_providers_is_active")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::IsActive)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oidc_providers_deactivated_at")
                    .table(OidcProviders::Table)
                    .col(OidcProviders::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // --- user_oidc_links ---
        manager
            .create_table(
                Table::create()
                    .table(UserOidcLinks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserOidcLinks::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserOidcLinks::UserId).uuid().not_null())
                    .col(ColumnDef::new(UserOidcLinks::ProviderId).uuid().not_null())
                    .col(string(UserOidcLinks::OidcSubject))
                    .col(timestamp(UserOidcLinks::LinkedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_oidc_links_user_id")
                            .from(UserOidcLinks::Table, UserOidcLinks::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_oidc_links_provider_id")
                            .from(UserOidcLinks::Table, UserOidcLinks::ProviderId)
                            .to(OidcProviders::Table, OidcProviders::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_oidc_links_provider_subject")
                    .table(UserOidcLinks::Table)
                    .col(UserOidcLinks::ProviderId)
                    .col(UserOidcLinks::OidcSubject)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_oidc_links_user_provider")
                    .table(UserOidcLinks::Table)
                    .col(UserOidcLinks::UserId)
                    .col(UserOidcLinks::ProviderId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_oidc_links_user_id")
                    .table(UserOidcLinks::Table)
                    .col(UserOidcLinks::UserId)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 4. Sessions & tokens (final schema with refresh token changes)
        // ============================================================

        // --- sessions ---
        // Merged from migrations 005 (create) + 009 (alter):
        // - token_hash → refresh_token_hash
        // - added token_type, revoked_at
        // - removed last_activity_at
        {
            let mut sessions_table = Table::create();
            sessions_table
                .table(Sessions::Table)
                .if_not_exists()
                .col(ColumnDef::new(Sessions::Id).uuid().not_null().primary_key())
                .col(ColumnDef::new(Sessions::UserId).uuid().not_null())
                .col(string_uniq(Sessions::RefreshTokenHash))
                .col(string(Sessions::AuthMethod))
                .col(
                    ColumnDef::new(Sessions::TokenType)
                        .string()
                        .not_null()
                        .default("refresh_token"),
                )
                .col(timestamp(Sessions::CreatedAt))
                .col(timestamp(Sessions::ExpiresAt))
                .col(string_null(Sessions::UserAgent))
                .col(string_null(Sessions::IpAddress))
                .col(ColumnDef::new(Sessions::OidcProviderId).uuid().null())
                .col(timestamp_null(Sessions::RevokedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(Sessions::Table, Sessions::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_sessions_oidc_provider_id")
                        .from(Sessions::Table, Sessions::OidcProviderId)
                        .to(OidcProviders::Table, OidcProviders::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                );

            // MariaDB/MySQL do not support CHECK constraints that reference other
            // columns. Application-level validation enforces this invariant on all
            // backends; the CHECK is a defence-in-depth layer for SQLite and PG.
            if manager.get_database_backend() != sea_orm::DbBackend::MySql {
                sessions_table.check(
                    Expr::col(Sessions::AuthMethod)
                        .ne("oidc")
                        .or(Expr::col(Sessions::OidcProviderId).is_not_null()),
                );
            }

            manager.create_table(sessions_table.to_owned()).await?;
        }

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_refresh_token_hash")
                    .table(Sessions::Table)
                    .col(Sessions::RefreshTokenHash)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_user_id")
                    .table(Sessions::Table)
                    .col(Sessions::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_expires_at")
                    .table(Sessions::Table)
                    .col(Sessions::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // --- api_tokens ---
        manager
            .create_table(
                Table::create()
                    .table(ApiTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ApiTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ApiTokens::UserId).uuid().not_null())
                    .col(string(ApiTokens::Name))
                    .col(string_uniq(ApiTokens::TokenHash))
                    .col(timestamp(ApiTokens::CreatedAt))
                    .col(timestamp_null(ApiTokens::LastUsedAt))
                    .col(timestamp_null(ApiTokens::RevokedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(ApiTokens::Table, ApiTokens::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_api_tokens_token_hash")
                    .table(ApiTokens::Table)
                    .col(ApiTokens::TokenHash)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_api_tokens_user_id")
                    .table(ApiTokens::Table)
                    .col(ApiTokens::UserId)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 5. Settings
        // ============================================================

        // --- settings ---
        manager
            .create_table(
                Table::create()
                    .table(Settings::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Settings::TenantId).uuid().not_null())
                    .col(ColumnDef::new(Settings::Key).string().not_null())
                    .col(ColumnDef::new(Settings::Value).json().not_null())
                    .col(timestamp(Settings::UpdatedAt))
                    .primary_key(Index::create().col(Settings::TenantId).col(Settings::Key))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_settings_tenant")
                            .from(Settings::Table, Settings::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // --- settings_version ---
        // Merged from migrations 022 (create) + 023 (add revocation_version)
        manager
            .create_table(
                Table::create()
                    .table(SettingsVersion::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SettingsVersion::TenantId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(big_integer(SettingsVersion::Version).default(0))
                    .col(big_integer(SettingsVersion::GlobalVersion).default(0))
                    .col(big_integer(SettingsVersion::RevocationVersion).default(0))
                    .col(timestamp(SettingsVersion::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_settings_version_tenant")
                            .from(SettingsVersion::Table, SettingsVersion::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Seed settings_version for all existing tenants
        let insert = Query::insert()
            .into_table(SettingsVersion::Table)
            .columns([
                SettingsVersion::TenantId,
                SettingsVersion::Version,
                SettingsVersion::GlobalVersion,
                SettingsVersion::RevocationVersion,
                SettingsVersion::UpdatedAt,
            ])
            .select_from(
                Query::select()
                    .column(Tenants::Id)
                    .expr(Expr::val(0i32))
                    .expr(Expr::val(0i32))
                    .expr(Expr::val(0i32))
                    .expr(Expr::current_timestamp())
                    .from(Tenants::Table)
                    .to_owned(),
            )
            .map_err(|e| DbErr::Migration(e.to_string()))?
            .to_owned();
        manager.exec_stmt(insert).await?;

        // ============================================================
        // 6. Enrollment tokens & services
        // ============================================================

        // --- enrollment_tokens ---
        manager
            .create_table(
                Table::create()
                    .table(EnrollmentTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EnrollmentTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(EnrollmentTokens::TenantId).uuid().not_null())
                    .col(ColumnDef::new(EnrollmentTokens::Name).text().not_null())
                    .col(
                        ColumnDef::new(EnrollmentTokens::TokenHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(EnrollmentTokens::AllowedCapabilities).text())
                    .col(ColumnDef::new(EnrollmentTokens::MaxUses).integer())
                    .col(integer(EnrollmentTokens::CurrentUses).not_null().default(0))
                    .col(ColumnDef::new(EnrollmentTokens::ExpiresAt).timestamp_with_time_zone())
                    .col(timestamp(EnrollmentTokens::CreatedAt))
                    .col(ColumnDef::new(EnrollmentTokens::RevokedAt).timestamp_with_time_zone())
                    .col(ColumnDef::new(EnrollmentTokens::CreatedByUserId).uuid())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_enrollment_tokens_tenant")
                            .from(EnrollmentTokens::Table, EnrollmentTokens::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_enrollment_tokens_created_by")
                            .from(EnrollmentTokens::Table, EnrollmentTokens::CreatedByUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_enrollment_tokens_tenant_id")
                    .table(EnrollmentTokens::Table)
                    .col(EnrollmentTokens::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_enrollment_tokens_tenant_revoked")
                    .table(EnrollmentTokens::Table)
                    .col(EnrollmentTokens::TenantId)
                    .col(EnrollmentTokens::RevokedAt)
                    .to_owned(),
            )
            .await?;

        // --- services ---
        manager
            .create_table(
                Table::create()
                    .table(Services::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Services::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Services::TenantId).uuid().not_null())
                    .col(
                        ColumnDef::new(Services::Capabilities)
                            .text()
                            .not_null()
                            .default("[]"),
                    )
                    .col(string(Services::Hostname))
                    .col(string(Services::FriendlyName))
                    .col(string_null(Services::IpAddress))
                    .col(
                        ColumnDef::new(Services::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(string_uniq(Services::EnrollmentSecretHash))
                    .col(string_null(Services::ClientVersion))
                    .col(timestamp_null(Services::LastSeenAt))
                    .col(timestamp(Services::CreatedAt))
                    .col(timestamp(Services::UpdatedAt))
                    .col(timestamp_null(Services::DeactivatedAt))
                    .col(
                        ColumnDef::new(Services::PingIntervalSeconds)
                            .integer()
                            .null(),
                    )
                    .col(ColumnDef::new(Services::EnrollmentTokenId).uuid().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_services_tenant")
                            .from(Services::Table, Services::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_services_enrollment_token")
                            .from(Services::Table, Services::EnrollmentTokenId)
                            .to(EnrollmentTokens::Table, EnrollmentTokens::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_services_tenant_id")
                    .table(Services::Table)
                    .col(Services::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_services_enrollment_secret_hash")
                    .table(Services::Table)
                    .col(Services::EnrollmentSecretHash)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_services_status")
                    .table(Services::Table)
                    .col(Services::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_services_deactivated_at")
                    .table(Services::Table)
                    .col(Services::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // --- service_certificates ---
        manager
            .create_table(
                Table::create()
                    .table(ServiceCertificates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ServiceCertificates::CaFingerprint)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ServiceCertificates::SerialNumber)
                            .string()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ServiceCertificates::CaFingerprint)
                            .col(ServiceCertificates::SerialNumber),
                    )
                    .col(
                        ColumnDef::new(ServiceCertificates::ServiceId)
                            .uuid()
                            .not_null(),
                    )
                    .col(timestamp(ServiceCertificates::NotBefore))
                    .col(timestamp(ServiceCertificates::NotAfter))
                    .col(timestamp_null(ServiceCertificates::RevokedAt))
                    .col(string_null(ServiceCertificates::RevocationReason))
                    .col(timestamp(ServiceCertificates::CreatedAt))
                    .col(timestamp_null(ServiceCertificates::LastSeenAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_certificates_service_id")
                            .from(ServiceCertificates::Table, ServiceCertificates::ServiceId)
                            .to(Services::Table, Services::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_certificates_ca_fingerprint")
                            .from(
                                ServiceCertificates::Table,
                                ServiceCertificates::CaFingerprint,
                            )
                            .to(CaCertificates::Table, CaCertificates::Fingerprint)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_service_certificates_service_id_revoked_at")
                    .table(ServiceCertificates::Table)
                    .col(ServiceCertificates::ServiceId)
                    .col(ServiceCertificates::RevokedAt)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 7. Hosts
        // ============================================================

        // --- hosts ---
        manager
            .create_table(
                Table::create()
                    .table(Hosts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Hosts::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Hosts::TenantId).uuid().not_null())
                    .col(string(Hosts::MachineId))
                    .col(string(Hosts::Hostname))
                    .col(string(Hosts::FriendlyName))
                    .col(string_null(Hosts::OsType))
                    .col(string_null(Hosts::OsVersion))
                    .col(string_null(Hosts::Architecture))
                    .col(string_null(Hosts::IpAddress))
                    .col(timestamp_null(Hosts::LastSeenAt))
                    .col(timestamp(Hosts::CreatedAt))
                    .col(timestamp(Hosts::UpdatedAt))
                    .col(timestamp_null(Hosts::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hosts_tenant")
                            .from(Hosts::Table, Hosts::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hosts_tenant_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_hosts_tenant_machine_id")
                    .table(Hosts::Table)
                    .col(Hosts::TenantId)
                    .col(Hosts::MachineId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hosts_deactivated_at")
                    .table(Hosts::Table)
                    .col(Hosts::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // --- service_hosts ---
        manager
            .create_table(
                Table::create()
                    .table(ServiceHosts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ServiceHosts::ServiceId).uuid().not_null())
                    .col(ColumnDef::new(ServiceHosts::HostId).uuid().not_null())
                    .col(timestamp(ServiceHosts::LinkedAt))
                    .primary_key(
                        Index::create()
                            .col(ServiceHosts::ServiceId)
                            .col(ServiceHosts::HostId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_hosts_service")
                            .from(ServiceHosts::Table, ServiceHosts::ServiceId)
                            .to(Services::Table, Services::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_hosts_host")
                            .from(ServiceHosts::Table, ServiceHosts::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 8. Plugins & software items
        // ============================================================

        // --- plugin_configs ---
        manager
            .create_table(
                Table::create()
                    .table(PluginConfigs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PluginConfigs::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(PluginConfigs::TenantId).uuid().not_null())
                    .col(string(PluginConfigs::Name))
                    .col(string(PluginConfigs::PluginType))
                    .col(json(PluginConfigs::Config))
                    .col(boolean(PluginConfigs::Enabled).default(true))
                    .col(timestamp(PluginConfigs::CreatedAt))
                    .col(timestamp(PluginConfigs::UpdatedAt))
                    .col(timestamp_null(PluginConfigs::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_plugin_configs_tenant")
                            .from(PluginConfigs::Table, PluginConfigs::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_plugin_configs_tenant_id")
                    .table(PluginConfigs::Table)
                    .col(PluginConfigs::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_plugin_configs_plugin_type")
                    .table(PluginConfigs::Table)
                    .col(PluginConfigs::PluginType)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_plugin_configs_deactivated_at")
                    .table(PluginConfigs::Table)
                    .col(PluginConfigs::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // Unique active plugin config name per tenant (partial: only non-deactivated rows).
        // Prevents duplicate names for active configs and makes find-or-create idempotent.
        // Note: MySQL does not support partial indexes; the WHERE clause is silently ignored.
        manager
            .create_index(
                Index::create()
                    .name("uq_plugin_configs_active_name")
                    .table(PluginConfigs::Table)
                    .col(PluginConfigs::TenantId)
                    .col(PluginConfigs::Name)
                    .unique()
                    .and_where(Expr::col(PluginConfigs::DeactivatedAt).is_null())
                    .to_owned(),
            )
            .await?;

        // --- software_items ---
        // Plugin coupling lives on host_software_items, not here.
        // Each SoftwareItem is a named catalog entry scoped to a tenant.
        manager
            .create_table(
                Table::create()
                    .table(SoftwareItems::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SoftwareItems::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SoftwareItems::TenantId).uuid().not_null())
                    .col(string(SoftwareItems::Name))
                    .col(boolean(SoftwareItems::Enabled).default(true))
                    .col(string_null(SoftwareItems::DiscoveryState))
                    .col(timestamp_null(SoftwareItems::LastCheckedAt))
                    .col(timestamp(SoftwareItems::CreatedAt))
                    .col(timestamp(SoftwareItems::UpdatedAt))
                    .col(timestamp_null(SoftwareItems::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_software_items_tenant")
                            .from(SoftwareItems::Table, SoftwareItems::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Partial unique index: enforce unique names per tenant among active items.
        // Soft-deleted items are excluded, enabling re-creation with the same name.
        // Note: MySQL does not support partial indexes; the WHERE clause is silently ignored.
        manager
            .create_index(
                Index::create()
                    .name("uq_software_items_active_name")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::TenantId)
                    .col(SoftwareItems::Name)
                    .unique()
                    .and_where(Expr::col(SoftwareItems::DeactivatedAt).is_null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_software_items_tenant_id")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_software_items_deactivated_at")
                    .table(SoftwareItems::Table)
                    .col(SoftwareItems::DeactivatedAt)
                    .to_owned(),
            )
            .await?;

        // --- host_software_items ---
        // Links a host to a software item. Per-host version tracking lives here;
        // plugin coupling (role, plugin_config, package_identifier) is in the
        // separate host_software_item_plugins table.
        manager
            .create_table(
                Table::create()
                    .table(HostSoftwareItems::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(HostSoftwareItems::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(HostSoftwareItems::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string_null(HostSoftwareItems::InstalledVersion))
                    .col(timestamp_null(
                        HostSoftwareItems::InstalledVersionDetectedAt,
                    ))
                    .col(string_null(HostSoftwareItems::LatestVersion))
                    .col(timestamp_null(HostSoftwareItems::LatestVersionFetchedAt))
                    .col(json_null(HostSoftwareItems::LatestReleaseMetadata))
                    .col(timestamp_null(HostSoftwareItems::LastUpdatedAt))
                    .col(timestamp(HostSoftwareItems::LinkedAt))
                    .primary_key(
                        Index::create()
                            .col(HostSoftwareItems::HostId)
                            .col(HostSoftwareItems::SoftwareItemId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_software_items_host")
                            .from(HostSoftwareItems::Table, HostSoftwareItems::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_software_items_software_item")
                            .from(HostSoftwareItems::Table, HostSoftwareItems::SoftwareItemId)
                            .to(SoftwareItems::Table, SoftwareItems::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // --- host_software_item_plugins ---
        // Role-based plugin assignments: each (host, software_item) pair can have
        // one plugin per role (detect_version, fetch_releases, execute_update).
        // The ordinal column enables future multi-instance roles (e.g. hooks).
        manager
            .create_table(
                Table::create()
                    .table(HostSoftwareItemPlugins::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(HostSoftwareItemPlugins::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(HostSoftwareItemPlugins::HostId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HostSoftwareItemPlugins::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HostSoftwareItemPlugins::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(HostSoftwareItemPlugins::Role))
                    .col(
                        ColumnDef::new(HostSoftwareItemPlugins::Ordinal)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(string(HostSoftwareItemPlugins::PackageIdentifier))
                    .col(ColumnDef::new(HostSoftwareItemPlugins::ConfigOverride).json())
                    .col(
                        ColumnDef::new(HostSoftwareItemPlugins::ExecutionSite)
                            .string()
                            .not_null()
                            .default("auto"),
                    )
                    .col(timestamp(HostSoftwareItemPlugins::CreatedAt))
                    .col(timestamp(HostSoftwareItemPlugins::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_hsip_plugin_config")
                            .from(
                                HostSoftwareItemPlugins::Table,
                                HostSoftwareItemPlugins::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        &mut ForeignKey::create()
                            .name("fk_hsip_host_software_item")
                            .from_tbl(HostSoftwareItemPlugins::Table)
                            .from_col(HostSoftwareItemPlugins::HostId)
                            .from_col(HostSoftwareItemPlugins::SoftwareItemId)
                            .to_tbl(HostSoftwareItems::Table)
                            .to_col(HostSoftwareItems::HostId)
                            .to_col(HostSoftwareItems::SoftwareItemId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;

        // One plugin per role per (host, software_item) pair (ordinal for future
        // multi-instance roles).
        manager
            .create_index(
                Index::create()
                    .name("uq_hsip_host_item_role_ordinal")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::HostId)
                    .col(HostSoftwareItemPlugins::SoftwareItemId)
                    .col(HostSoftwareItemPlugins::Role)
                    .col(HostSoftwareItemPlugins::Ordinal)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_plugin_config_id")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::PluginConfigId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_host_item")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::HostId)
                    .col(HostSoftwareItemPlugins::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_hsip_role_exec")
                    .table(HostSoftwareItemPlugins::Table)
                    .col(HostSoftwareItemPlugins::Role)
                    .col(HostSoftwareItemPlugins::ExecutionSite)
                    .to_owned(),
            )
            .await?;

        // --- autodiscovery_ignores ---
        manager
            .create_table(
                Table::create()
                    .table(AutodiscoveryIgnores::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::PluginConfigId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(AutodiscoveryIgnores::PackageIdentifier))
                    .col(timestamp(AutodiscoveryIgnores::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_autodiscovery_ignores_tenant")
                            .from(AutodiscoveryIgnores::Table, AutodiscoveryIgnores::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_autodiscovery_ignores_plugin_config")
                            .from(
                                AutodiscoveryIgnores::Table,
                                AutodiscoveryIgnores::PluginConfigId,
                            )
                            .to(PluginConfigs::Table, PluginConfigs::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_autodiscovery_ignores_tenant_config_package")
                    .table(AutodiscoveryIgnores::Table)
                    .col(AutodiscoveryIgnores::TenantId)
                    .col(AutodiscoveryIgnores::PluginConfigId)
                    .col(AutodiscoveryIgnores::PackageIdentifier)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // NOTE: available_versions table has been removed. Latest version
        // tracking is now per-host, stored directly in host_software_items
        // (latest_version, latest_version_fetched_at, latest_release_metadata).

        // ============================================================
        // 9. MQTT
        // ============================================================

        // --- mqtt_clients ---
        manager
            .create_table(
                Table::create()
                    .table(MqttClients::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttClients::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MqttClients::TenantId).uuid().not_null())
                    .col(boolean(MqttClients::Enabled).default(true))
                    .col(string(MqttClients::Transport).default("tcp"))
                    .col(string(MqttClients::Host))
                    .col(integer(MqttClients::Port).default(1883))
                    .col(string(MqttClients::ClientId).default("uptrakit-controller"))
                    .col(string_null(MqttClients::Username))
                    .col(string_null(MqttClients::Password))
                    .col(string(MqttClients::TopicPrefix).default("uptrakit"))
                    .col(
                        ColumnDef::new(MqttClients::ConnectionStatus)
                            .string()
                            .not_null()
                            .default("offline"),
                    )
                    .col(
                        ColumnDef::new(MqttClients::StatusUpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(timestamp(MqttClients::CreatedAt))
                    .col(timestamp(MqttClients::UpdatedAt))
                    .col(ColumnDef::new(MqttClients::CaCertPem).text())
                    .col(
                        ColumnDef::new(MqttClients::HaDiscovery)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(MqttClients::HaDiscoveryPrefix)
                            .string()
                            .not_null()
                            .default("homeassistant"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_clients_tenant")
                            .from(MqttClients::Table, MqttClients::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_clients_tenant_id")
                    .table(MqttClients::Table)
                    .col(MqttClients::TenantId)
                    .to_owned(),
            )
            .await?;

        // --- mqtt_leases ---
        manager
            .create_table(
                Table::create()
                    .table(MqttLeases::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MqttLeases::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MqttLeases::TenantId).uuid().not_null())
                    .col(ColumnDef::new(MqttLeases::MqttClientId).uuid().not_null())
                    .col(string(MqttLeases::InstanceId))
                    .col(timestamp(MqttLeases::HeartbeatAt))
                    .col(timestamp(MqttLeases::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_leases_tenant")
                            .from(MqttLeases::Table, MqttLeases::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mqtt_leases_mqtt_client")
                            .from(MqttLeases::Table, MqttLeases::MqttClientId)
                            .to(MqttClients::Table, MqttClients::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_mqtt_leases_mqtt_client_id")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::MqttClientId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_leases_instance_id")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::InstanceId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mqtt_leases_heartbeat_at")
                    .table(MqttLeases::Table)
                    .col(MqttLeases::HeartbeatAt)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 10. Update history
        // ============================================================

        // --- update_history ---
        manager
            .create_table(
                Table::create()
                    .table(UpdateHistory::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UpdateHistory::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UpdateHistory::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(UpdateHistory::SoftwareItemId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string_null(UpdateHistory::FromVersion))
                    .col(string(UpdateHistory::ToVersion))
                    .col(string(UpdateHistory::Status))
                    .col(ColumnDef::new(UpdateHistory::Output).text().not_null())
                    .col(
                        ColumnDef::new(UpdateHistory::OutputBytes)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(UpdateHistory::ActorType)
                            .string()
                            .not_null()
                            .default("legacy"),
                    )
                    .col(
                        ColumnDef::new(UpdateHistory::ActorId)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .col(timestamp(UpdateHistory::StartedAt))
                    .col(timestamp_null(UpdateHistory::CompletedAt))
                    .col(timestamp(UpdateHistory::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_history_host")
                            .from(UpdateHistory::Table, UpdateHistory::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_history_software_item")
                            .from(UpdateHistory::Table, UpdateHistory::SoftwareItemId)
                            .to(SoftwareItems::Table, SoftwareItems::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_host_id")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_software_item_id")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_status")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_history_host_software_item")
                    .table(UpdateHistory::Table)
                    .col(UpdateHistory::HostId)
                    .col(UpdateHistory::SoftwareItemId)
                    .to_owned(),
            )
            .await?;

        // --- update_output_lines ---
        manager
            .create_table(
                Table::create()
                    .table(UpdateOutputLines::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UpdateOutputLines::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UpdateOutputLines::UpdateHistoryId)
                            .uuid()
                            .not_null(),
                    )
                    .col(string(UpdateOutputLines::Stream))
                    .col(ColumnDef::new(UpdateOutputLines::Output).text().not_null())
                    .col(timestamp(UpdateOutputLines::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_output_lines_update_history")
                            .from(UpdateOutputLines::Table, UpdateOutputLines::UpdateHistoryId)
                            .to(UpdateHistory::Table, UpdateHistory::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_update_output_lines_update_history")
                    .table(UpdateOutputLines::Table)
                    .col(UpdateOutputLines::UpdateHistoryId)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 11. Pending auth stores
        // ============================================================

        // --- pending_device_flows ---
        manager
            .create_table(
                Table::create()
                    .table(PendingDeviceFlows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingDeviceFlows::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingDeviceFlows::DeviceCodeHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(PendingDeviceFlows::UserCode)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(PendingDeviceFlows::Status).text().not_null())
                    .col(ColumnDef::new(PendingDeviceFlows::UserId).uuid())
                    .col(ColumnDef::new(PendingDeviceFlows::ClientName).text())
                    .col(
                        ColumnDef::new(PendingDeviceFlows::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingDeviceFlows::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_device_flows_expires_at")
                    .table(PendingDeviceFlows::Table)
                    .col(PendingDeviceFlows::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // --- pending_oidc_flows ---
        manager
            .create_table(
                Table::create()
                    .table(PendingOidcFlows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingOidcFlows::CsrfState)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcFlows::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcFlows::PkceVerifier)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingOidcFlows::Nonce).text().not_null())
                    .col(
                        ColumnDef::new(PendingOidcFlows::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcFlows::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_oidc_flows_expires_at")
                    .table(PendingOidcFlows::Table)
                    .col(PendingOidcFlows::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // --- pending_account_links ---
        manager
            .create_table(
                Table::create()
                    .table(PendingAccountLinks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingAccountLinks::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::LinkTokenHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::OidcSubject)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingAccountLinks::Email).text().not_null())
                    .col(
                        ColumnDef::new(PendingAccountLinks::UserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingAccountLinks::FirstName).text())
                    .col(ColumnDef::new(PendingAccountLinks::LastName).text())
                    .col(
                        ColumnDef::new(PendingAccountLinks::MappedRoles)
                            .json()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingAccountLinks::ExistingLinkProviderId).uuid())
                    .col(
                        ColumnDef::new(PendingAccountLinks::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingAccountLinks::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_account_links_expires_at")
                    .table(PendingAccountLinks::Table)
                    .col(PendingAccountLinks::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // --- pending_oidc_token_exchanges ---
        manager
            .create_table(
                Table::create()
                    .table(PendingOidcTokenExchanges::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::ExchangeCodeHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::UserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcTokenExchanges::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_oidc_token_exchanges_expires_at")
                    .table(PendingOidcTokenExchanges::Table)
                    .col(PendingOidcTokenExchanges::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // --- pending_oidc_registrations ---
        manager
            .create_table(
                Table::create()
                    .table(PendingOidcRegistrations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::RegistrationCodeHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::ProviderId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::OidcSubject)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::Email)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PendingOidcRegistrations::FirstName).text())
                    .col(ColumnDef::new(PendingOidcRegistrations::LastName).text())
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::MappedRoles)
                            .json()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingOidcRegistrations::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_pending_oidc_registrations_expires_at")
                    .table(PendingOidcRegistrations::Table)
                    .col(PendingOidcRegistrations::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 12. Rate limiting
        // ============================================================

        // --- api_rate_limits ---
        manager
            .create_table(
                Table::create()
                    .table(ApiRateLimits::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ApiRateLimits::Key)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ApiRateLimits::RequestCount)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ApiRateLimits::WindowStart)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ApiRateLimits::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_api_rate_limits_expires_at")
                    .table(ApiRateLimits::Table)
                    .col(ApiRateLimits::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 13. Controller events (cross-instance notification outbox)
        // ============================================================

        // --- controller_events ---
        manager
            .create_table(
                Table::create()
                    .table(ControllerEvents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ControllerEvents::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ControllerEvents::SourceControllerId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ControllerEvents::TargetServiceId).uuid())
                    .col(ColumnDef::new(ControllerEvents::TargetCapability).text())
                    .col(
                        ColumnDef::new(ControllerEvents::MessageJson)
                            .json()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ControllerEvents::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_controller_events_source_id")
                    .table(ControllerEvents::Table)
                    .col(ControllerEvents::SourceControllerId)
                    .col(ControllerEvents::Id)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_controller_events_created_at")
                    .table(ControllerEvents::Table)
                    .col(ControllerEvents::CreatedAt)
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 14. Scheduled tasks
        // ============================================================

        manager
            .create_table(
                Table::create()
                    .table(ScheduledTasks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ScheduledTasks::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ScheduledTasks::TenantId).uuid().not_null())
                    .col(ColumnDef::new(ScheduledTasks::TaskType).text().not_null())
                    .col(string(ScheduledTasks::CronExpression))
                    .col(boolean(ScheduledTasks::Enabled).default(true))
                    .col(json_null(ScheduledTasks::TaskConfig))
                    .col(timestamp_null(ScheduledTasks::LastRunAt))
                    .col(timestamp(ScheduledTasks::NextRunAt))
                    .col(ColumnDef::new(ScheduledTasks::LockedBy).uuid())
                    .col(timestamp_null(ScheduledTasks::LockedAt))
                    .col(ColumnDef::new(ScheduledTasks::LastError).text())
                    .col(big_integer(ScheduledTasks::RunCount).default(0))
                    .col(timestamp(ScheduledTasks::CreatedAt))
                    .col(timestamp(ScheduledTasks::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_scheduled_tasks_tenant")
                            .from(ScheduledTasks::Table, ScheduledTasks::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_scheduled_tasks_next_run")
                    .table(ScheduledTasks::Table)
                    .col(ScheduledTasks::NextRunAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_scheduled_tasks_tenant_id")
                    .table(ScheduledTasks::Table)
                    .col(ScheduledTasks::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_scheduled_tasks_tenant_task_type")
                    .table(ScheduledTasks::Table)
                    .col(ScheduledTasks::TenantId)
                    .col(ScheduledTasks::TaskType)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Seed default tasks for all tenants
        let scheduled_task_types = [
            ("auth_cleanup", "*/5 * * * *"),
            ("stale_lease_cleanup", "*/5 * * * *"),
            ("ca_rotation_check", "0 3 * * *"),
            ("version_check", "0 */6 * * *"),
            ("service_cert_check", "0 */12 * * *"),
        ];

        let db = manager.get_connection();
        let tenant_id_select = Query::select()
            .column(Tenants::Id)
            .from(Tenants::Table)
            .to_owned();
        let tenant_rows = db.query_all(&tenant_id_select).await?;

        for tenant_row in &tenant_rows {
            use sea_orm::TryGetable;
            let tenant_id: Uuid = Uuid::try_get_by(tenant_row, "id")
                .map_err(|e| DbErr::Custom(format!("failed to get tenant ID: {e:?}")))?;

            for (task_type, cron_expr) in &scheduled_task_types {
                manager
                    .exec_stmt(
                        Query::insert()
                            .into_table(ScheduledTasks::Table)
                            .columns([
                                ScheduledTasks::Id,
                                ScheduledTasks::TenantId,
                                ScheduledTasks::TaskType,
                                ScheduledTasks::CronExpression,
                                ScheduledTasks::Enabled,
                                ScheduledTasks::NextRunAt,
                                ScheduledTasks::RunCount,
                                ScheduledTasks::CreatedAt,
                                ScheduledTasks::UpdatedAt,
                            ])
                            .values_panic([
                                Uuid::now_v7().into(),
                                tenant_id.into(),
                                (*task_type).into(),
                                (*cron_expr).into(),
                                true.into(),
                                now.into(),
                                0i64.into(),
                                now.into(),
                                now.into(),
                            ])
                            .to_owned(),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop tables in reverse FK-dependency order
        macro_rules! drop_tables {
            ($manager:expr, $($table:expr),+ $(,)?) => {
                $(
                    $manager
                        .drop_table(Table::drop().table($table).to_owned())
                        .await?;
                )+
            };
        }

        drop_tables!(
            manager,
            ScheduledTasks::Table,
            ControllerEvents::Table,
            ApiRateLimits::Table,
            PendingOidcRegistrations::Table,
            PendingOidcTokenExchanges::Table,
            PendingAccountLinks::Table,
            PendingOidcFlows::Table,
            PendingDeviceFlows::Table,
            UpdateOutputLines::Table,
            UpdateHistory::Table,
            MqttLeases::Table,
            MqttClients::Table,
            AutodiscoveryIgnores::Table,
            HostSoftwareItemPlugins::Table,
            HostSoftwareItems::Table,
            SoftwareItems::Table,
            PluginConfigs::Table,
            ServiceHosts::Table,
            Hosts::Table,
            ServiceCertificates::Table,
            Services::Table,
            EnrollmentTokens::Table,
            SettingsVersion::Table,
            Settings::Table,
            ApiTokens::Table,
            Sessions::Table,
            UserOidcLinks::Table,
            OidcProviders::Table,
            UserRoles::Table,
            RolePermissions::Table,
            Permissions::Table,
            Roles::Table,
            CaCertificates::Table,
            Users::Table,
            Tenants::Table,
        );

        Ok(())
    }
}

/// Seeds the final RBAC state: 3 roles (owner, admin, user) with all 9 permissions.
///
/// Role assignments:
/// - `owner`: all 9 permissions
/// - `admin`: all except `manage_global_settings` (8 permissions)
/// - `user`: all `view_*` permissions (view_settings, view_agents, view_software, view_hosts)
async fn seed_rbac(manager: &SchemaManager<'_>, now: time::OffsetDateTime) -> Result<(), DbErr> {
    // Insert all 9 permissions
    let permissions = [
        ("view_settings", "View system settings"),
        ("manage_settings", "Create and modify system settings"),
        ("view_agents", "View monitoring agents"),
        ("manage_agents", "Approve, reject, and manage agents"),
        (
            "manage_global_settings",
            "Manage global settings (network, CA, TLS, system alerts)",
        ),
        (
            "view_software",
            "View software items, plugin configs, and update history",
        ),
        (
            "manage_software",
            "Manage software items, plugin configs, version checks, updates, and scheduler",
        ),
        ("view_hosts", "View hosts"),
        ("manage_hosts", "Manage hosts (update, deactivate)"),
    ];

    let mut permission_ids = Vec::new();
    for (name, description) in &permissions {
        let id = Uuid::now_v7();
        permission_ids.push((id, *name));

        manager
            .exec_stmt(
                Query::insert()
                    .into_table(Permissions::Table)
                    .columns([
                        Permissions::Id,
                        Permissions::Name,
                        Permissions::Description,
                        Permissions::CreatedAt,
                    ])
                    .values_panic([id.into(), (*name).into(), (*description).into(), now.into()])
                    .to_owned(),
            )
            .await?;
    }

    // owner role: all 9 permissions
    let owner_role_id = Uuid::now_v7();
    manager
        .exec_stmt(
            Query::insert()
                .into_table(Roles::Table)
                .columns([Roles::Id, Roles::Name, Roles::Description, Roles::CreatedAt])
                .values_panic([
                    owner_role_id.into(),
                    "owner".into(),
                    "Owner with full access including global settings".into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await?;

    for (perm_id, _) in &permission_ids {
        manager
            .exec_stmt(
                Query::insert()
                    .into_table(RolePermissions::Table)
                    .columns([RolePermissions::RoleId, RolePermissions::PermissionId])
                    .values_panic([owner_role_id.into(), (*perm_id).into()])
                    .to_owned(),
            )
            .await?;
    }

    // admin role: all except manage_global_settings (8 permissions)
    let admin_role_id = Uuid::now_v7();
    manager
        .exec_stmt(
            Query::insert()
                .into_table(Roles::Table)
                .columns([Roles::Id, Roles::Name, Roles::Description, Roles::CreatedAt])
                .values_panic([
                    admin_role_id.into(),
                    "admin".into(),
                    "Administrator with full system access".into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await?;

    for (perm_id, perm_name) in &permission_ids {
        if *perm_name == "manage_global_settings" {
            continue;
        }
        manager
            .exec_stmt(
                Query::insert()
                    .into_table(RolePermissions::Table)
                    .columns([RolePermissions::RoleId, RolePermissions::PermissionId])
                    .values_panic([admin_role_id.into(), (*perm_id).into()])
                    .to_owned(),
            )
            .await?;
    }

    // user role: all view_* permissions (view_settings, view_agents, view_software, view_hosts)
    let user_role_id = Uuid::now_v7();
    manager
        .exec_stmt(
            Query::insert()
                .into_table(Roles::Table)
                .columns([Roles::Id, Roles::Name, Roles::Description, Roles::CreatedAt])
                .values_panic([
                    user_role_id.into(),
                    "user".into(),
                    "Standard user with limited access".into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await?;

    let user_permissions = [
        "view_settings",
        "view_agents",
        "view_software",
        "view_hosts",
    ];
    for (perm_id, perm_name) in &permission_ids {
        if user_permissions.contains(perm_name) {
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(RolePermissions::Table)
                        .columns([RolePermissions::RoleId, RolePermissions::PermissionId])
                        .values_panic([user_role_id.into(), (*perm_id).into()])
                        .to_owned(),
                )
                .await?;
        }
    }

    Ok(())
}

// ============================================================
// DeriveIden enums for all tables
// ============================================================

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
    Name,
    Slug,
    IsDefault,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    FirstName,
    LastName,
    PasswordHash,
    IsActive,
    DeactivatedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum CaCertificates {
    Table,
    Fingerprint,
    CertPem,
    KeyPem,
    NotBefore,
    NotAfter,
    ActivatedAt,
    DeactivatedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Roles {
    Table,
    Id,
    Name,
    Description,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Permissions {
    Table,
    Id,
    Name,
    Description,
    CreatedAt,
}

#[derive(DeriveIden)]
enum RolePermissions {
    Table,
    RoleId,
    PermissionId,
}

#[derive(DeriveIden)]
enum UserRoles {
    Table,
    TenantId,
    UserId,
    RoleId,
    AssignedAt,
}

#[derive(DeriveIden)]
enum OidcProviders {
    Table,
    Id,
    TenantId,
    Name,
    Slug,
    LogoUrl,
    IssuerUrl,
    ClientId,
    ClientSecret,
    Scopes,
    AutoCreateUsers,
    RoleClaimPath,
    RoleMapping,
    IsActive,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum UserOidcLinks {
    Table,
    Id,
    UserId,
    ProviderId,
    OidcSubject,
    LinkedAt,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    UserId,
    RefreshTokenHash,
    AuthMethod,
    TokenType,
    CreatedAt,
    ExpiresAt,
    UserAgent,
    IpAddress,
    OidcProviderId,
    RevokedAt,
}

#[derive(DeriveIden)]
enum ApiTokens {
    Table,
    Id,
    UserId,
    Name,
    TokenHash,
    CreatedAt,
    LastUsedAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum Settings {
    Table,
    TenantId,
    Key,
    Value,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum SettingsVersion {
    Table,
    TenantId,
    Version,
    GlobalVersion,
    RevocationVersion,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
    TenantId,
    Capabilities,
    Hostname,
    FriendlyName,
    IpAddress,
    Status,
    EnrollmentSecretHash,
    ClientVersion,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
    PingIntervalSeconds,
    EnrollmentTokenId,
}

#[derive(DeriveIden)]
enum EnrollmentTokens {
    Table,
    Id,
    TenantId,
    Name,
    TokenHash,
    AllowedCapabilities,
    MaxUses,
    CurrentUses,
    ExpiresAt,
    CreatedAt,
    RevokedAt,
    CreatedByUserId,
}

#[derive(DeriveIden)]
enum ServiceCertificates {
    Table,
    CaFingerprint,
    ServiceId,
    SerialNumber,
    NotBefore,
    NotAfter,
    RevokedAt,
    RevocationReason,
    CreatedAt,
    LastSeenAt,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
    TenantId,
    MachineId,
    Hostname,
    FriendlyName,
    OsType,
    OsVersion,
    Architecture,
    IpAddress,
    LastSeenAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum ServiceHosts {
    Table,
    ServiceId,
    HostId,
    LinkedAt,
}

#[derive(DeriveIden)]
enum PluginConfigs {
    Table,
    Id,
    TenantId,
    Name,
    PluginType,
    Config,
    Enabled,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum SoftwareItems {
    Table,
    Id,
    TenantId,
    Name,
    Enabled,
    DiscoveryState,
    LastCheckedAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum AutodiscoveryIgnores {
    Table,
    Id,
    TenantId,
    PluginConfigId,
    PackageIdentifier,
    CreatedAt,
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    HostId,
    SoftwareItemId,
    InstalledVersion,
    InstalledVersionDetectedAt,
    LatestVersion,
    LatestVersionFetchedAt,
    LatestReleaseMetadata,
    LastUpdatedAt,
    LinkedAt,
}

#[derive(DeriveIden)]
enum HostSoftwareItemPlugins {
    Table,
    Id,
    HostId,
    SoftwareItemId,
    PluginConfigId,
    Role,
    Ordinal,
    PackageIdentifier,
    ConfigOverride,
    ExecutionSite,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum MqttClients {
    Table,
    Id,
    TenantId,
    Enabled,
    Transport,
    Host,
    Port,
    ClientId,
    Username,
    Password,
    TopicPrefix,
    ConnectionStatus,
    StatusUpdatedAt,
    CreatedAt,
    UpdatedAt,
    CaCertPem,
    HaDiscovery,
    HaDiscoveryPrefix,
}

#[derive(DeriveIden)]
enum MqttLeases {
    Table,
    Id,
    TenantId,
    MqttClientId,
    InstanceId,
    HeartbeatAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    Id,
    HostId,
    SoftwareItemId,
    FromVersion,
    ToVersion,
    Status,
    Output,
    OutputBytes,
    ActorType,
    ActorId,
    StartedAt,
    CompletedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum UpdateOutputLines {
    Table,
    Id,
    UpdateHistoryId,
    Stream,
    Output,
    CreatedAt,
}

#[derive(DeriveIden)]
enum PendingDeviceFlows {
    Table,
    Id,
    DeviceCodeHash,
    UserCode,
    Status,
    UserId,
    ClientName,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingOidcFlows {
    Table,
    CsrfState,
    ProviderId,
    PkceVerifier,
    Nonce,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingAccountLinks {
    Table,
    Id,
    LinkTokenHash,
    ProviderId,
    OidcSubject,
    Email,
    UserId,
    FirstName,
    LastName,
    MappedRoles,
    ExistingLinkProviderId,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingOidcTokenExchanges {
    Table,
    Id,
    ExchangeCodeHash,
    UserId,
    ProviderId,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum PendingOidcRegistrations {
    Table,
    Id,
    RegistrationCodeHash,
    ProviderId,
    OidcSubject,
    Email,
    FirstName,
    LastName,
    MappedRoles,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum ApiRateLimits {
    Table,
    Key,
    RequestCount,
    WindowStart,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum ControllerEvents {
    Table,
    Id,
    SourceControllerId,
    TargetServiceId,
    TargetCapability,
    MessageJson,
    CreatedAt,
}

#[derive(DeriveIden)]
enum ScheduledTasks {
    Table,
    Id,
    TenantId,
    TaskType,
    CronExpression,
    Enabled,
    TaskConfig,
    LastRunAt,
    NextRunAt,
    LockedBy,
    LockedAt,
    LastError,
    RunCount,
    CreatedAt,
    UpdatedAt,
}
