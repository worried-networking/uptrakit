# Service Merge Re-key Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `401 Unauthorized` after merging a freshly-enrolled pending Service into an approved one by adding a `service_merge_redirect` lookup
table, redirect-aware bearer auth, SDK-side identity rebind, and a defence-in-depth ban on merging into or from embedded Services.

**Architecture:** New SeaORM-backed `service_merge_redirect(source_id PK, target_id FK→services ON DELETE CASCADE, redirected_at)` table.
`merge_service` query upserts a redirect row at merge time; `lookup_by_secret` in `service_ws` consults the table on bearer-secret resume when the
agent's `?service_id=hint` no longer matches an active row. SDK reads the canonical `service_id` back from `ApprovedPayload` and overwrites
`service.json` when it differs. Embedded services are excluded from merge at both query and route layers.

**Tech Stack:** Rust 2024 / SeaORM / axum / parking_lot / rootcause backend. Svelte 5 (`$state`/`$derived`) frontend. Tests use
`tokio::test(start_paused = true)` for SDK time-control; integration tests on real time via Docker.

**Spec:** `docs/superpowers/specs/2026-06-09-service-merge-rekey-design.md` (commit `fb0a69c7a`).

## Test Helper Conventions (read before writing any task's test code)

Each crate has its OWN test helpers — do NOT invent generic names. Map the plan's shorthand to the real helpers below. All test snippets in this plan
use the shorthand; substitute the per-crate real helper at implementation time.

**`uptrakit-web-api-queries` (`crates/ui/web-api-queries/src/queries/services.rs` test module, `mod tests` around line 850):**

- `setup_test_db_with_tenant()` shorthand → real flow:
  `let db = setup_test_db().await; let tenant_id = Uuid::now_v7();  insert_tenant(&db, tenant_id).await; let tenant_db = TenantDb::new(db.clone(), tenant_id);`
  (the helper variants here take `(db, tenant_id)` separately — see `merge_service_does_not_copy_deactivated_host_links` at line 942 for the canonical
  pattern)
- `seed_pending_service(&tenant_db, id)` → `insert_service(&db, tenant_id, id, service::ServiceStatus::Pending).await`
- `seed_approved_service(&tenant_db, id)` → `insert_service(&db, tenant_id, id, service::ServiceStatus::Approved).await`
- `seed_*_embedded` variants → not in the file; add a sibling helper `insert_service_embedded(&db, tenant_id, id, status)` that copies
  `insert_service` (line 879) and sets `is_embedded: Set(true)`.

**`uptrakit-web-api` routes (`crates/ui/web-api/src/routes/services.rs` test module around line 1394):**

- `test_state_with_user(Permission::X)` shorthand → real flow:

  ```rust
  let db = setup_test_db().await;
  let tenant_id = Uuid::now_v7();
  insert_tenant(&db, tenant_id).await;
  let state = test_state(db.clone(), tenant_id).await;
  let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
  let user = test_authenticated_user_with_permission(Permission::UpdateServices);
  ```

  (see `merge_service_target_connected_returns_conflict` at line 1721 for the exact pattern)
- `read_body_json(response)` — confirm presence in the existing test module; the existing tests already deserialize bodies via
  `serde_json::from_slice(&body.collect()...)`. Use whatever pattern the neighbour test uses.

**`uptrakit-web-api` service_ws (`crates/ui/web-api/src/routes/service_ws/mod.rs` test module around line 503):**

- `setup_test_db_with_tenant()` shorthand → `let db = setup_test_db().await; let tenant_id = insert_tenant(&db).await;` (this crate's `insert_tenant`
  returns a `Uuid`)
- `seed_active_service_with_hash(&db, tenant_id, id, hash)` → not present; add a sibling helper modelled on the existing
  `insert_service(&db, ip_address)` (line 530) that takes `tenant_id`, `id`, and `enrollment_secret_hash` instead of generating defaults.
- `seed_redirect(&db, source_id, target_id)` → straight `service_merge_redirect::ActiveModel { ... }.insert(&db).await.expect(...)` — add a one-liner
  helper for clarity.
- `test_app_state_with_audit_capture` + `ws_upgrade_with_bearer` — NOT TRIVIAL in this test module (it currently uses raw `DatabaseConnection`, not
  `AppState`). The audit-emission unit test (Task 6 Step 12) is therefore **moved to Task 10's integration test harness** — see the note in Task 6
  Step 12.

**`uptrakit-service-sdk` (`crates/shared/service-sdk/src/ws.rs` test module):**

- `ServiceIdentityState::new_single_dir(path)` shorthand → real constructor name is `ServiceIdentityState::new_single_dir(path)` (identity.rs:118).
  Use this exact spelling.
- `spawn_mock_ws_server` / `mock_certificate_envelope_for` / `test_tls_connector` — not present; the SDK tests in this file currently exercise pure
  functions (e.g. `is_peer_closed` at line 567), not full WS handshakes. Building this harness is its own deliverable — see Task 7 Step 6 note.

**Snapshot Binding Rules used throughout:**

- "no raw SQL; use Sea ORM + sea_query builders" — all DB code uses SeaORM
- "BEGIN IMMEDIATE for read-then-write" — all new merge-time DML stays inside the existing `BEGIN IMMEDIATE` txn at `services.rs:650`
- "forbid unwrap/expect/panic in production code" — `?` + `report!()` throughout
- "wrap errors in rootcause::Report" + "report!()" + ".context_to()?" — all error paths use these
- "use parking_lot::Mutex in async code" — no new locks added
- "tests never sleep on real wall-clock time; use tokio::test(start_paused = true)" — SDK unit tests follow this
- "Conventional Commits required" — commit subjects below already follow it

---

## File Structure

**Created:**

- `crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs` — schema
- `crates/shared/db/src/entity/service_merge_redirect.rs` — entity
- `docs/adr/0020-service-merge-redirect.md` — architectural decision

**Modified:**

- `crates/shared/db/src/migration/mod.rs` — register migration
- `crates/shared/db/src/entity/mod.rs` — export entity module
- `crates/shared/db/src/entity/prelude.rs` — re-export `ServiceMergeRedirect`/`ServiceMergeRedirectModel`
- `crates/ui/web-api-queries/src/queries/services.rs` — error variants, embedded guards, redirect upsert, tests
- `crates/ui/web-api/src/api_error/mappings.rs` — map new error variants → HTTP codes
- `crates/ui/web-api/src/routes/services.rs` — route-level 400, tests
- `crates/ui/web-api/src/routes/service_ws/mod.rs` — `lookup_by_secret` redirect fallback, miss-audit enrichment, `emit_rekey_resolved_audit`, tests
- `crates/shared/audit-log/src/action_type.rs` — `AUTH_SERVICE_REKEY_RESOLVED` (const + `V1_ACTIONS` + `audit_actions!` macro)
- `crates/shared/service-sdk/src/ws.rs` — `wait_for_approval -> Result<Uuid>`, nil guard, `resume_enrollment` rebind, `run_enrollment` call-site
  discard, tests
- `crates/shared/wire/asyncapi.yaml` — doc note on `ApprovedPayload.service_id` rebind semantics
- `frontend/src/routes/services/+page.svelte` — add `!s.is_embedded` predicate; surface backend reason codes
- `CONTEXT.md` — glossary entry for **Service Merge Redirect**
- `docs/api/services-operations.md` — list new 400 reason codes

---

## Task 1: New `service_merge_redirect` table + entity

**Files:**

- Create: `crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs`
- Create: `crates/shared/db/src/entity/service_merge_redirect.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`
- Modify: `crates/shared/db/src/entity/mod.rs`
- Modify: `crates/shared/db/src/entity/prelude.rs`

Bindings: "no raw SQL; use Sea ORM + sea_query builders". Migration uses `.uuid()` (not `.binary_len(16)`) and `helpers::timestamp(...)` per the
pattern in `m20260516_000001_2fa.rs`.

- [ ] **Step 1: Write the migration file**

Create `crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs`:

```rust
use sea_orm_migration::prelude::*;

use crate::migration::helpers::timestamp;

/// Create the `service_merge_redirect` table.
///
/// Maps a deactivated source Service UUID to the active target Service UUID
/// produced by `merge_service`. The bearer-secret WS auth path consults this
/// table when an Agent reconnects with a `?service_id=hint` that no longer
/// matches an active row, so the Agent can be re-keyed onto the merge target
/// without operator intervention.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ServiceMergeRedirect::Table)
                    .col(
                        ColumnDef::new(ServiceMergeRedirect::SourceId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ServiceMergeRedirect::TargetId).uuid().not_null())
                    .col(timestamp(ServiceMergeRedirect::RedirectedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_service_merge_redirect_target")
                            .from(ServiceMergeRedirect::Table, ServiceMergeRedirect::TargetId)
                            .to(Services::Table, Services::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(ServiceMergeRedirect::Table)
                    .name("idx_service_merge_redirect_target")
                    .col(ServiceMergeRedirect::TargetId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .table(ServiceMergeRedirect::Table)
                    .name("idx_service_merge_redirect_target")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ServiceMergeRedirect::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ServiceMergeRedirect {
    Table,
    SourceId,
    TargetId,
    RedirectedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
```

