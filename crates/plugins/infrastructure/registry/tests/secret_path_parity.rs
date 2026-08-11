//! Guard: schema-derived `sensitive_paths` cover everything the legacy
//! per-plugin `mask_secrets`/`restore_secrets` fn-pointers covered, over the
//! whole compiled catalog, BEFORE the legacy fn-pointers are deleted (Task 6).
//!
//! Every secret field is `Option` + `skip_serializing_if` + `None` default,
//! so `sample_config()` alone contains no secret keys and the `is_some()`-
//! guarded legacy maskers (docker/webhook/telegram) would pass a
//! sample-based subset check vacuously. `secret_fixtures()` below populates
//! every secret field with a `fixture-secret-`-marked value so the parity
//! check actually exercises the masking/restore paths it is meant to prove.
//!
//! Canonical run: `cargo test -p uptrakit-plugin-infrastructure-registry --all-features --test secret_path_parity`
//! (notification plugins are optional deps behind `notifications-webhook` /
//! `notifications-telegram`, so a default-feature run under-covers the catalog).

use serde_json::{Value, json};
use uptrakit_plugin_infrastructure_core::secret_paths::{
    SECRET_SENTINEL, mask_present_keys, restore_masked_keys,
};
use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, InstancePluginStates, PluginConfigOps, PluginTypeId, all_descriptors,
    build_catalog, plugin_ids,
};

fn sample_for(type_id: &str) -> Value {
    all_descriptors()
        .into_iter()
        .find(|desc| desc.type_id == type_id)
        .map(|desc| (desc.config.sample)())
        .unwrap_or(Value::Null)
}

/// Inserts/overwrites top-level fields on a JSON object, falling back to an
/// empty object if `base` was not one.
fn set_fields(mut base: Value, fields: &[(&str, Value)]) -> Value {
    if !base.is_object() {
        base = json!({});
    }
    if let Some(map) = base.as_object_mut() {
        for (key, value) in fields {
            map.insert((*key).to_string(), value.clone());
        }
    }
    base
}

/// Per-plugin populated fixtures, one per variant where variants exist.
/// Every secret value carries the `fixture-secret-` marker prefix. Every
/// plugin type id in `all_descriptors()` must have at least one fixture —
/// the default arm returns the (secret-free) sample so a genuinely
/// un-fixtured new secret-bearing plugin still surfaces via the marker-leaf
/// guard rather than silently going unfixtured.
fn secret_fixtures(type_id: &str) -> Vec<Value> {
    let sample = sample_for(type_id);

    if type_id == plugin_ids::RELEASES_DOCKER.as_str() {
        vec![
            set_fields(
                sample.clone(),
                &[(
                    "auth",
                    json!({"type": "basic", "username": "u", "password": "fixture-secret-pw"}),
                )],
            ),
            set_fields(
                sample,
                &[(
                    "auth",
                    json!({"type": "bearer", "token": "fixture-secret-tok"}),
                )],
            ),
        ]
    } else if type_id == plugin_ids::RELEASES_GITHUB.as_str()
        || type_id == plugin_ids::RELEASES_GITLAB.as_str()
    {
        vec![set_fields(
            sample,
            &[("auth_token", json!("fixture-secret-tok"))],
        )]
    } else if type_id == plugin_ids::RELEASES_FORGEJO.as_str() {
        vec![set_fields(
            sample,
            &[
                ("auth_token", json!("fixture-secret-tok")),
                ("api_base_url", json!("https://codeberg.org")),
            ],
        )]
    } else if type_id == plugin_ids::INFRASTRUCTURE_PROXMOX.as_str() {
        vec![set_fields(
            sample,
            &[
                ("api_url", json!("https://pve.example.com:8006")),
                ("api_token", json!("fixture-secret-user@pam!tok=abc")),
            ],
        )]
    } else if type_id == plugin_ids::TELEGRAM.as_str() {
        vec![set_fields(
            sample,
            &[
                ("chat_id", json!("-100123456")),
                ("bot_token", json!("fixture-secret-bot")),
                ("webhook_secret", json!("fixture-secret-wh")),
            ],
        )]
    } else if type_id == plugin_ids::WEBHOOK.as_str() {
        vec![set_fields(
            sample,
            &[
                ("url", json!("https://example.com/hook")),
                ("secret", json!("fixture-secret-hook")),
            ],
        )]
    } else if type_id == plugin_ids::GENERIC_SHELL.as_str() {
        // Not secret-bearing, but its default sample fails its own
        // `validate()` (requires at least one of version_command /
        // update_command) — the parity gate asserts fixture validity for
        // every descriptor, so give it a minimally valid fixture.
        vec![set_fields(
            sample,
            &[("version_command", json!("myapp --version"))],
        )]
    } else if type_id == plugin_ids::HOOK_SHELL.as_str() {
        // Same shape as generic.shell: not secret-bearing, but its default
        // sample fails its own `validate()` (requires at least one of
        // pre_command / post_command).
        vec![set_fields(sample, &[("pre_command", json!("echo pre"))])]
    } else if type_id == plugin_ids::HOOK_SYSTEMD.as_str() {
        // Not secret-bearing, but its default sample fails its own
        // `validate()` (service_name is required).
        vec![set_fields(
            sample,
            &[("service_name", json!("myapp.service"))],
        )]
    } else if type_id == plugin_ids::EMAIL.as_str() {
        // Not secret-bearing — SMTP credentials live in global settings, not
        // per-channel config — but its default sample fails its own
        // `validate()` (to_addresses must not be empty).
        vec![set_fields(
            sample,
            &[("to_addresses", json!(["user@example.com"]))],
        )]
    } else {
        vec![sample]
    }
}

