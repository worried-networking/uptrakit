use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uptrakit_web_api_types::oauth::{
    CanonicalUrlConfig, MCP_AUTH_SPEC_REVISION, ProtectedResourceMetadata,
};

use crate::state::McpState;

/// GET `/.well-known/oauth-protected-resource` (and `/mcp` sub-path).
///
/// Returns 404 when OAuth is disabled, RFC 9728 PRM JSON when enabled.
/// Includes `x-uptrakit-mcp-auth-spec-revision` per spec §23.1.
pub async fn get_prm(State(state): State<McpState>) -> Response {
    if !state.oauth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let canonical = match state.oauth_canonical.as_ref() {
        Some(c) => c,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let prm = build_prm(canonical.as_ref());

    // Serialize to Value and inject the spec-revision extension field.
    let mut json_val = match serde_json::to_value(&prm) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize PRM");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(obj) = json_val.as_object_mut() {
        obj.insert(
            "x-uptrakit-mcp-auth-spec-revision".to_owned(),
            serde_json::Value::String(MCP_AUTH_SPEC_REVISION.to_owned()),
        );
    }

    axum::Json(json_val).into_response()
}

fn build_prm(canonical: &CanonicalUrlConfig) -> ProtectedResourceMetadata {
    ProtectedResourceMetadata::new(
        canonical.primary_resource().as_str().to_owned(),
        vec![canonical.issuer().as_str().to_owned()],
        vec!["mcp:read".to_owned(), "mcp:write".to_owned()],
        vec!["header".to_owned()],
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_web_api_types::oauth::MCP_AUTH_SPEC_REVISION;

    #[test]
    fn spec_revision_constant_matches_expected() {
        assert_eq!(MCP_AUTH_SPEC_REVISION, "2025-11-25");
    }

    #[test]
    fn build_prm_sets_correct_resource_and_issuer() {
        let cfg = CanonicalUrlConfig::new("controller.example.com".into(), vec![]).unwrap();
        let prm = build_prm(&cfg);
        assert_eq!(prm.resource, "https://controller.example.com/mcp");
        assert_eq!(
            prm.authorization_servers,
            vec!["https://controller.example.com"]
        );
        assert_eq!(prm.bearer_methods_supported, vec!["header"]);
        assert!(prm.scopes_supported.contains(&"mcp:read".to_owned()));
        assert!(prm.scopes_supported.contains(&"mcp:write".to_owned()));
        assert!(prm.resource_documentation.is_none());
    }

    #[test]
    fn build_prm_serializes_with_spec_revision_extension() {
        let cfg = CanonicalUrlConfig::new("controller.example.com".into(), vec![]).unwrap();
        let prm = build_prm(&cfg);
        let mut json_val = serde_json::to_value(&prm).unwrap();
        if let Some(obj) = json_val.as_object_mut() {
            obj.insert(
                "x-uptrakit-mcp-auth-spec-revision".to_owned(),
                serde_json::Value::String(MCP_AUTH_SPEC_REVISION.to_owned()),
            );
        }
        assert_eq!(
            json_val["x-uptrakit-mcp-auth-spec-revision"],
            serde_json::Value::String("2025-11-25".to_owned())
        );
        assert_eq!(json_val["resource"], "https://controller.example.com/mcp");
        assert_eq!(
            json_val["authorization_servers"],
            serde_json::json!(["https://controller.example.com"])
        );
    }

    // Handler-level tests (get_prm with oauth_enabled=false → 404,
    // oauth_enabled=true → 200 + valid JSON) are covered by the Task 13
    // prefix-dispatch integration tests in uptrakit-mcp's integration test suite.
}
