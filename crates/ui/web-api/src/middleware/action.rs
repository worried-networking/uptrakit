//! Action-based route extractors over the [`AccessEngine`] (M1.4a).
//!
//! Converted route families declare authorization with these extractors +
//! native `security(("oauth2" = ["<action>"]), ("developer_token" = []))`
//! requirements; unconverted families keep `permission_extractor!` +
//! `x-required-permission` until the M1.4b sweep. Verdicts: 401 no
//! principal, 403 `Decision::Deny`, 500 engine unavailable (fail-closed).

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::response::Response;
use http::StatusCode;
use uptrakit_controller_core::access::{AccessContext, AccessEngine};
use uptrakit_shared_types::access::{Action, Decision, DenyReason, actions};

use crate::app_state::AccessState;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;

/// Per-request authorization state, inserted by `require_auth`.
///
/// `Unavailable` means the engine could not resolve the principal's grants
/// (DB failure): action extractors fail closed with 500; routes without an
/// action extractor (`me`, `logout`, unconverted families) proceed —
/// rendering the verdict here, not in the middleware, is what preserves
/// `me`'s documented 200-on-DB-blip guard (`routes/auth.rs`) until M1.7.
#[derive(Clone)]
#[non_exhaustive]
pub enum AccessAuthority {
    /// Grants resolved; carries the per-request context.
    Ready(AccessContext),
    /// Engine failure — no authority information for this request.
    Unavailable,
}

impl AccessAuthority {
    /// The resolved context, or `None` when no authority is available.
    ///
    /// Inline enforcement sites (M1.5) use this instead of matching the
    /// variant directly: `AccessAuthority` is `#[non_exhaustive]`, so a
    /// hand-written match needs a wildcard arm at every call site and a
    /// future variant would silently take the wildcard's verdict. Callers
    /// render their own `None` response — the interactive-WS route also
    /// emits an audit row there, so the response cannot be shared.
    #[must_use]
    pub(crate) fn ready(&self) -> Option<&AccessContext> {
        match self {
            Self::Ready(ctx) => Some(ctx),
            _ => None,
        }
    }
}

/// Generates a concrete Axum extractor struct for a single catalog action.
///
/// Same ergonomic shape as `permission_extractor!` (`middleware/permission.rs`)
/// plus one new outcome: 500 when [`AccessAuthority::Unavailable`].
macro_rules! action_extractor {
    ($(
        $(#[$meta:meta])*
        $name:ident => $action:expr
    ),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Debug)]
            pub struct $name(pub AuthenticatedUser);

            impl $name {
                /// Test-only constructor bypassing the authorization check,
                /// for direct handler unit tests.
                #[must_use]
                pub fn new(user: AuthenticatedUser) -> Self {
                    Self(user)
                }
            }

            impl<S> FromRequestParts<S> for $name
            where
                S: Send + Sync,
                AccessState: FromRef<S>,
            {
                type Rejection = Response;

                async fn from_request_parts(
                    parts: &mut Parts,
                    state: &S,
                ) -> Result<Self, Self::Rejection> {
                    let user = parts
                        .extensions
                        .get::<AuthenticatedUser>()
                        .cloned()
                        .ok_or_else(|| {
                            error_response(
                                StatusCode::UNAUTHORIZED,
                                "Authentication required",
                            )
                        })?;
                    let ctx = match parts.extensions.get::<AccessAuthority>() {
                        Some(AccessAuthority::Ready(ctx)) => ctx.clone(),
                        Some(AccessAuthority::Unavailable) => {
                            tracing::error!(
                                action = %$action,
                                user_id = %user.user_id,
                                "authorization unavailable: access engine failed for this request"
                            );
                            return Err(error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            ));
                        }
                        None => {
                            // Auth ran (AuthenticatedUser present) but the
                            // marker is missing: broken server invariant —
                            // require_auth inserts both together.
                            tracing::error!(
                                action = %$action,
                                "AccessAuthority extension missing on an authenticated request (wiring bug)"
                            );
                            return Err(error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            ));
                        }
                    };
                    let engine = AccessState::from_ref(state).0;
                    match engine.authorize(&ctx, &$action, None) {
                        Decision::Allow => Ok($name(user)),
                        Decision::Deny(reason) => {
                            tracing::debug!(
                                action = %$action,
                                user_id = %user.user_id,
                                reason = reason.as_str(),
                                "action denied"
                            );
                            metrics::counter!(
                                "uptrakit_access_denies_total",
                                "reason" => reason.as_str()
                            )
                            .increment(1);
                            Err(error_response(
                                StatusCode::FORBIDDEN,
                                "Insufficient permissions",
                            ))
                        }
                        // `Decision` is #[non_exhaustive] in another crate.
                        _ => Err(error_response(
                            StatusCode::FORBIDDEN,
                            "Insufficient permissions",
                        )),
                    }
                }
            }
        )*
    };
}

