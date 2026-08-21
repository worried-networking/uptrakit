//! `AccessEngine` — the single access decision point (PDP), M1.3.
//!
//! **Decision rule:** `Allow` iff grant ∧ scope ∧ selector all pass, in the
//! normative check order: dynamic-action registry → grant match → token
//! scope → target/selector (see [`AccessEngine::authorize`]).
//!
//! **Cache/TTL design:** bounded `moka::sync::Cache` keyed `(tenant_id,
//! user_id)` with a **first-party read-time staleness check** (60 s), not
//! moka `time_to_live` — moka's quanta-based expiry clock is unreachable by
//! `tokio::time::advance`, so the backstop would be untestable and asserting
//! it would test upstream behavior. `context()` treats a hit older than the
//! TTL as a miss and reloads; a stale entry that is never read again sits
//! harmlessly until size eviction.
//!
//! **Invalidation contract:** grant/role mutation sites (M1.6a) call
//! [`AccessEngine::invalidate_subjects`] locally and publish
//! `ControllerMessage::AccessInvalidated` through the existing
//! `publish_controller_event` path; remote instances route the payload to
//! [`AccessEngine::apply_remote_invalidation`] via the M1.4a
//! `deliver_controller_event` arm. Both flush the whole cache. The 60 s TTL
//! backstop covers lost events.
//!
//! **Dark-ship state:** library-complete in M1.3 with zero production
//! construction — M1.4a constructs the engine in `AppState` and builds
//! [`AccessContext`] in `require_auth`.
//!
//! Full design: `.superpowers/authn-and-authz-refactoring/07-decision-and-enforcement.md`
//! and `docs/superpowers/specs/2026-07-28-access-engine-design.md`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, QueryFilter};
use uuid::Uuid;

use uptrakit_shared_db::TenantDb;
use uptrakit_shared_db::access_grants::{
    AccessGrantError, ResolvedGrant, load_grants_for_principal,
};
use uptrakit_shared_db::entity::user_role;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::access::{Action, ActionPattern, CATALOG, Decision};
use uptrakit_shared_types::access::{DenyReason, TargetRef, Visibility};
use uptrakit_wire::AccessInvalidatedPayload;

/// Default TTL backstop for cached principal authority.
const ACCESS_CACHE_TTL: Duration = Duration::from_secs(60);
/// Bounded cache size (entries), per the M1.3 design.
const ACCESS_CACHE_MAX_CAPACITY: u64 = 10_000;

/// Errors from `AccessEngine` principal resolution.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum AccessEngineError {
    /// Loading the principal's role assignments failed.
    #[error("role resolution failed: {0}")]
    RoleResolution(sea_orm::DbErr),
    /// Loading the principal's access grants failed.
    #[error("grant resolution failed: {0}")]
    GrantResolution(AccessGrantError),
}

/// Module-wide result alias (covers every fn in this module).
pub type Result<T> = std::result::Result<T, rootcause::Report<AccessEngineError>>;

impl_report_conversion!(sea_orm::DbErr => AccessEngineError::RoleResolution);
impl_report_conversion!(AccessGrantError => AccessEngineError::GrantResolution);

/// Engine-owned registry seam for dynamic (`plugin.*` / `surface.*`) actions.
///
/// `None` in M1.3 means **every** dynamic action denies (fail-closed:
/// nothing is registered). M1.5 injects the live plugin-catalog and
/// surface-registry implementations. Narrow, workflow-scoped trait — the
/// typed-boundary pattern of ADR-0018.
pub trait DynamicActionRegistry: Send + Sync {
    /// Is this concrete dynamic action currently registered?
    fn is_registered(&self, action: &Action) -> bool;

    /// Every dynamic action currently registered — the catalog's dynamic
    /// section (M1.6b). Contract: an action appears here iff
    /// `is_registered` returns `true` for it.
    fn registered_actions(&self) -> Vec<Action>;
}

/// Point-in-time snapshot of one principal's resolved grants.
struct CachedAuthority {
    grants: Vec<ResolvedGrant>,
    loaded_at: tokio::time::Instant,
}

