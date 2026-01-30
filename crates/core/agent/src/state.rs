use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use rootcause::prelude::*;

const AGENT_STATE_FILE: &str = "agent.json";
const CA_CERT_FILE: &str = "ca.pem";
const AGENT_CERT_FILE: &str = "agent.crt";
const AGENT_KEY_FILE: &str = "agent.key";
const CERT_NOT_AFTER_FILE: &str = "cert_not_after_ts";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub agent_id: String,
    pub enrollment_secret: String,
}

impl AgentState {
    pub fn load(data_dir: &Path) -> Result<Option<Self>> {
        let path = data_dir.join(AGENT_STATE_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path).context_to::<Error>()?;
        let state: Self = serde_json::from_str(&contents).context_to::<Error>()?;
        Ok(Some(state))
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = data_dir.join(AGENT_STATE_FILE);
        let contents = serde_json::to_string_pretty(self).context_to::<Error>()?;
        std::fs::write(&path, contents).context_to::<Error>()?;
        set_secure_permissions(&path)?;
        Ok(())
    }

    pub fn delete(data_dir: &Path) -> Result<()> {
        let path = data_dir.join(AGENT_STATE_FILE);
        if path.exists() {
            std::fs::remove_file(path).context_to::<Error>()?;
        }
        Ok(())
    }
}

pub struct AgentCertState {
    pub cert_pem: String,
    pub key_pem: String,
}

impl AgentCertState {
    pub fn load(data_dir: &Path) -> Result<Option<Self>> {
        let cert_path = data_dir.join(AGENT_CERT_FILE);
        let key_path = data_dir.join(AGENT_KEY_FILE);
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        let cert_pem = std::fs::read_to_string(&cert_path).context_to::<Error>()?;
        let key_pem = std::fs::read_to_string(&key_path).context_to::<Error>()?;
        Ok(Some(Self { cert_pem, key_pem }))
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let cert_path = data_dir.join(AGENT_CERT_FILE);
        let key_path = data_dir.join(AGENT_KEY_FILE);
        std::fs::write(&cert_path, &self.cert_pem).context_to::<Error>()?;
        set_secure_permissions(&cert_path)?;
        std::fs::write(&key_path, &self.key_pem).context_to::<Error>()?;
        set_secure_permissions(&key_path)?;
        Ok(())
    }

    pub fn delete(data_dir: &Path) -> Result<()> {
        let cert_path = data_dir.join(AGENT_CERT_FILE);
        let key_path = data_dir.join(AGENT_KEY_FILE);
        if cert_path.exists() {
            std::fs::remove_file(cert_path).context_to::<Error>()?;
        }
        if key_path.exists() {
            std::fs::remove_file(key_path).context_to::<Error>()?;
        }
        Ok(())
    }
}

pub fn ca_cert_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CA_CERT_FILE)
}

pub fn save_ca_cert(data_dir: &Path, pem: &[u8]) -> Result<()> {
    let path = ca_cert_path(data_dir);
    std::fs::write(&path, pem).context_to::<Error>()?;
    set_secure_permissions(&path)?;
    Ok(())
}

pub fn load_ca_cert(data_dir: &Path) -> Result<Option<Vec<u8>>> {
    let path = ca_cert_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read(&path).context_to::<Error>()?;
    Ok(Some(contents))
}

pub fn save_cert_not_after_ts(data_dir: &Path, ts: i64) -> Result<()> {
    let path = data_dir.join(CERT_NOT_AFTER_FILE);
    std::fs::write(&path, ts.to_string()).context_to::<Error>()?;
    set_secure_permissions(&path)?;
    Ok(())
}

pub fn load_cert_not_after_ts(data_dir: &Path) -> Result<Option<i64>> {
    let path = data_dir.join(CERT_NOT_AFTER_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path).context_to::<Error>()?;
    match contents.trim().parse::<i64>() {
        Ok(ts) => Ok(Some(ts)),
        Err(_) => Ok(None),
    }
}

pub fn delete_cert_not_after_ts(data_dir: &Path) -> Result<()> {
    let path = data_dir.join(CERT_NOT_AFTER_FILE);
    if path.exists() {
        std::fs::remove_file(path).context_to::<Error>()?;
    }
    Ok(())
}

fn set_secure_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .context_to::<Error>()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
