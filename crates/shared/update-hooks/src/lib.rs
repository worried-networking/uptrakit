//! General-purpose plugin config merge utilities.
//!
//! Provides hierarchical configuration merging for the plugin system:
//!
//! - [`resolve_effective_config`] — three-layer merge (type settings → profile
//!   config → assignment config)
//! - [`merge_config`] — two-layer merge (base config → override)
//!
//! Each layer's top-level keys override the previous layer (shallow merge).

/// Resolve the effective configuration by merging three layers:
///
/// 1. **Type settings** (`plugin_type_settings.config`) — tenant-level defaults
/// 2. **Profile config** (`plugin_configs.config`) — credential/access profile
/// 3. **Assignment config** (`host_software_item_plugins.config`) — per-item overrides
///
/// Each layer's top-level keys override the previous layer (shallow merge).
/// `None` layers are skipped.
pub fn resolve_effective_config(
    type_settings: Option<&serde_json::Value>,
    profile_config: Option<&serde_json::Value>,
    assignment_config: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut merged = serde_json::Value::Object(Default::default());

    // Layer 1: type settings as the base.
    if let Some(ts) = type_settings {
        shallow_merge_into(&mut merged, ts);
    }

    // Layer 2: profile config overrides type settings.
    if let Some(pc) = profile_config {
        shallow_merge_into(&mut merged, pc);
    }

    // Layer 3: assignment config overrides everything.
    if let Some(ac) = assignment_config {
        shallow_merge_into(&mut merged, ac);
    }

    merged
}

/// Merge plugin config and override into a single config object.
///
/// The override object's keys completely replace the base object's keys.
pub fn merge_config(
    plugin_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut merged = plugin_config.clone();

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

/// Shallow-merge `source` object keys into `target`.
///
/// Non-object sources are ignored. Each source key completely replaces
/// the target key (no deep merge).
fn shallow_merge_into(target: &mut serde_json::Value, source: &serde_json::Value) {
    if let (Some(target_obj), Some(source_obj)) = (target.as_object_mut(), source.as_object()) {
        for (k, v) in source_obj {
            target_obj.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_config_basic() {
        let base = json!({
            "tag_strip_prefix": "v",
            "include_prereleases": false
        });
        let override_config = json!({
            "tag_strip_prefix": "release-",
            "asset_patterns": [".*\\.tar\\.gz$"]
        });

        let merged = merge_config(&base, Some(&override_config));

        assert_eq!(merged["tag_strip_prefix"], "release-");
        assert_eq!(merged["include_prereleases"], false);
        assert_eq!(merged["asset_patterns"][0], ".*\\.tar\\.gz$");
    }

    #[test]
    fn merge_config_no_override() {
        let base = json!({
            "tag_strip_prefix": "v",
            "include_prereleases": false
        });

        let merged = merge_config(&base, None);

        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_empty_override() {
        let base = json!({
            "tag_strip_prefix": "v",
            "include_prereleases": false
        });
        let override_config = json!({});

        let merged = merge_config(&base, Some(&override_config));

        assert_eq!(merged, base);
    }

    #[test]
    fn resolve_effective_config_all_three_layers() {
        let type_settings = json!({"discovery_filter": "all", "timeout": 30});
        let profile_config = json!({"auth_token": "secret", "timeout": 60});
        let assignment_config = json!({"include_prereleases": true, "timeout": 90});

        let result = resolve_effective_config(
            Some(&type_settings),
            Some(&profile_config),
            Some(&assignment_config),
        );

        assert_eq!(result["discovery_filter"], "all");
        assert_eq!(result["auth_token"], "secret");
        assert_eq!(result["include_prereleases"], true);
        assert_eq!(result["timeout"], 90);
    }

    #[test]
    fn resolve_effective_config_type_settings_only() {
        let type_settings = json!({"discovery_filter": "all"});

        let result = resolve_effective_config(Some(&type_settings), None, None);

        assert_eq!(result["discovery_filter"], "all");
    }

    #[test]
    fn resolve_effective_config_profile_only() {
        let profile = json!({"auth_token": "secret"});

        let result = resolve_effective_config(None, Some(&profile), None);

        assert_eq!(result["auth_token"], "secret");
    }

    #[test]
    fn resolve_effective_config_assignment_only() {
        let assignment = json!({"include_prereleases": true});

        let result = resolve_effective_config(None, None, Some(&assignment));

        assert_eq!(result["include_prereleases"], true);
    }

    #[test]
    fn resolve_effective_config_all_none() {
        let result = resolve_effective_config(None, None, None);
        assert_eq!(result, json!({}));
    }

    #[test]
    fn resolve_effective_config_later_layers_override_earlier() {
        let ts = json!({"key": "from_type_settings"});
        let pc = json!({"key": "from_profile"});
        let ac = json!({"key": "from_assignment"});

        // Profile overrides type settings.
        let r1 = resolve_effective_config(Some(&ts), Some(&pc), None);
        assert_eq!(r1["key"], "from_profile");

        // Assignment overrides profile.
        let r2 = resolve_effective_config(None, Some(&pc), Some(&ac));
        assert_eq!(r2["key"], "from_assignment");

        // Assignment overrides both.
        let r3 = resolve_effective_config(Some(&ts), Some(&pc), Some(&ac));
        assert_eq!(r3["key"], "from_assignment");
    }

    #[test]
    fn resolve_effective_config_non_object_layers_ignored() {
        let ts = json!("not an object");
        let result = resolve_effective_config(Some(&ts), None, None);
        assert_eq!(result, json!({}));
    }
}
