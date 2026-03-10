//! Docker credential resolution from `~/.docker/config.json`.
//!
//! When `use_system_credentials` is enabled in the plugin config, this module
//! reads the Docker credential store from the host where the plugin executes
//! (local filesystem for local agents; via `cat` over SSH for remote agents)
//! and resolves registry credentials from it.
//!
//! **Resolution order** when `use_system_credentials = true` and `auth` is `None`:
//! 1. `auths.<registry>` → base64-decoded `username:password`
//! 2. `credHelpers.<registry>` → invoke `docker-credential-{helper} get`
//!
//! Credential helper names are validated to only allow `[a-zA-Z0-9_-]+`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;
use uptrakit_plugin_infrastructure_core::SecretString;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};

use crate::config::DockerAuth;

/// Maximum time to wait for a credential helper subprocess.
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-registry credential cache entry.
#[derive(Clone)]
enum CacheEntry {
    Found(DockerAuth),
    NotFound,
}

/// Cache of resolved credentials, keyed by registry hostname.
pub(crate) struct CredentialCache {
    inner: Mutex<HashMap<String, CacheEntry>>,
}

impl CredentialCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, registry: &str) -> Option<Option<DockerAuth>> {
        self.inner.lock().get(registry).map(|e| match e {
            CacheEntry::Found(a) => Some(a.clone()),
            CacheEntry::NotFound => None,
        })
    }

    fn set(&self, registry: &str, auth: Option<DockerAuth>) {
        let entry = match auth {
            Some(a) => CacheEntry::Found(a),
            None => CacheEntry::NotFound,
        };
        self.inner.lock().insert(registry.to_string(), entry);
    }
}