action_extractor! {
    /// `hosts:read` — list/get hosts.
    CanReadHosts => actions::HOSTS_READ,
    /// `hosts:update` — edit host properties.
    CanUpdateHosts => actions::HOSTS_UPDATE,
    /// `hosts:delete` — deactivate hosts (single + batch).
    CanDeleteHosts => actions::HOSTS_DELETE,
    /// `checks:trigger` — trigger version checks / discovery.
    CanTriggerChecks => actions::CHECKS_TRIGGER,
    /// `services:read` — list/get tenant services.
    CanReadServices => actions::SERVICES_READ,
    /// `services:approve` — approve pending service enrollments.
    CanApproveServices => actions::SERVICES_APPROVE,
    /// `services:reject` — reject pending service enrollments.
    CanRejectServices => actions::SERVICES_REJECT,
    /// `services:delete` — deactivate/remove services.
    CanDeleteServices => actions::SERVICES_DELETE,
    /// `services:update` — update service settings.
    CanUpdateServices => actions::SERVICES_UPDATE,
    /// `settings.enrollment-tokens:manage` — manage tenant enrollment tokens.
    CanManageSettingsEnrollmentTokens => actions::SETTINGS_ENROLLMENT_TOKENS_MANAGE,
    /// `system.settings:manage` — manage global infrastructure settings.
    CanManageSystemSettings => actions::SYSTEM_SETTINGS_MANAGE,
    /// `software:read` — view software items, plugin configs, history.
    CanReadSoftware => actions::SOFTWARE_READ,
    /// `software:create` — create software items and plugin configs.
    CanCreateSoftware => actions::SOFTWARE_CREATE,
    /// `software:update` — edit software items and plugin configs.
    CanUpdateSoftware => actions::SOFTWARE_UPDATE,
    /// `software:delete` — delete software items and plugin configs.
    CanDeleteSoftware => actions::SOFTWARE_DELETE,
    /// `updates:trigger` — trigger update execution (single and batch).
    CanTriggerUpdates => actions::UPDATES_TRIGGER,
    /// `commands:manage` — modify command-bearing plugin config fields.
    CanManageCommands => actions::COMMANDS_MANAGE,
    /// `plugin-configs:trigger` — test plugin configurations against hosts.
    CanTriggerPluginConfigs => actions::PLUGIN_CONFIGS_TRIGGER,
    /// `discovery.ignores:manage` — manage autodiscovery ignore rules.
    CanManageDiscoveryIgnores => actions::DISCOVERY_IGNORES_MANAGE,
    /// `settings:read` — view all tenant settings.
    CanReadSettings => actions::SETTINGS_READ,
    /// `settings.auth:manage` — manage registration, authentication, OIDC providers.
    CanManageSettingsAuth => actions::SETTINGS_AUTH_MANAGE,
    /// `settings.certificates:manage` — manage agent certificate settings.
    CanManageSettingsCertificates => actions::SETTINGS_CERTIFICATES_MANAGE,
    /// `system.config-state:read` — view instance config reload state.
    CanReadSystemConfigState => actions::SYSTEM_CONFIG_STATE_READ,
    /// `system.config-state:manage` — manage instance config reload state.
    CanManageSystemConfigState => actions::SYSTEM_CONFIG_STATE_MANAGE,
    /// `hosts.tags:manage` — create/edit/delete/assign host tags
    /// (access-control authority under tag-scoped grants).
    CanManageHostTags => actions::HOSTS_TAGS_MANAGE,
    /// `scheduler:manage` — manage scheduled tasks.
    CanManageScheduler => actions::SCHEDULER_MANAGE,
    /// `audit:read` — view tenant-scoped audit log entries.
    CanReadAudit => actions::AUDIT_READ,
    /// `system.audit:read` — view system-level audit log entries.
    CanReadSystemAudit => actions::SYSTEM_AUDIT_READ,
    /// `notifications:read` — view notification channels, rules, log.
    CanReadNotifications => actions::NOTIFICATIONS_READ,
    /// `notifications:manage` — create/modify notification channels and rules.
    CanManageNotifications => actions::NOTIFICATIONS_MANAGE,
}

