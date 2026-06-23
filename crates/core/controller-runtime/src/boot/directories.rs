//! Phase 2: Application directory resolution and embedded-service installation ID.
//!
//! Resolves platform-default directories for the controller (`config/` and
//! `state/`), ensures they exist, and — when at least one embedded service is
//! compiled in — reads or generates the persistent controller installation ID
//! from `<state_dir>/controller-installation-id`.

use rootcause::prelude::*;

use crate::AppError;

/// Output of Phase 2: application directory layout and optional installation ID.
pub(crate) struct AppLayout {
    pub app_dirs: uptrakit_directories::AppDirs,
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    pub installation_id: uuid::Uuid,
}

/// Phase 2: resolve application directories and, for embedded-service builds,
/// read or generate the controller installation ID.
pub(crate) async fn resolve() -> crate::Result<AppLayout> {
    let app_dirs =
        uptrakit_directories::AppDirs::resolve("controller", None, None).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to resolve directories: {e}"
            )))
        })?;
    app_dirs.ensure_dirs().await.map_err(|e| {
        report!(AppError::Config(format!(
            "failed to create directories: {e}"
        )))
    })?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());
    #[cfg(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    ))]
    let installation_id = crate::boot::init::init_installation_id(app_dirs.state_dir()).await?;

    Ok(AppLayout {
        app_dirs,
        #[cfg(any(
            feature = "embedded-scheduler",
            feature = "embedded-agent",
            feature = "embedded-ssh-agent",
            feature = "embedded-mqtt"
        ))]
        installation_id,
    })
}
