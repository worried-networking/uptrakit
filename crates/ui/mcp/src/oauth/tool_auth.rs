use uptrakit_controller_core::auth::Permission;
use uptrakit_web_api_types::oauth::McpScope;

use crate::context::{McpAuthMethod, McpRequestContext};

/// Static descriptor of the OAuth scopes and permissions required by an MCP tool.
///
/// The `required_scopes` slice holds only unit variants (`McpScope::Read`,
/// `McpScope::Write`) and is valid in a `const` context.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ToolAuth {
    pub required_scopes: &'static [McpScope],
    pub required_permissions: &'static [Permission],
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
pub fn require_scopes(ctx: &McpRequestContext, required: &[McpScope]) -> Result<(), McpScopeError> {
    #[expect(
        unreachable_patterns,
        reason = "McpAuthMethod is #[non_exhaustive]; wildcard arm handles future variants"
    )]
    match &ctx.auth_method {
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ctx_api_token() -> McpRequestContext {
        McpRequestContext::new(
            Uuid::nil(),
            Uuid::nil(),
            Uuid::nil(),
            vec![],
            McpAuthMethod::ApiToken,
        )
    }

    fn ctx_oauth(scopes: Vec<McpScope>) -> McpRequestContext {
        McpRequestContext::new(
            Uuid::nil(),
            Uuid::nil(),
            Uuid::nil(),
            vec![],
            McpAuthMethod::OAuth {
                client_id: "x".into(),
                jti: Uuid::nil(),
                scopes,
            },
        )
    }

    #[test]
    fn api_token_bypasses_scope_check() {
        require_scopes(&ctx_api_token(), &[McpScope::Write]).unwrap();
    }

    #[test]
    fn oauth_passes_when_scope_present() {
        require_scopes(
            &ctx_oauth(vec![McpScope::Read, McpScope::Write]),
            &[McpScope::Write],
        )
        .unwrap();
    }

    #[test]
    fn oauth_rejects_when_scope_missing() {
        let result = require_scopes(&ctx_oauth(vec![McpScope::Read]), &[McpScope::Write]);
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
            &ctx_oauth(vec![McpScope::Read, McpScope::Write]),
            &[McpScope::Read, McpScope::Write],
        )
        .unwrap();
    }

    #[test]
    fn oauth_empty_required_scopes_always_passes() {
        require_scopes(&ctx_oauth(vec![]), &[]).unwrap();
    }
}
