use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "uptrakit-agent")]
#[command(about = "Uptrakit agent that connects to the controller")]
pub struct Args {
    /// Controller hostname or IP address
    #[arg(long)]
    pub host: String,

    /// Controller HTTPS/WSS port
    #[arg(long, default_value = "8443")]
    pub port: u16,

    /// Controller HTTP port (for fetching CA certificate)
    #[arg(long, default_value = "8080")]
    pub http_port: u16,
}
