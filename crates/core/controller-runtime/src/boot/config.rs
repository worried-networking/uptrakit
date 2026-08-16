//! Phase 0: TOML config load, bootstrap-arg parsing, and tracing init.
//!
//! This module owns the first phase of the boot sequence — everything that
//! must complete before any other startup phase can proceed.  The result is a
//! [`BootConfig`] value that is threaded into [`super::run_server`].

use clap::Parser as _;
use rootcause::prelude::*;
#[cfg(feature = "journald")]
use tracing_subscriber::prelude::*;
use uptrakit_build_info::BuildInfo;

use crate::AppError;

/// Output of Phase 0: holds the loaded TOML runtime config, parsed bootstrap
/// args, config-file path, and the original CLI args needed downstream.
pub(crate) struct BootConfig {
    pub booted: crate::boot::init::BootedConfig,
    pub oidc_bootstrap: crate::cli::OidcBootstrapArgs,
    pub enrollment_bootstrap: crate::cli::EnrollmentBootstrapArgs,
    pub config_path: std::path::PathBuf,
    pub args: crate::cli::Args,
}

/// Load Phase 0: resolve config path, parse TOML, parse bootstrap env args,
/// initialise tracing.  Returns [`BootConfig`] on success.
pub(crate) async fn load(args: crate::cli::Args, info: &BuildInfo) -> crate::Result<BootConfig> {
    tracing::info!(binary = %info.binary, version = %info.version, "starting controller");

    // Phase 0: Load TOML config — must happen before all other phases so that
    // all configuration comes from the file rather than CLI flags.
    let config_path = args.find_config_path().map_err(|e| {
        report!(AppError::Config(format!(
            "failed to resolve config path: {e}"
        )))
    })?;
    tracing::info!("toml config path: {}", config_path.display());
    let booted = crate::boot::init::boot_config(config_path.clone())
        .await
        .map_err(|e| report!(AppError::Config(format!("failed to load TOML config: {e}"))))?;

    // Parse bootstrap args from environment variables (no CLI flags; env only).
    let oidc_bootstrap = crate::cli::OidcBootstrapArgs::try_parse_from(["uptrakit-controller"])
        .unwrap_or_else(|_| {
            // Fallback: construct with all None/default values.
            // env vars are picked up by clap's env attribute when try_parse_from
            // is called with a minimal argv — the env attributes on each field
            // still apply, so env vars take effect here.
            crate::cli::OidcBootstrapArgs {
                oidc_issuer_url: std::env::var("UPTRAKIT_OIDC_ISSUER_URL").ok(),
                oidc_client_id: std::env::var("UPTRAKIT_OIDC_CLIENT_ID").ok(),
                oidc_client_secret: std::env::var("UPTRAKIT_OIDC_CLIENT_SECRET").ok(),
                oidc_provider_name: std::env::var("UPTRAKIT_OIDC_PROVIDER_NAME")
                    .ok()
                    .or_else(|| Some("SSO".to_string())),
                oidc_provider_slug: std::env::var("UPTRAKIT_OIDC_PROVIDER_SLUG")
                    .ok()
                    .or_else(|| Some("sso".to_string())),
                oidc_scopes: std::env::var("UPTRAKIT_OIDC_SCOPES")
                    .ok()
                    .or_else(|| Some("openid email profile groups".to_string())),
                oidc_allow_private_network_issuers: std::env::var(
                    "UPTRAKIT_OIDC_ALLOW_PRIVATE_NETWORK_ISSUERS",
                )
                .ok()
                .and_then(|v| v.parse().ok()),
            }
        });

    let enrollment_bootstrap =
        crate::cli::EnrollmentBootstrapArgs::try_parse_from(["uptrakit-controller"])
            .unwrap_or_else(|_| crate::cli::EnrollmentBootstrapArgs {
                bootstrap_enrollment_token: std::env::var("UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN")
                    .ok(),
                bootstrap_enrollment_token_max_uses: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_MAX_USES",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
                bootstrap_enrollment_token_ttl: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_TTL",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
                bootstrap_system_enrollment_token: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN",
                )
                .ok(),
                bootstrap_system_enrollment_token_max_uses: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_MAX_USES",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
                bootstrap_system_enrollment_token_ttl: std::env::var(
                    "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_TTL",
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            });

    // Initialise tracing. Log level from runtime.log in TOML; -v/-vv/-vvv on CLI overrides.
    let builder = uptrakit_tracing_init::TracingBuilder::new()
        .verbosity(args.verbose)
        .max_verbosity(3)
        .directives_for_verbosity(
            0,
            &[
                ("uptrakit_controller_runtime", "info"),
                ("uptrakit_web_api", "info"),
            ],
        )
        .directives_for_verbosity(
            1,
            &[
                ("uptrakit_controller_runtime", "debug"),
                ("uptrakit_web_api", "debug"),
            ],
        )
        .directives_for_verbosity(2, &[("uptrakit", "debug")])
        .directives_for_verbosity(3, &[("uptrakit", "trace")]);

    // The dedicated audit layer and the main-layer `uptrakit_audit`
    // exclusion must follow one predicate: layer constructed => both
    // installed; construction failed => neither (exclusion without the
    // layer would drop audit events from the journal entirely in
    // journald mode).
    #[cfg(feature = "journald")]
    let builder = match tracing_journald::layer() {
        Ok(journald) => {
            let journald =
                journald.with_filter(tracing_subscriber::EnvFilter::new("uptrakit_audit=info"));
            builder
                .extra_layer(Box::new(journald))
                .journald_exclude_exact("uptrakit_audit")
        }
        Err(error) => {
            eprintln!(
                "warning: journald unavailable ({error}); audit events will not be mirrored to the journal"
            );
            builder
        }
    };

    builder.init();

    Ok(BootConfig {
        booted,
        oidc_bootstrap,
        enrollment_bootstrap,
        config_path,
        args,
    })
}
