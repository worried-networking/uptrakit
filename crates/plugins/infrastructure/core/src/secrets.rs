use serde::Serialize;
use serde::de::DeserializeOwned;

/// Trait for plugin configs that may contain secrets (API tokens, passwords).
///
/// Plugins with no secrets can use the default no-op implementations.
/// The trait requires `Serialize + DeserializeOwned` because the registry
/// performs JSON round-tripping when masking/restoring secrets.
pub trait SecretMasking: Serialize + DeserializeOwned {
    /// Return a copy with secret fields replaced by `"***"`.
    fn with_secrets_masked(self) -> Self {
        self
    }

    /// Restore secret fields from an existing config where `self` contains `"***"` sentinels.
    fn restore_secrets_from(&mut self, _existing: &Self) {}
}
