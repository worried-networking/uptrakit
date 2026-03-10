use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::Response;
use http::StatusCode;

use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;

// ---------------------------------------------------------------------------
// Macro that generates one concrete extractor struct per permission
// ---------------------------------------------------------------------------

/// Generates a concrete Axum extractor struct for a single permission.
///
/// Each generated struct `CanXxx(pub AuthenticatedUser)` implements
/// [`FromRequestParts`]. It reads the [`AuthenticatedUser`] that was placed
/// into request extensions by the `require_auth` middleware, checks the
/// required permission, and returns:
///
/// - `401 Unauthorized` — if no `AuthenticatedUser` extension is present.
/// - `403 Forbidden` — if the user lacks the required permission.
/// - `Ok(CanXxx(user))` — on success; the inner `AuthenticatedUser` is
///   accessible via tuple-struct destructuring in handler arguments:
///
/// ```ignore
/// pub async fn list_hosts(
///     CanViewHosts(_user): CanViewHosts,
/// ) -> Response { … }
///
/// pub async fn trigger_update(
///     CanTriggerUpdates(user): CanTriggerUpdates,  // user.user_id needed
/// ) -> Response { … }
/// ```
macro_rules! permission_extractor {
    ($(
        $(#[$meta:meta])*
        $name:ident => $perm:expr
    ),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Debug)]
            pub struct $name(pub AuthenticatedUser);

            impl $name {
                /// Construct the extractor directly from an authenticated user,
                /// bypassing the HTTP extraction layer.
                ///
                /// Intended for unit tests that call handler functions directly
                /// rather than through a full HTTP round-trip. Does **not**
                /// perform any permission check — the caller is responsible for
                /// supplying an appropriate [`AuthenticatedUser`].
                pub fn new(user: AuthenticatedUser) -> Self {
                    Self(user)
                }
            }

            impl<S: Send + Sync> FromRequestParts<S> for $name {
                type Rejection = Response;

                async fn from_request_parts(
                    parts: &mut Parts,
                    _state: &S,
                ) -> Result<Self, Self::Rejection> {
                    let user = parts
                        .extensions
                        .get::<AuthenticatedUser>()
                        .cloned()
                        .ok_or_else(|| {
                            error_response(StatusCode::UNAUTHORIZED, "Authentication required")
                        })?;

                    if !user.has_permission($perm) {
                        return Err(error_response(
                            StatusCode::FORBIDDEN,
                            "Insufficient permissions",
                        ));
                    }

                    Ok($name(user))
                }
            }
        )*
    };
}

// ---------------------------------------------------------------------------
// Permission extractors (one per Permission variant)
// ---------------------------------------------------------------------------

