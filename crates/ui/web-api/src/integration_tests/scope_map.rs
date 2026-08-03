//! Scope-map golden: one-line security class per REST operation, derived
//! from the assembled OpenAPI document (spec 2026-08-03-access-route-sweep,
//! §Tests). Regenerate with UPDATE_SCOPE_MAP=1 and REVIEW THE DIFF — the
//! per-batch diff of this file is the review surface for every scope
//! string, and the only ongoing pin on extractor-less OR/dynamic
//! declarations. Not an independent oracle: it derives from the same
//! document the openapi.json staleness golden pins.

use std::collections::BTreeMap;

use crate::router::build_router_with_openapi;
use crate::test_harness::TestApp;

/// One operation → one class string.
fn classify(op: &serde_json::Value) -> String {
    if op
        .get("x-action-dynamic")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return "dynamic".to_string();
    }
    let Some(security) = op.get("security").and_then(serde_json::Value::as_array) else {
        return "public".to_string();
    };
    let mut oauth2_groups: Vec<Vec<String>> = Vec::new();
    for requirement in security {
        let Some(obj) = requirement.as_object() else {
            continue;
        };
        if obj.contains_key("bearer_token") {
            return "unconverted".to_string();
        }
        if let Some(scopes) = obj.get("oauth2").and_then(serde_json::Value::as_array) {
            let mut group: Vec<String> = scopes
                .iter()
                .filter_map(|scope| scope.as_str().map(str::to_string))
                .collect();
            group.sort();
            oauth2_groups.push(group);
        }
    }
    match oauth2_groups.as_slice() {
        [] => "no-oauth2".to_string(),
        [only] if only.is_empty() => "authenticated-only".to_string(),
        [only] => format!("oauth2:{}", only.join(",")),
        many => {
            let alternatives: Vec<String> = many.iter().map(|group| group.join(",")).collect();
            format!("or:{}", alternatives.join("|"))
        }
    }
}

const METHODS: [&str; 5] = ["get", "post", "put", "delete", "patch"];

#[tokio::test]
async fn scope_map_golden_is_up_to_date() {
    let app = TestApp::new().await;
    let (_router, api) = build_router_with_openapi(app.state.clone());
    let doc = serde_json::to_value(&api).expect("serialize OpenAPI");
    let paths = doc
        .get("paths")
        .and_then(serde_json::Value::as_object)
        .expect("paths object");
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for method in METHODS {
            if let Some(op) = item.get(method) {
                map.insert(format!("{method} {path}"), classify(op));
            }
        }
    }
    let generated = serde_json::to_string_pretty(&map).expect("serialize map") + "\n";
    let golden = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scope-map.golden.json");
    if std::env::var("UPDATE_SCOPE_MAP").is_ok() {
        std::fs::write(&golden, generated).expect("write scope-map.golden.json");
        return;
    }
    let committed = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
        panic!(
            "missing {}; regenerate with UPDATE_SCOPE_MAP=1 cargo test -p uptrakit-web-api --all-features scope_map",
            golden.display()
        )
    });
    assert_eq!(
        committed, generated,
        "scope-map.golden.json is stale; regenerate with UPDATE_SCOPE_MAP=1 and review the diff"
    );
}
