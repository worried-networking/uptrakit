//! Encrypt/decrypt plugin config fields in [`ControllerMessage`]s for NATS transit.
//!
//! Plugin configs (`PluginAssignment.config`, `DiscoveryPluginAssignment.config`,
//! `ExecuteBatchHostPackageUpdatePayload.plugin_config`) may contain sensitive
//! credentials (API tokens, registry passwords). These configs are encrypted
//! before NATS publication and decrypted on receipt.
//!
//! ## Mechanism
//!
//! - [`encrypt_message_configs`] — called before NATS publication. Serializes
//!   each `serde_json::Value` config to JSON, encrypts with
//!   [`uptrakit_crypto::encrypt_str`] (AES-256-GCM), and replaces the config
//!   with `Value::String("ENC:v1:...")`.
//! - [`decrypt_message_configs`] — called after NATS deserialization. Detects
//!   encrypted config strings via [`uptrakit_crypto::is_encrypted`], decrypts
//!   with [`decrypt_str`](uptrakit_crypto::decrypt_str), and restores the
//!   original `serde_json::Value`.
//!
//! ## Error handling
//!
//! Encryption or decryption failures are logged at `warn` level and the config
//! is left unchanged (graceful degradation). The agent will receive an encrypted
//! string instead of a JSON object and fail the plugin operation, but no crash
//! or data loss occurs.
//!
//! ## Backward compatibility
//!
//! [`decrypt_message_configs`] checks each config field: if it is already a
//! `Value::Object` (not encrypted), it is returned unchanged. This ensures
//! compatibility during rolling upgrades where one controller publishes
//! unencrypted messages while another has the new code.

use serde_json::Value;
use uptrakit_internal_wire::ControllerMessage;

/// Encrypt credential-bearing config fields in a [`ControllerMessage`] before
/// NATS publication.
///
/// Walks the message and replaces each `serde_json::Value` config field with
/// `Value::String(encrypt_str(json_string))`. Non-credential-bearing variants
/// are returned unchanged.
pub fn encrypt_message_configs(mut msg: ControllerMessage) -> ControllerMessage {
    match &mut msg {
        ControllerMessage::CheckVersions(payload) => {
            for assignment in &mut payload.assignments {
                if let Some(dv) = &mut assignment.detect_version {
                    dv.config = encrypt_config(&dv.config);
                }
                if let Some(fr) = &mut assignment.fetch_releases {
                    fr.config = encrypt_config(&fr.config);
                }
            }
        }
        ControllerMessage::ExecuteUpdate(payload) => {
            payload.execute_update_plugin.config =
                encrypt_config(&payload.execute_update_plugin.config);
            if let Some(dv) = &mut payload.detect_version_plugin {
                dv.config = encrypt_config(&dv.config);
            }
        }
        ControllerMessage::ExecuteBatchHostPackageUpdate(payload) => {
            payload.plugin_config = encrypt_config(&payload.plugin_config);
        }
        ControllerMessage::DiscoverSoftware(payload) => {
            for plugin in &mut payload.plugins {
                plugin.config = encrypt_config(&plugin.config);
            }
        }
        _ => {}
    }
    msg
}

/// Decrypt credential-bearing config fields received from NATS.
///
/// Walks the message and replaces each encrypted `Value::String("ENC:...")`
/// config field with the decrypted `serde_json::Value`. Non-encrypted or
/// non-credential-bearing variants are returned unchanged.
pub fn decrypt_message_configs(mut msg: ControllerMessage) -> ControllerMessage {
    match &mut msg {
        ControllerMessage::CheckVersions(payload) => {
            for assignment in &mut payload.assignments {
                if let Some(dv) = &mut assignment.detect_version {
                    dv.config = decrypt_config(&dv.config);
                }
                if let Some(fr) = &mut assignment.fetch_releases {
                    fr.config = decrypt_config(&fr.config);
                }
            }
        }
        ControllerMessage::ExecuteUpdate(payload) => {
            payload.execute_update_plugin.config =
                decrypt_config(&payload.execute_update_plugin.config);
            if let Some(dv) = &mut payload.detect_version_plugin {
                dv.config = decrypt_config(&dv.config);
            }
        }
        ControllerMessage::ExecuteBatchHostPackageUpdate(payload) => {
            payload.plugin_config = decrypt_config(&payload.plugin_config);
        }
        ControllerMessage::DiscoverSoftware(payload) => {
            for plugin in &mut payload.plugins {
                plugin.config = decrypt_config(&plugin.config);
            }
        }
        _ => {}
    }
    msg
}

