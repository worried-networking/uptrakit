use crate::config::{load_config, load_credentials};
use crate::error::{CliError, Result};
use rootcause::prelude::*;

pub use uptrakit_openapi_client::UptrakitClient;

/// Resolve server URL and API token from overrides or stored config/credentials.
///
/// Priority: explicit override > stored config/credentials. Returns
/// `CliError::NotLoggedIn` if either value is missing.
pub fn resolve_server_and_token(
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<(String, String)> {
    let config = load_config()?;
    let creds = load_credentials()?;

    let server = server_override
        .map(|s| s.to_string())
        .or(config.server)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    let token = token_override
        .map(|t| t.to_string())
        .or(creds.token)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    Ok((server, token))
}

/// Build an authenticated API client from stored config/credentials or overrides.
///
/// Loads `ca_pem` from the stored [`Config`] so that connections to controllers
/// whose TLS certificate was pinned with `uptrakit auth ca trust` succeed.
/// When the connection fails and a custom CA is configured, a recovery hint is
/// attached to the error pointing users toward `uptrakit auth ca trust` /
/// `uptrakit auth ca forget`.
pub fn authenticated_client(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UptrakitClient> {
    let config = load_config()?;
    let ca_pem = config.ca_pem.clone();
    let (server, token) = resolve_server_and_token(server, token)?;

    UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        ca_pem.as_deref(),
        request_timeout,
    )
    .map_err(|e| {
        if ca_pem.is_some() {
            e.attach(
                "connection failed with a pinned CA; if the controller CA has rotated, run \
                     'uptrakit auth ca trust' to re-establish trust; if the controller now uses \
                     a public CA, run 'uptrakit auth ca forget' to return to system roots",
            )
        } else {
            e
        }
    })
    .context_to()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-mutating tests so parallel test threads do not race on HOME.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Run `f` with HOME pointing at an empty temp directory, then restore the
    /// original value.  This prevents the test from reading real on-disk
    /// config/credentials left by a logged-in developer session.
    fn with_empty_home<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let tmp = std::env::temp_dir().join(format!(
            "uptrakit-cli-test-home-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).expect("create temp home");
        let prev = std::env::var_os("HOME");
        // SAFETY: ENV_LOCK is held for the duration of this function, so no
        // other thread in this process touches HOME concurrently.
        unsafe { std::env::set_var("HOME", &tmp) };
        f();
        // SAFETY: same lock is still held; restoring HOME to its previous value.
        match prev {
            // SAFETY: ENV_LOCK is still held; no concurrent HOME access.
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            // SAFETY: ENV_LOCK is still held; no concurrent HOME access.
            None => unsafe { std::env::remove_var("HOME") },
        }
        drop(std::fs::remove_dir_all(&tmp));
    }

    #[test]
    fn resolve_server_and_token_returns_not_logged_in_when_both_absent() {
        with_empty_home(|| {
            resolve_server_and_token(None, None).unwrap_err();
        });
    }

    #[test]
    fn resolve_server_and_token_uses_overrides() {
        let (server, token) =
            resolve_server_and_token(Some("https://example.com"), Some("tok")).expect("ok");
        assert_eq!(server, "https://example.com");
        assert_eq!(token, "tok");
    }
}
