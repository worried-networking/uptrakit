mod cli;

use clap::Parser;
use uptrakit_mqtt_runtime::{MQTT_SERVICE_APP_NAME, MqttHandler};

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    let _ = args.max_tenants;

    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            MQTT_SERVICE_APP_NAME,
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    tracing::info!("starting uptrakit-mqtt service");

    let mut handler = MqttHandler::new();

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        MQTT_SERVICE_APP_NAME,
        &args.common,
        &mut handler,
    )
    .await;
}
