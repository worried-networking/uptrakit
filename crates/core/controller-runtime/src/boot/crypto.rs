//! Boot crypto phases: master-key initialization and post-DB verification.
//!
//! Splits the crypto boot work by DB dependency:
//!
//! - [`init`] — Phase 1 (no DB): resolve master-key source and call
//!   `boot::init::init_master_key`. Runs immediately after config is loaded.
//! - [`verify_and_migrate`] — Phases 4/4b/4c/4d (needs DB): verify the key
//!   matches existing encrypted data, register column AAD mappings, initialise
//!   the data key ring, and migrate legacy values to `ENC:v3:`. Runs after the
//!   database connection is opened.

use crate::boot::config::BootConfig;

/// Carries the raw master-key hex produced by Phase 1.
///
/// `hex` is `None` when no master key is configured (encryption disabled).
/// The type is exactly what [`crate::boot::init::init_master_key`] returns —
/// no wrapping or unwrapping is applied.
pub(crate) struct MasterKey {
    pub hex: Option<uptrakit_wire::SecretString>,
}

/// Phase 1: resolve the master-key source and initialise the global crypto key.
///
/// Reads `--master-key-from` (CLI) or the `[runtime].master_key` TOML value
/// as a fallback; the TOML value already carries the full source string
/// (`file:`, `env:`, or inline hex) so no prefix injection is needed.
///
/// The result is stored in [`MasterKey::hex`] as-is — the exact
/// `Option<uptrakit_wire::SecretString>` returned by
/// `boot::init::init_master_key`, with no additional wrapping.
pub(crate) fn init(cfg: &BootConfig) -> crate::Result<MasterKey> {
    let runtime = &cfg.booted.runtime;
    let toml_key = runtime.master_key.expose_secret();
    let master_key_source = cfg.args.master_key_from.as_deref().or({
        if toml_key.is_empty() {
            None
        } else {
            Some(toml_key)
        }
    });
    let hex = crate::boot::init::init_master_key(master_key_source)?;
    Ok(MasterKey { hex })
}

/// Phases 4/4b/4c/4d: verify the master key against the DB, register column
/// AAD mappings, initialise the data key ring, and migrate encrypted values
/// to `ENC:v3:`.
///
/// Must be called **after** the database connection is opened (Phase 3).
///
/// Note: the parameter type is `&sea_orm::DatabaseConnection` rather than
/// `&boot::persistence::Persistence` because the `Persistence` struct does not
/// exist yet (it will be introduced in Task 6). A later task will adapt this
/// signature once `Persistence` is available.
pub(crate) async fn verify_and_migrate(db: &sea_orm::DatabaseConnection) -> crate::Result<()> {
    // Phase 4: Master key verification (HA safety)
    crate::boot::init::verify_master_key(db).await?;

    // Phase 4b: Register column AAD mappings (enables ENC:v2/v3 read support)
    crate::reencrypt::register_column_aad_mappings();

    // Phase 4c: Initialize data key ring (envelope encryption)
    crate::boot::init::init_data_key_ring(db).await?;

    // Phase 4d: Migrate all encrypted values to ENC:v3 (automatic)
    crate::reencrypt::reencrypt_to_v3(db).await;

    Ok(())
}