/// The single decision point: batched grant resolution + bounded cache +
/// pure in-memory decision evaluation.
pub struct AccessEngine {
    db: DatabaseConnection,
    /// Key order matches `context()`'s parameter order: `(tenant_id, user_id)`.
    cache: moka::sync::Cache<(Uuid, Uuid), Arc<CachedAuthority>>,
    registry: Option<Arc<dyn DynamicActionRegistry>>,
    ttl: Duration,
}

/// Per-request access context: the cached authority core plus per-request
/// credential scope.
///
/// Point-in-time snapshot. Per-request rebuild (the M1.4a contract) covers
/// ordinary handlers; a long-lived streaming holder (SSE/WS) IS one request
/// — it needs periodic re-`context()` or an invalidation-aware refresh,
/// which per-request rebuild does not provide. Carried as an M1.4a/M1.5
/// residual for the streaming enforcement sites.
#[derive(Clone)]
pub struct AccessContext {
    /// The authenticated principal.
    pub user_id: Uuid,
    /// Carried for M1.6b deny-audit and logging; unused by evaluation.
    pub tenant_id: Uuid,
    authority: Arc<CachedAuthority>,
    /// `None` = credential with no scope concept (pre-M3 session JWT).
    /// `Some(vec![])` = a scope ceiling that admits nothing.
    scope: Option<Vec<ActionPattern>>,
}

