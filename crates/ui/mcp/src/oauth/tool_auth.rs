use rmcp::ErrorData;
use uptrakit_shared_types::access::{Action, Decision};
use uptrakit_web_api_types::oauth::McpScope;

use crate::context::{McpAuthMethod, McpRequestContext};
use crate::state::McpState;

/// Static descriptor of the OAuth scopes and catalog actions required by an
/// MCP tool.
///
/// The `required_scopes` slice holds only unit variants (`McpScope::Read`,
/// `McpScope::Write`) and is valid in a `const` context.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ToolAuth {
    pub required_scopes: &'static [McpScope],
    pub required_actions: &'static [Action],
}

/// Error returned when the caller's OAuth scopes are insufficient.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum McpScopeError {
    #[error("insufficient scope: required {required:?}")]
    Insufficient { required: McpScope },
}

/// All-of scope check. API-token contexts bypass (no scope concept).
///
/// # Errors
///
/// Returns [`McpScopeError::Insufficient`] when an OAuth context is missing a
/// required scope.
pub fn require_scopes(
    auth_method: &McpAuthMethod,
    required: &[McpScope],
) -> Result<(), McpScopeError> {
    #[expect(
        unreachable_patterns,
        reason = "McpAuthMethod is #[non_exhaustive]; wildcard arm handles future variants"
    )]
    match auth_method {
        McpAuthMethod::ApiToken => Ok(()),
        McpAuthMethod::OAuth { scopes, .. } => {
            for r in required {
                if !scopes.contains(r) {
                    return Err(McpScopeError::Insufficient {
                        required: r.clone(),
                    });
                }
            }
            Ok(())
        }
        _ => {
            tracing::warn!("unhandled McpAuthMethod variant in require_scopes; defaulting to deny");
            Err(McpScopeError::Insufficient {
                required: required.first().cloned().unwrap_or(McpScope::Read),
            })
        }
    }
}

/// Unified per-tool authorization: OAuth scope check, then one engine
/// `authorize` per declared action. Declaration = enforcement: a tool whose
/// `ToolAuth` lists an action gets the engine check by construction.
pub fn require_tool_auth(
    state: &McpState,
    ctx: &McpRequestContext,
    auth: &ToolAuth,
) -> Result<(), ErrorData> {
    require_scopes(&ctx.auth_method, auth.required_scopes)
        .map_err(|e| ErrorData::invalid_request(format!("insufficient_scope: {e}"), None))?;
    for action in auth.required_actions {
        match state.access_engine.authorize(&ctx.access, action, None) {
            Decision::Allow => {}
            Decision::Deny(reason) => {
                metrics::counter!(
                    "uptrakit_access_denies_total",
                    "reason" => reason.as_str()
                )
                .increment(1);
                return Err(ErrorData::invalid_request(
                    format!("permission denied: {action} required"),
                    None,
                ));
            }
            // `Decision` is #[non_exhaustive] in another crate.
            _ => {
                return Err(ErrorData::invalid_request(
                    format!("permission denied: {action} required"),
                    None,
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn oauth_method(scopes: Vec<McpScope>) -> McpAuthMethod {
        McpAuthMethod::OAuth {
            client_id: "x".into(),
            jti: Uuid::nil(),
            scopes,
        }
    }

    #[test]
    fn api_token_bypasses_scope_check() {
        require_scopes(&McpAuthMethod::ApiToken, &[McpScope::Write]).unwrap();
    }

    #[test]
    fn oauth_passes_when_scope_present() {
        require_scopes(
            &oauth_method(vec![McpScope::Read, McpScope::Write]),
            &[McpScope::Write],
        )
        .unwrap();
    }

    #[test]
    fn oauth_rejects_when_scope_missing() {
        let result = require_scopes(&oauth_method(vec![McpScope::Read]), &[McpScope::Write]);
        assert!(matches!(
            result,
            Err(McpScopeError::Insufficient {
                required: McpScope::Write
            })
        ));
    }

    #[test]
    fn oauth_passes_when_all_required_scopes_present() {
        require_scopes(
            &oauth_method(vec![McpScope::Read, McpScope::Write]),
            &[McpScope::Read, McpScope::Write],
        )
        .unwrap();
    }

    #[test]
    fn oauth_empty_required_scopes_always_passes() {
        require_scopes(&oauth_method(vec![]), &[]).unwrap();
    }
}
