//! TOFU (Trust-On-First-Use) TLS verification types.
//!
//! Provides:
//! - [`Sha256Hash`] — a 32-byte SHA-256 hash with colon-hex parsing/display.
//! - [`TofuMode`] + [`TofuConfig`] — server-cert trust mode, parsed from CLI flags.
//! - [`ModeBasedVerifier`] — a `rustls` [`ServerCertVerifier`] that dispatches
//!   to the active mode.
//!
//! # Errors
//!
//! [`Sha256Hash::from_str`] returns [`Sha256ParseError`].
//! [`TofuConfig::from_flags`] returns [`TofuConfigError`].
//! [`ModeBasedVerifier`] methods return [`rustls::Error`].

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

// ── Sha256Hash ────────────────────────────────────────────────────────────────

/// SHA-256 hash, parsed from `aa:bb:cc:...` (colon-separated) or compact hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Hash([u8; 32]);

impl Sha256Hash {
    /// Construct from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return a reference to the underlying 32-byte array.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Return the hash as colon-separated lowercase hex, e.g. `aa:bb:cc:...`.
    pub fn to_colon_hex(&self) -> String {
        let mut s = String::with_capacity(32 * 3 - 1);
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                s.push(':');
            }
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}

impl fmt::Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Hash({})", self.to_colon_hex())
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_colon_hex())
    }
}

impl FromStr for Sha256Hash {
    type Err = Sha256ParseError;

    /// Parse from compact hex (64 chars) or colon-separated hex (32 pairs).
    ///
    /// # Errors
    ///
    /// Returns [`Sha256ParseError::BadLength`] if the cleaned string is not 64 hex chars.
    /// Returns [`Sha256ParseError::BadHex`] if any byte pair is not valid hexadecimal.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let cleaned: String = s.chars().filter(|c| *c != ':' && *c != ' ').collect();
        if cleaned.len() != 64 {
            return Err(Sha256ParseError::BadLength(cleaned.len()));
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in cleaned.as_bytes().chunks(2).enumerate() {
            let byte_str = std::str::from_utf8(chunk).map_err(|_e| {
                Sha256ParseError::BadHex(String::from_utf8_lossy(chunk).into_owned())
            })?;
            let byte = u8::from_str_radix(byte_str, 16)
                .map_err(|_e| Sha256ParseError::BadHex(byte_str.to_owned()))?;
            // `i` is bounded to 0..32 because `cleaned` has exactly 64 ASCII chars
            // and `chunks(2)` produces exactly 32 chunks; `bytes` is `[u8; 32]`.
            if let Some(slot) = bytes.get_mut(i) {
                *slot = byte;
            }
        }
        Ok(Self(bytes))
    }
}

/// Error returned by [`Sha256Hash::from_str`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Sha256ParseError {
    /// The input (after stripping colons/spaces) was not exactly 64 hex characters.
    #[error("expected 64 hex chars (with or without colons), got {0}")]
    BadLength(usize),
    /// A two-character hex byte was invalid.
    #[error("invalid hex byte: {0:?}")]
    BadHex(String),
}

// ── TofuMode + TofuConfig ─────────────────────────────────────────────────────

/// Server-cert trust mode. Selected at Service boot via mutually-exclusive CLI flags.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum TofuMode {
    /// Verify against trust composition (controller-CA bundle ± opt-in public/native roots).
    System,
    /// Accept any chain whose CA bundle SHA-256 matches the pinned value.
    PinFingerprint(Sha256Hash),
    /// Accept any chain where any cert's SubjectPublicKeyInfo SHA-256 matches the pinned value.
    PinSpki(Sha256Hash),
    /// Accept any chain; log WARN on every connection. Hostname check is forced off.
    InsecureTofu,
}

/// Resolved TLS trust configuration built from CLI flags.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct TofuConfig {
    /// Active trust mode.
    pub mode: TofuMode,
    /// Whether hostname verification is skipped.
    pub skip_hostname: bool,
    /// Operator-supplied fingerprint acknowledgement (required with `InsecureTofu`).
    pub fingerprint_acknowledge: Option<Sha256Hash>,
}