/// Parsed representation of `~/.docker/config.json`.
#[derive(Debug, Deserialize, Default)]
struct DockerConfigFile {
    #[serde(default)]
    auths: HashMap<String, AuthEntry>,
    #[serde(rename = "credHelpers", default)]
    cred_helpers: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
struct AuthEntry {
    #[serde(default)]
    auth: String,
}

/// Validate a credential helper name: only `[a-zA-Z0-9_-]+` allowed.
fn validate_helper_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Resolve credentials for `registry` from the system Docker config file.
///
/// - `executor`: command executor (local or SSH).
/// - `is_remote`: when `true`, the config is read via `cat ~/.docker/config.json`
///   over the SSH executor; when `false`, it is read from the local filesystem.
/// - `cache`: shared credential cache for this plugin instance.
///
/// Returns `None` if no credentials are found (not an error — fall through to
/// unauthenticated access).
pub(crate) async fn resolve_system_credentials(
    registry: &str,
    executor: &Arc<dyn CommandExecutor>,
    is_remote: bool,
    cache: &CredentialCache,
) -> Option<DockerAuth> {
    // Return cached result immediately.
    if let Some(cached) = cache.get(registry) {
        return cached;
    }

    let resolved = resolve_uncached(registry, executor, is_remote).await;
    cache.set(registry, resolved.clone());
    resolved
}

async fn resolve_uncached(
    registry: &str,
    executor: &Arc<dyn CommandExecutor>,
    is_remote: bool,
) -> Option<DockerAuth> {
    let config_json = read_docker_config(executor, is_remote).await?;
    let config: DockerConfigFile = serde_json::from_str(&config_json).ok()?;

    // 1. Check credHelpers for this registry.
    if let Some(helper_name) = config.cred_helpers.get(registry)
        && let Some(auth) =
            invoke_credential_helper(helper_name, registry, executor, is_remote).await
    {
        return Some(auth);
    }

    // 2. Fall back to auths map.
    if let Some(entry) = config.auths.get(registry)
        && let Some(auth) = decode_auth_entry(&entry.auth)
    {
        return Some(auth);
    }

    None
}

/// Read `~/.docker/config.json` from the local filesystem or via SSH executor.
async fn read_docker_config(
    executor: &Arc<dyn CommandExecutor>,
    is_remote: bool,
) -> Option<String> {
    if is_remote {
        // Read via SSH executor using cat.
        let result = tokio::time::timeout(
            HELPER_TIMEOUT,
            executor.execute_quiet(&CommandSpec::shell("cat ~/.docker/config.json 2>/dev/null")),
        )
        .await
        .ok()?
        .ok()?;

        if result.exit_code == 0 && !result.output.is_empty() {
            Some(result.output)
        } else {
            None
        }
    } else {
        // Read from local filesystem.
        let home = std::env::var("HOME").ok()?;
        let path = format!("{home}/.docker/config.json");
        std::fs::read_to_string(path).ok()
    }
}

/// Invoke a Docker credential helper and parse its output.
///
/// Runs `docker-credential-{helper} get` with the registry URL as stdin,
/// either locally or via SSH executor.
async fn invoke_credential_helper(
    helper_name: &str,
    registry: &str,
    executor: &Arc<dyn CommandExecutor>,
    _is_remote: bool,
) -> Option<DockerAuth> {
    if !validate_helper_name(helper_name) {
        tracing::warn!(
            helper = %helper_name,
            "skipping credential helper with invalid name"
        );
        return None;
    }

    // Build the command: echo registry | docker-credential-{helper} get
    let cmd = format!(
        "printf '%s' {} | docker-credential-{} get",
        shell_quote(registry),
        helper_name,
    );

    let result = tokio::time::timeout(
        HELPER_TIMEOUT,
        executor.execute_quiet(&CommandSpec::shell(&cmd)),
    )
    .await
    .ok()?
    .ok()?;

    if result.exit_code != 0 || result.output.is_empty() {
        return None;
    }

    #[derive(Deserialize)]
    struct HelperOutput {
        #[serde(rename = "Username")]
        username: String,
        #[serde(rename = "Secret")]
        secret: String,
    }

    let output: HelperOutput = serde_json::from_str(&result.output).ok()?;
    if output.username == "<token>" {
        Some(DockerAuth::Bearer {
            token: SecretString::new(output.secret),
        })
    } else {
        Some(DockerAuth::Basic {
            username: output.username,
            password: SecretString::new(output.secret),
        })
    }
}

/// Decode a base64-encoded `username:password` auth entry.
fn decode_auth_entry(auth_b64: &str) -> Option<DockerAuth> {
    if auth_b64.is_empty() {
        return None;
    }
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(auth_b64)
        .ok()?;
    let s = String::from_utf8(decoded).ok()?;
    let (username, password) = s.split_once(':')?;
    if username.is_empty() {
        return None;
    }
    Some(DockerAuth::Basic {
        username: username.to_string(),
        password: SecretString::new(password.to_string()),
    })
}

/// Minimal shell quoting: wrap in single quotes, escaping existing single quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_helper_name_accepts_alphanumeric_and_separators() {
        assert!(validate_helper_name("osxkeychain"));
        assert!(validate_helper_name("desktop"));
        assert!(validate_helper_name("my-helper_v2"));
    }

    #[test]
    fn validate_helper_name_rejects_empty() {
        assert!(!validate_helper_name(""));
    }

    #[test]
    fn validate_helper_name_rejects_path_traversal() {
        assert!(!validate_helper_name("../evil"));
        assert!(!validate_helper_name("helper; rm -rf /"));
        assert!(!validate_helper_name("helper$(cmd)"));
    }

    #[test]
    fn decode_auth_entry_basic() {
        // base64("user:pass")
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode("user:pass");
        let auth = decode_auth_entry(&b64).unwrap();
        match auth {
            DockerAuth::Basic { username, password } => {
                assert_eq!(username, "user");
                assert_eq!(password.expose_secret(), "pass");
            }
            _ => panic!("expected Basic"),
        }
    }

    #[test]
    fn decode_auth_entry_empty_returns_none() {
        assert!(decode_auth_entry("").is_none());
    }

    #[test]
    fn decode_auth_entry_invalid_b64_returns_none() {
        assert!(decode_auth_entry("not-valid-base64!!!").is_none());
    }
}
