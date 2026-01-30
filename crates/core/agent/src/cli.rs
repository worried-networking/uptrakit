use std::path::PathBuf;

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

    /// Data directory for persistent state (CA cert, agent.json).
    /// Supports `~` for home directory expansion.
    #[arg(long, default_value = "~/.uptrakit-agent")]
    pub data_dir: String,

    /// Friendly name for this agent (defaults to system hostname)
    #[arg(long)]
    pub friendly_name: Option<String>,

    /// Pre-shared enrollment token for auto-approval
    #[arg(long)]
    pub enrollment_token: Option<String>,

    /// Seconds between enrollment status polls
    #[arg(long, default_value = "30")]
    pub enrollment_poll_interval: u64,
}

impl Args {
    /// Resolve `data_dir` by expanding `~` to the user's home directory.
    pub fn resolve_data_dir(&self) -> Result<PathBuf, String> {
        let path = if self.data_dir.starts_with("~/") {
            let home = home_dir().ok_or("could not determine home directory")?;
            home.join(&self.data_dir[2..])
        } else if self.data_dir == "~" {
            home_dir().ok_or("could not determine home directory")?
        } else {
            PathBuf::from(&self.data_dir)
        };
        Ok(path)
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