/// Encrypt a single config `Value` to an encrypted string.
///
/// Returns the original value unchanged on failure (graceful degradation).
fn encrypt_config(config: &Value) -> Value {
    let json_str = match serde_json::to_string(config) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize config for encryption; leaving unchanged");
            return config.clone();
        }
    };
    match uptrakit_crypto::encrypt_str(&json_str) {
        Ok(encrypted) => Value::String(encrypted),
        Err(e) => {
            tracing::warn!(error = %e, "failed to encrypt config; leaving unchanged");
            config.clone()
        }
    }
}

/// Decrypt a single config `Value` from an encrypted string.
///
/// If the value is already a `Value::Object` or `Value::Array` (not encrypted),
/// it is returned unchanged. Returns the original value on failure (graceful
/// degradation).
fn decrypt_config(config: &Value) -> Value {
    let encrypted_str = match config.as_str() {
        Some(s) if uptrakit_crypto::is_encrypted(s) => s,
        _ => {
            // Not an encrypted string — return unchanged (backward compat).
            return config.clone();
        }
    };
    match uptrakit_crypto::decrypt_str(encrypted_str) {
        Ok(plaintext) => match serde_json::from_str(&plaintext) {
            Ok(val) => val,
            Err(e) => {
                tracing::warn!(error = %e, "failed to deserialize decrypted config; leaving unchanged");
                config.clone()
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "failed to decrypt config; leaving unchanged");
            config.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uptrakit_internal_wire::*;

    /// Initialize crypto with a test key (idempotent).
    fn init_test_crypto() {
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
    }


    #[test]
    fn encrypt_then_decrypt_roundtrip_check_versions() {
        init_test_crypto();

        let original_config = json!({"auth_token": "secret123"});
        let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
            host_machine_id: "host-1".to_string(),
            assignments: vec![VersionCheckAssignment {
                software_item_id: uuid::Uuid::nil(),
                name: "test".to_string(),
                detect_version: Some(PluginAssignment {
                    plugin_type: PluginType::ReleasesGithub,
                    package_identifier: "pkg".to_string(),
                    config: original_config.clone(),
                }),
                fetch_releases: Some(PluginAssignment {
                    plugin_type: PluginType::ReleasesGithub,
                    package_identifier: "pkg".to_string(),
                    config: original_config.clone(),
                }),
                host_package_id: None,
            }],
        });

        let encrypted = encrypt_message_configs(msg);

        // Verify configs are encrypted strings.
        if let ControllerMessage::CheckVersions(ref p) = encrypted {
            let dv = p.assignments[0].detect_version.as_ref().unwrap();
            assert!(
                dv.config.is_string(),
                "detect_version config should be an encrypted string"
            );
            assert!(
                uptrakit_crypto::is_encrypted(dv.config.as_str().unwrap()),
                "detect_version config should start with ENC:"
            );

            let fr = p.assignments[0].fetch_releases.as_ref().unwrap();
            assert!(
                fr.config.is_string(),
                "fetch_releases config should be an encrypted string"
            );
        } else {
            panic!("expected CheckVersions variant");
        }

        // Decrypt and verify roundtrip.
        let decrypted = decrypt_message_configs(encrypted);
        if let ControllerMessage::CheckVersions(ref p) = decrypted {
            let dv = p.assignments[0].detect_version.as_ref().unwrap();
            assert_eq!(dv.config, original_config);
            let fr = p.assignments[0].fetch_releases.as_ref().unwrap();
            assert_eq!(fr.config, original_config);
        } else {
            panic!("expected CheckVersions variant");
        }
    }

    #[test]
    fn encrypt_then_decrypt_roundtrip_execute_update() {
        init_test_crypto();

        let original_config = json!({"registry_password": "p@ss"});
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            host_machine_id: "host-1".to_string(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "test".to_string(),
            to_version: "1.0.0".to_string(),
            detect_version_plugin: Some(PluginAssignment {
                plugin_type: PluginType::GenericShell,
                package_identifier: "pkg".to_string(),
                config: original_config.clone(),
            }),
            execute_update_plugin: PluginAssignment {
                plugin_type: PluginType::GenericShell,
                package_identifier: "pkg".to_string(),
                config: original_config.clone(),
            },
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: 300,
        }));

        let encrypted = encrypt_message_configs(msg);
        if let ControllerMessage::ExecuteUpdate(ref p) = encrypted {
            assert!(p.execute_update_plugin.config.is_string());
            assert!(p.detect_version_plugin.as_ref().unwrap().config.is_string());
        } else {
            panic!("expected ExecuteUpdate variant");
        }

        let decrypted = decrypt_message_configs(encrypted);
        if let ControllerMessage::ExecuteUpdate(ref p) = decrypted {
            assert_eq!(p.execute_update_plugin.config, original_config);
            assert_eq!(
                p.detect_version_plugin.as_ref().unwrap().config,
                original_config
            );
        } else {
            panic!("expected ExecuteUpdate variant");
        }
    }

    #[test]
    fn encrypt_then_decrypt_roundtrip_batch_update() {
        init_test_crypto();

        let original_config = json!({"api_key": "key123"});
        let msg =
            ControllerMessage::ExecuteBatchHostPackageUpdate(Box::new(
                ExecuteBatchHostPackageUpdatePayload {
                    host_machine_id: "host-1".to_string(),
                    batch_id: uuid::Uuid::nil(),
                    plugin_type: PluginType::PackageManagerApt,
                    plugin_config: original_config.clone(),
                    updates: vec![],
                    pre_update_hooks: vec![],
                    post_update_hooks: vec![],
                    timeout_seconds: 300,
                },
            ));

        let encrypted = encrypt_message_configs(msg);
        if let ControllerMessage::ExecuteBatchHostPackageUpdate(ref p) = encrypted {
            assert!(p.plugin_config.is_string());
        } else {
            panic!("expected ExecuteBatchHostPackageUpdate variant");
        }

        let decrypted = decrypt_message_configs(encrypted);
        if let ControllerMessage::ExecuteBatchHostPackageUpdate(ref p) = decrypted {
            assert_eq!(p.plugin_config, original_config);
        } else {
            panic!("expected ExecuteBatchHostPackageUpdate variant");
        }
    }

    #[test]
    fn encrypt_then_decrypt_roundtrip_discover_software() {
        init_test_crypto();

        let original_config = json!({"token": "ghcr_secret"});
        let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
            host_machine_id: "host-1".to_string(),
            plugins: vec![DiscoveryPluginAssignment {
                plugin_config_id: Some(uuid::Uuid::nil()),
                plugin_type: PluginType::ReleasesDocker,
                config: original_config.clone(),
            }],
        });

        let encrypted = encrypt_message_configs(msg);
        if let ControllerMessage::DiscoverSoftware(ref p) = encrypted {
            assert!(p.plugins[0].config.is_string());
        } else {
            panic!("expected DiscoverSoftware variant");
        }

        let decrypted = decrypt_message_configs(encrypted);
        if let ControllerMessage::DiscoverSoftware(ref p) = decrypted {
            assert_eq!(p.plugins[0].config, original_config);
        } else {
            panic!("expected DiscoverSoftware variant");
        }
    }

    #[test]
    fn non_credential_variants_unchanged() {
        init_test_crypto();

        let msg = ControllerMessage::Pong(PongPayload::new(0, 0));
        let encrypted = encrypt_message_configs(msg.clone());
        assert_eq!(encrypted, msg);

        let decrypted = decrypt_message_configs(encrypted);
        assert_eq!(decrypted, msg);
    }

    #[test]
    fn decrypt_non_encrypted_config_unchanged() {
        init_test_crypto();

        let plain_config = json!({"some_key": "some_value"});
        let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
            host_machine_id: "host-1".to_string(),
            assignments: vec![VersionCheckAssignment {
                software_item_id: uuid::Uuid::nil(),
                name: "test".to_string(),
                detect_version: Some(PluginAssignment {
                    plugin_type: PluginType::ReleasesGithub,
                    package_identifier: "pkg".to_string(),
                    config: plain_config.clone(),
                }),
                fetch_releases: None,
                host_package_id: None,
            }],
        });

        // Decrypt without prior encryption — config should pass through unchanged.
        let decrypted = decrypt_message_configs(msg);
        if let ControllerMessage::CheckVersions(ref p) = decrypted {
            assert_eq!(
                p.assignments[0]
                    .detect_version
                    .as_ref()
                    .unwrap()
                    .config,
                plain_config,
                "non-encrypted config should pass through unchanged"
            );
        } else {
            panic!("expected CheckVersions variant");
        }
    }
}
