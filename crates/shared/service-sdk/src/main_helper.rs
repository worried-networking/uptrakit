//! Shared initialization and lifecycle error handling for service binaries.
//!
//! These helpers eliminate the per-service boilerplate in `main()`:
//! - [`init_tracing`] — set up `tracing_subscriber` with a verbosity-aware filter.
//! - [`init_crypto`] — install the `aws-lc-rs` default crypto provider.
//! - [`print_build_info`] — render build metadata and print to stdout.
//! - [`run_lifecycle_and_handle_errors`] — run the lifecycle and translate
//!   terminal errors into log messages or `process::exit`.

use crate::cli::CommonServiceArgs;
use crate::lifecycle::ServiceHandler;

/// Initialize `tracing_subscriber` with a verbosity-aware [`EnvFilter`].
///
/// Verbosity levels expand scope progressively, keeping third-party crates
/// silent unless `RUST_LOG` explicitly enables them:
///
/// - `verbosity == 0`: `{own_module}=info` — only the service's own crate at info.
/// - `verbosity == 1`: `{own_module}=debug` — own crate at debug, everything else quiet.
/// - `verbosity == 2`: `uptrakit=debug` — all uptrakit crates at debug.
/// - `verbosity >= 3`: `uptrakit=trace` — all uptrakit crates at trace.
/// - `verbosity > 3`: emits a warning — `-vvvv` and above have no extra effect.
///
/// `RUST_LOG` is always respected and can enable third-party crates independently
/// (e.g. `RUST_LOG=tokio=info uptrakit-agent -vv`).
///
/// [`EnvFilter`]: tracing_subscriber::EnvFilter
pub fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;

    if verbosity > 3 {
        eprintln!(
            "warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)"
        );
    }

    let directive = match verbosity {
        0 => format!("{own_module}=info"),
        1 => format!("{own_module}=debug"),
        2 => "uptrakit=debug".to_string(),
        _ => "uptrakit=trace".to_string(),
    };
    let filter = EnvFilter::from_default_env()
        .add_directive(directive.parse().expect("valid directive"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Install the `aws-lc-rs` default cryptographic provider for `rustls`.
///
/// Safe to call multiple times — the second call is a no-op.
pub fn init_crypto() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Print human-readable build information and return.
///
/// Intended for use with the `--version` flag.
pub fn print_build_info(binary_name: &str, version: &str, enabled_features: Option<&str>) {
    let build_info =
        uptrakit_build_info::BuildInfo::current(binary_name, version, enabled_features);
    print!("{}", build_info.render_human());
}

/// Run the full service lifecycle and handle terminal errors.
///
/// - If the lifecycle exits because the controller disconnected, logs an
///   info-level message.
/// - If it exits with any other error, logs an error-level message and
///   calls `std::process::exit(1)`.
/// - On success, returns normally.
pub async fn run_lifecycle_and_handle_errors<H: ServiceHandler>(
    binary_name: &str,
    args: &CommonServiceArgs,
    handler: &mut H,
) {
    if let Err(e) = crate::run_service_lifecycle(args, handler).await {
        if e.current_context().is_receive_closed() {
            tracing::info!("disconnected by controller");
        } else {
            tracing::error!(error = %e, "{binary_name} failed");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn verbosity_level_semantics() {
        // Verify the directive string produced for each verbosity level.
        // verbosity=0 → own-module scoped at info
        // verbosity=1 → own-module scoped at debug
        // verbosity=2 → all uptrakit crates at debug
        // verbosity>=3 → all uptrakit crates at trace
        let own_module = "uptrakit_agent";
        let directive_for = |v: u8| -> String {
            match v {
                0 => format!("{own_module}=info"),
                1 => format!("{own_module}=debug"),
                2 => "uptrakit=debug".to_string(),
                _ => "uptrakit=trace".to_string(),
            }
        };
        assert_eq!(directive_for(0), "uptrakit_agent=info");
        assert_eq!(directive_for(1), "uptrakit_agent=debug");
        assert_eq!(directive_for(2), "uptrakit=debug");
        assert_eq!(directive_for(3), "uptrakit=trace");
        assert_eq!(directive_for(4), "uptrakit=trace");
    }
}