// ── Local helpers: plain recursive walks over `serde_json::Value`, no
// indexing — same traversal style as `secret_paths::navigate`. ──────────────

/// Navigates a dotted path through `Value::Object` layers only, one segment
/// per level (mirrors `secret_paths::navigate`, which is private to its module).
fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

/// Collects every leaf (non-object) reachable from `value` as a dotted path.
fn leaf_paths(value: &Value, prefix: &str, out: &mut Vec<String>) {
    match value.as_object() {
        Some(map) => {
            for (key, val) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_paths(val, &path, out);
            }
        }
        None => out.push(prefix.to_string()),
    }
}

/// Dotted paths present in `before` whose value differs from the value at
/// the same path in `after`. Because the walk starts from `before`'s own
/// structure, every returned path is guaranteed to have existed in `before`.
fn paths_where_values_differ(before: &Value, after: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    leaf_paths(before, "", &mut paths);
    paths
        .into_iter()
        .filter(|path| value_at(before, path) != value_at(after, path))
        .collect()
}

/// Every string-valued leaf in `value`, as `(dotted path, string value)`.
fn string_leaves(value: &Value) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    leaf_paths(value, "", &mut paths);
    paths
        .into_iter()
        .filter_map(|path| {
            let leaf = value_at(value, &path)?.as_str()?.to_string();
            Some((path, leaf))
        })
        .collect()
}

/// The non-noop legacy masker set: plugins whose `with_secrets_masked` does
/// something observable on a populated fixture. Hard-coded on purpose — a
/// rename must make `legacy_maskers_actually_mask` red, not silently drop
/// the id from coverage.
const NON_VACUOUS_LEGACY_MASKER_IDS: [&str; 7] = [
    "releases.docker",
    "releases.github",
    "releases.gitlab",
    "releases.forgejo",
    "infrastructure.proxmox",
    "notifications.webhook",
    "notifications.telegram",
];

