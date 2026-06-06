mod cli;
mod handler;

use clap::Parser;

use cli::Args;
use handler::SchedulerHandler;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        let info = uptrakit_build_info::build_info!();
        print!("{}", info.render_human());
        return;
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    let mut handler = SchedulerHandler::new(args.poll_interval_secs);
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-scheduler",
        &args.common,
        &mut handler,
    )
    .await;
}
