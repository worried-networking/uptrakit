mod cli;
mod handler;
mod nats_notifier;

use clap::Parser;

use cli::Args;
use handler::SchedulerHandler;

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

    init_tracing(args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    let mut handler = SchedulerHandler::new(args.poll_interval_secs);
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-scheduler",
        &args.common,
        &mut handler,
    )
    .await;
}

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
fn init_tracing(verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    if verbosity > 2 {
        eprintln!(
            "warning: -vvv or more has no additional effect; maximum verbosity is -vv (trace)"
        );
    }

    let directive = match verbosity {
        0 => "uptrakit=info".to_string(),
        1 => "uptrakit=debug".to_string(),
        _ => "uptrakit=trace".to_string(),
    };
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .init();
}
