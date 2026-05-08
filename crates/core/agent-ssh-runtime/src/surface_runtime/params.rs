use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use uptrakit_shared_types::SecretString;

use crate::operations::bootstrap::BootstrapParams;
use crate::operations::sync;
use crate::ssh_target::SshTarget;

/// Sensitive authentication parameters extracted from the ECIES sealed box.
///
/// Used by both bootstrap and sync actions.
#[derive(Debug, Deserialize)]
pub(super) struct SensitiveAuthParams {
    auth_password: Option<String>,
    auth_private_key: Option<String>,
}

/// Parsed bootstrap request params used by connect/execute handlers.
pub(super) struct BootstrapActionRequest {
    pub(super) bootstrap_params: BootstrapParams,
    pub(super) skip_actions: HashSet<String>,
}

/// Parsed sync request params shared by connect/execute handlers.
pub(super) struct SyncActionRequest {
    pub(super) host_id: String,
    pub(super) auth_override: Option<sync::SyncAuthOverride>,
    pub(super) allow_all: bool,
    pub(super) skip_actions: HashSet<String>,
}

/// Decrypt sensitive auth params from sealed extension payload.
pub(super) fn decrypt_sensitive_auth_params(
    sensitive_params_sealed: Option<&str>,
    private_key_der: Option<&[u8]>,
) -> Result<Option<SensitiveAuthParams>, String> {
    uptrakit_service_sdk::decrypt_sensitive_params(sensitive_params_sealed, private_key_der)
}

/// Parse all bootstrap action params from raw action payload.
pub(super) fn parse_bootstrap_request(
    params: &Value,
    sensitive: Option<&SensitiveAuthParams>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
) -> Result<BootstrapActionRequest, String> {
    let bootstrap_params = parse_bootstrap_params(params, sensitive, service_id, tenant_id)?;
    let skip_actions = parse_skip_actions(params);
    Ok(BootstrapActionRequest {
        bootstrap_params,
        skip_actions,
    })
}

/// Parse required sync host id from raw action payload.
pub(super) fn parse_sync_host_id(params: &Value) -> Result<String, String> {
    params
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "missing required field 'id'".to_string())
}

/// Parse sync action request params from raw action payload.
pub(super) fn parse_sync_request(
    params: &Value,
    host_id: String,
    sensitive: Option<&SensitiveAuthParams>,
) -> Result<SyncActionRequest, String> {
    let auth_override = build_sync_auth_override(params, sensitive)?;
    let allow_all = param_bool(params, "allow_all");
    let skip_actions = parse_skip_actions(params);
    Ok(SyncActionRequest {
        host_id,
        auth_override,
        allow_all,
        skip_actions,
    })
}

