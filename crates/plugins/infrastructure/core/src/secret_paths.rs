//! Key-set masking helpers over dotted JSON paths.
//!
//! Sensitive fields are addressed by dotted paths (`auth_token`,
//! `auth.password`). Masking touches only keys present in the object, so
//! sparse configs (type settings, per-host overrides) never gain keys.

use serde_json::Value;

/// The masking sentinel stored in place of secret values in API responses.
///
/// A literal config value equal to this sentinel is unsettable by design:
/// the post-restore sentinel assertion rejects it.
pub const SECRET_SENTINEL: &str = "***";

fn navigate<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

fn navigate_mut<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.as_object_mut()?.get_mut(segment)?;
    }
    Some(current)
}

/// Replace values only at paths present in the object with `"***"`.
/// Absent paths stay absent (sparse-preserving by construction).
pub fn mask_present_keys(value: &Value, paths: &[String]) -> Value {
    let mut masked = value.clone();
    for path in paths {
        if let Some(slot) = navigate_mut(&mut masked, path) {
            *slot = Value::String(SECRET_SENTINEL.to_string());
        }
    }
    masked
}

/// For each path where `incoming` holds the sentinel, copy the stored
/// value. A sentinel with no stored counterpart is left in place so the
/// post-restore sentinel assertion can reject the write.
pub fn restore_masked_keys(incoming: &mut Value, stored: &Value, paths: &[String]) {
    for path in paths {
        let is_sentinel = navigate(incoming, path)
            .and_then(Value::as_str)
            .is_some_and(|s| s == SECRET_SENTINEL);
        if !is_sentinel {
            continue;
        }
        if let Some(stored_value) = navigate(stored, path) {
            let replacement = stored_value.clone();
            if let Some(slot) = navigate_mut(incoming, path) {
                *slot = replacement;
            }
        }
    }
}

/// First sensitive path whose value still equals the sentinel, if any.
pub fn first_sentinel_path(value: &Value, paths: &[String]) -> Option<String> {
    paths
        .iter()
        .find(|p| {
            navigate(value, p)
                .and_then(Value::as_str)
                .is_some_and(|s| s == SECRET_SENTINEL)
        })
        .cloned()
}

/// True when any sensitive path differs between the two configs
/// (covers add, change, and removal of a credential).
pub fn sensitive_value_changed(incoming: &Value, stored: &Value, paths: &[String]) -> bool {
    paths
        .iter()
        .any(|p| navigate(incoming, p) != navigate(stored, p))
}

/// First sensitive path present in the object, if any (layer-3 reject).
pub fn first_sensitive_path_present(value: &Value, paths: &[String]) -> Option<String> {
    paths.iter().find(|p| navigate(value, p).is_some()).cloned()
}

/// True when any sensitive path holds a non-empty, non-sentinel string —
/// i.e. a live credential value. Used for `credential_updated_at` stamping
/// on create (spec §8): the `"***"`-on-create case is already a 400 via
/// `assert_no_sentinel`; the sentinel check here is belt-and-braces so the
/// stamp can never read "credential freshly set" for a sentinel.
///
/// `navigate()` returns `None` for a non-object root (e.g. `value` is a
/// JSON array or scalar), which this function treats the same as "path
/// absent" -- deliberate, not a silent gap, but the guarantee behind it is
/// narrower than it looks: for the REST `plugin_configs` writers,
/// `validate_config` rejects a non-object config before this is ever
/// reached, so a non-object root there would be a validation-layer bug.
/// That guarantee does NOT hold for
/// `autodiscovery::find_or_create_default_plugin_config`'s caller in
/// `discovery_items.rs` -- `DiscoveryTarget.plugin_config` is untyped
/// `serde_json::Value` from an untrusted agent report, and only
/// `assert_no_sentinel` runs on it (which itself no-ops on a non-object
/// root via this same `navigate` fallback), never `validate_config`. On
/// that path a non-object root is a real, reachable input: this function
/// under-counts (reports "no live secret") rather than lying the other
/// way, consistent with the sibling traversal helpers in this module and
/// with the "unknowable" bucket the startup residue sweep already carves
/// out for the identical blind spot (`reencrypt.rs`).
pub fn has_live_secret_value(value: &Value, paths: &[String]) -> bool {
    paths.iter().any(|p| {
        navigate(value, p)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && s != SECRET_SENTINEL)
    })
}

/// Remove every sensitive path present in the object; returns the paths
/// actually removed (autodiscovery strip-and-warn).
pub fn strip_sensitive_paths(value: &mut Value, paths: &[String]) -> Vec<String> {
    let mut removed = Vec::new();
    for path in paths {
        if remove_at_path(value, path) {
            removed.push(path.clone());
        }
    }
    removed
}