impl TofuConfig {
    /// Build from the four mutually-exclusive CLI flag values.
    ///
    /// At most one of `fingerprint`, `spki`, `insecure` may be set.
    /// `skip_hostname` is only permitted with a pin or insecure mode.
    /// `fingerprint_acknowledge` is only permitted with `insecure`.
    ///
    /// # Errors
    ///
    /// Returns [`TofuConfigError::MultipleModes`] if more than one exclusive flag is set.
    /// Returns [`TofuConfigError::SkipHostnameRequiresPinOrInsecure`] when `skip_hostname`
    /// is set without a pin or insecure mode.
    /// Returns [`TofuConfigError::AcknowledgeRequiresInsecure`] when
    /// `fingerprint_acknowledge` is set without the insecure mode.
    pub fn from_flags(
        fingerprint: Option<Sha256Hash>,
        spki: Option<Sha256Hash>,
        insecure: bool,
        skip_hostname: bool,
        fingerprint_acknowledge: Option<Sha256Hash>,
    ) -> Result<Self, TofuConfigError> {
        let mode = match (fingerprint, spki, insecure) {
            (Some(h), None, false) => TofuMode::PinFingerprint(h),
            (None, Some(h), false) => TofuMode::PinSpki(h),
            (None, None, true) => TofuMode::InsecureTofu,
            (None, None, false) => TofuMode::System,
            _ => return Err(TofuConfigError::MultipleModes),
        };

        let effective_skip = matches!(mode, TofuMode::InsecureTofu) || skip_hostname;

        if skip_hostname && matches!(mode, TofuMode::System) {
            return Err(TofuConfigError::SkipHostnameRequiresPinOrInsecure);
        }

        if fingerprint_acknowledge.is_some() && !matches!(mode, TofuMode::InsecureTofu) {
            return Err(TofuConfigError::AcknowledgeRequiresInsecure);
        }

        Ok(Self {
            mode,
            skip_hostname: effective_skip,
            fingerprint_acknowledge,
        })
    }
}

/// Error returned by [`TofuConfig::from_flags`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum TofuConfigError {
    /// More than one exclusive trust-mode flag was set.
    #[error("at most one of --tofu-fingerprint, --tofu-spki, --tofu-insecure may be set")]
    MultipleModes,
    /// `--tofu-skip-hostname` requires a pin or insecure mode.
    #[error(
        "--tofu-skip-hostname requires one of --tofu-fingerprint, --tofu-spki, --tofu-insecure"
    )]
    SkipHostnameRequiresPinOrInsecure,
    /// `--tofu-fingerprint-acknowledge` is only valid with `--tofu-insecure`.
    #[error("--tofu-fingerprint-acknowledge is only valid with --tofu-insecure")]
    AcknowledgeRequiresInsecure,
}

// ── Internal crypto helpers ───────────────────────────────────────────────────

pub(crate) fn sha256_of_bytes(bytes: &[u8]) -> Sha256Hash {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Sha256Hash::from_bytes(out)
}

pub(crate) fn spki_sha256(cert_der: &[u8]) -> Result<Sha256Hash, x509_cert::der::Error> {
    use x509_cert::der::{Decode, Encode as _};
    let cert = x509_cert::Certificate::from_der(cert_der)?;
    let spki_der = cert.tbs_certificate.subject_public_key_info.to_der()?;
    Ok(sha256_of_bytes(&spki_der))
}

