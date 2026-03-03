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

    uptrakit_service_sdk::init_tracing("uptrakit_scheduler", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    let mut handler = SchedulerHandler::new(args.poll_interval_secs);
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-scheduler",
        &args.common,
        &mut handler,
    )
    .await;
}
