//! Encrypted plugin-config column newtypes (compile-time AAD).
//!
//! Defined next to their entities so the AAD string and the table stay in
//! one crate. See ADR "Compile-time AAD encrypted column newtypes".

use uptrakit_crypto::encrypted_column;

encrypted_column!(
    /// `plugin_configs.config`, encrypted at rest.
    EncryptedPluginConfig,
    "uptrakit:plugin_configs:config"
);
encrypted_column!(
    /// `plugin_type_settings.config`, encrypted at rest.
    EncryptedPluginTypeConfig,
    "uptrakit:plugin_type_settings:config"
);
encrypted_column!(
    /// `instance_plugin_setting.config`, encrypted at rest.
    EncryptedInstancePluginConfig,
    "uptrakit:instance_plugin_setting:config"
);