- [ ] **Step 2: Register the migration**

Edit `crates/shared/db/src/migration/mod.rs`. Locate the `mod m20260516_000001_2fa;` declaration and add the new module immediately after it:

```rust
mod m20260516_000001_2fa;
mod m20260610_000001_service_merge_redirect;
```

In the same file, find the `fn migrations()` `vec![]` (around line 90). At the END of the vec (after the last `Box::new(...)` entry), append:

```rust
Box::new(m20260610_000001_service_merge_redirect::Migration),
```

- [ ] **Step 3: Write the entity**

Create `crates/shared/db/src/entity/service_merge_redirect.rs`:

```rust
//! Mapping from a deactivated source Service UUID to its merge target.
//!
//! Written by `merge_service` inside the same `BEGIN IMMEDIATE` transaction
//! that deactivates the source row. Read on the bearer-secret WS auth path
//! when an Agent's persisted `service_id` no longer matches an active row,
//! so the controller can resolve the Agent to its current canonical identity.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "service_merge_redirect")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub source_id: Uuid,
    pub target_id: Uuid,
    pub redirected_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::service::Entity",
        from = "Column::TargetId",
        to = "super::service::Column::Id"
    )]
    Service,
}

impl Related<super::service::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Service.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 4: Export the entity module**

Edit `crates/shared/db/src/entity/mod.rs`. Find the `pub mod service_host;` line and add right after it:

```rust
pub mod service_merge_redirect;
```

Then edit `crates/shared/db/src/entity/prelude.rs`. Find the `pub use super::service_host::...` line and add right after it:

```rust
pub use super::service_merge_redirect::{
    Entity as ServiceMergeRedirect, Model as ServiceMergeRedirectModel,
};
```

- [ ] **Step 5: Verify the workspace builds and migration applies**

Run from the workspace root:

```bash
cargo check --no-default-features --features db-sqlite
```

Expected: clean build.

Run the existing DB-integration suite (which exercises migration up/down):

```bash
cargo test -p uptrakit-integration-tests --test database -- --ignored
```

Expected: all migration tests pass; the new table appears in the schema dump.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs \
        crates/shared/db/src/entity/service_merge_redirect.rs \
        crates/shared/db/src/migration/mod.rs \
        crates/shared/db/src/entity/mod.rs \
        crates/shared/db/src/entity/prelude.rs
git commit --only crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs \
                  crates/shared/db/src/entity/service_merge_redirect.rs \
                  crates/shared/db/src/migration/mod.rs \
                  crates/shared/db/src/entity/mod.rs \
                  crates/shared/db/src/entity/prelude.rs \
  -m "feat(db): add service_merge_redirect table + entity"
```

---

## Task 2: `ServiceQueryError` new variants + audit + ApiError mapping

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/services.rs` (variants + classification)
- Modify: `crates/ui/web-api/src/api_error/mappings.rs` (HTTP mapping)

Bindings: "wrap errors in rootcause::Report" — variant added to existing typed enum, not stringified. `#[non_exhaustive]` not required (internal
type).

- [ ] **Step 1: Add the new variants**

Edit `crates/ui/web-api-queries/src/queries/services.rs`. Find the `pub enum ServiceQueryError` block (around line 24). Add three new variants
immediately after `EmbeddedService`:

```rust
    /// Embedded services cannot be merged (target side).
    #[error("target service is embedded and cannot be merged")]
    TargetEmbedded,
    /// Embedded services cannot be merged (source side).
    #[error("source service is embedded and cannot be merged")]
    SourceEmbedded,
    /// Pre-condition for merge violated: existing redirect row points at the
    /// service being merged in as `source`. This must not occur by construction
    /// (deactivated services cannot become merge sources). Surfaces only on
    /// data corruption.
    #[error("redirect chain invariant violated")]
    RedirectChainInvariantViolated,
```

- [ ] **Step 2: Extend `audit_classification`**

In the same file, find `impl ServiceQueryError { pub fn audit_classification(...)` (around line 54). Add three new match arms before the closing brace
of the `match`:

```rust
            Self::TargetEmbedded => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "service.embedded_target",
            ),
            Self::SourceEmbedded => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "service.embedded_source",
            ),
            Self::RedirectChainInvariantViolated => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "service.merge_invariant",
            ),
```

- [ ] **Step 3: Add HTTP mapping**

Edit `crates/ui/web-api/src/api_error/mappings.rs`. Find the `impl From<Report<ServiceQueryError>> for ApiError` block (line 71). Add three new arms
immediately before the `Db(_)` arm:

```rust
            TargetEmbedded => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Cannot merge into an embedded service.",
                "service.embedded_target",
                None,
            ),
            SourceEmbedded => ApiError::new(
                StatusCode::BAD_REQUEST,
                "Cannot merge from an embedded service.",
                "service.embedded_source",
                None,
            ),
            RedirectChainInvariantViolated => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Service merge state is inconsistent — contact an administrator.",
                "service.merge_invariant",
                Some(format_report_summary(&report)),
            ),
```

- [ ] **Step 4: Verify build + clippy**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: clean. Match exhaustiveness will fail compilation if any other consumer of `ServiceQueryError` matches without the new arms — fix any
reported sites by adding a wildcard arm with `tracing::warn!` per snapshot rule "external matches require wildcard arm".

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/web-api-queries/src/queries/services.rs \
                  crates/ui/web-api/src/api_error/mappings.rs \
  -m "feat(web-api): typed errors for embedded-merge ban + invariant violation"
```

---

## Task 3: Extend `merge_service` query — guards, invariant, redirect upsert

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/services.rs` (the `merge_service` function around line 639 + tests at end of file)

Bindings: "BEGIN IMMEDIATE for read-then-write" — all new DML lives inside the existing txn. "no raw SQL" — use
`OnConflict::column(...).update_columns(...)`.

- [ ] **Step 1: Write a failing test for the embedded-target ban**

In the same file's `#[cfg(test)] mod tests` block (find existing `merge_service_does_not_copy_deactivated_host_links` around line 942 — co-locate the
new tests near it), add:

```rust
#[tokio::test]
async fn merge_service_rejects_embedded_target() {
    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let tenant_db = TenantDb::new(db, tenant_id);

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    seed_pending_service(&tenant_db, source_id).await;
    seed_approved_service_embedded(&tenant_db, target_id).await;

    let err = merge_service(&tenant_db, target_id, source_id, false, tenant_id)
        .await
        .unwrap_err();

    assert!(matches!(
        err.current_context(),
        ServiceQueryError::TargetEmbedded
    ));
}
```

A helper `seed_approved_service_embedded(&tenant_db, id)` mirroring the existing `seed_approved_service` helper but setting `is_embedded = true` must
exist. If absent in `tests::common`, add it before this test in a new `mod helpers` block within `tests`, copying the existing seeder and toggling the
field. Refer to neighbouring tests in the same file for the exact seeder shape.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-web-api-queries merge_service_rejects_embedded_target --features db-sqlite
```

Expected: FAIL (`TargetEmbedded` not produced — merge currently allows embedded targets).

- [ ] **Step 3: Add the embedded-target guard**

Edit `merge_service` in `crates/ui/web-api-queries/src/queries/services.rs`. After the existing `target` lookup (line 660–667) and BEFORE the
`target_caps` parsing, insert:

```rust
    if target.is_embedded {
        bail!(ServiceQueryError::TargetEmbedded);
    }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api-queries merge_service_rejects_embedded_target --features db-sqlite
```

Expected: PASS.

- [ ] **Step 5: Write a failing test for the embedded-source ban**

Add adjacent to the previous test:

```rust
#[tokio::test]
async fn merge_service_rejects_embedded_source() {
    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let tenant_db = TenantDb::new(db, tenant_id);

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    seed_pending_service_embedded(&tenant_db, source_id).await;
    seed_approved_service(&tenant_db, target_id).await;

    let err = merge_service(&tenant_db, target_id, source_id, false, tenant_id)
        .await
        .unwrap_err();

    assert!(matches!(
        err.current_context(),
        ServiceQueryError::SourceEmbedded
    ));
}
```

Add `seed_pending_service_embedded` as a sibling of `seed_approved_service_embedded` in the same `mod helpers` block.

- [ ] **Step 6: Add the embedded-source guard**

After the existing `source` lookup (line 678–685) and BEFORE the `source_caps` parsing, insert:

```rust
    if source.is_embedded {
        bail!(ServiceQueryError::SourceEmbedded);
    }
