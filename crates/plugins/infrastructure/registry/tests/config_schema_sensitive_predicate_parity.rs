//! Guard: the frontend's per-host layer-3 override sensitive predicate
//! (`sensitive || field_type in {password, ssh_private_key}`, evaluated over
//! the CONFIG form schema only — see `getFormFields`/`isSensitiveField` in
//! `EditHostAssignmentModal.svelte`) can never diverge from the server's
//! effective `sensitive_paths` union in a way that would let the UI render
//! and submit a field the server rejects as sensitive.
//!
//! `PluginConfigOps::sensitive_paths()` (`plugin_ops.rs` :298-327) folds
//! together the CONFIG, type-settings, and instance-config form schemas —
//! a normalized path can land in the union purely because of a
//! type-settings/instance-config field, while an unrelated CONFIG-schema
//! field happens to share the same normalized key without itself being
//! flagged sensitive there. The frontend predicate only ever looks at the
//! CONFIG schema, so that scenario is exactly the gap this test closes:
//! every `sensitive_paths(id)` entry that resolves to a CONFIG form-schema
//! key must itself be flagged sensitive (or password/ssh-key-typed) in that
//! CONFIG schema.
//!
//! Key-matching caveat: `sensitive_paths` entries are normalized via
//! `normalize_form_key` (strips one leading `_` per dotted segment, e.g.
//! docker's `auth._type` -> `auth.type`), so CONFIG schema keys must go
//! through the same normalization before comparison, or dotted/underscore
//! keys produce false mismatches.
//!
//! Canonical run: `cargo test -p uptrakit-plugin-infrastructure-registry --all-features --test config_schema_sensitive_predicate_parity`
//! (notification plugins are optional deps behind `notifications-webhook` /
//! `notifications-telegram`, so a default-feature run under-covers the catalog).
//!
//! KNOWN GAP: the parity check below matches only *exact* normalized keys
//! between `sensitive_paths` and the CONFIG form schema. A `sensitive_paths`
//! entry that is nested *under* a schema key rather than equal to one (e.g.
//! sensitive path `auth.password` against a CONFIG schema key `auth`, with
//! no separate `auth.password` field descriptor) has no exact match, so it
//! silently falls into the "no matching CONFIG-schema key" branch and is
//! skipped — even though the frontend predicate can never have flagged it
//! either, and the test's own anti-vacuity guard (`checked_any_sensitive_path`)
//! still passes as long as some *other* sensitive path matches exactly. A
//! future reader should not treat a green run of this test as proof that
//! every nested sensitive path has frontend/server parity.

use std::collections::HashMap;

use uptrakit_plugin_infrastructure_core::secret_paths::normalize_form_key;
use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, FormFieldType, InstancePluginStates, PluginConfigOps, PluginTypeId,
    all_descriptors, build_catalog,
};

/// True when the frontend's `isSensitiveField` predicate would flag this
/// CONFIG form-schema field.
fn frontend_flags_sensitive(
    field: &uptrakit_plugin_infrastructure_registry::FormFieldDescriptor,
) -> bool {
    field.sensitive
        || matches!(
            field.field_type,
            FormFieldType::Password | FormFieldType::SshPrivateKey
        )
}

#[test]
fn every_sensitive_path_matching_a_config_schema_key_is_flagged_there() {
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

    let mut checked_any_sensitive_path = false;

    for desc in &descriptors {
        let type_id = PluginTypeId::from_static(desc.type_id);
        let sensitive_paths = ops.sensitive_paths(&type_id);
        if sensitive_paths.is_empty() {
            continue;
        }

        // Normalized CONFIG form-schema key -> whether the frontend predicate
        // flags it sensitive there.
        let config_fields = (desc.config.form_schema)();
        let config_schema_flags: HashMap<String, bool> = config_fields
            .iter()
            .map(|field| {
                (
                    normalize_form_key(&field.key),
                    frontend_flags_sensitive(field),
                )
            })
            .collect();

        for path in &sensitive_paths {
            let Some(&flagged_in_config_schema) = config_schema_flags.get(path) else {
                // Path has no matching CONFIG-schema key at all (e.g. it is
                // type-settings/instance-config only) — nothing the layer-3
                // override form can ever render, so not a UI hazard.
                continue;
            };
            checked_any_sensitive_path = true;
            assert!(
                flagged_in_config_schema,
                "{}: sensitive_paths entry '{path}' matches a CONFIG form-schema key that is \
                 NOT flagged sensitive/password/ssh_private_key there — the frontend layer-3 \
                 override form would render and submit it, but the server rejects it as \
                 sensitive (sensitive_paths = {sensitive_paths:?})",
                desc.type_id
            );
        }
    }

    assert!(
        checked_any_sensitive_path,
        "no descriptor's sensitive_paths matched any CONFIG form-schema key — this guard \
         would be vacuously green; a plugin with a CONFIG-schema secret field (e.g. docker's \
         auth.password) must exist under --all-features"
    );
}
