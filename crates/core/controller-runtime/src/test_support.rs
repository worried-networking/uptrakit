//! Test-only crate-internal exposure, gated behind `test-support`.
//!
//! `crate::reencrypt::reencrypt_to_v3`, `crate::reencrypt::register_column_aad_mappings`,
//! and `crate::migration::run_migrations` are all `pub(crate)` (the latter's
//! error type, `crate::db::DbError`, is `pub(crate)` too) -- unreachable from
//! a `tests/` target, which only sees the crate's `pub` surface. This module
//! adds thin `pub` wrappers so `tests/rotation_claim.rs` (its own compiled
//! test binary -- required so it can freely initialize the DEK ring, a
//! process-global `OnceLock`, without affecting the `reencrypt` module's own
//! `#[cfg(test)]` unit tests, several of which hard-depend on the ring never
//! being active in their shared `--lib` test binary) can drive the real
//! startup path end to end.
//!
//! Existing items keep their `pub(crate)` visibility untouched -- this module
//! only adds a new, additive `pub` surface.

use sea_orm::DatabaseConnection;

/// Test-only wrapper around the private `crate::migration::run_migrations`.
/// Maps the `pub(crate)` error to a `String` since the real error type is not
/// reachable from a `pub` signature.
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), String> {
    crate::migration::run_migrations(db)
        .await
        .map_err(|e| e.to_string())
}

/// Test-only wrapper around `crate::reencrypt::register_column_aad_mappings`.
pub fn register_column_aad_mappings() {
    crate::reencrypt::register_column_aad_mappings();
}

/// Test-only wrapper around `crate::reencrypt::reencrypt_to_v3`.
pub async fn reencrypt_to_v3(db: &DatabaseConnection) {
    crate::reencrypt::reencrypt_to_v3(db).await;
}