/// Extract a boolean parameter from an extension params object.
///
/// Accepts both JSON booleans (`true`/`false`) and the string representations
/// `"true"`/`"false"` that form-based UIs may emit when all field values are
/// carried as strings. Returns `false` for absent or unrecognised values.
pub(super) fn param_bool(params: &Value, key: &str) -> bool {
    match params.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

/// Parse `skip_actions` from params as a `HashSet<String>`.
///
/// Expects a JSON array of strings at `params["skip_actions"]`.
/// Returns an empty set if the key is absent or not an array.
pub(super) fn parse_skip_actions(params: &Value) -> HashSet<String> {
    params
        .get("skip_actions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `BootstrapParams` from extension request params and sensitive params.
fn parse_bootstrap_params(
    params: &Value,
    sensitive: Option<&SensitiveAuthParams>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
) -> Result<BootstrapParams, String> {
    let target_str = params
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required field 'target'".to_string())?;

    let parsed_target: SshTarget = target_str
        .parse()
        .map_err(|e| format!("invalid target: {e}"))?;

    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| parsed_target.hostname.clone());

    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("password");

    let auth_password = sensitive.and_then(|s| s.auth_password.clone().map(SecretString::new));
    let auth_private_key =
        sensitive.and_then(|s| s.auth_private_key.clone().map(SecretString::new));

    match auth_method {
        "password" if auth_password.is_none() => {
            return Err("auth_method is 'password' but no password provided".to_string());
        }
        "private_key" if auth_private_key.is_none() => {
            return Err("auth_method is 'private_key' but no private key provided".to_string());
        }
        _ => {}
    }

    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();

    let host_key_fingerprint = params
        .get("host_key_fingerprint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let strict_host_key_checking = param_bool(params, "strict_host_key_checking");
    let allow_all = param_bool(params, "allow_all");
    let remove_stale_keys = param_bool(params, "remove_stale_keys");

    let host_id = uuid::Uuid::now_v7();

    Ok(BootstrapParams {
        name,
        hostname: parsed_target.hostname,
        port: parsed_target.port.unwrap_or(22) as i32,
        auth_username: parsed_target.username.unwrap_or_else(|| "root".to_string()),
        auth_password,
        auth_private_key_pem: auth_private_key,
        use_ssh_agent: false,
        target_username,
        target_private_key_pem: None,
        host_key_fingerprint,
        strict_host_key_checking,
        allow_all,
        host_id,
        service_id,
        tenant_id,
        remove_stale_keys,
    })
}

/// Build a `SyncAuthOverride` from extension params and sensitive params.
fn build_sync_auth_override(
    params: &Value,
    sensitive: Option<&SensitiveAuthParams>,
) -> Result<Option<sync::SyncAuthOverride>, String> {
    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("stored");

    match auth_method {
        "stored" => Ok(None),
        "password" => {
            let password = sensitive.and_then(|s| s.auth_password.as_deref());
            match password {
                Some(pw) => Ok(Some(sync::SyncAuthOverride {
                    username: params
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .to_string(),
                    auth_password: Some(pw.to_string()),
                    auth_private_key_pem: None,
                })),
                None => Err("auth_method is 'password' but no password provided".to_string()),
            }
        }
        "private_key" => {
            let key = sensitive.and_then(|s| s.auth_private_key.as_deref());
            match key {
                Some(pem) => Ok(Some(sync::SyncAuthOverride {
                    username: params
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .to_string(),
                    auth_password: None,
                    auth_private_key_pem: Some(pem.to_string()),
                })),
                None => Err("auth_method is 'private_key' but no private key provided".to_string()),
            }
        }
        other => Err(format!("unknown auth_method '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sensitive(password: Option<&str>, private_key: Option<&str>) -> SensitiveAuthParams {
        SensitiveAuthParams {
            auth_password: password.map(str::to_string),
            auth_private_key: private_key.map(str::to_string),
        }
    }

    #[test]
    fn parse_bootstrap_request_accepts_password_auth_with_password() {
        let params = json!({
            "target": "root@example.com:2222",
            "auth_method": "password",
        });
        let sealed = sensitive(Some("secret"), None);

        let parsed = parse_bootstrap_request(&params, Some(&sealed), None, None)
            .expect("bootstrap parse should succeed");

        assert_eq!(parsed.bootstrap_params.hostname, "example.com");
        assert_eq!(parsed.bootstrap_params.port, 2222);
        assert!(parsed.bootstrap_params.auth_password.is_some());
        assert!(parsed.bootstrap_params.auth_private_key_pem.is_none());
    }

    #[test]
    fn parse_bootstrap_request_accepts_private_key_auth_with_private_key() {
        let params = json!({
            "target": "root@example.com",
            "auth_method": "private_key",
        });
        let sealed = sensitive(None, Some("-----BEGIN PRIVATE KEY-----"));

        let parsed = parse_bootstrap_request(&params, Some(&sealed), None, None)
            .expect("bootstrap parse should succeed");

        assert!(parsed.bootstrap_params.auth_password.is_none());
        assert!(parsed.bootstrap_params.auth_private_key_pem.is_some());
    }

    #[test]
    fn parse_bootstrap_request_rejects_password_auth_without_password() {
        let params = json!({
            "target": "root@example.com",
            "auth_method": "password",
        });

        let err = match parse_bootstrap_request(&params, None, None, None) {
            Ok(_) => panic!("bootstrap parse should fail"),
            Err(err) => err,
        };

        assert_eq!(err, "auth_method is 'password' but no password provided");
    }

    #[test]
    fn parse_bootstrap_request_rejects_private_key_auth_without_private_key() {
        let params = json!({
            "target": "root@example.com",
            "auth_method": "private_key",
        });

        let err = match parse_bootstrap_request(&params, None, None, None) {
            Ok(_) => panic!("bootstrap parse should fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "auth_method is 'private_key' but no private key provided"
        );
    }

    #[test]
    fn parse_sync_request_handles_stored_auth_override() {
        let params = json!({});

        let parsed = parse_sync_request(&params, "host-1".to_string(), None)
            .expect("sync parse should succeed");

        assert_eq!(parsed.host_id, "host-1");
        assert!(parsed.auth_override.is_none());
    }

    #[test]
    fn parse_sync_request_handles_password_auth_override() {
        let params = json!({
            "auth_method": "password",
            "username": "admin",
        });
        let sealed = sensitive(Some("pw"), None);

        let parsed = parse_sync_request(&params, "host-1".to_string(), Some(&sealed))
            .expect("sync parse should succeed");

        let auth = parsed.auth_override.expect("auth override should exist");
        assert_eq!(auth.username, "admin");
        assert_eq!(auth.auth_password.as_deref(), Some("pw"));
        assert!(auth.auth_private_key_pem.is_none());
    }

    #[test]
    fn parse_sync_request_handles_private_key_auth_override() {
        let params = json!({
            "auth_method": "private_key",
        });
        let sealed = sensitive(None, Some("KEY"));

        let parsed = parse_sync_request(&params, "host-1".to_string(), Some(&sealed))
            .expect("sync parse should succeed");

        let auth = parsed.auth_override.expect("auth override should exist");
        assert_eq!(auth.username, "root");
        assert!(auth.auth_password.is_none());
        assert_eq!(auth.auth_private_key_pem.as_deref(), Some("KEY"));
    }

    #[test]
    fn parse_sync_request_rejects_unknown_auth_method() {
        let params = json!({
            "auth_method": "token",
        });

        let err = match parse_sync_request(&params, "host-1".to_string(), None) {
            Ok(_) => panic!("sync parse should fail"),
            Err(err) => err,
        };

        assert_eq!(err, "unknown auth_method 'token'");
    }

    #[test]
    fn param_bool_coerces_bool_string_and_absent_values() {
        let params = json!({
            "native_true": true,
            "native_false": false,
            "string_true": "true",
            "string_false": "false",
        });

        assert!(param_bool(&params, "native_true"));
        assert!(!param_bool(&params, "native_false"));
        assert!(param_bool(&params, "string_true"));
        assert!(!param_bool(&params, "string_false"));
        assert!(!param_bool(&params, "missing"));
    }

    #[test]
    fn parse_skip_actions_collects_string_entries_into_set() {
        let params = json!({
            "skip_actions": ["sync", "discover", "sync", 123, {"x": 1}],
        });

        let actual = parse_skip_actions(&params);
        let expected: HashSet<String> = ["sync".to_string(), "discover".to_string()]
            .into_iter()
            .collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn parse_skip_actions_returns_empty_when_absent_or_invalid() {
        assert!(parse_skip_actions(&json!({})).is_empty());
        assert!(parse_skip_actions(&json!({"skip_actions": "sync"})).is_empty());
    }
}
