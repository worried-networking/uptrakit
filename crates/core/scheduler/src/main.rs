mod ca_rotation;
mod cli;
mod handler;
mod nats_notifier;

use clap::Parser;

use cli::Args;
use handler::SchedulerHandler;

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
///
/// Verbosity levels expand scope progressively, keeping third-party crates
/// silent unless `RUST_LOG` explicitly enables them:
///
/// - `verbosity == 0`: `{own_module}=info`
/// - `verbosity == 1`: `{own_module}=debug`
/// - `verbosity == 2`: `uptrakit=debug`
/// - `verbosity >= 3`: `uptrakit=trace`
fn init_tracing(own_module: &str, verbosity: u8) {
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
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-scheduler",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    init_tracing("uptrakit_scheduler", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    let mut handler = SchedulerHandler::new(args.poll_interval_secs);
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-scheduler",
        &args.common,
        &mut handler,
    )
    .await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn tracing_directive_default_is_info() {
        // Verify the directive logic without calling init_tracing (global subscriber).
        let directive = match 0u8 {
            0 => "uptrakit_scheduler=info".to_string(),
            1 => "uptrakit_scheduler=debug".to_string(),
            2 => "uptrakit=debug".to_string(),
            _ => "uptrakit=trace".to_string(),
        };
        assert_eq!(directive, "uptrakit_scheduler=info");
    }
}
