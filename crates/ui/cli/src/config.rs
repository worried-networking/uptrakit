use crate::error::Result;
use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_directories::AppDirs;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Credentials {
    #[serde(default)]
    pub token: Option<String>,
}

fn app_dirs() -> Result<AppDirs> {
    AppDirs::resolve("cli", None, None).context_to()
}

pub fn load_config() -> Result<Config> {
    let dirs = app_dirs()?;
    let path = dirs.config_path("config.json").context_to()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let data = std::fs::read_to_string(&path).context_to()?;
    serde_json::from_str(&data).context_to()
}

pub fn save_config(config: &Config) -> Result<()> {
    let dirs = app_dirs()?;
    let path = dirs.config_path("config.json").context_to()?;
    let data = serde_json::to_string_pretty(config).context_to()?;
    uptrakit_directories::write_secure_file_str(&path, &data).context_to()
}

pub fn load_credentials() -> Result<Credentials> {
    let dirs = app_dirs()?;
    let path = dirs.state_path("credentials.json").context_to()?;
    if !path.exists() {
        return Ok(Credentials::default());
    }
    let data = std::fs::read_to_string(&path).context_to()?;
    serde_json::from_str(&data).context_to()
}

pub fn save_credentials(creds: &Credentials) -> Result<()> {
    let dirs = app_dirs()?;
    dirs.ensure_state_dir().context_to()?;
    let path = dirs.state_path("credentials.json").context_to()?;
    let data = serde_json::to_string_pretty(creds).context_to()?;
    uptrakit_directories::write_secure_file_str(&path, &data).context_to()
}