/// Proves the new schema-derived `sensitive_paths` cover everything the
/// legacy per-plugin maskers covered, and that mask/restore round-trip
/// correctly through the new key-set mechanism, over the whole compiled
/// catalog.
#[test]
fn parity_gate() {
    let ops = build_catalog(
        &CatalogConfig::default(),
        InstancePluginStates::all_disabled(),
    )
    .expect("catalog should build from the compiled-in descriptor set");
    let descriptors = all_descriptors();
    assert!(
        descriptors.len() >= 20,
        "expected at least 20 compiled-in descriptors under --all-features, got {} — \
         a feature-starved run must be red, not vacuously green",
        descriptors.len()
    );

    for desc in &descriptors {
        let type_id = PluginTypeId::from_static(desc.type_id);
        let fixtures = secret_fixtures(desc.type_id);
        assert!(
            !fixtures.is_empty(),
            "{}: secret_fixtures() returned no fixtures — every plugin type id must have \
             at least one",
            desc.type_id
        );
        let paths = ops.sensitive_paths(&type_id);

        for fixture in &fixtures {
            ops.validate_config(&type_id, fixture).unwrap_or_else(|e| {
                panic!("{}: fixture failed validate_config: {e}", desc.type_id)
            });

            // Legacy-masked set must be a subset of the derived sensitive paths.
            let legacy_masked = (desc.config.mask_secrets)(fixture);
            for path in paths_where_values_differ(fixture, &legacy_masked) {
                assert!(
                    paths.contains(&path),
                    "{}: legacy mask_secrets changed '{path}' but it is not covered by the \
                     schema-derived sensitive_paths ({paths:?}) — add a sensitive_paths \
                     declaration to this plugin's declare_plugin! (immediately before `roles:`)",
                    desc.type_id
                );
            }

            // Marker-leaf guard: every fixture-secret- leaf is covered and masks to the sentinel.
            let masked = mask_present_keys(fixture, &paths);
            for (leaf_path, leaf_value) in string_leaves(fixture) {
                if !leaf_value.starts_with("fixture-secret-") {
                    continue;
                }
                assert!(
                    paths.contains(&leaf_path),
                    "{}: fixture leaf '{leaf_path}' carries a fixture-secret- marker value but \
                     is not covered by sensitive_paths ({paths:?})",
                    desc.type_id
                );
                assert_eq!(
                    value_at(&masked, &leaf_path).and_then(Value::as_str),
                    Some(SECRET_SENTINEL),
                    "{}: mask_present_keys did not mask '{leaf_path}' to the sentinel",
                    desc.type_id
                );
            }

            // Restore parity (same-variant), per sensitive path, values not whole documents.
            let mut new_restored = masked.clone();
            restore_masked_keys(&mut new_restored, fixture, &paths);
            let mut legacy_restored = legacy_masked.clone();
            (desc.config.restore_secrets)(&mut legacy_restored, fixture);

            for path in &paths {
                let original = value_at(fixture, path);
                assert_eq!(
                    value_at(&new_restored, path),
                    original,
                    "{}: restore_masked_keys did not recover '{path}' to its fixture value",
                    desc.type_id
                );
                if original.is_some() {
                    assert_eq!(
                        value_at(&legacy_restored, path),
                        original,
                        "{}: legacy restore_secrets did not recover '{path}' to its fixture \
                         value (same-variant)",
                        desc.type_id
                    );
                }
            }
        }

        // Path-resolution assertion: a declared path that resolves nowhere is a bug.
        for path in &paths {
            let resolves = fixtures.iter().any(|f| value_at(f, path).is_some());
            assert!(
                resolves,
                "{}: sensitive_paths entry '{path}' does not resolve in any fixture — a path \
                 that matches nothing is a bug, not a no-op",
                desc.type_id
            );
        }
    }
}

/// Non-vacuity guard: `parity_gate`'s subset check only proves something if
/// the legacy masker actually changes the fixture. A fixture that leaves a
/// secret unset would let an `is_some()`-guarded legacy masker mask nothing
/// and pass the subset check vacuously — this test proves that hole is
/// closed for the plugins whose legacy masking has an `is_some()`/unconditional
/// shape worth proving non-vacuous.
#[test]
fn legacy_maskers_actually_mask() {
    let descriptors = all_descriptors();
    for id in NON_VACUOUS_LEGACY_MASKER_IDS {
        let desc = descriptors
            .iter()
            .find(|desc| desc.type_id == id)
            .unwrap_or_else(|| panic!("'{id}' does not resolve to a compiled-in descriptor"));
        for fixture in secret_fixtures(id) {
            let masked = (desc.config.mask_secrets)(&fixture);
            assert_ne!(
                masked, fixture,
                "{id}: legacy mask_secrets is a no-op on its own populated fixture — the \
                 vacuity hole this test exists to catch is not actually closed"
            );
        }
    }
}

/// Type-settings and instance-config plugins have no per-host/per-profile
/// secret surface — their effective `sensitive_paths` must stay empty, even
/// if an explicit declaration were added on a schema-less plugin.
#[test]
fn type_settings_and_instance_plugins_are_secret_free() {
    let ops = build_catalog(
        &CatalogConfig::default(),
        InstancePluginStates::all_disabled(),
    )
    .expect("catalog should build from the compiled-in descriptor set");
    for desc in all_descriptors() {
        if desc.type_settings.is_none() && desc.instance_config.is_none() {
            continue;
        }
        let type_id = PluginTypeId::from_static(desc.type_id);
        let paths = ops.sensitive_paths(&type_id);
        assert!(
            paths.is_empty(),
            "{}: type_settings/instance_config plugin must be secret-free but declares {paths:?}",
            desc.type_id
        );
    }
}
