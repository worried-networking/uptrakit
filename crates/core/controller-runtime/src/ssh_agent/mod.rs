//! Embedded SSH agent service for single-tenant controller deployments.

use std::collections::BTreeSet;

use uptrakit_agent_ssh_runtime::ssh_agent_capabilities as runtime_capabilities;
use uptrakit_wire::Capability;

pub(crate) fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

pub(crate) fn generate_ecies_keypair() -> rootcause::Result<(Option<Vec<u8>>, String)> {
    use rootcause::prelude::*;
    let (private_der, public_b64) = uptrakit_service_sdk::generate_p256_keypair_for_ecies()
        .map_err(|e| {
            report!(std::io::Error::other(format!(
                "embedded SSH agent: ECIES keygen failed: {e}"
            )))
        })?;
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
}