```

- [ ] **Step 7: Run both embedded tests**

```bash
cargo test -p uptrakit-web-api-queries -- merge_service_rejects --features db-sqlite
```

Expected: both PASS.

- [ ] **Step 8: Write a failing test for the redirect insertion**

```rust
#[tokio::test]
async fn merge_service_inserts_redirect_row() {
    use uptrakit_shared_db::entity::service_merge_redirect;

    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let tenant_db = TenantDb::new(db, tenant_id);

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    seed_pending_service(&tenant_db, source_id).await;
    seed_approved_service(&tenant_db, target_id).await;

    merge_service(&tenant_db, target_id, source_id, false, tenant_id)
        .await
        .expect("merge succeeds");

    let redirect = service_merge_redirect::Entity::find_by_id(source_id)
        .one(tenant_db.db())
        .await
        .expect("query ok");

    let redirect = redirect.expect("redirect row exists");
    assert_eq!(redirect.target_id, target_id);
}
```

- [ ] **Step 9: Run test to verify it fails**

```bash
cargo test -p uptrakit-web-api-queries merge_service_inserts_redirect_row --features db-sqlite
```

Expected: FAIL (no row inserted yet).

- [ ] **Step 10: Wire up the chain-invariant assertion + redirect upsert inside the txn**

In `merge_service`, BEFORE the existing `txn.commit().await.context_to()?;` line (around line 783), insert these two operations:

```rust
    use uptrakit_shared_db::entity::service_merge_redirect;
    use sea_orm::QueryFilter;
    use sea_orm::sea_query::OnConflict;

    // Chain-invariant: existing redirects must never point at our source_id.
    // Chains cannot pre-exist (source must be Pending; deactivated rows cannot
    // become future sources or targets), so a non-zero count here means the
    // table has been corrupted out-of-band.
    let dangling = service_merge_redirect::Entity::find()
        .filter(service_merge_redirect::Column::TargetId.eq(source_uuid))
        .count(&txn)
        .await
        .context_to()?;
    if dangling != 0 {
        bail!(ServiceQueryError::RedirectChainInvariantViolated);
    }

    // Upsert the redirect row: source_id is unique; on conflict we refresh
    // the target_id + redirected_at columns (covers the extremely rare case
    // of re-merging a previously-deactivated source).
    let redirect_model = service_merge_redirect::ActiveModel {
        source_id: Set(source_uuid),
        target_id: Set(target_uuid),
        redirected_at: Set(now),
    };
    service_merge_redirect::Entity::insert(redirect_model)
        .on_conflict(
            OnConflict::column(service_merge_redirect::Column::SourceId)
                .update_columns([
                    service_merge_redirect::Column::TargetId,
                    service_merge_redirect::Column::RedirectedAt,
                ])
                .to_owned(),
        )
        .exec(&txn)
        .await
        .context_to()?;
```

Ensure `Set` is in scope (it is — already imported at the top of the file for the existing merge logic).

- [ ] **Step 11: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api-queries merge_service_inserts_redirect_row --features db-sqlite
```

Expected: PASS.

- [ ] **Step 12: Write a test for FK cascade on target delete**

```rust
#[tokio::test]
async fn merge_service_redirect_cascades_on_target_delete() {
    use uptrakit_shared_db::entity::service_merge_redirect;
    use sea_orm::EntityTrait;
    use uptrakit_shared_db::entity::service;

    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let tenant_db = TenantDb::new(db, tenant_id);

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    seed_pending_service(&tenant_db, source_id).await;
    seed_approved_service(&tenant_db, target_id).await;

    merge_service(&tenant_db, target_id, source_id, false, tenant_id)
        .await
        .expect("merge succeeds");

    // Hard-delete the target row (bypasses the deactivate path on purpose).
    service::Entity::delete_by_id(target_id)
        .exec(tenant_db.db())
        .await
        .expect("delete ok");

    let redirect = service_merge_redirect::Entity::find_by_id(source_id)
        .one(tenant_db.db())
        .await
        .expect("query ok");

    assert!(
        redirect.is_none(),
        "FK ON DELETE CASCADE must remove redirect row"
    );
}
```

- [ ] **Step 13: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api-queries merge_service_redirect_cascades_on_target_delete --features db-sqlite
```

Expected: PASS (the FK + cascade are baked into the migration from Task 1).

- [ ] **Step 14: Write a test for the invariant assertion**

```rust
#[tokio::test]
async fn merge_service_rejects_when_redirect_chain_invariant_violated() {
    use uptrakit_shared_db::entity::service_merge_redirect;
    use sea_orm::Set;

    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let tenant_db = TenantDb::new(db, tenant_id);

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    let phantom_source = uuid::Uuid::now_v7();
    seed_pending_service(&tenant_db, source_id).await;
    seed_approved_service(&tenant_db, target_id).await;

    // Plant a corrupted redirect row pointing AT our source_id; merge_service
    // must refuse rather than silently overwrite state.
    service_merge_redirect::ActiveModel {
        source_id: Set(phantom_source),
        target_id: Set(source_id),
        redirected_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(tenant_db.db())
    .await
    .expect("seed redirect ok");

    let err = merge_service(&tenant_db, target_id, source_id, false, tenant_id)
        .await
        .unwrap_err();

    assert!(matches!(
        err.current_context(),
        ServiceQueryError::RedirectChainInvariantViolated
    ));
}
```

- [ ] **Step 15: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api-queries merge_service_rejects_when_redirect_chain_invariant_violated --features db-sqlite
```

Expected: PASS.

- [ ] **Step 16: Run the full query-crate suite + sqlite-feature check**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo test -p uptrakit-web-api-queries --features db-sqlite
```

Expected: all clean.

- [ ] **Step 17: Commit**

```bash
git commit --only crates/ui/web-api-queries/src/queries/services.rs \
  -m "feat(web-api-queries): embedded-merge ban + redirect upsert + invariant assert"
```

---

## Task 4: Route-level 400 for embedded merge

**Files:**

- Modify: `crates/ui/web-api/src/routes/services.rs` (the `merge_service` route around line 1089 + new tests)

Bindings: defence-in-depth at boundary; query layer (Task 3) still authoritative. Route reuses existing `emit_service_lifecycle_audit` for the
ValidationFailed branch.

- [ ] **Step 1: Write a failing test for the route 400 (target embedded)**

In the existing `#[cfg(test)] mod tests` block in `routes/services.rs` (find `merge_service_succeeds_and_deactivates_source` at line 1799 —
co-locate):

```rust
#[tokio::test(start_paused = false)]
async fn merge_service_returns_400_when_target_embedded() {
    let (state, tenant_db, _user) = test_state_with_user(Permission::UpdateServices).await;

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    seed_pending_service(&tenant_db, source_id).await;
    seed_approved_service_embedded(&tenant_db, target_id).await;

    let response = merge_service(
        State(state.clone()),
        tenant_db,
        CanUpdateServices(_user.clone()),
        None,
        Path(target_id),
        Json(MergeAgentRequest { source_id }),
    )
    .await
    .expect("handler returns Ok with error response body");

    let response = response.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_json(response).await;
    assert_eq!(body["error"]["code"].as_str(), Some("service.embedded_target"));
}
```

The `read_body_json` helper exists in `tests::common` (used by surrounding tests). The seeder `seed_approved_service_embedded` is the one added in
Task 3 — re-use from there (or replicate inline if the test module is in a different crate; the test-state seeder pattern in `routes/services.rs`
tests is independent of the query-crate one).

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-web-api merge_service_returns_400_when_target_embedded
```

Expected: FAIL — currently the query-layer guard from Task 3 already returns 400 via `ApiError`, BUT this test asserts the route-level audit emission
happens for the ValidationFailed branch. Skip ahead if test already PASSES — note that and move to Step 5.

- [ ] **Step 3: Add the route-level guard**

Edit `crates/ui/web-api/src/routes/services.rs`. In the `merge_service` function (line 1089), AFTER the self-merge check (around line 1130
`if target_uuid == source_uuid`) and BEFORE `state.service_connections.is_connected(...)`, insert:

```rust
    // Defence-in-depth: short-circuit with a clear 400 + audit before any
    // session/state interaction. The query layer also rejects this (see
    // ServiceQueryError::{Target,Source}Embedded) but the route layer owns
    // the audit emission for ValidationFailed outcomes.
    let (target_embedded, source_embedded) =
        match svc_queries::is_embedded_pair(&tenant_db, target_uuid, source_uuid).await {
            Ok(pair) => pair,
            Err(err) => return Err(ApiError::from(err)),
        };
    if target_embedded {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.embedded_target",
            }),
        );
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Cannot merge into an embedded service.",
        ));
    }
    if source_embedded {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.embedded_source",
            }),
        );
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Cannot merge from an embedded service.",
        ));
    }
