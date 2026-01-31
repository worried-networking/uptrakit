use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

fn config_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| CliError::Other("HOME environment variable not set".into()))?;
    Ok(PathBuf::from(home).join(".config").join("uptrakit"))
}

pub fn load_config() -> Result<Config> {
    let path = config_dir()?.join("config.json");
    if !path.exists() {
        return Ok(Config::default());
    }
    let data = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(())
}

pub fn load_credentials() -> Result<Credentials> {
    let path = config_dir()?.join("credentials.json");
    if !path.exists() {
        return Ok(Credentials::default());
    }
    let data = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn save_credentials(creds: &Credentials) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("credentials.json");
    let data = serde_json::to_string_pretty(creds)?;
    std::fs::write(&path, &data)?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}
