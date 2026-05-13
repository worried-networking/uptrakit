use crate::error::Result;
use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_directories::AppDirs;

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: Option<String>,
    /// PEM-encoded certificate of the trusted controller CA.
    /// `None` = use system roots. Set by `auth login --tofu` or `auth ca trust`.
    #[serde(default)]
    pub ca_pem: Option<String>,
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

pub async fn save_config(config: &Config) -> Result<()> {
    let dirs = app_dirs()?;
    dirs.ensure_config_dir().await.context_to()?;
    let path = dirs.config_path("config.json").context_to()?;
    let data = serde_json::to_string_pretty(config).context_to()?;
    uptrakit_directories::write_secure_file_str(&path, &data)
        .await
        .context_to()
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

pub async fn save_credentials(creds: &Credentials) -> Result<()> {
    let dirs = app_dirs()?;
    dirs.ensure_state_dir().await.context_to()?;
    let path = dirs.state_path("credentials.json").context_to()?;
    let data = serde_json::to_string_pretty(creds).context_to()?;
    uptrakit_directories::write_secure_file_str(&path, &data)
        .await
        .context_to()
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "test code: panics on failure are acceptable"
    )]

    use super::*;

    #[test]
    fn config_roundtrip_with_ca_pem() {
        let original = Config {
            server: Some("https://example.com".into()),
            ca_pem: Some("-----BEGIN CERTIFICATE-----\nABC\n-----END CERTIFICATE-----\n".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.server, original.server);
        assert_eq!(parsed.ca_pem, original.ca_pem);
    }

    #[test]
    fn config_roundtrip_without_ca_pem() {
        let original = Config {
            server: Some("https://example.com".into()),
            ca_pem: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.ca_pem, None);
    }

    #[test]
    fn config_missing_ca_pem_field_deserializes_as_none() {
        let json = r#"{"server":"https://example.com"}"#;
        let parsed: Config = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed.ca_pem, None);
    }
}
