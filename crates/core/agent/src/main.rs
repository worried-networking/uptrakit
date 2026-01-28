mod cli;
mod client;
mod error;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::Args;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("uptrakit_agent=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    if let Err(e) = run(&args).await {
        tracing::error!(error = %e, "agent failed");
        std::process::exit(1);
    }
}

async fn run(args: &Args) -> error::Result<()> {
    // Fetch CA certificate from controller
    let ca_pem = client::fetch_ca_certificate(&args.host, args.http_port).await?;

    // Build TLS connector with the fetched CA
    let tls_connector = client::build_tls_connector(&ca_pem)?;

    // Connect and perform ping/pong
    client::connect_and_ping(&args.host, args.port, tls_connector).await?;

    Ok(())
}
