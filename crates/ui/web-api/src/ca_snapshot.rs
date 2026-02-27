use std::fmt;
use std::sync::Arc;

use zeroize::Zeroizing;

/// Type alias for the watch receiver carrying public CA snapshot data.
pub type CaSnapshotReceiver = tokio::sync::watch::Receiver<CaPublicSnapshot>;

/// Public-facing CA material — safe to clone, debug, and distribute to all handlers.
#[derive(Clone, Debug)]
pub struct CaPublicSnapshot {
    pub active_cert_pem: String,
    pub active_fingerprint: String,
    pub previous_cert_pem: Option<String>,
    pub previous_fingerprint: Option<String>,
    pub trusted_cas: Vec<TrustedCaPublic>,
    pub trusted_ca_cns: Vec<String>,
    pub bundle_pem: String,
    pub bundle_hash: String,
    pub managed: bool,
    pub active_not_after: time::OffsetDateTime,
    pub pki_addr: Option<String>,
}

/// Public part of a trusted CA (no private key).
#[derive(Clone, Debug)]
pub struct TrustedCaPublic {
    pub cert_pem: String,
    pub fingerprint: String,
    pub not_after: time::OffsetDateTime,
}

/// Private key material for a single trusted CA.
pub struct TrustedCaKey {
    pub fingerprint: String,
    pub key_pem: Zeroizing<String>,
}

/// Private key store — NOT Clone, NOT Debug. Only accessible by OCSP, CRL, and cert signers.
pub struct CaKeyStore {
    pub active_key_pem: Zeroizing<String>,
    pub previous_key_pem: Option<Zeroizing<String>>,
    pub trusted_ca_keys: Vec<TrustedCaKey>,
}

impl fmt::Debug for CaKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaKeyStore")
            .field("active_key_pem", &"[REDACTED]")
            .field(
                "previous_key_pem",
                &self.previous_key_pem.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "trusted_ca_keys",
                &format!("[{} keys REDACTED]", self.trusted_ca_keys.len()),
            )
            .finish()
    }
}

/// Type alias for the shared key store.
pub type CaKeyStoreRef = Arc<tokio::sync::RwLock<CaKeyStore>>;

/// Input parameters for building both a public snapshot and a key store.
pub struct SplitSnapshotInput {
    pub active_cert_pem: String,
    pub active_key_pem: String,
    pub active_fingerprint: String,
    pub previous_cert_pem: Option<String>,
    pub previous_key_pem: Option<String>,
    pub previous_fingerprint: Option<String>,
    pub trusted_cas_public: Vec<TrustedCaPublic>,
    pub trusted_ca_keys: Vec<TrustedCaKey>,
    pub trusted_ca_cns: Vec<String>,
    pub bundle_pem: String,
    pub bundle_hash: String,
    pub managed: bool,
    pub active_not_after: time::OffsetDateTime,
    pub pki_addr: Option<String>,
}

/// Build both a public snapshot and a key store from component parts.
pub fn split_snapshot(input: SplitSnapshotInput) -> (CaPublicSnapshot, CaKeyStore) {
    let public = CaPublicSnapshot {
        active_cert_pem: input.active_cert_pem,
        active_fingerprint: input.active_fingerprint,
        previous_cert_pem: input.previous_cert_pem,
        previous_fingerprint: input.previous_fingerprint,
        trusted_cas: input.trusted_cas_public,
        trusted_ca_cns: input.trusted_ca_cns,
        bundle_pem: input.bundle_pem,
        bundle_hash: input.bundle_hash,
        managed: input.managed,
        active_not_after: input.active_not_after,
        pki_addr: input.pki_addr,
    };
    let keys = CaKeyStore {
        active_key_pem: Zeroizing::new(input.active_key_pem),
        previous_key_pem: input.previous_key_pem.map(Zeroizing::new),
        trusted_ca_keys: input.trusted_ca_keys,
    };
    (public, keys)
}
