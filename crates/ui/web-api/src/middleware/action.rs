//! Action-based route extractors over the [`AccessEngine`] (M1.4a).
//!
//! Converted route families declare authorization with these extractors +
//! native `security(("oauth2" = ["<action>"]), ("developer_token" = []))`
//! requirements. The legacy macro-generated permission-extractor model and
//! its OpenAPI extension are fully retired; `middleware/permission.rs` was
//! deleted in M1.7 and the legacy vocabulary itself in M1.8.
//! Verdicts: 401 no principal, 403 `Decision::Deny`, 500 engine unavailable
//! (fail-closed).

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::response::Response;
use http::StatusCode;
use uptrakit_audit_log::{
    AuditActionType, AuditActorType, AuditEmitter, AuditEntry, AuditOutcome, Event,
};
use uptrakit_controller_core::access::{AccessContext, AccessEngine};
use uptrakit_shared_types::access::{Action, Decision, DenyReason, actions};

use crate::app_state::{AccessState, AuditEmitterState};
use crate::error_response::{error_response, error_response_with_code};
use crate::middleware::require_auth::AuthenticatedUser;

/// Per-request authorization state, inserted by `require_auth`.
///
/// `Unavailable` means the engine could not resolve the principal's grants
/// (DB failure): action extractors fail closed with 500; routes without an
/// action extractor (`logout`, unconverted families) proceed; `me` renders
/// `Unavailable` as `authority: "unavailable"` with HTTP 200 (M1.7).
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

/// Record one policy deny on `uptrakit_access_denies_total{reason}`.
///
/// Single owner for the counter: the `action_extractor!` single-action arm,
/// the [`authorize_any`] OR-gate, the surface read/invoke gate
/// (`routes/surfaces.rs`), and the self-or-`users:manage` fine check in
/// `routes/users.rs::update_profile` all funnel through here, so the metric
/// name and label shape cannot drift apart across the deny paths. Every one
/// of those sites reaches this fn only through [`record_access_denied`],
/// which additionally emits the `access.denied` audit Event for qualifying
/// denials (M1.6b) — never call this fn directly from a new site; funnel
/// through the Event helper.
pub(crate) fn record_access_deny(reason: &DenyReason) {
    metrics::counter!(
        "uptrakit_access_denies_total",
        "reason" => reason.as_str()
    )
    .increment(1);
}

/// One deny through the funnel: counter increment plus, when EVERY denied
/// action qualifies under `deny_event_worthy`, an `access.denied` audit
/// Event (spec §4). Single-action sites pass a one-element slice; OR-gates
/// pass every alternative — a mixed gate (any non-qualifying alternative)
/// emits no Event by design (an ordinary operation with a sensitive
/// allow-alternative is an ordinary denial).
///
/// Scope follows the first action's plane: `system.*` → system scope,
/// otherwise the caller's tenant. Never called on the engine-unavailable
/// (500) paths — outages are `Failed`, not `Denied`, and must not fire
/// deny dashboards.
///
/// Not a route handler: the audit-coverage walker sees it only through
/// this marker (catalog row keys this fn).
#[uptrakit_audit_log::audit_required]
pub(crate) fn record_access_denied(
    emitter: &AuditEmitter,
    ctx: &AccessContext,
    denied_actions: &[&Action],
    reason: &DenyReason,
) {
    record_access_deny(reason);
    let Some(first) = denied_actions.first() else {
        return;
    };
    if !denied_actions
        .iter()
        .all(|action| uptrakit_shared_types::access::deny_event_worthy(action))
    {
        return;
    }
    let builder = AuditEntry::<Event>::builder_event(AuditActionType::ACCESS_DENIED);
    let builder = if first.resource().is_system() {
        builder.system_scope()
    } else {
        builder.tenant_scope(ctx.tenant_id)
    };
    if let Ok(entry) = builder
        .actor(AuditActorType::User, Some(ctx.user_id))
        .target("action", first.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "actions": denied_actions
                .iter()
                .map(|action| action.to_string())
                .collect::<Vec<_>>(),
            "reason": reason.as_str(),
        }))
        .build()
    {
        emitter.emit_event(entry);
    }
}

/// Generates a concrete Axum extractor struct for a single catalog action.
///
/// Same ergonomic shape as the retired legacy permission extractor
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
                AuditEmitterState: FromRef<S>,
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
                        decision => {
                            let reason = match decision {
                                Decision::Deny(reason) => reason,
                                // `Decision` is #[non_exhaustive] in another
                                // crate: unknown variants deny fail-closed,
                                // counted/audited as no_grant.
                                _ => DenyReason::NoGrant,
                            };
                            tracing::debug!(
                                action = %$action,
                                user_id = %user.user_id,
                                reason = reason.as_str(),
                                "action denied"
                            );
                            let emitter = AuditEmitterState::from_ref(state).0;
                            record_access_denied(&emitter, &ctx, &[&$action], &reason);
                            Err(error_response(
                                StatusCode::FORBIDDEN,
                                "Insufficient permissions",
                            ))
                        }
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
    /// `system.services:read` — list/get system services.
    CanReadSystemServices => actions::SYSTEM_SERVICES_READ,
    /// `system.services:approve` — approve pending system services.
    CanApproveSystemServices => actions::SYSTEM_SERVICES_APPROVE,
    /// `system.services:reject` — reject pending system services.
    CanRejectSystemServices => actions::SYSTEM_SERVICES_REJECT,
    /// `system.services:delete` — deactivate system services.
    CanDeleteSystemServices => actions::SYSTEM_SERVICES_DELETE,
    /// `system.services:update` — update system service settings.
    CanUpdateSystemServices => actions::SYSTEM_SERVICES_UPDATE,
    /// `access:manage` — manage grants, roles, and role assignments
    /// (authority administration).
    CanManageAccess => actions::ACCESS_MANAGE,
    /// `users:manage` — user lifecycle (activate/deactivate, lifecycle reads).
    CanManageUsers => actions::USERS_MANAGE,
}

