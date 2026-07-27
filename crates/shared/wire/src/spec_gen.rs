//! Test-only AsyncAPI 3.0.0 document generator. The committed
//! `asyncapi.yaml` is produced by this module via the golden test added in a
//! later task (`UPDATE_ASYNCAPI=1`). See docs/adr/0029-asyncapi-codegen.md.
//!
//! This whole module only exists in test builds (`#[cfg(test)] mod spec_gen;`
//! in `lib.rs`) and its schemars/serde_yaml_ng-dependent items are further
//! gated on the additive `schema` feature, which is off by default. Each item
//! carries its own `#[cfg(feature = "schema")]` rather than the module being
//! declared under a combined `#[cfg(all(test, feature = "schema"))]`
//! predicate: clippy's `allow-expect-in-tests` / `allow-indexing-slicing-in-tests`
//! only recognize a literal `#[cfg(test)]` ancestor (the `mod spec_gen;`
//! declaration in lib.rs), never a compound `all(...)` predicate. The
//! `.expect()`/indexing calls below are intentional loud-fail guards against a
//! schemars shape change — see the probe methodology note in the task-3
//! brief.

#[cfg(feature = "schema")]
use std::collections::BTreeMap;

#[cfg(feature = "schema")]
use serde_json::{Map, Value, json};

/// Envelope fields injected into every message payload schema — the wire
/// flattens `ServiceMessage`/`ControllerMessage` into the envelope object
/// (`#[serde(flatten)]`, envelope.rs), so each documented message is one flat
/// object. `pagination` only exists on the service->controller side.
#[cfg(feature = "schema")]
fn envelope_properties(service_side: bool) -> Map<String, Value> {
    let mut props = Map::new();
    props.insert(
        "protocol_version".into(),
        json!({"type": "integer", "format": "uint32", "minimum": 0}),
    );
    props.insert(
        "seq".into(),
        json!({"type": "integer", "format": "uint64", "minimum": 0}),
    );
    props.insert(
        "trace_context".into(),
        json!({"$ref": "#/components/schemas/TraceContext"}),
    );
    if service_side {
        props.insert(
            "pagination".into(),
            json!({"$ref": "#/components/schemas/ReportPagination"}),
        );
    }
    props
}

/// Rewrite every `#/$defs/Foo` ref to `#/components/schemas/Foo`, recursively.
/// schemars 1.x (2020-12 dialect) emits `$defs`; AsyncAPI documents keep
/// shared schemas under `components/schemas`.
#[cfg(feature = "schema")]
fn rewrite_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get_mut("$ref")
                && let Some(name) = r.strip_prefix("#/$defs/")
            {
                *r = format!("#/components/schemas/{name}");
            }
            for v in map.values_mut() {
                rewrite_refs(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                rewrite_refs(v);
            }
        }
        _ => {}
    }
}

/// Recursively collect every `"$ref"` string value under `value`.
#[cfg(feature = "schema")]
fn collect_refs(value: &Value, refs: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$ref") {
                refs.push(r.clone());
            }
            for v in map.values() {
                collect_refs(v, refs);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_refs(v, refs);
            }
        }
        _ => {}
    }
}

/// Probe finding (verified against the real schemars 1.x output for
/// `ServiceMessage`/`ControllerMessage`, both internally-tagged
/// `#[serde(tag = "type", rename_all = "snake_case")]` enums): each `oneOf`
/// element is a flat object carrying `properties.type.const` (the
/// discriminant) plus, for variants with a non-empty payload struct, a
/// *sibling* `"$ref": "#/$defs/XPayload"` at the same level (JSON Schema
/// 2020-12 allows `$ref` alongside other keywords). This is neither the
/// brief's "flat" nor "allOf" branch verbatim — it is the indirected case
/// with the indirection expressed as a sibling `$ref` instead of `allOf`.
/// Variants whose payload schema is trivially "any" (`#[schemars(with =
/// "serde_json::Value")]` surface-bearing payloads) have no `$ref` at all —
/// schemars omits referencing a no-op schema.
///
/// Both cases are handled by the same merge: when `$ref` is present, fold the
/// referenced `$defs` entry's `properties`/`required` into the variant (never
/// mutate the shared `$defs` entry itself — clone it first) and record its
/// name as orphaned so the caller can drop it from `components/schemas`
/// afterward (nothing else references it: each payload struct backs exactly
/// one message); when absent, the variant is already the complete
/// (permissive) payload.
#[cfg(feature = "schema")]
fn merge_variant(
    mut variant: Value,
    defs: &Map<String, Value>,
    orphaned: &mut Vec<String>,
) -> Value {
    let ref_name = variant
        .as_object()
        .expect("oneOf element is an object")
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
        .map(str::to_string);

    if let Some(name) = ref_name {
        let payload_def = defs
            .get(&name)
            .expect("referenced $defs entry exists")
            .clone();
        let payload_obj = payload_def.as_object().expect("payload def is an object");

        let obj = variant.as_object_mut().expect("variant is an object");
        obj.remove("$ref");
        let props = obj
            .entry("properties")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("properties is an object");
        if let Some(Value::Object(payload_props)) = payload_obj.get("properties") {
            for (k, v) in payload_props {
                props.insert(k.clone(), v.clone());
            }
        }
        let required = obj
            .entry("required")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("required is an array");
        if let Some(Value::Array(payload_required)) = payload_obj.get("required") {
            for r in payload_required {
                required.push(r.clone());
            }
        }

        orphaned.push(name);
    }

    variant
}

