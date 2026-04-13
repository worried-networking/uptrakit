//! Embedded SSH agent service for single-tenant controller deployments.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use uptrakit_agent_ssh::runtime_support::AgentSshRuntimeSupport;
use uptrakit_agent_ssh::{
    ServiceExtensionProxy, reencrypt_ssh_to_v3, register_ssh_column_aad, ssh_pool,
};
use uptrakit_agent_ssh_runtime::{
    SshAgentIdentity, SshAgentRuntime, SshAgentRuntimeConfig, SshAgentSettings,
    ssh_agent_capabilities as runtime_capabilities,
};
use uptrakit_internal_wire::{Capability, DisconnectReason, ServiceTransport};

use crate::embedded::EmbeddedShutdownTokens;
use crate::embedded::types::EmbeddedTransport;

const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub(crate) fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

pub(crate) async fn run_embedded_ssh_agent(
    mut transport: EmbeddedTransport,
    tokens: EmbeddedShutdownTokens,
    state_dir: PathBuf,
    db: sea_orm::DatabaseConnection,
) {
    let ssh_state_dir = state_dir.join("embedded-ssh-agent");
    if let Err(error) = tokio::fs::create_dir_all(&ssh_state_dir).await {
        tracing::error!(error = %error, "failed to create embedded SSH agent state directory");
        return;
    }

    register_ssh_column_aad();
    reencrypt_ssh_to_v3(&db).await;

    let (private_key_der, encryption_public_key) = match generate_ecies_keypair() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(error = %error, "failed to generate ECIES key pair");
            return;
        }
    };

    let infra_bundles = {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config)
            .expect("plugin catalog must build successfully");
        Arc::new(catalog.create_infra_bundles(&catalog_config))
    };
    let support = AgentSshRuntimeSupport::new(
        db,
        ssh_state_dir.clone(),
        ssh_pool::SshConnectionPool::new(),
        Arc::new(ServiceExtensionProxy::new()),
        infra_bundles,
        false,
    );
    let mut runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::new(
        support,
        ssh_state_dir.join("update-freeze"),
    ));

    if let Err(error) = runtime
        .on_connected(
            &mut transport,
            SshAgentIdentity {
                service_id: None,
                private_key_der,
                encryption_public_key: Some(encryption_public_key),
            },
        )
        .await
    {
        tracing::error!(error = %error, "embedded SSH agent: failed to initialize runtime");
        return;
    }
    if let Err(error) = runtime
        .apply_settings(
            SshAgentSettings {
                ui_extensions_enabled: true,
                persist_tenant_id: false,
                tenant_id: None,
            },
            &mut transport,
        )
        .await
    {
        tracing::error!(error = %error, "embedded SSH agent: failed to apply initial settings");
        return;
    }

    tracing::info!("embedded SSH agent started");

    loop {
        tokio::select! {
            biased;

            () = tokens.drain.cancelled() => {
                tracing::info!("embedded SSH agent: draining");
                runtime
                    .shutdown(
                        &mut transport,
                        SHUTDOWN_TIMEOUT,
                        DisconnectReason::Shutdown,
                        uptrakit_agent_core::LoopOutcome::Shutdown,
                    )
                    .await;
                break;
            }

            () = tokens.abort.cancelled() => {
                tracing::info!("embedded SSH agent: aborting");
                break;
            }

            event = runtime.poll_event() => {
                if let Some(outcome) = runtime.handle_event(event, &mut transport).await {
                    tracing::warn!(?outcome, "embedded SSH agent: runtime requested loop exit");
                    break;
                }
            }

            msg = transport.transport_recv() => {
                let Some(msg) = msg else {
                    tracing::info!("embedded SSH agent: transport closed");
                    break;
                };

                if transport.is_yielded() {
                    tracing::debug!("embedded SSH agent: yielded, ignoring controller message");
                    continue;
                }

                runtime.handle_controller_message(msg, &mut transport).await;
            }
        }
    }

    runtime.drain_background_results(&mut transport).await;
    tracing::info!("embedded SSH agent stopped");
}

fn generate_ecies_keypair() -> Result<(Option<Vec<u8>>, String), String> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|error| format!("P-256 key generation failed: {error}"))?;
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
        assert!(caps.contains(&uptrakit_internal_wire::Capability::SoftwareDiscovery));
        assert!(caps.contains(&uptrakit_internal_wire::Capability::SshRemote));
        assert!(caps.contains(&uptrakit_internal_wire::Capability::UiExtensions));
        assert!(caps.contains(&uptrakit_internal_wire::Capability::GracefulShutdown));
    }

    #[cfg(feature = "interactive")]
    #[test]
    fn ssh_agent_capabilities_includes_interactive_when_feature_enabled() {
        let caps = ssh_agent_capabilities();
        assert!(caps.contains(&uptrakit_internal_wire::Capability::InteractiveUpdates));
    }

    #[test]
    fn generate_ecies_keypair_produces_valid_pair() {
        let (private_key, public_key) = generate_ecies_keypair().expect("keygen");
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
