use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use rootcause::prelude::*;

const AGENT_STATE_FILE: &str = "agent.json";
const CA_CERT_FILE: &str = "ca.pem";

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
        Ok(())
    }
}

pub fn ca_cert_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CA_CERT_FILE)
}

pub fn save_ca_cert(data_dir: &Path, pem: &[u8]) -> Result<()> {
    let path = ca_cert_path(data_dir);
    std::fs::write(&path, pem).context_to::<Error>()?;
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
