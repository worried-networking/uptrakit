mod cli;

use clap::Parser;
use uptrakit_mqtt_runtime::bootstrap::{MQTT_SERVICE_APP_NAME, new_handler};

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    let _ = args.max_tenants;

    if args.common.version {
        let info = uptrakit_build_info::build_info!();
        print!("{}", info.render_human());
        return;
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    tracing::info!("starting uptrakit-mqtt service");

    let mut handler = new_handler();

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        MQTT_SERVICE_APP_NAME,
        &args.common,
        &mut handler,
    )
    .await;
}