/// Strip the two fields that legitimately differ between the
/// `ServiceMessage` and `ControllerMessage` renderings of the *same*
/// discriminant (`surface_action_request`/`surface_action_response` are
/// declared in both enums — see messages.rs): `description` (each enum
/// documents the variant from its own side, different doc comment text) and
/// `properties.pagination` (injected only for `service_side`, mirroring the
/// real `ServiceEnvelope`/`ControllerEnvelope` asymmetry in envelope.rs — not
/// a property of the message itself). Anything else that differs after this
/// normalization is a genuine divergence.
#[cfg(feature = "schema")]
fn normalize_for_direction_comparison(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("description");
        if let Some(Value::Object(props)) = obj.get_mut("properties") {
            props.remove("pagination");
        }
    }
    value
}

/// Insert `(key, value)` into `map`, tolerating a re-insert of the *same*
/// key when the two values are equal up to the known, by-design
/// service/controller asymmetry (see `normalize_for_direction_comparison`).
/// When both carry `pagination`-bearing content, the entry that documents it
/// wins — the field is always optional, so documenting it on the direction
/// that never sends it is harmless and strictly more informative. Panics
/// loudly on any other divergence — that would be a real bug, not a
/// legitimate duplicate.
#[cfg(feature = "schema")]
fn insert_or_assert_same(map: &mut Map<String, Value>, key: String, value: Value) {
    if let Some(existing) = map.get(&key) {
        assert_eq!(
            normalize_for_direction_comparison(existing),
            normalize_for_direction_comparison(&value),
            "key {key:?} inserted twice with incompatible schemas (beyond the expected \
             description/pagination direction asymmetry) — ServiceMessage/ControllerMessage \
             discriminant collision has diverged: existing={existing:?} new={value:?}"
        );
        let existing_has_pagination = existing
            .get("properties")
            .and_then(|p| p.get("pagination"))
            .is_some();
        if !existing_has_pagination {
            map.insert(key, value);
        }
        return;
    }
    map.insert(key, value);
}