permission_extractor! {
    // -- Settings ----------------------------------------------------------
    /// Extractor that requires [`Permission::ViewSettings`].
    CanViewSettings => Permission::ViewSettings,
    /// Extractor that requires [`Permission::ManageGlobalSettings`].
    CanManageGlobalSettings => Permission::ManageGlobalSettings,
    /// Extractor that requires [`Permission::ManageAuthSettings`].
    CanManageAuthSettings => Permission::ManageAuthSettings,
    /// Extractor that requires [`Permission::ManageEnrollmentTokens`].
    CanManageEnrollmentTokens => Permission::ManageEnrollmentTokens,
    /// Extractor that requires [`Permission::ManageAgentCerts`].
    CanManageAgentCerts => Permission::ManageAgentCerts,

    // -- Services (formerly Agents) ----------------------------------------
    /// Extractor that requires [`Permission::ViewServices`].
    CanViewServices => Permission::ViewServices,
    /// Extractor that requires [`Permission::ApproveServices`].
    CanApproveServices => Permission::ApproveServices,
    /// Extractor that requires [`Permission::RejectServices`].
    CanRejectServices => Permission::RejectServices,
    /// Extractor that requires [`Permission::RemoveServices`].
    CanRemoveServices => Permission::RemoveServices,
    /// Extractor that requires [`Permission::UpdateServices`].
    CanUpdateServices => Permission::UpdateServices,

    // -- Software ----------------------------------------------------------
    /// Extractor that requires [`Permission::ViewSoftware`].
    CanViewSoftware => Permission::ViewSoftware,
    /// Extractor that requires [`Permission::CreateSoftware`].
    CanCreateSoftware => Permission::CreateSoftware,
    /// Extractor that requires [`Permission::UpdateSoftware`].
    CanUpdateSoftware => Permission::UpdateSoftware,
    /// Extractor that requires [`Permission::DeleteSoftware`].
    CanDeleteSoftware => Permission::DeleteSoftware,
    /// Extractor that requires [`Permission::TriggerChecks`].
    CanTriggerChecks => Permission::TriggerChecks,
    /// Extractor that requires [`Permission::TriggerUpdates`].
    CanTriggerUpdates => Permission::TriggerUpdates,
    /// Extractor that requires [`Permission::ManageScheduler`].
    CanManageScheduler => Permission::ManageScheduler,

    // -- Commands ----------------------------------------------------------
    /// Extractor that requires [`Permission::ManageCommands`].
    ///
    /// Grants ability to modify command-bearing plugin config fields. Treat
    /// this as equivalent to root access on all managed hosts.
    CanManageCommands => Permission::ManageCommands,

    // -- Hosts -------------------------------------------------------------
    /// Extractor that requires [`Permission::ViewHosts`].
    CanViewHosts => Permission::ViewHosts,
    /// Extractor that requires [`Permission::UpdateHosts`].
    CanUpdateHosts => Permission::UpdateHosts,
    /// Extractor that requires [`Permission::DeactivateHosts`].
    CanDeactivateHosts => Permission::DeactivateHosts,

    // -- Notifications -----------------------------------------------------
    /// Extractor that requires [`Permission::ViewNotifications`].
    CanViewNotifications => Permission::ViewNotifications,
    /// Extractor that requires [`Permission::ManageNotifications`].
    CanManageNotifications => Permission::ManageNotifications,

    // -- System services ---------------------------------------------------
    /// Extractor that requires [`Permission::ViewSystemServices`].
    CanViewSystemServices => Permission::ViewSystemServices,
    /// Extractor that requires [`Permission::ApproveSystemServices`].
    CanApproveSystemServices => Permission::ApproveSystemServices,
    /// Extractor that requires [`Permission::RejectSystemServices`].
    CanRejectSystemServices => Permission::RejectSystemServices,
    /// Extractor that requires [`Permission::RemoveSystemServices`].
    CanRemoveSystemServices => Permission::RemoveSystemServices,
    /// Extractor that requires [`Permission::UpdateSystemServices`].
    CanUpdateSystemServices => Permission::UpdateSystemServices,

    // -- Audit -------------------------------------------------------------
    /// Extractor that requires [`Permission::ViewAuditLogs`].
    CanViewAuditLogs => Permission::ViewAuditLogs,
    /// Extractor that requires [`Permission::ViewSystemAuditLogs`].
    CanViewSystemAuditLogs => Permission::ViewSystemAuditLogs,

    // -- Users -------------------------------------------------------------
    /// Extractor that requires [`Permission::ManageUsers`].
    CanManageUsers => Permission::ManageUsers,

    // -- Ignores -----------------------------------------------------------
    /// Extractor that requires [`Permission::ManageIgnores`].
    CanManageIgnores => Permission::ManageIgnores,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::FromRequestParts;
    use axum::http::Request;

    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;

    /// Build request `Parts` with an `AuthenticatedUser` pre-inserted.
    fn parts_with_user(permissions: Vec<Permission>) -> axum::http::request::Parts {
        let mut req = Request::new(Body::empty());
        req.extensions_mut().insert(AuthenticatedUser {
            user_id: uuid::Uuid::nil(),
            auth_method: AuthMethod::Password,
            permissions,
        });
        req.into_parts().0
    }

    /// Build request `Parts` with **no** `AuthenticatedUser` extension.
    fn parts_without_user() -> axum::http::request::Parts {
        Request::new(Body::empty()).into_parts().0
    }

    #[tokio::test]
    async fn missing_auth_extension_returns_401() {
        let mut parts = parts_without_user();
        let result = CanViewHosts::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 401);
    }

    #[tokio::test]
    async fn correct_permission_passes_through() {
        let mut parts = parts_with_user(vec![Permission::ViewHosts]);
        let result = CanViewHosts::from_request_parts(&mut parts, &()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wrong_permission_returns_403() {
        // User only has ViewHosts, but extractor requires UpdateHosts
        let mut parts = parts_with_user(vec![Permission::ViewHosts]);
        let result = CanUpdateHosts::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 403);
    }

    #[tokio::test]
    async fn no_permissions_returns_403() {
        let mut parts = parts_with_user(vec![]);
        let result = CanViewHosts::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), 403);
    }

    #[tokio::test]
    async fn multiple_permissions_one_matches() {
        let mut parts = parts_with_user(vec![
            Permission::CreateSoftware,
            Permission::ViewHosts,
            Permission::ViewServices,
        ]);
        let result = CanViewHosts::from_request_parts(&mut parts, &()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn authenticated_user_is_accessible_in_extractor() {
        let user_id = uuid::Uuid::now_v7();
        let mut parts = Request::new(Body::empty()).into_parts().0;
        parts.extensions.insert(AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewHosts],
        });
        let CanViewHosts(extracted_user) = CanViewHosts::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(extracted_user.user_id, user_id);
    }

    #[tokio::test]
    async fn new_constructor_bypasses_check() {
        let user = AuthenticatedUser {
            user_id: uuid::Uuid::nil(),
            auth_method: AuthMethod::Password,
            permissions: vec![], // no permissions
        };
        // new() skips the permission check — intended for direct handler tests
        let extractor = CanApproveServices::new(user.clone());
        assert_eq!(extractor.0.user_id, user.user_id);
    }
}
