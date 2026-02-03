//! Hook merging logic for software updates.
//!
//! Merges pre/post-update commands from provider config (base) with
//! software item config_override (override). The override completely
//! replaces the base when present.

/// Extract string array from JSON value at the given key.
fn extract_string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Merge hooks from provider config and config_override.
///
/// Strategy: Override keys completely replace base keys. Missing keys
/// fall back to base.
///
/// # Arguments
///
/// * `provider_config` - Base provider configuration JSON
/// * `config_override` - Optional override configuration JSON from software item
///
/// # Returns
///
/// Tuple of `(pre_update_commands, post_update_commands)`
pub fn merge_hooks(
    provider_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> (Vec<String>, Vec<String>) {
    let base_pre = extract_string_array(provider_config, "pre_update_commands");
    let base_post = extract_string_array(provider_config, "post_update_commands");

    let Some(override_config) = config_override else {
        return (base_pre, base_post);
    };

    // Check if override has the key - if so, use it (even if empty), otherwise fall back
    let pre = if override_config.get("pre_update_commands").is_some() {
        extract_string_array(override_config, "pre_update_commands")
    } else {
        base_pre
    };

    let post = if override_config.get("post_update_commands").is_some() {
        extract_string_array(override_config, "post_update_commands")
    } else {
        base_post
    };

    (pre, post)
}

/// Merge provider config and override into a single config object.
///
/// The override object's keys completely replace the base object's keys.
pub fn merge_config(
    provider_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut merged = provider_config.clone();

    if let Some(override_config) = config_override
        && let (Some(base_obj), Some(over_obj)) =
            (merged.as_object_mut(), override_config.as_object())
    {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_hooks_base_only() {
        let base = json!({
            "owner": "nodejs",
            "repo": "node",
            "pre_update_commands": ["systemctl stop app"],
            "post_update_commands": ["systemctl start app"]
        });

        let (pre, post) = merge_hooks(&base, None);

        assert_eq!(pre, vec!["systemctl stop app"]);
        assert_eq!(post, vec!["systemctl start app"]);
    }

    #[test]
    fn merge_hooks_override_replaces_base() {
        let base = json!({
            "pre_update_commands": ["base-pre-1", "base-pre-2"],
            "post_update_commands": ["base-post"]
        });
        let override_config = json!({
            "pre_update_commands": ["override-pre"],
            "post_update_commands": ["override-post-1", "override-post-2"]
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert_eq!(pre, vec!["override-pre"]);
        assert_eq!(post, vec!["override-post-1", "override-post-2"]);
    }

    #[test]
    fn merge_hooks_partial_override() {
        let base = json!({
            "pre_update_commands": ["base-pre"],
            "post_update_commands": ["base-post"]
        });
        let override_config = json!({
            "pre_update_commands": ["override-pre"]
            // post_update_commands not present - should fall back to base
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert_eq!(pre, vec!["override-pre"]);
        assert_eq!(post, vec!["base-post"]);
    }

    #[test]
    fn merge_hooks_override_clears_with_empty_array() {
        let base = json!({
            "pre_update_commands": ["base-pre"],
            "post_update_commands": ["base-post"]
        });
        let override_config = json!({
            "pre_update_commands": [],
            "post_update_commands": []
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert!(pre.is_empty());
        assert!(post.is_empty());
    }

    #[test]
    fn merge_hooks_empty_base_and_override() {
        let base = json!({});
        let override_config = json!({});

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert!(pre.is_empty());
        assert!(post.is_empty());
    }

    #[test]
    fn merge_hooks_no_hooks_in_base() {
        let base = json!({
            "owner": "nodejs",
            "repo": "node"
        });

        let (pre, post) = merge_hooks(&base, None);

        assert!(pre.is_empty());
        assert!(post.is_empty());
    }

    #[test]
    fn merge_hooks_only_pre_in_override() {
        let base = json!({});
        let override_config = json!({
            "pre_update_commands": ["pre-1", "pre-2"]
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert_eq!(pre, vec!["pre-1", "pre-2"]);
        assert!(post.is_empty());
    }

    #[test]
    fn merge_config_basic() {
        let base = json!({
            "owner": "nodejs",
            "repo": "node",
            "base_key": "base_value"
        });
        let override_config = json!({
            "owner": "custom-owner",
            "new_key": "new_value"
        });

        let merged = merge_config(&base, Some(&override_config));

        assert_eq!(merged["owner"], "custom-owner");
        assert_eq!(merged["repo"], "node");
        assert_eq!(merged["base_key"], "base_value");
        assert_eq!(merged["new_key"], "new_value");
    }

    #[test]
    fn merge_config_no_override() {
        let base = json!({
            "owner": "nodejs",
            "repo": "node"
        });

        let merged = merge_config(&base, None);

        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_empty_override() {
        let base = json!({
            "owner": "nodejs",
            "repo": "node"
        });
        let override_config = json!({});

        let merged = merge_config(&base, Some(&override_config));

        assert_eq!(merged, base);
    }
}
