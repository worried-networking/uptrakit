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
///     CanManageSoftware(user): CanManageSoftware,  // user.user_id needed
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
    /// Extractor that requires [`Permission::ViewSettings`].
    CanViewSettings => Permission::ViewSettings,
    /// Extractor that requires [`Permission::ManageSettings`].
    CanManageSettings => Permission::ManageSettings,
    /// Extractor that requires [`Permission::ViewAgents`].
    CanViewAgents => Permission::ViewAgents,
    /// Extractor that requires [`Permission::ManageAgents`].
    CanManageAgents => Permission::ManageAgents,
    /// Extractor that requires [`Permission::ManageGlobalSettings`].
    CanManageGlobalSettings => Permission::ManageGlobalSettings,
    /// Extractor that requires [`Permission::ViewSoftware`].
    CanViewSoftware => Permission::ViewSoftware,
    /// Extractor that requires [`Permission::ManageSoftware`].
    CanManageSoftware => Permission::ManageSoftware,
    /// Extractor that requires [`Permission::ViewHosts`].
    CanViewHosts => Permission::ViewHosts,
    /// Extractor that requires [`Permission::ManageHosts`].
    CanManageHosts => Permission::ManageHosts,
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
        // User only has ViewHosts, but extractor requires ManageHosts
        let mut parts = parts_with_user(vec![Permission::ViewHosts]);
        let result = CanManageHosts::from_request_parts(&mut parts, &()).await;
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
            Permission::ManageSoftware,
            Permission::ViewHosts,
            Permission::ViewAgents,
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
        let extractor = CanManageAgents::new(user.clone());
        assert_eq!(extractor.0.user_id, user.user_id);
    }
}
