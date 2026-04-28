//! Phase 10: JWT initialization.

use rootcause::prelude::*;

use crate::AppError;

/// Migrate file-based JWT key (if present) and load or generate the DB-stored JWT signing key.
pub(crate) async fn init_jwt(
    db: &sea_orm::DatabaseConnection,
    state_dir: &std::path::Path,
) -> crate::Result<uptrakit_web_api::auth::jwt::JwtManager> {
    uptrakit_web_api::settings_store::migrate_file_jwt_key(db, state_dir)
        .await
        .context(AppError::Config("JWT key migration failed".into()))?;

    let jwt_secret = uptrakit_web_api::settings_store::load_or_generate_jwt_key(db)
        .await
        .context(AppError::Config("JWT key initialization failed".into()))?;

    let jwt_manager = uptrakit_web_api::auth::jwt::JwtManager::from_secret(&jwt_secret);
    tracing::info!("JWT signing key initialized from database");
    Ok(jwt_manager)
}
