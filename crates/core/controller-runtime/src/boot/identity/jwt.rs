//! Phase 10: JWT signing-key initialization.
//!
//! Thin wrapper around [`crate::startup::init_jwt`] so that `identity::init`
//! can delegate to a module-local function without reaching into `startup`
//! directly from `identity/mod.rs`.

use uptrakit_web_api::auth::jwt::JwtManager;

/// Phase 10: initialize the JWT signing key.
///
/// Migrates a legacy file-based key (if present) and loads or generates the
/// DB-stored signing key.  Delegates to [`crate::startup::init_jwt`].
pub(super) async fn init(
    db: &sea_orm::DatabaseConnection,
    state_dir: &std::path::Path,
) -> crate::Result<JwtManager> {
    crate::startup::init_jwt(db, state_dir).await
}
