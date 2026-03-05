//! Shared initialization and lifecycle error handling for service binaries.
//!
//! These helpers eliminate the per-service boilerplate in `main()`:
//! - [`init_crypto`] — install the `aws-lc-rs` default crypto provider.
//! - [`init_tracing`] — configure the global tracing subscriber with
//!   verbosity-aware filtering.
//! - [`print_build_info`] — render build metadata and print to stdout.
//! - [`run_lifecycle_and_handle_errors`] — run the lifecycle and translate
//!   terminal errors into log messages or `process::exit`.
//!
//! [`init_tracing`] is provided as a convenience that each binary calls
//! explicitly in its own `main()`. The SDK never installs the tracing
//! subscriber autonomously — the call-site remains in the binary.

use crate::cli::CommonServiceArgs;
use crate::lifecycle::ServiceHandler;

/// Install the `aws-lc-rs` default cryptographic provider for `rustls`.
///
/// Safe to call multiple times — the second call is a no-op.
pub fn init_crypto() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
///
/// Verbosity levels expand scope progressively, keeping third-party crates
/// silent unless `RUST_LOG` explicitly enables them:
///
/// - `verbosity == 0`: `{own_module}=info`
/// - `verbosity == 1`: `{own_module}=debug`
/// - `verbosity == 2`: `uptrakit=debug`
/// - `verbosity >= 3`: `uptrakit=trace`
///
/// Each binary calls this explicitly in `main()` — the SDK does not install
/// the global tracing dispatcher autonomously.
pub fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

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
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    // Use registry-based subscriber so an OpenTelemetry layer can be added
    // later as a one-line change.
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .init();
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