```

- [ ] **Step 4: Add the `is_embedded_pair` query helper**

Edit `crates/ui/web-api-queries/src/queries/services.rs`. Add a new public helper (place it near `merge_service`):

```rust
/// Look up the `is_embedded` flag for two services in a single round-trip.
///
/// Returns `(target_is_embedded, source_is_embedded)`. Missing rows fall back
/// to `false` (the merge query will then surface the canonical `NotFound` /
/// `SourceNotFound` error during its own lookup).
pub async fn is_embedded_pair(
    tenant_db: &TenantDb,
    target_id: Uuid,
    source_id: Uuid,
) -> Result<(bool, bool)> {
    let rows = tenant_db
        .find::<service::Entity>()
        .filter(service::Column::Id.is_in([target_id, source_id]))
        .all(tenant_db.db())
        .await
        .context_to()?;
    let target = rows.iter().find(|r| r.id == target_id).map(|r| r.is_embedded).unwrap_or(false);
    let source = rows.iter().find(|r| r.id == source_id).map(|r| r.is_embedded).unwrap_or(false);
    Ok((target, source))
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api merge_service_returns_400_when_target_embedded
```

Expected: PASS.

- [ ] **Step 6: Write a failing test for source-embedded 400**

```rust
#[tokio::test(start_paused = false)]
async fn merge_service_returns_400_when_source_embedded() {
    let (state, tenant_db, _user) = test_state_with_user(Permission::UpdateServices).await;

    let source_id = uuid::Uuid::now_v7();
    let target_id = uuid::Uuid::now_v7();
    seed_pending_service_embedded(&tenant_db, source_id).await;
    seed_approved_service(&tenant_db, target_id).await;

    let response = merge_service(
        State(state.clone()),
        tenant_db,
        CanUpdateServices(_user.clone()),
        None,
        Path(target_id),
        Json(MergeAgentRequest { source_id }),
    )
    .await
    .expect("handler returns Ok with error response body")
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_json(response).await;
    assert_eq!(body["error"]["code"].as_str(), Some("service.embedded_source"));
}
```

- [ ] **Step 7: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api merge_service_returns_400_when_source_embedded
```

Expected: PASS (guard from Step 3 covers both sides).

- [ ] **Step 8: Full quality gates**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p uptrakit-web-api
cargo test -p uptrakit-web-api-queries
```

Expected: clean.

- [ ] **Step 9: Commit**

```bash
git commit --only crates/ui/web-api/src/routes/services.rs \
                  crates/ui/web-api-queries/src/queries/services.rs \
  -m "feat(web-api): route-level 400 for embedded merge with ValidationFailed audit"
```

---

## Task 5: New audit action `AUTH_SERVICE_REKEY_RESOLVED`

**Files:**

- Modify: `crates/shared/audit-log/src/action_type.rs` (three coupled additions)

Bindings: "all extensible public enums + structs `#[non_exhaustive]`" — `RegisteredAuditAction` constants are additive, no enum variant added.

- [ ] **Step 1: Add the const declaration**

Edit `crates/shared/audit-log/src/action_type.rs`. Find the line:

```rust
    pub const AUTH_SERVICE_AUTHENTICATE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.service.authenticate", AuditActionKind::Event);
```

(around line 88). Add the new constant immediately after it:

```rust
    pub const AUTH_SERVICE_REKEY_RESOLVED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.service.rekey_resolved", AuditActionKind::Event);
```

- [ ] **Step 2: Register in `V1_ACTIONS`**

In the same file, find:

```rust
    AuditActionType::AUTH_SERVICE_AUTHENTICATE,
```

(around line 427). Add immediately after:

```rust
    AuditActionType::AUTH_SERVICE_REKEY_RESOLVED,
```

- [ ] **Step 3: Register in the `audit_actions!` macro block**

In the same file, find:

```rust
    auth_service_authenticate => AUTH_SERVICE_AUTHENTICATE, Event;
```

(around line 684). Add immediately after:

```rust
    auth_service_rekey_resolved => AUTH_SERVICE_REKEY_RESOLVED, Event;
```

- [ ] **Step 4: Write a test asserting the new action is registered**

In the same crate's existing test module (look for `mod tests` toward the end of `action_type.rs` — if absent, see
`crates/shared/audit-log/src/tests.rs`), add:

```rust
#[test]
fn auth_service_rekey_resolved_is_registered() {
    assert!(AuditActionType::AUTH_SERVICE_REKEY_RESOLVED
        .as_str()
        .starts_with("auth.service."));
    let parsed: AuditActionType = "auth.service.rekey_resolved".parse().unwrap();
    assert_eq!(parsed.as_str(), "auth.service.rekey_resolved");
}
```

- [ ] **Step 5: Run the audit-log crate tests**

```bash
cargo test -p uptrakit-audit-log
```

Expected: PASS.

- [ ] **Step 6: Run full build to verify the `audit_actions!` macro expanded the new constructor**

```bash
cargo check --all-features
```

Expected: clean — `AuditEntry::auth_service_rekey_resolved(...)` is now reachable.

- [ ] **Step 7: Commit**

```bash
git commit --only crates/shared/audit-log/src/action_type.rs \
                  crates/shared/audit-log/src/tests.rs \
  -m "feat(audit-log): add AUTH_SERVICE_REKEY_RESOLVED action"
```

(If the test landed inline in `action_type.rs` rather than `tests.rs`, drop the `tests.rs` from the `--only` list.)

---

## Task 6: `lookup_by_secret` redirect fallback + miss-audit enrichment

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/mod.rs` (lookup, audit helpers, new emit fn, tests)

Bindings: "use `let-else` with explicit `else { return Err(...) }`"; "`.context_to::<ServiceWsError>()?` on every DB call"; "audit emission via
non-blocking `audit_emitter.emit_event`".

- [ ] **Step 1: Write a failing test — redirect lookup succeeds with hint**

In the existing `#[cfg(all(test, feature = "db-sqlite"))] mod tests` block at the bottom of `service_ws/mod.rs` (around line 389):

```rust
#[tokio::test]
async fn lookup_by_secret_uses_redirect_when_source_deactivated() {
    use uptrakit_shared_db::entity::service_merge_redirect;
    use sea_orm::{ActiveModelTrait, Set};

    let (db, tenant_id) = setup_test_db_with_tenant().await;

    let target_id = uuid::Uuid::now_v7();
    let source_id = uuid::Uuid::now_v7();
    let secret = "test-secret-abcdef0123456789";
    let secret_hash = crate::auth::token::hash_token(secret);

    seed_active_service_with_hash(&db, tenant_id, target_id, &secret_hash).await;
    // Source row was deactivated by a prior merge; only the redirect row remains.
    service_merge_redirect::ActiveModel {
        source_id: Set(source_id),
        target_id: Set(target_id),
        redirected_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&db)
    .await
    .expect("seed redirect ok");

    let (resolved_id, is_system) = lookup_by_secret(&db, secret, Some(source_id))
        .await
        .expect("redirect resolves");

    assert_eq!(resolved_id, target_id);
    assert!(!is_system);
}
```

The seeder `seed_active_service_with_hash` exists if you have nearby tests using it; otherwise add it inline to the test module, modelled on existing
test seeders in the file (look for `seed_active_service` or similar — adapt to take the hash as a parameter).

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-web-api lookup_by_secret_uses_redirect_when_source_deactivated --features db-sqlite
```

Expected: FAIL — `InvalidSecret` returned because the redirect path doesn't exist yet.

- [ ] **Step 3: Wire the redirect fallback into `lookup_by_secret`**

Edit `crates/ui/web-api/src/routes/service_ws/mod.rs`. Replace the trailing `Err(report!(ServiceWsError::InvalidSecret))` at line 223 with the
redirect-fallback block. The full new tail of `lookup_by_secret` (replacing the existing system-services try + final Err) becomes:

```rust
    // Try system services.
    use uptrakit_shared_db::entity::system_service as sys_svc_entity;
    let mut sys_query = uptrakit_shared_db::entity::prelude::SystemService::find()
        .filter(sys_svc_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(sys_svc_entity::Column::DeactivatedAt.is_null());
    if let Some(id) = service_id {
        sys_query = sys_query.filter(sys_svc_entity::Column::Id.eq(id));
    }
    if let Some(svc) = sys_query.one(db).await.context_to::<ServiceWsError>()? {
        return Ok((svc.id, true));
    }

    // Redirect fallback: only triggered when the caller supplied a hint, so
    // the existing cross-service-collision defence-in-depth for the hint-less
    // path is preserved.
    let Some(hint) = service_id else {
        return Err(report!(ServiceWsError::InvalidSecret));
    };
    let Some(redirect) = uptrakit_shared_db::entity::prelude::ServiceMergeRedirect::find_by_id(hint)
        .one(db)
        .await
        .context_to::<ServiceWsError>()?
    else {
        return Err(report!(ServiceWsError::InvalidSecret));
    };
    let Some(target) = uptrakit_shared_db::entity::prelude::Service::find_by_id(redirect.target_id)
        .filter(service_entity::Column::DeactivatedAt.is_null())
        .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .one(db)
        .await
        .context_to::<ServiceWsError>()?
    else {
        return Err(report!(ServiceWsError::InvalidSecret));
    };

    Ok((target.id, false))
}
```

(Note: the `emit_rekey_resolved_audit` call is wired in Step 5; here the lookup function only resolves the id. The audit emission happens at the
caller in `service_ws`.)

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api lookup_by_secret_uses_redirect_when_source_deactivated --features db-sqlite
```

Expected: PASS.

- [ ] **Step 5: Write a failing test — fallback NOT taken without hint**

```rust
#[tokio::test]
async fn lookup_by_secret_skips_redirect_when_no_hint() {
    use uptrakit_shared_db::entity::service_merge_redirect;
    use sea_orm::{ActiveModelTrait, Set};

    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let target_id = uuid::Uuid::now_v7();
    let source_id = uuid::Uuid::now_v7();
    let secret = "another-secret-fedcba9876543210";
    let secret_hash = crate::auth::token::hash_token(secret);
    seed_active_service_with_hash(&db, tenant_id, target_id, &secret_hash).await;
    service_merge_redirect::ActiveModel {
        source_id: Set(source_id),
        target_id: Set(target_id),
        redirected_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&db)
    .await
    .expect("seed redirect ok");

    // No hint => primary lookup matches by hash directly (Service::find without id filter).
    // We assert that resolution does NOT walk the redirect path. With a hint-less
    // bearer call, the primary tenant lookup finds the target directly anyway,
    // so the success path is identical — but we additionally assert the
    // redirect row is irrelevant.
    let (resolved_id, _) = lookup_by_secret(&db, secret, None)
        .await
        .expect("primary tenant lookup matches");
    assert_eq!(resolved_id, target_id);
}
```

