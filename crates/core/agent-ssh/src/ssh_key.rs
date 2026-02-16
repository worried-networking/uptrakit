use std::io::Read;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rootcause::prelude::*;

use crate::db::entity::ssh_host::SshKeyType;
use crate::error::Error;

/// Read a private key from a file path, or from stdin if `path` is `-`.
pub fn read_private_key(path: &Path) -> crate::error::Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context_to::<Error>()?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).context_to::<Error>()
    }
}

/// Detect the SSH key type from PEM-encoded key content.
///
/// Supports:
/// - `BEGIN RSA PRIVATE KEY` → RSA (PKCS#1)
/// - `BEGIN EC PRIVATE KEY` → ECDSA (SEC1)
/// - `BEGIN OPENSSH PRIVATE KEY` → decode and inspect the key type string
/// - `BEGIN PRIVATE KEY` → PKCS#8 (decode and inspect OID)
pub fn detect_key_type(pem_content: &str) -> crate::error::Result<SshKeyType> {
    let trimmed = pem_content.trim();

    if trimmed.contains("BEGIN RSA PRIVATE KEY") {
        return Ok(SshKeyType::Rsa);
    }
    if trimmed.contains("BEGIN EC PRIVATE KEY") {
        return Ok(SshKeyType::Ecdsa);
    }
    if trimmed.contains("BEGIN OPENSSH PRIVATE KEY") {
        return detect_openssh_key_type(trimmed);
    }
    if trimmed.contains("BEGIN PRIVATE KEY") {
        return detect_pkcs8_key_type(trimmed);
    }

    bail!(Error::UnsupportedKeyType(
        "unrecognized PEM key format".to_string()
    ));
}

/// Decode an OpenSSH private key and inspect the key type string.
///
/// OpenSSH format: `openssh-key-v1\0` magic, then after some fields,
/// the key type is stored as a length-prefixed string.
fn detect_openssh_key_type(pem: &str) -> crate::error::Result<SshKeyType> {
    let b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();

    let decoded = STANDARD.decode(&b64).map_err(|e| {
        report!(Error::UnsupportedKeyType(format!(
            "base64 decode error: {e}"
        )))
    })?;

    // Check for OpenSSH magic: "openssh-key-v1\0"
    const MAGIC: &[u8] = b"openssh-key-v1\0";
    if decoded.len() < MAGIC.len() || &decoded[..MAGIC.len()] != MAGIC {
        bail!(Error::UnsupportedKeyType(
            "not a valid OpenSSH key".to_string()
        ));
    }

    // After the magic, the format has:
    //   string ciphername
    //   string kdfname
    //   string kdfoptions
    //   uint32 number of keys
    //   string pubkey (which itself starts with a string key_type)
    //
    // We need to skip ciphername, kdfname, kdfoptions, number_of_keys,
    // then read the pubkey blob's first string (key type).
    let mut pos = MAGIC.len();

    // Skip 3 strings: ciphername, kdfname, kdfoptions
    for _ in 0..3 {
        pos = skip_openssh_string(&decoded, pos)?;
    }

    // Read uint32 number of keys
    if pos + 4 > decoded.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }
    pos += 4; // skip number of keys

    // Read the public key blob (length-prefixed)
    if pos + 4 > decoded.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }
    let pubkey_len = read_u32_be(&decoded, pos) as usize;
    pos += 4;

    if pos + pubkey_len > decoded.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }

    // Inside the pubkey blob, the first field is the key type string
    let key_type_str = read_openssh_string(&decoded, pos)?;

    match key_type_str.as_str() {
        "ssh-ed25519" => Ok(SshKeyType::Ed25519),
        "ssh-rsa" => Ok(SshKeyType::Rsa),
        s if s.starts_with("ecdsa-sha2-") => Ok(SshKeyType::Ecdsa),
        other => bail!(Error::UnsupportedKeyType(format!(
            "unsupported OpenSSH key type: {other}"
        ))),
    }
}

/// Detect key type from a PKCS#8 PEM by inspecting known OID prefixes.
fn detect_pkcs8_key_type(pem: &str) -> crate::error::Result<SshKeyType> {
    let b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();

    let decoded = STANDARD.decode(&b64).map_err(|e| {
        report!(Error::UnsupportedKeyType(format!(
            "base64 decode error: {e}"
        )))
    })?;

    // OID for RSA: 1.2.840.113549.1.1.1 → 06 09 2A 86 48 86 F7 0D 01 01 01
    const RSA_OID: &[u8] = &[
        0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01,
    ];
    // OID for Ed25519: 1.3.101.112 → 06 03 2B 65 70
    const ED25519_OID: &[u8] = &[0x06, 0x03, 0x2B, 0x65, 0x70];
    // OID for EC (id-ecPublicKey): 1.2.840.10045.2.1 → 06 07 2A 86 48 CE 3D 02 01
    const EC_OID: &[u8] = &[0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];

    if contains_bytes(&decoded, ED25519_OID) {
        return Ok(SshKeyType::Ed25519);
    }
    if contains_bytes(&decoded, EC_OID) {
        return Ok(SshKeyType::Ecdsa);
    }
    if contains_bytes(&decoded, RSA_OID) {
        return Ok(SshKeyType::Rsa);
    }

    bail!(Error::UnsupportedKeyType(
        "unrecognized PKCS#8 key algorithm".to_string()
    ));
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn read_u32_be(data: &[u8], pos: usize) -> u32 {
    u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
}

fn skip_openssh_string(data: &[u8], pos: usize) -> crate::error::Result<usize> {
    if pos + 4 > data.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }
    let len = read_u32_be(data, pos) as usize;
    let end = pos + 4 + len;
    if end > data.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }
    Ok(end)
}