/// Assemble the complete AsyncAPI 3.0.0 document from the schemars output of
/// the wire message enums and envelope helper types. Deterministic: two
/// successive calls produce byte-identical output. `serde_json::Map` is
/// `IndexMap`-backed here (the workspace enables schemars' `preserve_order`
/// feature, which also turns on serde_json's), i.e. **insertion order**, not
/// alphabetical — so `schemas`/`messages` are sorted into `BTreeMap`s before
/// being handed to `json!` below, keeping output byte-stable independent of
/// iteration order over the two schemars calls.
#[cfg(feature = "schema")]
pub(crate) fn generate_asyncapi_yaml() -> String {
    let mut schemas: Map<String, Value> = Map::new();
    let mut messages: Map<String, Value> = Map::new();
    let mut service_message_names = Vec::new();
    let mut controller_message_names = Vec::new();

    for (enum_schema, service_side) in [
        (schemars::schema_for!(crate::ServiceMessage), true),
        (schemars::schema_for!(crate::ControllerMessage), false),
    ] {
        let mut root = serde_json::to_value(&enum_schema).expect("schema to JSON");

        // Hoist shared definitions first (RAW, pre-rewrite — merge_variant
        // below matches `$ref`s in their original `#/$defs/Foo` form), so
        // merge_variant can look payload structs up by name.
        let mut defs = Map::new();
        if let Some(Value::Object(d)) = root
            .as_object_mut()
            .expect("root is an object")
            .remove("$defs")
        {
            defs = d;
        }

        let variants = root
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("oneOf array present (verify vs probe)")
            .clone();

        let mut orphaned = Vec::new();
        for variant in variants {
            let discriminant = variant["properties"]["type"]["const"]
                .as_str()
                .expect("type const discriminant (verify vs probe)")
                .to_string();

            let mut merged = merge_variant(variant, &defs, &mut orphaned);

            // Inject envelope fields (flat wire object).
            let obj = merged.as_object_mut().expect("merged variant is an object");
            let props = obj
                .entry("properties")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("properties is an object");
            for (k, v) in envelope_properties(service_side) {
                props.insert(k, v);
            }
            let required = obj
                .entry("required")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("required is an array");
            for r in ["protocol_version", "seq"] {
                required.push(json!(r));
            }

            // Now that the payload struct's own fields (which may themselves
            // reference other `$defs` entries, e.g. `Capability`) are folded
            // in, rewrite every `#/$defs/Foo` ref in the merged object to
            // `#/components/schemas/Foo`.
            rewrite_refs(&mut merged);

            let payload_schema_key = format!("{discriminant}Payload");
            insert_or_assert_same(&mut schemas, payload_schema_key.clone(), merged);
            insert_or_assert_same(
                &mut messages,
                discriminant.clone(),
                json!({
                    "name": discriminant,
                    "payload": { "$ref": format!("#/components/schemas/{payload_schema_key}") },
                }),
            );
            if service_side {
                service_message_names.push(discriminant);
            } else {
                controller_message_names.push(discriminant);
            }
        }

        // Hoist the remaining (non-orphaned) shared definitions — field
        // types like `Capability`, `UpdateCategory`, nested structs —
        // referenced from within payload properties.
        for (name, mut def) in defs {
            if orphaned.contains(&name) {
                continue;
            }
            rewrite_refs(&mut def);
            insert_or_assert_same(&mut schemas, name, def);
        }
    }

    // TraceContext / ReportPagination for the envelope refs.
    for (name, schema) in [
        (
            "TraceContext",
            serde_json::to_value(schemars::schema_for!(crate::TraceContext))
                .expect("schema to JSON"),
        ),
        (
            "ReportPagination",
            serde_json::to_value(schemars::schema_for!(crate::ReportPagination))
                .expect("schema to JSON"),
        ),
    ] {
        let mut s = schema;
        rewrite_refs(&mut s);
        let obj = s.as_object_mut().expect("schema is an object");
        obj.remove("$schema");
        schemas.insert(name.to_string(), s);
    }

    // `schemas`/`messages` are `IndexMap`-backed (insertion order, see the
    // doc comment above); sort them into `BTreeMap`s so the golden document
    // is byte-stable independent of the order the two schemars calls (and
    // their internal $defs ordering) happen to produce.
    let schemas: BTreeMap<String, Value> = schemas.into_iter().collect();
    let messages: BTreeMap<String, Value> = messages.into_iter().collect();

    service_message_names.sort();
    controller_message_names.sort();

    let channel_messages: Map<String, Value> = service_message_names
        .iter()
        .chain(controller_message_names.iter())
        .map(|name| {
            (
                name.clone(),
                json!({"$ref": format!("#/components/messages/{name}")}),
            )
        })
        .collect();

    let send_message_refs: Vec<Value> = service_message_names
        .iter()
        .map(|name| json!({"$ref": format!("#/channels/service/messages/{name}")}))
        .collect();
    let receive_message_refs: Vec<Value> = controller_message_names
        .iter()
        .map(|name| json!({"$ref": format!("#/channels/service/messages/{name}")}))
        .collect();

    let doc = json!({
        "asyncapi": "3.0.0",
        "info": {
            "title": "Uptrakit Service-Controller Protocol",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Generated from the Rust wire types (uptrakit-wire). Do not edit by hand; \
                regenerate with the golden test in tests.rs (UPDATE_ASYNCAPI=1). Protocol narrative \
                (enrollment lifecycle, CSR issuance, reconnection semantics): docs/api/wire-protocol.md.",
        },
        "channels": {
            "service": {
                "address": "/api/v1/ws/service",
                "messages": channel_messages,
            }
        },
        "operations": {
            "sendToController": {
                "action": "send",
                "channel": { "$ref": "#/channels/service" },
                "messages": send_message_refs,
            },
            "receiveFromController": {
                "action": "receive",
                "channel": { "$ref": "#/channels/service" },
                "messages": receive_message_refs,
            },
        },
        "components": { "schemas": schemas, "messages": messages },
    });
    serde_yaml_ng::to_string(&doc).expect("YAML serialize")
}