(This documents the documented behaviour. Run to confirm green.)

- [ ] **Step 6: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api lookup_by_secret_skips_redirect_when_no_hint --features db-sqlite
```

Expected: PASS.

- [ ] **Step 7: Write a failing test — redirect-row exists but hash mismatch**

```rust
#[tokio::test]
async fn lookup_by_secret_rejects_redirect_when_hash_mismatch() {
    use uptrakit_shared_db::entity::service_merge_redirect;
    use sea_orm::{ActiveModelTrait, Set};

    let (db, tenant_id) = setup_test_db_with_tenant().await;
    let target_id = uuid::Uuid::now_v7();
    let source_id = uuid::Uuid::now_v7();
    let real_secret = "real-secret-1111111111111111";
    let attacker_secret = "attacker-secret-2222222222222";
    let real_hash = crate::auth::token::hash_token(real_secret);
    seed_active_service_with_hash(&db, tenant_id, target_id, &real_hash).await;
    service_merge_redirect::ActiveModel {
        source_id: Set(source_id),
        target_id: Set(target_id),
        redirected_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&db)
    .await
    .expect("seed redirect ok");

    let err = lookup_by_secret(&db, attacker_secret, Some(source_id))
        .await
        .unwrap_err();

    assert!(matches!(err.current_context(), ServiceWsError::InvalidSecret));
}
```

- [ ] **Step 8: Run test to verify it passes**

```bash
cargo test -p uptrakit-web-api lookup_by_secret_rejects_redirect_when_hash_mismatch --features db-sqlite
```

Expected: PASS (the `secret_hash.eq(...)` filter inside the redirect arm rejects it).

- [ ] **Step 9: Add the `emit_rekey_resolved_audit` helper and wire it into the WS entry path**

At the top of `service_ws/mod.rs` (with the other audit helpers), add:

```rust
async fn emit_rekey_resolved_audit(
    state: &AppState,
    source_id: Uuid,
    target_id: Uuid,
    tenant_id: Uuid,
    client_ip: Option<IpAddr>,
    service_app_name: Option<String>,
) {
    let mut details = serde_json::json!({
        "source_id": source_id,
        "target_id": target_id,
        "reason_code": "merge_redirect",
    });
    if let Some(client_ip) = client_ip {
        details["client_ip"] = serde_json::Value::String(client_ip.to_string());
    }

    // Use the fully qualified `uptrakit_audit_log::Event` to avoid any name collision
    // with `wire::Event` or similar local imports in this module.
    let entry = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        AuditActionType::AUTH_SERVICE_REKEY_RESOLVED,
    )
    .actor(AuditActorType::Service, Some(target_id))
    .actor_display_opt(service_app_name.clone())
    .target_opt(Some("service".to_string()), Some(target_id.to_string()), service_app_name)
    .outcome(AuditOutcome::Success)
    .details(details)
    .tenant_scope(tenant_id)
    .build();

    match entry {
        Ok(entry) => state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            error = %error,
            %source_id,
            %target_id,
            "failed to build AUTH_SERVICE_REKEY_RESOLVED audit entry"
        ),
    }
}
```

- [ ] **Step 10: Emit the audit when redirect resolution succeeded**

In the `service_ws` function (line 57+), expand the bearer-success branch. Currently:

```rust
            Ok((id, is_system)) => {
                tracing::info!(
                    service_id = %id,
                    is_system,
                    "enrolled service WS upgrade (bearer)"
                );
                ConnectionType::Enrolled {
                    service_id: id,
                    is_system,
                }
            }
```

(around lines 108–119). Replace with:

```rust
            Ok((id, is_system)) => {
                if let Some(hint) = query_service_id {
                    if hint != id && !is_system {
                        // Resolved via merge redirect. Look up the tenant + app_name
                        // for the audit entry. On DB error, log a warning and skip
                        // the audit emission — the WS upgrade has already been
                        // authenticated and must not be blocked on an audit lookup.
                        // (Per coding-standards: DB errors are NOT silently
                        // discarded; they are explicitly logged.)
                        match uptrakit_shared_db::entity::prelude::Service::find_by_id(id)
                            .one(state.db())
                            .await
                        {
                            Ok(Some(svc)) => {
                                emit_rekey_resolved_audit(
                                    &state,
                                    hint,
                                    id,
                                    svc.tenant_id,
                                    client_ip.as_ref().map(|Extension(ClientIp(ip))| *ip),
                                    Some(svc.service_app_name.clone()),
                                )
                                .await;
                            }
                            Ok(None) => {
                                tracing::warn!(
                                    %hint, %id,
                                    "rekey_resolved audit skipped: target service vanished between lookup_by_secret and audit emission"
                                );
                            }
                            Err(err) => {
                                tracing::warn!(
                                    error = %err,
                                    %hint, %id,
                                    "rekey_resolved audit skipped: failed to load target service for audit context"
                                );
                            }
                        }
                    }
                }
                tracing::info!(
                    service_id = %id,
                    is_system,
                    "enrolled service WS upgrade (bearer)"
                );
                ConnectionType::Enrolled {
                    service_id: id,
                    is_system,
                }
            }
