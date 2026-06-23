//! Phases 5, 6, 7, 7b, 7c, 8: settings load, reconcile, seeding, and validation.
//!
//! This module groups the settings-related boot phases into a single async
//! function that produces a [`SettingsBundle`] consumed by the caller
//! (`boot::run_server`) and all downstream phases.

use rootcause::prelude::*;
use uptrakit_web_api::settings::Settings;

use crate::AppError;
use crate::boot::config::BootConfig;
use crate::boot::persistence::Persistence;

/// Output of Phases 5–8: loaded settings, reconciled addresses, and validated config.
pub(crate) struct SettingsBundle {
    pub settings: uptrakit_web_api::settings::Settings,
    pub reconciled: crate::boot::init::ReconciledSettings,
    pub validated: crate::boot::init::ValidatedConfig,
}

/// Phases 5, 6, 7, 7b, 7c, 8: load settings from the database, reconcile with
/// TOML values, run bootstrap seeding (OIDC, enrollment tokens, OAuth defaults),
/// and validate the resulting configuration.
///
/// Phase 7d (`boot_oauth_state`) is intentionally excluded — it belongs to the
/// identity boot phase and remains in `boot::run_server`.
pub(crate) async fn load_and_seed(
    cfg: &BootConfig,
    db: &Persistence,
) -> crate::Result<SettingsBundle> {
    let runtime = &cfg.booted.runtime;

    // Phase 5: Load settings
    let (settings, global_raw, _tenant_raw, reg_token) =
        Settings::load(&db.db, db.default_tenant_id)
            .await
            .context(AppError::Settings)?;
    if let Some(token) = reg_token {
        eprintln!("==========================================================");
        eprintln!("  No users found. Use this one-time registration token:");
        eprintln!("  {token}");
        eprintln!("==========================================================");
    }

    // Phase 6: Reconcile settings — use TOML values as seeds
    let reconciled =
        crate::boot::init::reconcile_all_settings(&db.db, runtime, &settings, &global_raw).await?;

    // Phase 7: OIDC bootstrap
    crate::boot::init::bootstrap_oidc(&db.db, db.default_tenant_id, &cfg.oidc_bootstrap).await?;

    // Phase 7b: Enrollment token bootstrap
    crate::boot::init::bootstrap_enrollment_tokens(
        &db.db,
        db.default_tenant_id,
        &cfg.enrollment_bootstrap,
    )
    .await?;

    // Phase 7c: OAuth settings defaults
    crate::boot::init::seed_oauth_defaults(&db.db).await?;

    // Phase 8: Validate configuration
    let validated = crate::boot::init::validate_configuration(runtime, &reconciled)?;

    Ok(SettingsBundle {
        settings,
        reconciled,
        validated,
    })
}