#[cfg(feature = "schema")]
#[test]
fn generated_doc_has_no_dangling_refs() {
    let yaml = generate_asyncapi_yaml();
    assert!(
        !yaml.contains("#/$defs/"),
        "unrewritten $defs ref left in document"
    );
    let doc: Value = serde_yaml_ng::from_str(&yaml).expect("parse own output");
    let schemas = doc["components"]["schemas"].as_object().expect("schemas");
    let messages = doc["components"]["messages"].as_object().expect("messages");
    let mut refs = Vec::new();
    collect_refs(&doc, &mut refs);
    for r in &refs {
        // Three legitimate ref namespaces: payload schemas, the
        // channel/operation message refs, and the channel self-refs used by
        // operations.
        if let Some(name) = r.strip_prefix("#/components/schemas/") {
            assert!(schemas.contains_key(name), "dangling schema $ref: {r}");
        } else if let Some(name) = r.strip_prefix("#/components/messages/") {
            assert!(messages.contains_key(name), "dangling message $ref: {r}");
        } else if r.starts_with("#/channels/service/messages/") || r == "#/channels/service" {
            // Structural refs into the channel we generated ourselves.
        } else {
            panic!("ref outside known namespaces: {r}");
        }
    }
}

#[cfg(feature = "schema")]
#[test]
fn enroll_payload_is_complete_after_merge() {
    // Regression guard: `merge_variant` must actually fold the referenced
    // `$defs` payload struct's fields into the message schema. `EnrollPayload`
    // (message "enroll") has real named fields — if the merge silently
    // no-ops (e.g. because refs were already rewritten to
    // `#/components/schemas/...` before the merge ran its `#/$defs/` prefix
    // match), the payload schema below would contain only the envelope
    // fields plus a leftover `$ref`, and the raw `EnrollPayload` `$defs`
    // entry would survive unreferenced under its own name.
    let yaml = generate_asyncapi_yaml();
    let doc: Value = serde_yaml_ng::from_str(&yaml).expect("parse");
    let schemas = doc["components"]["schemas"].as_object().expect("schemas");

    let enroll_payload = schemas
        .get("enrollPayload")
        .expect("enrollPayload schema present");
    assert!(
        enroll_payload.get("$ref").is_none(),
        "enrollPayload still carries a raw $ref — merge did not run: {enroll_payload:?}"
    );
    let props = enroll_payload["properties"]
        .as_object()
        .expect("properties");
    for field in [
        "hostname",
        "friendly_name",
        "capabilities",
        "service_app_name",
    ] {
        assert!(
            props.contains_key(field),
            "enrollPayload missing real field {field:?} — merge did not fold payload fields in: {props:?}"
        );
    }

    // The raw `$defs`-named twin must not survive as an orphaned,
    // unreferenced schema after the merge.
    assert!(
        !schemas.contains_key("EnrollPayload"),
        "orphaned EnrollPayload $defs entry was not dropped after merge"
    );
}

#[cfg(feature = "schema")]
#[test]
fn generated_doc_is_deterministic() {
    assert_eq!(generate_asyncapi_yaml(), generate_asyncapi_yaml());
}

#[cfg(feature = "schema")]
#[test]
fn every_real_variant_has_a_message_and_unknown_is_absent() {
    let yaml = generate_asyncapi_yaml();
    let doc: Value = serde_yaml_ng::from_str(&yaml).expect("parse");
    let messages = doc["components"]["messages"].as_object().expect("messages");
    for expected in [
        "ping",
        "pong",
        "surface_registration",
        "audit_event",
        "token_revoked",
        "workload_claim_sync_request",
        "test_plugin_config",
    ] {
        assert!(
            messages.contains_key(expected),
            "missing message: {expected}"
        );
    }
    assert!(
        !messages.contains_key("unknown"),
        "schemars(skip) on Unknown regressed"
    );
    assert!(
        !messages.contains_key("Unknown"),
        "schemars(skip) on Unknown regressed"
    );
}