fn cert_dns_sans_match(cert: &x509_cert::Certificate, name: &str) -> bool {
    use der::Decode as _;
    use x509_cert::ext::pkix::{SubjectAltName, name::GeneralName};

    let Some(exts) = &cert.tbs_certificate.extensions else {
        return false;
    };
    let oid = const_oid::db::rfc5280::ID_CE_SUBJECT_ALT_NAME;
    for ext in exts {
        if ext.extn_id != oid {
            continue;
        }
        if let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes()) {
            for gn in san.0 {
                if let GeneralName::DnsName(dns) = gn
                    && dns.as_str().eq_ignore_ascii_case(name)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn cert_ip_sans_match(cert: &x509_cert::Certificate, dialed: &[u8]) -> bool {
    use der::Decode as _;
    use x509_cert::ext::pkix::{SubjectAltName, name::GeneralName};

    let Some(exts) = &cert.tbs_certificate.extensions else {
        return false;
    };
    let oid = const_oid::db::rfc5280::ID_CE_SUBJECT_ALT_NAME;
    for ext in exts {
        if ext.extn_id != oid {
            continue;
        }
        if let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes()) {
            for gn in san.0 {
                if let GeneralName::IpAddress(ip_bytes) = gn
                    && ip_bytes.as_bytes() == dialed
                {
                    return true;
                }
            }
        }
    }
    false
}

// ── ModeBasedVerifier ─────────────────────────────────────────────────────────

/// Mode-dispatched server cert verifier.
///
/// Wraps the standard webpki verifier and replaces/disables checks per the
/// active [`TofuMode`].
#[non_exhaustive]
#[derive(Debug)]
pub struct ModeBasedVerifier {
    /// Active TOFU configuration.
    pub config: TofuConfig,
    /// Raw PEM bytes of the controller's CA certificate bundle (used for fingerprint pinning).
    pub controller_ca_pem: Vec<u8>,
    /// Underlying system verifier (used for System mode and for handshake-signature checks).
    pub system_verifier: Arc<dyn ServerCertVerifier>,
}

impl ModeBasedVerifier {
    /// Create a new `ModeBasedVerifier`.
    pub fn new(
        config: TofuConfig,
        controller_ca_pem: Vec<u8>,
        system_verifier: Arc<dyn ServerCertVerifier>,
    ) -> Self {
        Self {
            config,
            controller_ca_pem,
            system_verifier,
        }
    }
}

impl ServerCertVerifier for ModeBasedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match &self.config.mode {
            TofuMode::System => self.system_verifier.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
            TofuMode::PinFingerprint(expected) => {
                let actual = sha256_of_bytes(&self.controller_ca_pem);
                if &actual != expected {
                    tracing::warn!(
                        expected = %expected,
                        actual = %actual,
                        "TOFU fingerprint mismatch"
                    );
                    return Err(TlsError::InvalidCertificate(
                        rustls::CertificateError::Other(rustls::OtherError(Arc::new(
                            std::io::Error::other("fingerprint mismatch"),
                        ))),
                    ));
                }
                if !self.config.skip_hostname {
                    self.verify_hostname(end_entity, server_name)?;
                }
                Ok(ServerCertVerified::assertion())
            }
            TofuMode::PinSpki(expected) => {
                let chain_matches = std::iter::once(end_entity.as_ref())
                    .chain(intermediates.iter().map(|c| c.as_ref()))
                    .any(|c| match spki_sha256(c) {
                        Ok(h) => &h == expected,
                        Err(_) => false,
                    });
                if !chain_matches {
                    return Err(TlsError::InvalidCertificate(
                        rustls::CertificateError::Other(rustls::OtherError(Arc::new(
                            std::io::Error::other("SPKI not in chain"),
                        ))),
                    ));
                }
                if !self.config.skip_hostname {
                    self.verify_hostname(end_entity, server_name)?;
                }
                Ok(ServerCertVerified::assertion())
            }
            TofuMode::InsecureTofu => {
                tracing::warn!("TLS verification disabled (insecure-tofu); accepting any cert");
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    // Handshake-signature verification is delegated to the system verifier even in pin/insecure
    // modes. Pin/insecure modes relax chain trust only — the server must still hold the private key.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.system_verifier
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.system_verifier
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.system_verifier.supported_verify_schemes()
    }
}

impl ModeBasedVerifier {
    fn verify_hostname(
        &self,
        end_entity: &CertificateDer<'_>,
        server_name: &ServerName<'_>,
    ) -> Result<(), TlsError> {
        use x509_cert::der::Decode as _;
        let cert = x509_cert::Certificate::from_der(end_entity.as_ref()).map_err(|e| {
            TlsError::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
                Arc::new(std::io::Error::other(e.to_string())),
            )))
        })?;
        match server_name {
            ServerName::DnsName(n) => {
                if cert_dns_sans_match(&cert, n.as_ref()) {
                    Ok(())
                } else {
                    Err(TlsError::InvalidCertificate(
                        rustls::CertificateError::NotValidForName,
                    ))
                }
            }
            ServerName::IpAddress(ip) => {
                let dialed_bytes: Vec<u8> = match ip {
                    rustls::pki_types::IpAddr::V4(v) => v.as_ref().to_vec(),
                    rustls::pki_types::IpAddr::V6(v) => v.as_ref().to_vec(),
                };
                if cert_ip_sans_match(&cert, &dialed_bytes) {
                    Ok(())
                } else {
                    Err(TlsError::InvalidCertificate(
                        rustls::CertificateError::NotValidForName,
                    ))
                }
            }
            _ => Err(TlsError::InvalidCertificate(
                rustls::CertificateError::NotValidForName,
            )),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compact_hex() {
        let s = "aabbccdd00112233445566778899aabbccdd0011223344556677889900112233";
        let h: Sha256Hash = s.parse().expect("parse");
        assert_eq!(h.as_bytes()[0], 0xaa);
        assert_eq!(h.as_bytes()[31], 0x33);
    }

    #[test]
    fn parse_colon_hex() {
        let s = "aa:bb:cc:dd:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:00:11:22:33:44:55:66:77:88:99:00:11:22:33";
        let h: Sha256Hash = s.parse().expect("parse");
        assert_eq!(h.as_bytes()[0], 0xaa);
        assert_eq!(h.as_bytes()[31], 0x33);
    }

    #[test]
    fn parse_rejects_bad_length() {
        assert!(matches!(
            "abcd".parse::<Sha256Hash>(),
            Err(Sha256ParseError::BadLength(4))
        ));
    }

    #[test]
    fn round_trip_to_colon_hex() {
        let s = "aabbccdd00112233445566778899aabbccdd0011223344556677889900112233";
        let h: Sha256Hash = s.parse().expect("parse");
        // Round trip: display is colon-hex, strip colons to get back to compact
        let displayed = h.to_colon_hex();
        let stripped: String = displayed.chars().filter(|c| *c != ':').collect();
        assert_eq!(stripped, s);
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn hash() -> Sha256Hash {
        "aa".repeat(32).parse().expect("hash")
    }

    #[test]
    fn no_flags_is_system_mode() {
        let cfg = TofuConfig::from_flags(None, None, false, false, None).expect("system mode");
        assert!(matches!(cfg.mode, TofuMode::System));
        assert!(!cfg.skip_hostname);
    }

    #[test]
    fn fingerprint_only_is_pin_fingerprint() {
        let cfg =
            TofuConfig::from_flags(Some(hash()), None, false, false, None).expect("fingerprint");
        assert!(matches!(cfg.mode, TofuMode::PinFingerprint(_)));
    }

    #[test]
    fn spki_only_is_pin_spki() {
        let cfg = TofuConfig::from_flags(None, Some(hash()), false, false, None).expect("spki");
        assert!(matches!(cfg.mode, TofuMode::PinSpki(_)));
    }

    #[test]
    fn insecure_implies_skip_hostname() {
        let cfg = TofuConfig::from_flags(None, None, true, false, None).expect("insecure mode");
        assert!(cfg.skip_hostname, "insecure mode must force skip_hostname");
    }

    #[test]
    fn two_pin_flags_rejected() {
        let r = TofuConfig::from_flags(Some(hash()), Some(hash()), false, false, None);
        assert!(matches!(r, Err(TofuConfigError::MultipleModes)));
    }

    #[test]
    fn skip_hostname_without_pin_rejected() {
        let r = TofuConfig::from_flags(None, None, false, true, None);
        assert!(matches!(
            r,
            Err(TofuConfigError::SkipHostnameRequiresPinOrInsecure)
        ));
    }

    #[test]
    fn acknowledge_without_insecure_rejected() {
        let r = TofuConfig::from_flags(Some(hash()), None, false, false, Some(hash()));
        assert!(matches!(
            r,
            Err(TofuConfigError::AcknowledgeRequiresInsecure)
        ));
    }
}
