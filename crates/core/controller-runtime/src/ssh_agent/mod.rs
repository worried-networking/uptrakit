//! Embedded SSH agent service for single-tenant controller deployments.

use std::collections::BTreeSet;

use uptrakit_agent_ssh_runtime::ssh_agent_capabilities as runtime_capabilities;
use uptrakit_wire::Capability;

pub(crate) fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

pub(crate) fn generate_ecies_keypair() -> rootcause::Result<(Option<Vec<u8>>, String)> {
    use rootcause::prelude::*;
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| {
        report!(std::io::Error::other(format!(
            "P-256 key generation failed: {e}"
        )))
    })?;
    let private_der = key_pair.serialize_der();
    let public_raw = key_pair.public_key_raw().to_vec();
    let public_b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&public_raw)
    };
    Ok((Some(private_der), public_b64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_agent_capabilities_includes_expected_set() {
        let caps = ssh_agent_capabilities();
        assert!(caps.contains(&uptrakit_wire::Capability::SoftwareDiscovery));
        assert!(caps.contains(&uptrakit_wire::Capability::SshRemote));
        assert!(caps.contains(&uptrakit_wire::Capability::UpdateHooks));
        assert!(caps.contains(&uptrakit_wire::Capability::UiSurfaces));
        assert!(caps.contains(&uptrakit_wire::Capability::GracefulShutdown));
    }

    #[cfg(feature = "interactive")]
    #[test]
    fn ssh_agent_capabilities_includes_interactive_when_feature_enabled() {
        let caps = ssh_agent_capabilities();
        assert!(caps.contains(&uptrakit_wire::Capability::InteractiveUpdates));
    }

    #[test]
    fn generate_ecies_keypair_produces_valid_pair() {
        let (private_key, public_key) = generate_ecies_keypair().unwrap();
        assert!(private_key.is_some());
        let private_key = private_key.expect("private key");
        assert!(!private_key.is_empty());
        assert!(!public_key.is_empty());

        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&public_key)
            .expect("valid base64");
        assert_eq!(decoded.len(), 65);
        assert_eq!(decoded[0], 0x04);
    }
}
