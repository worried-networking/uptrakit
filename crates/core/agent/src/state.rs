use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use rootcause::prelude::*;

const AGENT_STATE_FILE: &str = "agent.json";
const CA_CERT_FILE: &str = "ca.pem";
const AGENT_CERT_FILE: &str = "agent.crt";
const AGENT_KEY_FILE: &str = "agent.key";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Agent identity UUID assigned by the controller.
    /// Serde alias "client_id" provides backward compatibility with existing agent.json files.
    #[serde(alias = "client_id")]
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

    /// Extract the certificate expiry timestamp from the PEM-encoded certificate.
    ///
    /// Returns the `not_after` timestamp as milliseconds since the Unix epoch.
    /// Returns `None` if the certificate cannot be parsed.
    pub fn cert_not_after_ms(&self) -> Option<i64> {
        cert_not_after_from_pem(&self.cert_pem)
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

/// Save a private key PEM to disk during enrollment (before cert exists).
pub fn save_agent_key(data_dir: &Path, key_pem: &str) -> Result<()> {
    let path = data_dir.join(AGENT_KEY_FILE);
    std::fs::write(&path, key_pem).context_to::<Error>()?;
    set_secure_permissions(&path)?;
    Ok(())
}

/// Load the private key PEM from disk.
pub fn load_agent_key(data_dir: &Path) -> Result<Option<String>> {
    let path = data_dir.join(AGENT_KEY_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path).context_to::<Error>()?;
    Ok(Some(contents))
}

/// Delete the agent key file.
pub fn delete_agent_key(data_dir: &Path) -> Result<()> {
    let path = data_dir.join(AGENT_KEY_FILE);
    if path.exists() {
        std::fs::remove_file(path).context_to::<Error>()?;
    }
    Ok(())
}

/// Extract the certificate expiry timestamp from a PEM-encoded certificate.
///
/// Returns the `not_after` timestamp as milliseconds since the Unix epoch.
/// Returns `None` if the certificate cannot be parsed.
pub fn cert_not_after_from_pem(cert_pem: &str) -> Option<i64> {
    let der = pem_to_der(cert_pem)?;
    let (_, cert) = x509_parser::parse_x509_certificate(&der).ok()?;
    let not_after = cert.validity().not_after;
    let datetime = not_after.to_datetime();
    // x509-parser returns ASN1Time, which we convert to timestamp
    let ts = datetime.unix_timestamp();
    // Convert seconds to milliseconds
    Some(ts * 1000)
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

/// Decode the first PEM block into DER bytes.
fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let start_marker = "-----BEGIN CERTIFICATE-----";
    let end_marker = "-----END CERTIFICATE-----";
    let start = pem.find(start_marker)? + start_marker.len();
    let end = pem[start..].find(end_marker)? + start;
    let b64: String = pem[start..end]
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();

    base64_decode(&b64)
}

/// Minimal base64 decoder (standard alphabet, no padding required).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn val(c: u8) -> Option<u8> {
        TABLE.iter().position(|&b| b == c).map(|p| p as u8)
    }

    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let mut buf: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            buf |= (val(b)? as u32) << (18 - 6 * i);
        }
        match chunk.len() {
            4 => {
                out.push((buf >> 16) as u8);
                out.push((buf >> 8) as u8);
                out.push(buf as u8);
            }
            3 => {
                out.push((buf >> 16) as u8);
                out.push((buf >> 8) as u8);
            }
            2 => {
                out.push((buf >> 16) as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn agent_state_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let state = AgentState {
            agent_id: "test-id".to_string(),
            enrollment_secret: "test-secret".to_string(),
        };
        state.save(dir.path()).expect("save");

        let loaded = AgentState::load(dir.path()).expect("load").expect("some");
        assert_eq!(loaded.agent_id, "test-id");
        assert_eq!(loaded.enrollment_secret, "test-secret");
    }

    #[test]
    fn agent_cert_state_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let state = AgentCertState {
            cert_pem: "test-cert".to_string(),
            key_pem: "test-key".to_string(),
        };
        state.save(dir.path()).expect("save");

        let loaded = AgentCertState::load(dir.path())
            .expect("load")
            .expect("some");
        assert_eq!(loaded.cert_pem, "test-cert");
        assert_eq!(loaded.key_pem, "test-key");
    }

    #[test]
    fn ca_cert_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let pem = b"test-ca-cert";
        save_ca_cert(dir.path(), pem).expect("save");

        let loaded = load_ca_cert(dir.path()).expect("load").expect("some");
        assert_eq!(loaded, pem);
    }

    #[test]
    fn pem_to_der_basic() {
        // Verify our minimal PEM decoder works for a trivial payload.
        let encoded = "-----BEGIN CERTIFICATE-----\nSGVsbG8=\n-----END CERTIFICATE-----\n";
        let der = pem_to_der(encoded).expect("decode");
        assert_eq!(der, b"Hello");
    }

    #[test]
    fn cert_not_after_from_real_cert() {
        // Generate a real self-signed cert
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        let cert = params.self_signed(&kp).expect("self-sign");
        let cert_pem = cert.pem();

        // The timestamp should be extracted successfully
        let ts = cert_not_after_from_pem(&cert_pem);
        assert!(ts.is_some());

        // Should be in the future (default cert validity)
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_millis() as i64;
        assert!(ts.unwrap() > now_ms);
    }

    #[test]
    fn cert_not_after_from_invalid_pem() {
        assert!(cert_not_after_from_pem("not a certificate").is_none());
    }

    #[test]
    fn file_permissions() {
        let dir = TempDir::new().expect("tempdir");
        let state = AgentState {
            agent_id: "test".to_string(),
            enrollment_secret: "secret".to_string(),
        };
        state.save(dir.path()).expect("save");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.path().join("agent.json");
            let metadata = std::fs::metadata(&path).expect("metadata");
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