impl AccessEngine {
    /// Build an engine over the given connection with no dynamic-action
    /// registry (every dynamic action denies until M1.5 injects one).
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            cache: moka::sync::Cache::builder()
                .max_capacity(ACCESS_CACHE_MAX_CAPACITY)
                .build(),
            registry: None,
            ttl: ACCESS_CACHE_TTL,
        }
    }

    /// Inject the dynamic-action registry (M1.5 wires the live impls).
    #[must_use]
    pub fn with_registry(mut self, registry: Arc<dyn DynamicActionRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Every currently registered dynamic action (the catalog's dynamic
    /// section, M1.6b); empty when no registry is injected — matching
    /// `authorize`'s fail-closed treatment of dynamic actions.
    #[must_use]
    pub fn dynamic_actions(&self) -> Vec<Action> {
        self.registry
            .as_ref()
            .map(|registry| registry.registered_actions())
            .unwrap_or_default()
    }

    /// Resolve a principal's access context.
    ///
    /// Fresh cache hit → wrap and return. Miss or TTL-stale hit → exactly two
    /// queries (role ids via `TenantDb`, then the batched
    /// `{user} ∪ roles` grant load) and re-insert. Errors propagate — never
    /// an empty-but-authorized context; nothing is cached on error.
    /// Concurrent misses for one key may duplicate the load (idempotent
    /// reads, last-insert-wins — accepted at deployment scale; moka's
    /// coalescing `try_get_with` cannot await the async DB load).
    pub async fn context(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
        scope: Option<Vec<ActionPattern>>,
    ) -> Result<AccessContext> {
        let key = (tenant_id, user_id);
        let authority = match self.cache.get(&key) {
            Some(entry) if entry.loaded_at.elapsed() <= self.ttl => entry,
            stale => {
                let reason = if stale.is_some() { "stale" } else { "miss" };
                let entry = self.load_authority(tenant_id, user_id).await?;
                self.cache.insert(key, Arc::clone(&entry));
                metrics::counter!("uptrakit_access_context_loads_total", "reason" => reason)
                    .increment(1);
                entry
            }
        };
        Ok(AccessContext {
            user_id,
            tenant_id,
            authority,
            scope,
        })
    }

    async fn load_authority(&self, tenant_id: Uuid, user_id: Uuid) -> Result<Arc<CachedAuthority>> {
        let tenant_db = TenantDb::new(self.db.clone(), tenant_id);
        let role_ids: Vec<Uuid> = tenant_db
            .find::<user_role::Entity>()
            .filter(user_role::Column::UserId.eq(user_id))
            .all(tenant_db.db())
            .await
            .context_to()?
            .into_iter()
            .map(|row| row.role_id)
            .collect();

        let load = load_grants_for_principal(&self.db, tenant_id, user_id, &role_ids)
            .await
            .context_to()?;

        if load.corrupt_skipped > 0 {
            // Deliberately label-free: the only per-call label candidates
            // (user/tenant ids) are unbounded-cardinality; the single fixed
            // cause is already encoded in the name. Per-principal detail
            // rides the companion warn below.
            metrics::counter!("uptrakit_access_corrupt_grant_rows_skipped_total")
                .increment(load.corrupt_skipped as u64);
            tracing::warn!(
                %tenant_id,
                %user_id,
                corrupt_skipped = load.corrupt_skipped,
                "skipped corrupt access grant rows while resolving principal authority"
            );
        }

        Ok(Arc::new(CachedAuthority {
            grants: load.grants,
            loaded_at: tokio::time::Instant::now(),
        }))
    }
}

impl AccessEngine {
    /// Shared decision core; `target: None` = the coarse (targetless) gate.
    fn decide(
        &self,
        ctx: &AccessContext,
        action: &Action,
        target: Option<(&TargetRef, &BTreeSet<Uuid>)>,
    ) -> Decision {
        let resource = action.resource();
        let is_dynamic = resource.plugin_type().is_some() || resource.surface_id().is_some();
        if is_dynamic {
            let registered = self
                .registry
                .as_ref()
                .is_some_and(|registry| registry.is_registered(action));
            if !registered {
                return Decision::Deny(DenyReason::UnknownAction);
            }
        }

        let matching_grants: Vec<&ResolvedGrant> = ctx
            .authority
            .grants
            .iter()
            .filter(|g| g.patterns.iter().any(|pattern| pattern.matches(action)))
            .collect();
        if matching_grants.is_empty() {
            return Decision::Deny(DenyReason::NoGrant);
        }

        if let Some(scope) = &ctx.scope
            && !scope.iter().any(|pattern| pattern.matches(action))
        {
            return Decision::Deny(DenyReason::OutOfScope);
        }

        if let Some((target, host_tags)) = target
            && !matching_grants
                .iter()
                .any(|g| g.selector.covers(target, host_tags))
        {
            return Decision::Deny(DenyReason::OutsideSelector);
        }

        Decision::Allow
    }

    /// Coarse (targetless) decision: dynamic-registry gate, grant match,
    /// token scope. Selector coverage is NOT evaluated here — a principal
    /// whose only matching grant carries a non-`All` selector is allowed
    /// at this gate; target-aware sites use
    /// [`AccessEngine::authorize_target`] for the fine (selector) step.
    ///
    /// Normative check order:
    /// 1. **Dynamic-action registry** (`plugin.*`/`surface.*` resources
    ///    only): unregistered → `Deny(UnknownAction)`. Built-in actions skip
    ///    this step — parse-time catalog membership is their registration.
    /// 2. **Grant match**: no grant pattern matches → `Deny(NoGrant)`.
    /// 3. **Token scope**: `None` → vacuously true (credential with no scope
    ///    concept); `Some` with no matching pattern → `Deny(OutOfScope)`
    ///    (an empty `Some` vec denies everything).
    pub fn authorize(&self, ctx: &AccessContext, action: &Action) -> Decision {
        self.decide(ctx, action, None)
    }

    /// Visibility verdict for `action`: grants matching the action **and**
    /// surviving scope intersection — any → `Full` (every M1 selector is
    /// `All`), none → `Visibility::None`. M2.3 adds the selector-union →
    /// `Filter` arm to this match-then-union shape.
    ///
    /// Deliberately omits the dynamic-action registry gate that
    /// [`AccessEngine::authorize`] applies (per the PDP design): M1.5 list
    /// sites over dynamic (`plugin.*`/`surface.*`) resources must not rely
    /// on `visibility()` alone to filter unknown/unregistered actions.
    pub fn visibility(&self, ctx: &AccessContext, action: &Action) -> Visibility {
        if let Some(scope) = &ctx.scope
            && !scope.iter().any(|pattern| pattern.matches(action))
        {
            return Visibility::None;
        }
        let has_matching_grant = ctx
            .authority
            .grants
            .iter()
            .any(|g| g.patterns.iter().any(|pattern| pattern.matches(action)));
        if has_matching_grant {
            Visibility::Full
        } else {
            Visibility::None
        }
    }

    /// Coarse capability summary: actions the principal holds ANY matching
    /// grant for, regardless of selector narrowing (pinned by test). A
    /// selector-scoped grant contributes its actions here — per-action
    /// visibility summaries are M2.6/D13 UI work, not this method.
    #[must_use]
    pub fn allowed_actions(&self, ctx: &AccessContext) -> Vec<Action> {
        let built_ins = CATALOG.iter().flat_map(|entry| {
            entry.verbs.iter().filter_map(|verb_entry| {
                // Catalog literals always parse — the macro emits only
                // valid pairs (same idiom as routes/access_catalog.rs).
                verb_entry.action_str.parse::<Action>().ok()
            })
        });
        let mut actions: Vec<Action> = built_ins
            .chain(self.dynamic_actions())
            .filter(|action| matches!(self.authorize(ctx, action), Decision::Allow))
            .collect();
        actions.sort_unstable_by_key(ToString::to_string);
        actions
    }
}

impl AccessEngine {
    /// Flush the whole cache after a local grant/role mutation.
    ///
    /// Full flush per event is provably correct and uses only infallible
    /// cache API (granular `invalidate_entries_if` needs builder opt-in and
    /// returns a `Result` the no-unwrap invariant cannot swallow for an auth
    /// invalidation); grant/role mutations are rare admin operations. The
    /// subject lists are logged for observability only.
    ///
    /// Effect: reflected on the next request absent a concurrent in-flight
    /// load — a load already in flight when the flush fires may insert
    /// pre-mutation authority after it, serving stale data until the TTL
    /// backstop (the same 60 s envelope already accepted for a lost NATS
    /// event). The engine does NOT publish NATS: mutation sites (M1.6a)
    /// call this locally and publish `ControllerMessage::AccessInvalidated`
    /// via the existing `publish_controller_event` path.
    pub fn invalidate_subjects(&self, user_ids: &[Uuid], role_ids: &[Uuid]) {
        self.cache.invalidate_all();
        metrics::counter!("uptrakit_access_invalidations_total", "origin" => "local").increment(1);
        tracing::debug!(
            ?user_ids,
            ?role_ids,
            "flushed access cache after local grant/role mutation"
        );
    }

    /// Flush the whole cache for a received `AccessInvalidated` event
    /// (routed here by the M1.4a `deliver_controller_event` arm). Receivers
    /// always flush — the payload's lists are diagnostic; there is no
    /// `tenant_id` by design (a global-grant revoke must invalidate every
    /// tenant's entries).
    pub fn apply_remote_invalidation(&self, payload: &AccessInvalidatedPayload) {
        self.cache.invalidate_all();
        metrics::counter!("uptrakit_access_invalidations_total", "origin" => "remote").increment(1);
        tracing::debug!(
            user_ids = ?payload.user_ids,
            role_ids = ?payload.role_ids,
            "flushed access cache after remote AccessInvalidated event"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sea_orm::sea_query::{Alias, Query};
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend,
        EntityTrait, MockDatabase, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
    use uptrakit_shared_db::entity::{role, tenant, user};
    use uptrakit_shared_types::MaskedEmail;
    use uptrakit_shared_types::access::Selector;
    use uptrakit_shared_types::access::{Resource, Verb, actions};

    use super::*;

    async fn test_db() -> DatabaseConnection {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("connect to test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    async fn default_tenant_id(db: &DatabaseConnection) -> Uuid {
        tenant::Entity::find()
            .one(db)
            .await
            .expect("query tenants")
            .expect("default tenant is seeded")
            .id
    }

    async fn seed_user(db: &DatabaseConnection) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(id),
            email: Set(MaskedEmail::new(format!("u-{id}@example.com"))),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user");
        id
    }

    async fn grant(db: &DatabaseConnection, tenant_id: Uuid, subject: GrantSubject, pattern: &str) {
        let patterns = vec![pattern.parse::<ActionPattern>().expect("valid pattern")];
        insert_grant(
            db,
            NewGrant {
                subject,
                tenant_id: Some(tenant_id),
                patterns: &patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert grant");
    }

    async fn seed_corrupt_grant_row(db: &DatabaseConnection, tenant_id: Uuid, user_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        let stmt = Query::insert()
            .into_table(Alias::new("access_grants"))
            .columns([
                Alias::new("id"),
                Alias::new("tenant_id"),
                Alias::new("subject_type"),
                Alias::new("subject_id"),
                Alias::new("patterns"),
                Alias::new("selector"),
                Alias::new("created_at"),
                Alias::new("updated_at"),
            ])
            .values_panic([
                Uuid::now_v7().into(),
                tenant_id.into(),
                "user".into(),
                user_id.into(),
                serde_json::json!(["not a pattern ["]).into(),
                serde_json::json!({"type": "all"}).into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        db.execute(&stmt).await.expect("seed corrupt grant row");
    }

    fn dummy_engine() -> AccessEngine {
        AccessEngine::new(MockDatabase::new(DbBackend::Sqlite).into_connection())
    }

    fn resolved_grant(pattern: &str) -> ResolvedGrant {
        ResolvedGrant {
            id: Uuid::nil(),
            tenant_id: Some(Uuid::nil()),
            subject: GrantSubject::User(Uuid::nil()),
            patterns: vec![pattern.parse().expect("valid pattern")],
            selector: Selector::All,
            description: None,
        }
    }

    fn resolved_grant_with_selector(pattern: &str, selector: Selector) -> ResolvedGrant {
        let mut grant = resolved_grant(pattern);
        grant.selector = selector;
        grant
    }

    fn ctx_with(grants: Vec<ResolvedGrant>, scope: Option<Vec<ActionPattern>>) -> AccessContext {
        AccessContext {
            user_id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            authority: Arc::new(CachedAuthority {
                grants,
                loaded_at: tokio::time::Instant::now(),
            }),
            scope,
        }
    }

    fn scope_of(patterns: &[&str]) -> Option<Vec<ActionPattern>> {
        Some(
            patterns
                .iter()
                .map(|p| p.parse().expect("valid pattern"))
                .collect(),
        )
    }

    #[tokio::test]
    async fn c1a_direct_user_grant_allows() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = seed_user(&db).await;
        grant(&db, tenant_id, GrantSubject::User(user_id), "hosts:read").await;

        let engine = AccessEngine::new(db);
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context resolves");
        assert_eq!(
            engine.authorize(&ctx, &actions::HOSTS_READ),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn c1b_role_inherited_grant_allows() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = seed_user(&db).await;
        let viewer_id = role::Entity::find()
            .filter(role::Column::Name.eq("viewer"))
            .one(&db)
            .await
            .expect("query roles")
            .expect("viewer role is seeded")
            .id;
        user_role::ActiveModel {
            tenant_id: Set(tenant_id),
            user_id: Set(user_id),
            role_id: Set(viewer_id),
            assigned_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(&db)
        .await
        .expect("insert user_role");

        let engine = AccessEngine::new(db);
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context resolves");
        assert_eq!(
            engine.authorize(&ctx, &actions::HOSTS_READ),
            Decision::Allow,
            "viewer role's *:read grant must be inherited"
        );
    }

    #[test]
    fn c1c_wildcard_pattern_allows() {
        let engine = dummy_engine();
        let ctx = ctx_with(vec![resolved_grant("hosts:*")], None);
        assert_eq!(
            engine.authorize(&ctx, &actions::HOSTS_READ),
            Decision::Allow
        );
    }

    #[test]
    fn c4_no_grant_denies() {
        let engine = dummy_engine();
        let ctx = ctx_with(vec![], None);
        assert_eq!(
            engine.authorize(&ctx, &actions::HOSTS_READ),
            Decision::Deny(DenyReason::NoGrant)
        );
    }

    #[test]
    fn c5_scope_ceiling_denies_out_of_scope() {
        let engine = dummy_engine();
        let ctx = ctx_with(
            vec![resolved_grant("hosts:read")],
            scope_of(&["services:read"]),
        );
        assert_eq!(
            engine.authorize(&ctx, &actions::HOSTS_READ),
            Decision::Deny(DenyReason::OutOfScope)
        );
    }

    /// Registry seam used only by C7 — proves the seam, not the stub.
    struct StubRegistry {
        registered: Vec<Action>,
    }

    impl DynamicActionRegistry for StubRegistry {
        fn is_registered(&self, action: &Action) -> bool {
            self.registered.contains(action)
        }

        fn registered_actions(&self) -> Vec<Action> {
            self.registered.clone()
        }
    }

    #[test]
    fn c7_dynamic_action_registry_seam() {
        let dynamic = Resource::plugin("foo").expect("valid plugin resource");
        let action = Action::new(dynamic, Verb::Manage).expect("dynamic action");
        let ctx = ctx_with(vec![resolved_grant("plugin.foo:manage")], None);

        let no_registry = dummy_engine();
        assert_eq!(
            no_registry.authorize(&ctx, &action),
            Decision::Deny(DenyReason::UnknownAction),
            "no registry at all must deny dynamic actions"
        );

        let empty_registry =
            dummy_engine().with_registry(Arc::new(StubRegistry { registered: vec![] }));
        assert_eq!(
            empty_registry.authorize(&ctx, &action),
            Decision::Deny(DenyReason::UnknownAction),
            "an empty registry must still deny"
        );

        let populated_registry = dummy_engine().with_registry(Arc::new(StubRegistry {
            registered: vec![action.clone()],
        }));
        assert_eq!(
            populated_registry.authorize(&ctx, &action),
            Decision::Allow,
            "registering the action must allow once grant/scope pass"
        );
    }

    fn engine_with_stub_registry() -> AccessEngine {
        let action = Action::new(
            Resource::surface("test-stub").expect("valid surface resource"),
            Verb::Use,
        )
        .expect("dynamic action");
        dummy_engine().with_registry(Arc::new(StubRegistry {
            registered: vec![action],
        }))
    }

    #[test]
    fn allowed_actions_expands_wildcard_grant_to_concrete_catalog_verbs() {
        let engine = dummy_engine();
        let ctx = ctx_with(vec![resolved_grant("software:*")], None);
        let actions = engine.allowed_actions(&ctx);
        let strs: Vec<String> = actions.iter().map(ToString::to_string).collect();
        assert!(
            strs.contains(&"software:read".to_string()),
            "read expanded: {strs:?}"
        );
        assert!(
            strs.contains(&"software:update".to_string()),
            "update expanded: {strs:?}"
        );
        assert!(
            strs.iter().all(|s| s.starts_with("software:")),
            "wildcard must not leak other resources: {strs:?}"
        );
    }

    #[test]
    fn allowed_actions_applies_scope_ceiling_intersection() {
        let engine = dummy_engine();
        let ctx = ctx_with(vec![resolved_grant("*:*")], scope_of(&["hosts:read"]));
        let strs: Vec<String> = engine
            .allowed_actions(&ctx)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            strs,
            vec!["hosts:read".to_string()],
            "scope must cap the grant"
        );
    }

    #[test]
    fn allowed_actions_is_sorted_and_omits_dynamic_without_registry() {
        let engine = dummy_engine();
        let ctx = ctx_with(vec![resolved_grant("*:*")], None);
        let strs: Vec<String> = engine
            .allowed_actions(&ctx)
            .iter()
            .map(ToString::to_string)
            .collect();
        let mut sorted = strs.clone();
        sorted.sort_unstable();
        assert_eq!(strs, sorted, "deterministic wire order");
        assert!(
            strs.iter()
                .all(|s| !s.starts_with("surface.") && !s.starts_with("plugin.")),
            "no registry configured => no dynamic entries: {strs:?}"
        );
    }

    #[test]
    fn allowed_actions_includes_registered_dynamic_action_when_granted() {
        let engine = engine_with_stub_registry();
        let ctx = ctx_with(vec![resolved_grant("surface.test-stub:use")], None);
        let strs: Vec<String> = engine
            .allowed_actions(&ctx)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(strs, vec!["surface.test-stub:use".to_string()]);
    }

    #[tokio::test]
    async fn allowed_actions_includes_actions_held_only_via_non_all_grants() {
        let engine = dummy_engine();
        let ctx = ctx_with(
            vec![resolved_grant_with_selector(
                "hosts:read",
                Selector::Hosts {
                    ids: vec![Uuid::from_u128(1)],
                },
            )],
            None,
        );
        let actions = engine.allowed_actions(&ctx);
        assert!(
            actions.iter().any(|a| a.to_string() == "hosts:read"),
            "coarse allowed_actions must include selector-scoped grants: {actions:?}"
        );
    }

    #[tokio::test]
    async fn coarse_authorize_allows_non_all_selector_grants_targetless() {
        // Pins the M2.1 split-API contract: the targetless gate never
        // evaluates selectors; fine checks live in authorize_target.
        let engine = dummy_engine();
        let ctx = ctx_with(
            vec![resolved_grant_with_selector(
                "hosts:read",
                Selector::Hosts {
                    ids: vec![Uuid::from_u128(1)],
                },
            )],
            None,
        );
        let action = "hosts:read".parse::<Action>().expect("valid action");
        assert_eq!(engine.authorize(&ctx, &action), Decision::Allow);
    }

    #[test]
    fn c9_visibility_m1_arms() {
        let engine = dummy_engine();

        let full = ctx_with(vec![resolved_grant("hosts:read")], None);
        assert_eq!(
            engine.visibility(&full, &actions::HOSTS_READ),
            Visibility::Full
        );

        let no_grant = ctx_with(vec![], None);
        assert_eq!(
            engine.visibility(&no_grant, &actions::HOSTS_READ),
            Visibility::None
        );

        let out_of_scope = ctx_with(
            vec![resolved_grant("hosts:read")],
            scope_of(&["services:read"]),
        );
        assert_eq!(
            engine.visibility(&out_of_scope, &actions::HOSTS_READ),
            Visibility::None
        );
    }

    #[test]
    fn c14_scope_grant_intersection_both_directions() {
        let engine = dummy_engine();

        let wide_grant_narrow_scope =
            ctx_with(vec![resolved_grant("*:*")], scope_of(&["hosts:read"]));
        assert_eq!(
            engine.authorize(&wide_grant_narrow_scope, &actions::HOSTS_READ),
            Decision::Allow
        );
        assert_eq!(
            engine.authorize(&wide_grant_narrow_scope, &actions::SERVICES_READ),
            Decision::Deny(DenyReason::OutOfScope)
        );

        let narrow_grant_wide_scope =
            ctx_with(vec![resolved_grant("hosts:read")], scope_of(&["*:*"]));
        assert_eq!(
            engine.authorize(&narrow_grant_wide_scope, &actions::HOSTS_READ),
            Decision::Allow
        );
        assert_eq!(
            engine.authorize(&narrow_grant_wide_scope, &actions::SERVICES_READ),
            Decision::Deny(DenyReason::NoGrant)
        );
    }

    #[test]
    fn c15_no_scope_vs_empty_scope() {
        let engine = dummy_engine();

        let no_scope_concept = ctx_with(vec![resolved_grant("hosts:read")], None);
        assert_eq!(
            engine.authorize(&no_scope_concept, &actions::HOSTS_READ),
            Decision::Allow,
            "grants alone must authorize when the credential has no scope concept"
        );

        let empty_scope_ceiling = ctx_with(vec![resolved_grant("hosts:read")], Some(vec![]));
        assert_eq!(
            engine.authorize(&empty_scope_ceiling, &actions::HOSTS_READ),
            Decision::Deny(DenyReason::OutOfScope),
            "an empty Some(scope) ceiling must admit nothing"
        );
    }

    #[test]
    fn target_arm_selector_all_covers_any_target() {
        let engine = dummy_engine();
        let ctx = ctx_with(vec![resolved_grant("hosts:read")], None);
        assert_eq!(
            engine.decide(
                &ctx,
                &actions::HOSTS_READ,
                Some((&TargetRef::Host(Uuid::nil()), &BTreeSet::new()))
            ),
            Decision::Allow
        );
    }

    #[tokio::test(start_paused = true)]
    async fn context_ttl_backstop_reloads_after_sixty_seconds() {
        // MockDatabase, not sqlite::memory: — start_paused + sqlx pool is
        // forbidden (testing.md: pool timers fire under auto-advance).
        // Two loads expected: initial + past-TTL reload; each consumes one
        // (roles, grants) result pair.
        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([Vec::<user_role::Model>::new()])
            .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
            .append_query_results([Vec::<user_role::Model>::new()])
            .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
            .into_connection();
        let engine = AccessEngine::new(db);
        let tenant_id = Uuid::nil();
        let user_id = Uuid::nil();

        engine
            .context(tenant_id, user_id, None)
            .await
            .expect("initial load");

        tokio::time::advance(Duration::from_secs(30)).await;
        engine
            .context(tenant_id, user_id, None)
            .await
            .expect("within-TTL hit");

        tokio::time::advance(Duration::from_secs(31)).await;
        engine
            .context(tenant_id, user_id, None)
            .await
            .expect("past-TTL reload");

        let statement_count: usize = engine
            .db
            .clone()
            .into_transaction_log()
            .iter()
            .map(|tx| tx.statements().len())
            .sum();
        assert_eq!(
            statement_count, 4,
            "exactly two loads of two statements each: the within-TTL hit must issue no \
             queries (caching works) and the past-TTL read must reload (staleness works)"
        );
    }

    #[tokio::test]
    async fn context_propagates_db_errors_never_empty_authority() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let engine = AccessEngine::new(db);
        let result = engine.context(Uuid::nil(), Uuid::nil(), None).await;
        assert!(
            result.is_err(),
            "resolution against a missing schema must error, not return empty authority"
        );
    }

    #[tokio::test]
    async fn context_skips_corrupt_rows_and_keeps_valid_authority() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = seed_user(&db).await;
        grant(&db, tenant_id, GrantSubject::User(user_id), "hosts:read").await;
        seed_corrupt_grant_row(&db, tenant_id, user_id).await;

        let engine = AccessEngine::new(db);
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("corrupt rows loud-skip; the call must succeed");
        assert_eq!(
            ctx.authority.grants.len(),
            1,
            "only the valid grant's authority survives"
        );
    }

    #[tokio::test]
    async fn context_resolution_is_exactly_two_queries() {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::nil();
        let user_id = Uuid::now_v7();
        let roles: Vec<user_role::Model> = (0..5)
            .map(|_| user_role::Model {
                tenant_id,
                user_id,
                role_id: Uuid::now_v7(),
                assigned_at: now,
            })
            .collect();

        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_query_results([roles])
            .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
            .into_connection();

        let engine = AccessEngine::new(db);
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("mock resolution succeeds");
        assert_eq!(ctx.authority.grants.len(), 0);

        let statement_count: usize = engine
            .db
            .clone()
            .into_transaction_log()
            .iter()
            .map(|tx| tx.statements().len())
            .sum();
        assert_eq!(
            statement_count, 2,
            "principal resolution must be exactly two round-trips (roles, grants) for 5 roles"
        );
    }

    #[tokio::test]
    async fn invalidate_subjects_takes_effect_on_next_context() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = seed_user(&db).await;
        grant(&db, tenant_id, GrantSubject::User(user_id), "hosts:read").await;
        let engine = AccessEngine::new(db.clone());

        let first = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("first load");
        assert_eq!(first.authority.grants.len(), 1);

        grant(&db, tenant_id, GrantSubject::User(user_id), "services:read").await;

        // Positive control: within TTL and without invalidation the cached
        // (stale) authority is served — caching exists.
        let cached = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("cached");
        assert_eq!(
            cached.authority.grants.len(),
            1,
            "pre-invalidation reads must serve the cache"
        );

        engine.invalidate_subjects(&[user_id], &[]);
        let fresh = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("post-flush reload");
        assert_eq!(
            fresh.authority.grants.len(),
            2,
            "next context() must reflect the mutation"
        );
    }

    #[tokio::test]
    async fn apply_remote_invalidation_takes_effect_on_next_context() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = seed_user(&db).await;
        grant(&db, tenant_id, GrantSubject::User(user_id), "hosts:read").await;
        let engine = AccessEngine::new(db.clone());

        let first = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("first load");
        assert_eq!(first.authority.grants.len(), 1);

        grant(&db, tenant_id, GrantSubject::User(user_id), "services:read").await;

        engine.apply_remote_invalidation(&uptrakit_wire::AccessInvalidatedPayload::new(
            vec![user_id],
            vec![],
        ));
        let fresh = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("post-flush reload");
        assert_eq!(
            fresh.authority.grants.len(),
            2,
            "remote invalidation must flush the cache"
        );
    }
}