fn read_openssh_string(data: &[u8], pos: usize) -> crate::error::Result<String> {
    if pos + 4 > data.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }
    let len = read_u32_be(data, pos) as usize;
    let start = pos + 4;
    let end = start + len;
    if end > data.len() {
        bail!(Error::UnsupportedKeyType(
            "truncated OpenSSH key".to_string()
        ));
    }
    String::from_utf8(data[start..end].to_vec()).map_err(|e| {
        report!(Error::UnsupportedKeyType(format!(
            "key type string is not UTF-8: {e}"
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_rsa_pkcs1() {
        let pem = "\
-----BEGIN RSA PRIVATE KEY-----
MIIBogIBAAJBALRiMLAHudeSA/x3hB2f+2NRkT0E4Y7+FjzR4iQS3t4AfMKLe1T
Vb1roacMSH4W7Vi12j4GC0U1+n1gALR42gECAwEAAQJAMRz0HqB1h+JVpAMwrz0e
x7k7RFQHB0t1MiGNrk2u0gXwH+RqjGOmw/qNFN+0j7zuxF0lPmOJt6GUBBiJPAQ
oQIhAN1BFYHCbRTD1K+nFR/kFD0t+Iq3YB/LUUvhEtl8BT0pAiEA0+tB2VLF+e4T
h9rLSJU7B4ASPO2eRYBkrfxTDfDVvukCIGh0sSn4IAFNg7o+2WBTlqy6zPKmH60H
k7bxkTpFMlVhAiEAj+0Lv8DFnfEM7EavmbEBFHtIVkErs7t5T7aN+j3TykkCIBm5
pJpARR7obPW2RYQBC/9VPLtG/Kp0+m+B7Ny6Y5x7
-----END RSA PRIVATE KEY-----";
        let key_type = detect_key_type(pem).expect("should detect RSA");
        assert_eq!(key_type, SshKeyType::Rsa);
    }

    #[test]
    fn detect_ecdsa_sec1() {
        let pem = "\
-----BEGIN EC PRIVATE KEY-----
MHQCAQEEIBkg4LVWM9nuwNSk3yByxZpYRTBnVJk5GkMcEDqOzDQVoAcGBSuBBAAi
oWQDYgAEkVQ4HWPH7wNHTLMMHK1FGY4GmUmqVE0gEN1vZJ3RLxdEJ/Rx/h3GX7y1
-----END EC PRIVATE KEY-----";
        let key_type = detect_key_type(pem).expect("should detect ECDSA");
        assert_eq!(key_type, SshKeyType::Ecdsa);
    }

    #[test]
    fn detect_ed25519_openssh() {
        // Minimal synthetic OpenSSH key for testing detection.
        // Real keys are longer but we only need the header structure.
        let key = build_test_openssh_key("ssh-ed25519");
        let pem = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            STANDARD.encode(&key)
        );
        let key_type = detect_key_type(&pem).expect("should detect Ed25519");
        assert_eq!(key_type, SshKeyType::Ed25519);
    }

    #[test]
    fn detect_rsa_openssh() {
        let key = build_test_openssh_key("ssh-rsa");
        let pem = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            STANDARD.encode(&key)
        );
        let key_type = detect_key_type(&pem).expect("should detect RSA");
        assert_eq!(key_type, SshKeyType::Rsa);
    }

    #[test]
    fn detect_ecdsa_openssh() {
        let key = build_test_openssh_key("ecdsa-sha2-nistp256");
        let pem = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            STANDARD.encode(&key)
        );
        let key_type = detect_key_type(&pem).expect("should detect ECDSA");
        assert_eq!(key_type, SshKeyType::Ecdsa);
    }

    #[test]
    fn detect_unknown_format_fails() {
        let pem = "-----BEGIN UNKNOWN KEY-----\ndata\n-----END UNKNOWN KEY-----";
        assert!(detect_key_type(pem).is_err());
    }

    #[test]
    fn read_private_key_from_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key_path = dir.path().join("test.key");
        std::fs::write(&key_path, "test-key-content").expect("write");
        let content = read_private_key(&key_path).expect("read");
        assert_eq!(content, "test-key-content");
    }

    #[test]
    fn read_private_key_missing_file() {
        let path = std::path::Path::new("/nonexistent/path/key.pem");
        assert!(read_private_key(path).is_err());
    }

    /// Build a minimal synthetic OpenSSH key binary with the given key type string.
    fn build_test_openssh_key(key_type: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        // Magic
        buf.extend_from_slice(b"openssh-key-v1\0");
        // ciphername: "none"
        write_openssh_string(&mut buf, b"none");
        // kdfname: "none"
        write_openssh_string(&mut buf, b"none");
        // kdfoptions: empty
        write_openssh_string(&mut buf, b"");
        // number of keys: 1
        buf.extend_from_slice(&1u32.to_be_bytes());
        // pubkey blob: contains key type string + dummy data
        let mut pubkey = Vec::new();
        write_openssh_string(&mut pubkey, key_type.as_bytes());
        write_openssh_string(&mut pubkey, b"dummy-pubkey-data");
        write_openssh_string(&mut buf, &pubkey);
        // private key section (not needed for detection, add minimal data)
        write_openssh_string(&mut buf, b"dummy-private");
        buf
    }

    fn write_openssh_string(buf: &mut Vec<u8>, data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(data);
    }
}