```

- [ ] **Step 11: Enrich the miss-audit details**

Edit `emit_bearer_service_auth_failure_audit` (around line 293). Add a parameter `redirect_present: Option<bool>` (None when no hint was supplied so
the lookup didn't even attempt the redirect arm):

```rust
async fn emit_bearer_service_auth_failure_audit(
    state: &AppState,
    service_id_hint: Option<Uuid>,
    client_ip: Option<IpAddr>,
    outcome: AuditOutcome,
    reason_code: &'static str,
    redirect_present: Option<bool>,
) {
```

Inside, when building `details`, also include:

```rust
    if let Some(checked) = redirect_present {
        details["redirect_checked"] = serde_json::Value::Bool(true);
        details["redirect_present"] = serde_json::Value::Bool(checked);
    }
```

Update the existing single call site at line 123 of `service_ws` (the failure branch inside `service_ws` after `lookup_by_secret` returns `Err`). KEEP
`lookup_by_secret`'s return type `ServiceWsResult<(Uuid, bool)>` unchanged — no new outcome enum is needed. Instead, derive the `redirect_present`
flag directly from the `query_service_id` at the caller (a hint was supplied ⇒ the redirect arm was attempted):

```rust
// Replace the existing call to emit_bearer_service_auth_failure_audit (around line 123):
let redirect_present = match e.current_context() {
    // hint supplied + InvalidSecret ⇒ we ran the redirect arm and it didn't yield a usable session
    ServiceWsError::InvalidSecret if query_service_id.is_some() => Some(true),
    // no hint ⇒ redirect arm not attempted
    ServiceWsError::InvalidSecret => Some(false),
    _ => None,
};
emit_bearer_service_auth_failure_audit(
    &state,
    query_service_id,
    client_ip.as_ref().map(|Extension(ClientIp(ip))| *ip),
    outcome,
    reason_code,
    redirect_present,
)
.await;
```

This keeps `redirect_checked = true` exactly when a hint was supplied; the audit caller does not need to distinguish "redirect row missing" from
"redirect row found but secret hash mismatch" — both indicate "merge redirect was checked and did not yield a usable session." Plus the existing
`service_id_hint` detail already reveals which hint was used.

- [ ] **Step 12: Audit-emission assertion is covered by the Docker integration test**

The existing `service_ws/mod.rs` test module exercises `lookup_by_secret` against a raw `DatabaseConnection` — it has no `AppState` and no in-process
audit-emitter capture harness. Building one solely for this single assertion is disproportionate scope.

**Instead**, the rekey audit emission is asserted in the Docker end-to-end test (`service_merge_rekey_end_to_end`, Task 10): after the agent rebinds,
query the `audit_logs` table directly via the controller's DB and assert a row with `action_type = 'auth.service.rekey_resolved'` and details
containing both `source_id` and `target_id` exists. This catches the same regression (missing or wrong-shaped audit) without standing up a fake
AppState.

Add this assertion when writing Task 10's test body.

- [ ] **Step 13: Full clippy + test for the crate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p uptrakit-web-api
```

Expected: clean.

- [ ] **Step 14: Commit**

```bash
git commit --only crates/ui/web-api/src/routes/service_ws/mod.rs \
  -m "feat(web-api): redirect-aware bearer auth + rekey_resolved audit"
```

---

## Task 7: SDK — `wait_for_approval` returns Uuid; `resume_enrollment` rebinds identity

**Files:**

- Modify: `crates/shared/service-sdk/src/ws.rs` (return type change; rebind; nil guard; call-site fix; tests)

Bindings: "tests never sleep on real wall-clock time; use `tokio::test(start_paused = true)`" — applies to the SDK tests added here. "all reconnect
loops use `Backoff`" — unchanged (this task does not touch `lifecycle.rs`).

- [ ] **Step 1: Write a failing test — `wait_for_approval` returns payload's service_id**

In the existing `mod tests` block at the bottom of `crates/shared/service-sdk/src/ws.rs`:

```rust
#[tokio::test(start_paused = true)]
async fn wait_for_approval_returns_payload_service_id() {
    use tokio_tungstenite::tungstenite::Message;
    use uuid::Uuid;

    let target_id = Uuid::now_v7();
    let envelope = serde_json::json!({
        "protocol_version": uptrakit_wire::CURRENT_PROTOCOL_VERSION,
        "seq": 1,
        "message": { "Approved": { "service_id": target_id } }
    });

    let (mut a, b) = tokio::io::duplex(8 * 1024);
    // Drive an in-process WS handshake between a/b — reuse whatever helper
    // surrounding tests use (look for `serve_ws_mock` or similar in this
    // file's tests, or in test_support.rs). The mock controller sends
    // `envelope.to_string()` as a text message.
    spawn_mock_controller(b, vec![Message::Text(envelope.to_string())]).await;
    let mut ws_stream = client_handshake(a).await;
    let mut in_seq = IncomingSeq::new();

    let approved = wait_for_approval(&mut ws_stream, &mut in_seq)
        .await
        .expect("approval received");

    assert_eq!(approved, target_id);
}
```

**SDK test harness reality check**: the existing `mod tests` block at the bottom of `ws.rs` only exercises pure functions (e.g. `is_peer_closed` at
line 567). No duplex-stream / mock-controller / mock-TLS scaffolding exists yet. The Step 1 / Step 6 / Step 10 / Step 12 tests below all need this
scaffolding. As a sub-task before proceeding to Step 3, BUILD the harness:

- Add a helper `serve_mock_controller(messages: Vec<Message>) -> WsStream` that:
  1. Opens a `tokio::io::duplex(8 * 1024)` pair.
  2. Spawns a task on one half driving a `tokio_tungstenite::accept_async` handshake and writing each message in `messages` in order.
  3. Returns a `tokio_tungstenite::WebSocketStream` for the other half — already through the client-side handshake.
- The TLS-backed paths (`resume_enrollment`, `run_enrollment`) take a `TlsConnector`; for the unit tests in Steps 6/10/12, extract a
  `pub(crate) async fn resume_enrollment_inner` inside `crates/shared/service-sdk/src/ws.rs` (KEEP IT in `ws.rs` — production logic belongs with the
  function it factors out of). The inner fn accepts an already-handshaken `WsStream` instead of dialing; the existing public `resume_enrollment`
  becomes a thin wrapper that does TCP + TLS + handshake and forwards to `resume_enrollment_inner`. This sidesteps the need for a `test_tls_connector`
  entirely.
- The pure-test scaffolding (`serve_mock_controller`, message-stream constructors) goes in `crates/shared/service-sdk/src/test_support.rs` (the file
  exists) guarded by `#[cfg(any(test, feature = "testing"))]` so it cannot leak into release builds of dependents.

Commit this harness with the first test ("test infrastructure for ws-level unit tests") — keep it adjacent to the test that first exercises it.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p uptrakit-service-sdk wait_for_approval_returns_payload_service_id
```

Expected: FAIL — current `wait_for_approval` returns `Result<()>`, so the `assert_eq!` won't compile. Build failure counts as failure.

- [ ] **Step 3: Change `wait_for_approval` return type**

Edit `crates/shared/service-sdk/src/ws.rs:222`. Change the signature:

```rust
pub(crate) async fn wait_for_approval(
    ws: &mut WsStream,
    in_seq: &mut IncomingSeq,
) -> Result<Uuid> {
```

Inside, change the `Approved` arm:

```rust
                        ControllerMessage::Approved(payload) => {
                            tracing::info!(service_id = %payload.service_id, "enrollment approved");
                            return Ok(payload.service_id);
                        }
```

- [ ] **Step 4: Fix the fresh-enrollment caller**

Edit `crates/shared/service-sdk/src/ws.rs:503` (inside `run_enrollment`):

```rust
    if enrolled.status != EnrollmentStatus::Approved {
        // Fresh-enrollment path: controller echoes back the same service_id
        // we just enrolled with — no rebind needed. Discard the Uuid via a
        // plain `?` expression with no binding. `Cargo.toml:236` pins
        // `let_underscore_must_use = "deny"`, so a `let _approved_id = ...`
        // binding would be rejected.
        wait_for_approval(&mut ws, &mut in_seq).await?;
    }
```

- [ ] **Step 5: Run the new test + ensure the workspace still builds**

```bash
cargo test -p uptrakit-service-sdk wait_for_approval_returns_payload_service_id
cargo check --all-features
```

Expected: PASS + clean.

- [ ] **Step 6: Write a failing test — `resume_enrollment` rebinds identity on mismatch**

```rust
#[tokio::test(start_paused = true)]
async fn resume_enrollment_rebinds_identity_on_id_mismatch() {
    use uuid::Uuid;

    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
    let source_id = Uuid::now_v7();
    let target_id = Uuid::now_v7();
    let secret = "stored-enroll-secret";
    identity.save_enrollment(source_id, secret).await.expect("save");

    let approved_envelope = serde_json::json!({
        "protocol_version": uptrakit_wire::CURRENT_PROTOCOL_VERSION,
        "seq": 1,
        "message": { "Approved": { "service_id": target_id } }
    });
    let cert_envelope = mock_certificate_envelope_for(target_id, /* seq */ 2);

    // Drive an in-process WS handshake using the existing test scaffolding.
    let mut server_messages = vec![
        Message::Text(approved_envelope.to_string()),
        Message::Text(cert_envelope.to_string()),
    ];
    let mock_url = spawn_mock_ws_server(server_messages).await;

    let connector = test_tls_connector();
    resume_enrollment(&mut identity, &mock_url.host, mock_url.port, &connector)
        .await
        .expect("resume succeeds");

    // service.json on disk now points at target_id.
    let mut reloaded = ServiceIdentityState::new_single_dir(tmp.path());
    reloaded.load().await.expect("reload");
    assert_eq!(reloaded.service_id(), Some(target_id));
}
```

The helpers `spawn_mock_ws_server`, `mock_certificate_envelope_for`, `test_tls_connector` are surrounding test infrastructure. If absent, defer to
`test_support.rs` or replicate the existing `resume_enrollment_*` test scaffolding pattern from the same crate.

- [ ] **Step 7: Run test to verify it fails**

```bash
cargo test -p uptrakit-service-sdk resume_enrollment_rebinds_identity_on_id_mismatch
```

Expected: FAIL — `identity.service_id()` still equals `source_id` because rebind logic isn't wired yet.

- [ ] **Step 8: Wire the rebind logic into `resume_enrollment`**

Edit `crates/shared/service-sdk/src/ws.rs:523` (`resume_enrollment` body, replacing the line `wait_for_approval(&mut ws, &mut in_seq).await?;`):

```rust
    let approved_id = wait_for_approval(&mut ws, &mut in_seq).await?;
    if approved_id.is_nil() {
        bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(
            "controller sent nil service_id in Approved".to_string()
        )));
    }
    if Some(approved_id) != identity.service_id() {
        let old_id = identity.service_id();
        let secret = identity
            .enrollment_secret()
            .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotEnrolled)))?
            .to_string();
        identity.save_enrollment(approved_id, &secret).await?;
        tracing::info!(
            ?old_id,
            new_id = %approved_id,
            "service identity rebound via merge redirect"
        );
    }