/// Authorize the first allowed action of `actions` (OR-gate). On overall
/// deny, returns the first non-`NoGrant` reason seen (`NoGrant` never masks
/// a scope/ceiling deny from another arm). One deny shape for every inline
/// site converted in M1.5.
///
/// Increments `uptrakit_access_denies_total` on deny, exactly as
/// `action_extractor!` does for the single-action case — callers render the
/// 403 (and any audit row) but never have to remember the counter.
pub(crate) fn authorize_any(
    engine: &AccessEngine,
    ctx: &AccessContext,
    actions: &[Action],
) -> Result<(), DenyReason> {
    let mut deny = None;
    for action in actions {
        match engine.authorize(ctx, action, None) {
            Decision::Allow => return Ok(()),
            Decision::Deny(reason) => {
                if matches!(deny, None | Some(DenyReason::NoGrant)) {
                    deny = Some(reason);
                }
            }
            // `Decision` is #[non_exhaustive] in another crate: unknown
            // variants deny, fail-closed.
            _ => {}
        }
    }
    let reason = deny.unwrap_or(DenyReason::NoGrant);
    metrics::counter!(
        "uptrakit_access_denies_total",
        "reason" => reason.as_str()
    )
    .increment(1);
    Err(reason)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]

    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::{FromRef, FromRequestParts};
    use axum::http::Request;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, EntityTrait};
    use uptrakit_controller_core::access::AccessEngine;
    use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
    use uptrakit_shared_db::entity::tenant;
    use uptrakit_shared_types::access::{ActionPattern, Selector};

    use super::*;

    use crate::auth::AuthMethod;

    struct TestState(AccessState);

    impl FromRef<TestState> for AccessState {
        fn from_ref(state: &TestState) -> Self {
            state.0.clone()
        }
    }

    async fn test_db() -> DatabaseConnection {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("connect test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    async fn default_tenant_id(db: &DatabaseConnection) -> uuid::Uuid {
        tenant::Entity::find()
            .one(db)
            .await
            .expect("query tenant")
            .expect("seeded default tenant")
            .id
    }

    fn parts_with(user_id: uuid::Uuid, authority: Option<AccessAuthority>) -> Parts {
        let mut req = Request::new(Body::empty());
        req.extensions_mut().insert(AuthenticatedUser::new(
            user_id,
            AuthMethod::Password,
            vec![],
            None,
        ));
        if let Some(authority) = authority {
            req.extensions_mut().insert(authority);
        }
        req.into_parts().0
    }

    #[tokio::test]
    async fn missing_user_extension_is_401() {
        let db = test_db().await;
        let state = TestState(AccessState(Arc::new(AccessEngine::new(db))));
        let mut parts = Request::new(Body::empty()).into_parts().0;
        let result = CanReadHosts::from_request_parts(&mut parts, &state).await;
        assert_eq!(
            result.expect_err("rejects").status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn missing_marker_with_user_is_500() {
        let db = test_db().await;
        let state = TestState(AccessState(Arc::new(AccessEngine::new(db))));
        let mut parts = parts_with(uuid::Uuid::now_v7(), None);
        let result = CanReadHosts::from_request_parts(&mut parts, &state).await;
        assert_eq!(
            result.expect_err("rejects").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn unavailable_marker_is_500() {
        let db = test_db().await;
        let state = TestState(AccessState(Arc::new(AccessEngine::new(db))));
        let mut parts = parts_with(uuid::Uuid::now_v7(), Some(AccessAuthority::Unavailable));
        let result = CanReadHosts::from_request_parts(&mut parts, &state).await;
        assert_eq!(
            result.expect_err("rejects").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn no_grant_is_403() {
        let db = test_db().await;
        let engine = Arc::new(AccessEngine::new(db.clone()));
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let state = TestState(AccessState(engine));
        let mut parts = parts_with(user_id, Some(AccessAuthority::Ready(ctx)));
        let result = CanReadHosts::from_request_parts(&mut parts, &state).await;
        assert_eq!(result.expect_err("rejects").status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn granted_action_allows() {
        let db = test_db().await;
        let engine = Arc::new(AccessEngine::new(db.clone()));
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let patterns = vec![
            "hosts:read"
                .parse::<ActionPattern>()
                .expect("valid pattern"),
        ];
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_id),
                tenant_id: Some(tenant_id),
                patterns: &patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert grant");
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let state = TestState(AccessState(engine));
        let mut parts = parts_with(user_id, Some(AccessAuthority::Ready(ctx)));
        let result = CanReadHosts::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok());
    }
}