fn remove_at_path(value: &mut Value, path: &str) -> bool {
    let (parent_path, leaf) = match path.rsplit_once('.') {
        Some((parent, leaf)) => (Some(parent), leaf),
        None => (None, path),
    };
    let parent = match parent_path {
        Some(p) => match navigate_mut(value, p) {
            Some(v) => v,
            None => return false,
        },
        None => value,
    };
    parent
        .as_object_mut()
        .is_some_and(|m| m.remove(leaf).is_some())
}

/// Mirror the frontend's form-key → JSON-key convention: every segment of
/// a multi-segment key drops one leading `_`; single-segment keys are
/// written literally (see `unflattenConfig` in `PluginConfigsTab.svelte`).
pub fn normalize_form_key(key: &str) -> String {
    if !key.contains('.') {
        return key.to_string();
    }
    key.split('.')
        .map(|seg| seg.strip_prefix('_').unwrap_or(seg))
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mask_is_sparse_preserving() {
        let sparse = json!({"auth_token": "tok-1"});
        let paths = vec!["auth_token".to_string(), "auth.password".to_string()];
        let masked = mask_present_keys(&sparse, &paths);
        assert_eq!(masked, json!({"auth_token": "***"}));
    }

    #[test]
    fn mask_handles_nested_paths() {
        let cfg = json!({"auth": {"password": "pw", "username": "u"}});
        let paths = vec!["auth.password".to_string()];
        let masked = mask_present_keys(&cfg, &paths);
        assert_eq!(
            masked,
            json!({"auth": {"password": "***", "username": "u"}})
        );
    }

    #[test]
    fn restore_replaces_sentinel_only() {
        let mut incoming = json!({"auth_token": "***", "url": "https://new"});
        let stored = json!({"auth_token": "tok-1", "url": "https://old"});
        let paths = vec!["auth_token".to_string()];
        restore_masked_keys(&mut incoming, &stored, &paths);
        assert_eq!(
            incoming,
            json!({"auth_token": "tok-1", "url": "https://new"})
        );
    }

    #[test]
    fn unresolvable_sentinel_survives_restore_and_is_detected() {
        let mut incoming = json!({"auth_token": "***"});
        let stored = json!({});
        let paths = vec!["auth_token".to_string()];
        restore_masked_keys(&mut incoming, &stored, &paths);
        assert_eq!(
            first_sentinel_path(&incoming, &paths),
            Some("auth_token".to_string())
        );
    }

    #[test]
    fn absent_paths_are_never_injected() {
        let cfg = json!({"url": "https://x"});
        let paths = vec!["auth_token".to_string()];
        assert_eq!(mask_present_keys(&cfg, &paths), cfg);
        assert_eq!(first_sentinel_path(&cfg, &paths), None);
    }

    #[test]
    fn changed_detects_add_change_and_removal() {
        let paths = vec!["auth_token".to_string()];
        let with = json!({"auth_token": "a"});
        let with_b = json!({"auth_token": "b"});
        let without = json!({});
        assert!(sensitive_value_changed(&with, &without, &paths));
        assert!(sensitive_value_changed(&with_b, &with, &paths));
        assert!(sensitive_value_changed(&without, &with, &paths));
        assert!(!sensitive_value_changed(&with, &with, &paths));
    }

    #[test]
    fn strip_removes_and_reports() {
        let mut cfg = json!({"auth": {"password": "pw"}, "channel": "stable"});
        let paths = vec!["auth.password".to_string(), "auth_token".to_string()];
        let removed = strip_sensitive_paths(&mut cfg, &paths);
        assert_eq!(removed, vec!["auth.password".to_string()]);
        assert_eq!(cfg, json!({"auth": {}, "channel": "stable"}));
    }

    #[test]
    fn has_live_secret_value_detects_live_value() {
        let paths = vec!["auth_token".to_string()];
        assert!(has_live_secret_value(
            &json!({"auth_token": "tok-1"}),
            &paths
        ));
    }

    #[test]
    fn has_live_secret_value_rejects_empty_sentinel_and_absent() {
        let paths = vec!["auth_token".to_string()];
        assert!(!has_live_secret_value(&json!({"auth_token": ""}), &paths));
        assert!(!has_live_secret_value(
            &json!({"auth_token": "***"}),
            &paths
        ));
        assert!(!has_live_secret_value(&json!({}), &paths));
    }

    #[test]
    fn normalize_strips_underscores_on_nested_keys_only() {
        assert_eq!(normalize_form_key("auth._type"), "auth.type");
        assert_eq!(
            normalize_form_key("compose_restart._enabled"),
            "compose_restart.enabled"
        );
        assert_eq!(normalize_form_key("_solo"), "_solo");
        assert_eq!(normalize_form_key("auth_token"), "auth_token");
    }
}