/// Authorize the first allowed action of `actions` (OR-gate). On overall
/// deny, returns the first non-`NoGrant` reason seen (`NoGrant` never masks
/// a scope/ceiling deny from another arm). One deny shape for every inline
/// site converted in M1.5.
///
/// Increments `uptrakit_access_denies_total` on deny, exactly as
/// `action_extractor!` does for the single-action case — callers render the
/// 403 (and any audit row) but never have to remember the counter. Also
/// funnels through [`record_access_denied`]: an all-qualifying gate (spec
/// §4 OR-gate rule) emits an `access.denied` Event; a mixed gate does not.
pub(crate) fn authorize_any(
    engine: &AccessEngine,
    ctx: &AccessContext,
    emitter: &AuditEmitter,
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
    let refs: Vec<&Action> = actions.iter().collect();
    record_access_denied(emitter, ctx, &refs, &reason);
    Err(reason)
}

/// Engine-backed `system.access:manage` fine check for request-dependent
/// system-plane requirements (M1.6a): minting/removing system-plane grant
/// rows, deleting roles that hold them, or assigning roles that reach the
/// system plane. Verdicts mirror `action_extractor!`: authority
/// `Unavailable`/absent → 500 fail-closed; engine deny → 403 + deny counter.
/// The `Unavailable` branch is the engine-outage 500 path (spec §4 wildcard
/// rule) and never reaches the deny funnel.
pub(crate) fn require_system_access(
    engine: &AccessEngine,
    emitter: &AuditEmitter,
    authority: &AccessAuthority,
) -> Option<Response> {
    let Some(ctx) = authority.ready() else {
        return Some(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Authorization authority unavailable",
        ));
    };
    match engine.authorize(ctx, &actions::SYSTEM_ACCESS_MANAGE, None) {
        Decision::Allow => None,
        decision => {
            let reason = match decision {
                Decision::Deny(reason) => reason,
                // `Decision` is #[non_exhaustive] in another crate: unknown
                // variants deny fail-closed, counted/audited as no_grant.
                _ => DenyReason::NoGrant,
            };
            record_access_denied(emitter, ctx, &[&actions::SYSTEM_ACCESS_MANAGE], &reason);
            Some(error_response_with_code(
                StatusCode::FORBIDDEN,
                "This operation confers system-plane authority and requires system.access:manage",
                "forbidden",
            ))
        }
    }
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

    struct TestState(AccessState, AuditEmitterState);

    impl FromRef<TestState> for AccessState {
        fn from_ref(state: &TestState) -> Self {
            state.0.clone()
        }
    }

    impl FromRef<TestState> for AuditEmitterState {
        fn from_ref(state: &TestState) -> Self {
            state.1.clone()
        }
    }

    fn noop_emitter() -> uptrakit_audit_log::AuditEmitter {
        uptrakit_audit_log::AuditEmitter::new(uptrakit_audit_log::AuditLogDispatcher::new(
            std::sync::Arc::new(uptrakit_audit_log::NoopBackend),
        ))
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
        req.extensions_mut()
            .insert(AuthenticatedUser::new(user_id, AuthMethod::Password, None));
        if let Some(authority) = authority {
            req.extensions_mut().insert(authority);
        }
        req.into_parts().0
    }

    #[tokio::test]
    async fn missing_user_extension_is_401() {
        let db = test_db().await;
        let state = TestState(
            AccessState(Arc::new(AccessEngine::new(db))),
            AuditEmitterState(noop_emitter()),
        );
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
        let state = TestState(
            AccessState(Arc::new(AccessEngine::new(db))),
            AuditEmitterState(noop_emitter()),
        );
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
        let state = TestState(
            AccessState(Arc::new(AccessEngine::new(db))),
            AuditEmitterState(noop_emitter()),
        );
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
        let state = TestState(AccessState(engine), AuditEmitterState(noop_emitter()));
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
        let state = TestState(AccessState(engine), AuditEmitterState(noop_emitter()));
        let mut parts = parts_with(user_id, Some(AccessAuthority::Ready(ctx)));
        let result = CanReadHosts::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok());
    }

    /// Pins the outage/deny split: an unavailable authority short-circuits to
    /// 500 before any `authorize` call, so it never enters the deny funnel. A
    /// wildcard arm that classified `Unavailable` as a denial would answer 403
    /// here (see `require_system_access_denies_without_grant`), reddening this
    /// test.
    #[tokio::test]
    async fn require_system_access_unavailable_authority_is_500() {
        let db = test_db().await;
        let engine = AccessEngine::new(db);
        let response =
            require_system_access(&engine, &noop_emitter(), &AccessAuthority::Unavailable)
                .expect("must reject");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn require_system_access_denies_without_grant() {
        let db = test_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let response =
            require_system_access(&engine, &noop_emitter(), &AccessAuthority::Ready(ctx))
                .expect("must reject");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn require_system_access_allows_system_plane_grant() {
        let db = test_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let patterns = vec![
            "system.access:manage"
                .parse::<ActionPattern>()
                .expect("valid pattern"),
        ];
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_id),
                tenant_id: None,
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
        let response =
            require_system_access(&engine, &noop_emitter(), &AccessAuthority::Ready(ctx));
        assert!(response.is_none());
    }
}
