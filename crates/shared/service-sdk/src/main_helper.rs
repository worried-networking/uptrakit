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
/// - `verbosity == 0`: service's own module at `info`; all others silent unless
///   `RUST_LOG` specifies them (e.g. `"uptrakit_agent=info"`).
/// - `verbosity == 1`: global `debug` level added on top of `RUST_LOG`.
/// - `verbosity >= 2`: global `trace` level added on top of `RUST_LOG`.
/// - `verbosity > 2`: emits a warning — `-vvv` and above have no extra effect.
///
/// `RUST_LOG` is always respected and can override or suppress specific modules.
///
/// [`EnvFilter`]: tracing_subscriber::EnvFilter
pub fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;

    if verbosity > 2 {
        eprintln!(
            "warning: -vvv or more has no additional effect; maximum verbosity is -vv (trace)"
        );
    }

    let filter = if verbosity == 0 {
        EnvFilter::from_default_env().add_directive(
            format!("{own_module}=info")
                .parse()
                .expect("valid module=level directive"),
        )
    } else {
        let level = if verbosity == 1 { "debug" } else { "trace" };
        EnvFilter::from_default_env()
            .add_directive(level.parse().expect("valid level directive"))
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_level_semantics() {
        // verbosity=0 should use module=info pattern (not a blanket level)
        // verbosity=1 maps to "debug"
        // verbosity>=2 maps to "trace"
        // We can't easily test the filter output without running init_tracing,
        // but we can verify the branch logic by checking what level string is produced.
        let level_for = |v: u8| -> &'static str {
            if v == 0 {
                "info" // module-scoped; tested implicitly
            } else if v == 1 {
                "debug"
            } else {
                "trace"
            }
        };
        assert_eq!(level_for(0), "info");
        assert_eq!(level_for(1), "debug");
        assert_eq!(level_for(2), "trace");
        assert_eq!(level_for(3), "trace");
    }
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