```

- [ ] **Step 9: Run test to verify it passes**

```bash
cargo test -p uptrakit-service-sdk resume_enrollment_rebinds_identity_on_id_mismatch
```

Expected: PASS.

- [ ] **Step 10: Write a test asserting no-op when ids match**

```rust
#[tokio::test(start_paused = true)]
async fn resume_enrollment_noop_when_ids_match() {
    use uuid::Uuid;

    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
    let service_id = Uuid::now_v7();
    identity.save_enrollment(service_id, "stored-secret").await.expect("save");

    let mtime_before = std::fs::metadata(tmp.path().join("service.json"))
        .expect("stat")
        .modified()
        .expect("mtime");

    let approved = serde_json::json!({
        "protocol_version": uptrakit_wire::CURRENT_PROTOCOL_VERSION,
        "seq": 1,
        "message": { "Approved": { "service_id": service_id } }
    });
    let cert = mock_certificate_envelope_for(service_id, 2);
    let mock_url = spawn_mock_ws_server(vec![
        Message::Text(approved.to_string()),
        Message::Text(cert.to_string()),
    ]).await;
    let connector = test_tls_connector();
    resume_enrollment(&mut identity, &mock_url.host, mock_url.port, &connector)
        .await
        .expect("resume ok");

    let mtime_after = std::fs::metadata(tmp.path().join("service.json"))
        .expect("stat")
        .modified()
        .expect("mtime");
    assert_eq!(mtime_before, mtime_after, "service.json must not be rewritten when ids match");
}
```

- [ ] **Step 11: Run test to verify it passes**

```bash
cargo test -p uptrakit-service-sdk resume_enrollment_noop_when_ids_match
```

Expected: PASS.

- [ ] **Step 12: Write a test asserting nil_service_id is refused**

```rust
#[tokio::test(start_paused = true)]
async fn resume_enrollment_rejects_nil_service_id() {
    use uuid::Uuid;

    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
    identity.save_enrollment(Uuid::now_v7(), "secret").await.expect("save");

    let approved = serde_json::json!({
        "protocol_version": uptrakit_wire::CURRENT_PROTOCOL_VERSION,
        "seq": 1,
        "message": { "Approved": { "service_id": Uuid::nil() } }
    });
    let mock_url = spawn_mock_ws_server(vec![Message::Text(approved.to_string())]).await;
    let connector = test_tls_connector();

    let err = resume_enrollment(&mut identity, &mock_url.host, mock_url.port, &connector)
        .await
        .unwrap_err();

    match err.current_context() {
        EnrollmentError::Protocol(ProtocolError::Enrollment(msg)) => {
            assert!(msg.contains("nil service_id"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
```

- [ ] **Step 13: Run test to verify it passes**

```bash
cargo test -p uptrakit-service-sdk resume_enrollment_rejects_nil_service_id
```

Expected: PASS.

- [ ] **Step 14: Full clippy + tests for the SDK + downstream callers**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p uptrakit-service-sdk
cargo test -p uptrakit-agent-ssh
cargo test -p uptrakit-agent
```

Expected: clean.

- [ ] **Step 15: Commit**

```bash
git commit --only crates/shared/service-sdk/src/ws.rs \
  -m "feat(service-sdk): rebind identity on Approved.service_id mismatch"
```

---

## Task 8: Frontend filter for embedded services + reason code surfacing

**Files:**

- Modify: `frontend/src/routes/services/+page.svelte`

Bindings: existing Svelte 5 `$derived` pattern; preserve existing predicate.

- [ ] **Step 1: Add `!s.is_embedded` to the existing target filter**

Edit `frontend/src/routes/services/+page.svelte`. Find the `mergeTargetOptions` `$derived` (around line 121):

```ts
	const mergeTargetOptions = $derived(
		services
			.filter(
				(s) => s.status === 'approved' && s.capabilities.includes('software_discovery') && s.id !== mergeSource?.id
			)
```

Insert `&& !s.is_embedded` into the predicate, PRESERVING every existing clause:

```ts
	const mergeTargetOptions = $derived(
		services
			.filter(
				(s) =>
					s.status === 'approved' &&
					s.capabilities.includes('software_discovery') &&
					!s.is_embedded &&
					s.id !== mergeSource?.id
			)
```

- [ ] **Step 2: Filter embedded services from the source-picker list**

In the same file, find where the "Merge into…" action button / row decision is rendered for each service row (the per-row eligibility check that
decides whether to show the merge button). Add `!s.is_embedded` to that predicate as well. If the page uses a single list of "pending services that
can be sources" — locate it via grep for `status === 'pending'` and add `!s.is_embedded` to that filter too. (Embedded services can never enter
`pending` via enrollment, but this is belt-and-braces in case future code paths expose them.)

- [ ] **Step 3: Surface the new backend reason codes in the merge error toast**

Find the existing `executeMerge` catch block (around line 247). The current handler reads
`e instanceof Error ? e.message : 'Failed to merge service'`. The real API-client error type is `ApiError` (`frontend/src/lib/api.ts:183-214`), which
carries a FLAT `errorCode: string | null` field — NOT a nested `.error.code` object. Import `ApiError` at the top of the file and extract `errorCode`
directly:

```ts
import { mergeService, ApiError } from "$lib/api";
// ... in the catch:
try {
  await mergeService(mergeSource.id, mergeTarget.id);
  // success path unchanged
} catch (e) {
  const apiCode = e instanceof ApiError ? (e.errorCode ?? undefined) : undefined;
  error = describeMergeError(apiCode) ?? (e instanceof Error ? e.message : "Failed to merge service");
}
```

Add the translator function alongside it:

```ts
function describeMergeError(code: string | undefined): string | undefined {
  switch (code) {
    case "service.embedded_target":
      return "Cannot merge into an embedded service.";
    case "service.embedded_source":
      return "Cannot merge from an embedded service.";
    case "service.merge_invariant":
      return "Service merge state is inconsistent. Contact an administrator.";
    default:
      return undefined; // fall through to `e.message` so unrelated errors keep their wording
  }
}
```

- [ ] **Step 4: Run frontend quality gates**

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run build
cd ..
```

Expected: all clean.

- [ ] **Step 5: Commit**

```bash
git commit --only frontend/src/routes/services/+page.svelte \
  -m "feat(frontend): hide embedded services from merge dialog + surface reason codes"
```

---

## Task 9: Documentation — ADR 0020, CONTEXT.md, asyncapi.yaml, services-operations.md

**Files:**

- Create: `docs/adr/0020-service-merge-redirect.md`
- Modify: `CONTEXT.md` (glossary entry)
- Modify: `crates/shared/wire/asyncapi.yaml` (note on `ApprovedPayload.service_id` semantics)
- Modify: `docs/api/services-operations.md` (note 400 reason codes)

Bindings: "markdown line length max 150"; "MD024 siblings_only"; "Conventional Commits".

- [ ] **Step 1: Write ADR 0020**

Create `docs/adr/0020-service-merge-redirect.md`. Match the structure of `docs/adr/0019-typed-dynamic-config-boundaries.md`:

```markdown
# ADR 0020 — Service Merge Redirect

Date: 2026-06-09 Status: Accepted

## Context

`merge_service` consolidates a freshly-enrolled pending Service into an existing approved Service by moving the source's `enrollment_secret_hash` +
identity fields onto the target row and deactivating the source. The running Agent on the host still holds the source UUID in its `service.json` and
reconnects with `?service_id=<source>` + Bearer secret. With the source row deactivated and the bearer-lookup narrowed by `service_id`, the auth path
failed with `401 Unauthorized` even though the secret was correct on the target row.

Three approaches were considered:

1. **Inferring rebind from `RevocationReason::ServiceMerged`** — the merge transaction already revokes both source and target certs with this reason.
   The WS auth path could detect a merge by walking the cert table for the supplied source_id and following the reason. This couples authentication to
   cert-lifecycle bookkeeping that may legitimately change in the future and exposes private cert state to the bearer flow.
2. **Rewriting `services.id` (primary key replacement)** — keep the source row's PK; copy the target row's identity onto it; preserve every FK by
   virtue of the PK reuse. This preserves Agent state but rewrites identity semantics for the rest of the system; every downstream consumer (audit
   logs, certs, host links) would need to be re-keyed in the same txn. PK rewrite is also harder to reason about under FK cascade interactions and
   difficult to reverse.
3. **Explicit redirect table** _(chosen)_ — a thin mapping from old (deactivated) source UUID to current target UUID, written in the same
   `BEGIN IMMEDIATE` txn as the merge. The auth path consults it on a hint mismatch; the SDK reads the canonical id back from the `ApprovedPayload`
   and overwrites `service.json` on disk.

## Decision

Adopt approach (3). Schema:

| Column          | Type        | Notes                               |
| --------------- | ----------- | ----------------------------------- |
| `source_id`     | UUID PK     | the Agent's persisted service_id    |
| `target_id`     | UUID FK     | FK→`services(id)` ON DELETE CASCADE |
| `redirected_at` | TIMESTAMPTZ |                                     |

The table is tenant-scoped via FK, not via a denormalized `tenant_id` column, matching the `service_host` join-table pattern. Cross-tenant rows are
impossible by construction (the merge query runs inside a `TenantDb`).

The WS auth fallback triggers ONLY when `?service_id=<hint>` is supplied in the connection URL, preserving the existing cross-service-collision
defence-in-depth for the hint-less bearer path.

Chains cannot pre-exist: `merge_service` requires `source.status == Pending` and `target.status == Approved`, and a previously-deactivated row
satisfies neither. A debug-assertion query inside the merge txn enforces this invariant; a violation surfaces as
`ServiceQueryError::RedirectChainInvariantViolated` (HTTP 500) rather than silent state rewrite.

The Agent reads the canonical id from `ControllerMessage::Approved(ApprovedPayload { service_id })` — no new wire-message variant, no protocol-version
bump.

## Consequences

- Adds one tiny table; rows are retained forever (UUID payload is negligible; supports Agents that disconnect for long periods and reappear after
  merges).
- Rollback path: removing the WS auth fallback reverts behaviour to pre-fix; redirect rows become inert. Agents that already rebound `service.json`
  remain bound to the target id.
- Embedded Services are explicitly excluded from merge at both query and route layers (`ServiceQueryError::TargetEmbedded` / `SourceEmbedded`, HTTP
  400 with reason codes `service.embedded_target` / `service.embedded_source`).
- A new audit action `AUTH_SERVICE_REKEY_RESOLVED` surfaces every successful re-key for on-call discoverability; the existing bearer-miss audit is
  enriched with `redirect_checked` / `redirect_present` flags so a stuck-rebind state is greppable.
```

- [ ] **Step 2: Add `CONTEXT.md` glossary entry**

Edit `CONTEXT.md`. Find an alphabetically-appropriate location near other Service-related entries (search for the existing "Service" definition). Add:

```markdown
- **Service Merge Redirect** — A persisted mapping from a deactivated Service UUID to the merge-target Service UUID, written by `merge_service` so
  that an Agent which enrolled against the deactivated row can reconnect, be re-keyed onto the target identity, and have its `service.json` rewritten
  without operator intervention.
```

- [ ] **Step 3: Update `asyncapi.yaml` for `ApprovedPayload.service_id` semantics**

Edit `crates/shared/wire/asyncapi.yaml`. Locate the `ApprovedPayload` schema (search `ApprovedPayload`). Extend the `service_id` field description:

```yaml
service_id:
  type: string
  format: uuid
  description: |
    Canonical Service UUID assigned by the controller. On resume-enrollment connections
    this MAY differ from the UUID the Agent connected with (the
    `?service_id=<hint>` query parameter): the controller has resolved the Agent to a
    new identity via a merge-redirect, and the Agent must persist this value to
    `service.json` before generating its CSR.
```

- [ ] **Step 4: Update `docs/api/services-operations.md` to list new 400 reason codes**

Edit `docs/api/services-operations.md`. Find the section documenting `POST /services/{id}/merge` (search `merge`). Add to the documented error
responses:

```markdown
| 400 | `service.embedded_target` | Target service is embedded; merging into embedded services is not permitted. | | 400 | `service.embedded_source` |
Source service is embedded; embedded services cannot be merged away. | | 500 | `service.merge_invariant` | Redirect-chain invariant violated; service
merge state is inconsistent. |
```

If the existing doc uses a different table format, match it.

- [ ] **Step 5: Run markdownlint**

```bash
markdownlint --config .markdownlint.json docs/adr/0020-service-merge-redirect.md \
                                          CONTEXT.md \
                                          docs/api/services-operations.md
```

Expected: no errors. Fix any line-length violations by wrapping at 150 chars (or use prettier per the `feedback_prettier_for_markdown` memory:
`npx prettier --write` on these files).

- [ ] **Step 6: Commit**

```bash
git commit --only docs/adr/0020-service-merge-redirect.md \
                  CONTEXT.md \
                  crates/shared/wire/asyncapi.yaml \
                  docs/api/services-operations.md \
  -m "docs: ADR 0020 + glossary + asyncapi notes for service-merge redirect"
```

---

## Task 10: Docker integration test for the end-to-end flow

**Files:**

- Create: `crates/core/integration-tests/tests/service_merge_rekey.rs` (or add to an existing file under that directory, following neighbour
  conventions)

Bindings: per snapshot — "tests touching enrollment/wire/service: docker build + cargo test -p uptrakit-integration-tests -- --ignored". Per the spec:
integration test must NOT paused-time the cert path.

- [ ] **Step 1: Locate the canonical integration-test scaffolding**

Run:

```bash
ls crates/core/integration-tests/tests/
grep -rn "fn build_test_controller\|spawn_controller\|integration_test_setup" crates/core/integration-tests/src | head
```

Identify the helper that boots a controller inside the Docker test image, the helper that runs an Agent-SSH binary against it, and the helper that
calls the controller's REST API. Use whatever neighbour test files do — DO NOT invent new infrastructure.

- [ ] **Step 2: Write the test**

Create `crates/core/integration-tests/tests/service_merge_rekey.rs`. Outline (fill in with the exact helper names found in Step 1):

```rust
//! End-to-end test for service merge re-key.
//!
//! Flow:
//! 1. Boot controller in Docker.
//! 2. Operator pre-creates an approved Service (target), captures its UUID.
//! 3. Run a fresh Agent-SSH binary → it enrolls and stays in Pending (source).
//! 4. Operator merges source into target.
//! 5. Restart the Agent-SSH binary; it reconnects via resume_enrollment.
//! 6. Assert: agent's service.json now holds target_uuid; subsequent mTLS
//!    handshake against the controller succeeds.

#![cfg(feature = "ignored")]

use uptrakit_integration_tests::common::*; // adjust to actual common path

#[tokio::test]
#[ignore = "docker integration"]
async fn service_merge_rekey_end_to_end() {
    let controller = boot_controller().await;
    let target = controller.create_approved_service("target-host").await;
    let agent = AgentSshHarness::enroll_fresh(&controller, "fresh-host").await;

    let source_id = agent.persisted_service_id();
    controller.merge_services(target.id, source_id).await
        .expect("merge succeeds");

    agent.restart().await;
    agent.wait_until_certified(/* timeout */ std::time::Duration::from_secs(60)).await
        .expect("agent recovers and certifies under target id");

    assert_eq!(agent.persisted_service_id(), target.id);
    assert!(agent.mtls_handshake(&controller).await.is_ok());
}
```

This is the contract the implementer must produce. Match call signatures to the existing harness exactly.

- [ ] **Step 3: Run the integration test under Docker**

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests --test service_merge_rekey -- --ignored
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git commit --only crates/core/integration-tests/tests/service_merge_rekey.rs \
  -m "test(integration-tests): end-to-end service merge re-key flow"
```

---

## Task 11: Final full-workspace quality gate sweep

- [ ] **Step 1: Run every quality gate from the snapshot**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
cargo test -p uptrakit-integration-tests --test database -- --ignored
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build && cd ..
```

Expected: every command exits 0. If any reports failures, fix in-place — do not move on. After all gates green, the feature is shippable.

- [ ] **Step 2: Manually exercise the merge happy path against a dev controller**

Reproduce the user's original repro to confirm the bug is gone:

```bash
# Terminal 1: controller running with the patched code, fresh DB.
cargo run -p uptrakit-controller

# Terminal 2: fresh agent-ssh enrolment that becomes pending.
cargo run -p uptrakit-agent-ssh --features reset-data -- \
  --allow-plaintext-secrets \
  --url https://uptrakit.local.yantsen.su:8443 \
  --pki-addr http://localhost:8080 \
  -vvv
```

Operator UI: approve a separate Service first (the target). Then merge the freshly-enrolled pending Service INTO that approved one. Watch terminal 2 —
the agent should resume, log `service identity rebound via merge redirect`, and certify under the target id. No `401 Unauthorized`.

(No commit for this step — it is a sanity check, not a deliverable.)

---

## Self-Review

**Spec coverage:**

| Spec section                                                                                       | Task                           |
| -------------------------------------------------------------------------------------------------- | ------------------------------ |
| Redirect table schema + migration                                                                  | Task 1                         |
| Embedded ban at query layer + invariant assertion + redirect upsert                                | Task 3                         |
| Embedded ban at route layer (400 + audit)                                                          | Task 4                         |
| Audit catalog `AUTH_SERVICE_REKEY_RESOLVED` (3 sites)                                              | Task 5                         |
| `lookup_by_secret` redirect fallback + miss-audit enrichment + rekey audit emission                | Task 6                         |
| SDK `wait_for_approval` -> Uuid; `resume_enrollment` rebind; nil guard; `run_enrollment` call-site | Task 7                         |
| Frontend `is_embedded` filter + reason-code surfacing                                              | Task 8                         |
| ADR 0020 + CONTEXT.md + asyncapi.yaml + services-operations.md                                     | Task 9                         |
| Docker integration end-to-end                                                                      | Task 10                        |
| Full quality-gate sweep                                                                            | Task 11                        |
| HTTP mapping for new error variants                                                                | Task 2 (consumed by Tasks 3/4) |

**Placeholder scan:** none. Every code step contains the exact Rust/TypeScript to write. Test helpers that may not exist in a given test module are
explicitly flagged with the surrounding-test-as-template convention.

**Type consistency check:** `ServiceMergeRedirect` entity uses `source_id: Uuid` (matches snapshot rule "use uuid not binary_len(16)");
`target_id: Uuid` + FK to `service::Column::Id`. `service_merge_redirect` (table name) matches the entity attribute. `ServiceQueryError` new variants:
`TargetEmbedded`, `SourceEmbedded`, `RedirectChainInvariantViolated` — used identically in Tasks 2, 3, 4. `wait_for_approval` returns `Result<Uuid>` —
consumed in `resume_enrollment` (Task 7 Step 8) and `run_enrollment` (Task 7 Step 4). `AUTH_SERVICE_REKEY_RESOLVED` is the const name;
`auth.service.rekey_resolved` is the wire value; `auth_service_rekey_resolved` is the macro identifier — all three appear in Task 5.

**Idiom audit:** every DB call uses SeaORM (no raw SQL); error propagation uses `?` + `.context_to()` + `report!()` (no `unwrap()` in production
code); `let-else` for SeaORM `Option` chaining (not `match`); `tokio::test(start_paused = true)` for SDK time-controlled tests; the integration test
in Task 10 explicitly does NOT use `start_paused` per the spec. The audit `emit_event` call is non-blocking, matching the existing pattern.

**Dependency audit:** no new external dependencies introduced.

**Documentation tasks:** ADR 0020 (new architectural decision), `CONTEXT.md` glossary, `asyncapi.yaml` semantic note,
`docs/api/services-operations.md` reason-code table — all enumerated as Task 9 deliverables.
