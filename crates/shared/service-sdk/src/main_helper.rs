//! Shared initialization and lifecycle error handling for service binaries.
//!
//! These helpers eliminate the per-service boilerplate in `main()`:
//! - [`init_crypto`] — install the `aws-lc-rs` default crypto provider.
//! - [`print_build_info`] — render build metadata and print to stdout.
//! - [`run_lifecycle_and_handle_errors`] — run the lifecycle and translate
//!   terminal errors into log messages or `process::exit`.

use crate::cli::CommonServiceArgs;
use crate::lifecycle::ServiceHandler;

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
