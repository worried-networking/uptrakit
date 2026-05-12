# Auto-Update Policy Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`)
> syntax for tracking.

**Goal:** Land the four refactors (R1–R4) from
`docs/superpowers/specs/2026-05-10-auto-update-policy-foundation-spec.md` so a future
`UpdatePolicy` feature can drop in without further touching the dispatch path.

**Architecture:** Each refactor is one focused commit on `main`. R1 (extend `ActorType` enum) must
land before R4 (typed `actor_type` in three params structs) because R4's call-site sweep depends on
R1's canonical variants. R2 (`find_hosts_with_any_tag` helper) and R3 (pluralize `categories` +
add `plugin_type_ids` on candidate queries) are independent and may interleave with R1/R4 in any
order, subject to the R1→R4 hard ordering.

**Tech Stack:** Rust workspace, SeaORM (SQLite + Postgres backends), Axum, tokio, parking_lot,
rootcause errors, in-memory SQLite for unit tests.

**Snapshot binding:** Every task references applicable rules from
`.superpowers/standards-snapshot.md` and `docs/development/coding-standards.md`. Quality gates per
`docs/development/quality-gates.md`. Commits use Conventional Commits per
`docs/development/commit-messages.md`.

---

## Phase 1 — R1: extend `ActorType` and consolidate ad-hoc actor strings

This phase produces one commit. Sequence is: (a) extend the enum, (b) add a typed mapping from
agent `service_app_name` to `ActorType`, (c) refactor the two production call sites that pass raw
`service_app_name` into `actor_type`, (d) update tests, (e) commit.

**Binding rules:**

- `coding-standards.md` §"Typed enums for internal write-path discriminators": `ActorType` is
  internal, not wire — no `#[non_exhaustive]`, no `Other(String)`. `as_str()` returns
  `&'static str`; convert with `.as_str().to_string()` when writing to SeaORM `Set()`.
- `rust-idioms.md`: "prefer typed enums or newtypes over raw String mode flags".
- `commit-messages.md`: Conventional Commits, `refactor(actor-type): …`.

### Task 1.1: Add `Service`, `SystemService`, `Mqtt`, `System` variants and `FromStr`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_types.rs:1-30`
- Test: same file (extend the existing test module or add one if absent)

- [ ] **Step 1: Write the failing test for new variants and round-trip**

Append to `update_types.rs` (or create a `#[cfg(test)] mod tests` block if not present):

```rust
#[cfg(test)]
mod actor_type_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn actor_type_as_str_matches_on_disk_strings() {
        assert_eq!(ActorType::User.as_str(), "user");
        assert_eq!(ActorType::ApiToken.as_str(), "api_token");
        assert_eq!(ActorType::Scheduler.as_str(), "scheduler");
        assert_eq!(ActorType::Service.as_str(), "service");
        assert_eq!(ActorType::SystemService.as_str(), "system_service");
        assert_eq!(ActorType::Mqtt.as_str(), "uptrakit-mqtt");
        assert_eq!(ActorType::System.as_str(), "system");
    }

    #[test]
    fn actor_type_from_str_round_trips_every_variant() {
        for variant in [
            ActorType::User,
            ActorType::ApiToken,
            ActorType::Scheduler,
            ActorType::Service,
            ActorType::SystemService,
            ActorType::Mqtt,
            ActorType::System,
        ] {
            let s = variant.as_str();
            let parsed = ActorType::from_str(s).expect("known variant must parse");
            assert_eq!(parsed, variant, "round-trip mismatch for {s:?}");
        }
    }

    #[test]
    fn actor_type_from_str_rejects_unknown() {
        assert!(matches!(
            ActorType::from_str("nope"),
            Err(ParseActorTypeError::Invalid)
        ));
        assert!(matches!(
            ActorType::from_str(""),
            Err(ParseActorTypeError::Invalid)
        ));
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail to compile**

Run: `cargo test -p uptrakit-web-api-queries actor_type --no-run`
Expected: compile error — `Service`, `SystemService`, `Mqtt`, `System`, `ParseActorTypeError`,
`FromStr` not in scope.

- [ ] **Step 3: Extend `ActorType` with the new variants**

Replace the existing `ActorType` definition and `impl` in `update_types.rs` with:

```rust
use std::str::FromStr;
use thiserror::Error;

/// Typed actor that initiated an update or batch operation.
///
/// Stored as a snake_case string in the database (`actor_type` column). Internal write-path
/// discriminator — not a wire type. Per `docs/development/coding-standards.md`
/// §"Typed enums for internal write-path discriminators", this enum does not carry
/// `#[non_exhaustive]` and does not need an `Other(String)` variant: the set of strings written
/// to `update_history.actor_type` and `update_batches.actor_type` is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorType {
    /// Triggered by a human operator via the REST API.
    User,
    /// Triggered by an API token via the REST API.
    ApiToken,
    /// Triggered by a scheduled task.
    Scheduler,
    /// Triggered by a Service (Agent, Agent-SSH) over the service WS transport,
    /// except MQTT which carries its own variant for backwards compatibility with
    /// the on-disk `"uptrakit-mqtt"` string.
    Service,
    /// Triggered by an internal system path that does not correspond to a single
    /// Service identity (e.g. unattended bootstrap).
    SystemService,
    /// Triggered by the MQTT Service. Canonical on-disk value is `"uptrakit-mqtt"`
    /// (legacy spelling preserved — see `coding-standards.md`).
    Mqtt,
    /// Triggered by an instance-wide system path (e.g. scheduler cleanup that writes
    /// to `update_history`). Distinct from `AuditActorType::System` which targets
    /// the audit-log family.
    System,
}

impl ActorType {
    /// Returns the canonical snake_case string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Scheduler => "scheduler",
            Self::Service => "service",
            Self::SystemService => "system_service",
            Self::Mqtt => "uptrakit-mqtt",
            Self::System => "system",
        }
    }
}

impl std::fmt::Display for ActorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by `ActorType::from_str` for unknown strings.
///
/// `ActorType` is an internal closed enum; an unrecognised string is treated as a caller
/// bug, not a forward-compat case.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseActorTypeError {
    #[error("invalid actor_type value")]
    Invalid,
}

impl FromStr for ActorType {
    type Err = ParseActorTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "api_token" => Ok(Self::ApiToken),
            "scheduler" => Ok(Self::Scheduler),
            "service" => Ok(Self::Service),
            "system_service" => Ok(Self::SystemService),
            "uptrakit-mqtt" => Ok(Self::Mqtt),
            "system" => Ok(Self::System),
            _ => Err(ParseActorTypeError::Invalid),
        }
    }
}
```

- [ ] **Step 4: Run tests to confirm they pass**

Run: `cargo test -p uptrakit-web-api-queries actor_type`
Expected: 3 passed.

### Task 1.2: Add `ActorType::from_service_app_name(...)` mapping

**Why:** The two production callers in `service_ws/handler/update_tracking.rs:122,267` currently
pass `actor_type: service_app_name` (a raw String from the Service registration). Known
`service_app_name` values are `"uptrakit-mqtt"`, `"uptrakit-agent-ssh"`, and the fallback
`"unknown"`. Mapping at the call site turns the open string into a typed `ActorType`.

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_types.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

Append inside `actor_type_tests`:

```rust
#[test]
fn from_service_app_name_maps_known_binaries() {
    assert_eq!(ActorType::from_service_app_name("uptrakit-mqtt"), ActorType::Mqtt);
}

#[test]
fn from_service_app_name_falls_back_to_service_for_unknown() {
    assert_eq!(ActorType::from_service_app_name("uptrakit-agent-ssh"), ActorType::Service);
    assert_eq!(ActorType::from_service_app_name("uptrakit-agent"), ActorType::Service);
    assert_eq!(ActorType::from_service_app_name("unknown"), ActorType::Service);
    assert_eq!(ActorType::from_service_app_name(""), ActorType::Service);
}
```

- [ ] **Step 2: Run tests to confirm they fail to compile**

Run: `cargo test -p uptrakit-web-api-queries actor_type --no-run`
Expected: `from_service_app_name` not found.

- [ ] **Step 3: Implement `from_service_app_name`**

Add inside `impl ActorType { ... }` in `update_types.rs`:

```rust
/// Map a Service binary's `service_app_name` to the typed actor.
///
/// `"uptrakit-mqtt"` maps to [`ActorType::Mqtt`] (backwards-compatible with the legacy on-disk
/// spelling). Every other value — including the registration fallback `"unknown"` and the
/// agent-ssh binary `"uptrakit-agent-ssh"` — maps to [`ActorType::Service`]. The granular Service
/// identity is carried separately in the row's `actor_id` (the Service UUID), so collapsing here
/// loses no information that wasn't already available via a JOIN to `service.service_app_name`.
pub fn from_service_app_name(service_app_name: &str) -> Self {
    match service_app_name {
        "uptrakit-mqtt" => Self::Mqtt,
        _ => Self::Service,
    }
}
```

- [ ] **Step 4: Run tests to confirm they pass**

Run: `cargo test -p uptrakit-web-api-queries actor_type`
Expected: 5 passed.

### Task 1.3: Refactor production callers of `actor_type: service_app_name`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs:122,267`

- [ ] **Step 1: Read the current call shapes**

Run: `sed -n '110,130p;260,275p' crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`

Confirm both sites currently pass `actor_type: service_app_name` (a `&str` from the Service's
registration). Capture the surrounding context — these calls feed `TriggerUpdateParams` (line ~117)
and `CreateBatchParams` (line ~264).

- [ ] **Step 2: Replace both call sites with the typed mapping**

At each site, replace `actor_type: service_app_name` with:

```rust
actor_type: ActorType::from_service_app_name(service_app_name).as_str(),
```

Add the import at the top of the file if not present:

```rust
use crate::queries::update_types::ActorType;
```

**Note:** This leaves `actor_type` typed as `&str` inside the params structs for now (R4 will
flip the params structs to `actor_type: ActorType`). The `.as_str()` call here will be dropped in
Task 4.4. This intermediate state is intentional: R1 is the variant inventory + mapping; R4 is
the type-change at the params struct boundary.

- [ ] **Step 3: Confirm the build passes**

Run: `cargo check -p uptrakit-web-api`
Expected: success.

### Task 1.4: Audit-log readers and `coding-standards.md` note

**Files:**

- Modify: `docs/development/coding-standards.md` (if R1's mapping affects on-disk semantics worth
  documenting)
- Inspect: every reader of raw `update_history.actor_type` or `update_batches.actor_type` strings
  to ensure the new variants render correctly in UI/CLI surfaces

- [ ] **Step 1: Inventory readers and production write paths**

Run two greps. First, readers across both `crates/ui` and `crates/core` (per spec §R1
inventory requirement):

```bash
grep -rn 'actor_type' crates/ui crates/core --include="*.rs" \
  | grep -v 'Set(\|: Set\|tests/\|#\[cfg(test)\]\|#\[test\]\|// '
```

Confirm no reader matches on specific literals in a way that the four new variants would break
(e.g. an `if actor_type == "user" else "system"` ladder that would silently route `"service"`
incorrectly). The audit-log family (`system_audit_log`, `tenant_audit_log`) uses
`AuditActorType` — out of scope. Note `crates/core/scheduler-runtime/src/executors/audit_log_cleanup.rs:162`
writes `"system"` to `system_audit_log.actor_type`, not `update_history.actor_type` —
confirm and exclude.

Second, direct `Set(...)` writes to the two in-scope columns:

```bash
grep -rn 'actor_type: Set\|actor_type: sea_orm::Set' crates --include="*.rs" \
  | grep -E 'update_history|update_batch' \
  | grep -v 'tests/\|#\[cfg(test)\]\|#\[test\]\|fn test_\|fn insert_'
```

Confirm every production hit goes through `CreateUpdateRecordParams` or `CreateBatchParams`. If
a stray direct `Set(...)` write surfaces outside the params structs, treat as a bug and either
route through the typed entry point or open a follow-up issue.

**Expected false positives:** test fixtures inside `#[cfg(test)] mod tests { ... }` blocks at
the file level (e.g. multiple sites in `update_history.rs` and `update_batches/dispatch.rs`)
have `actor_type: Set(...)` lines that the simple line-level grep cannot exclude. Manually
verify each hit is inside a test module by reading the surrounding context. Test fixtures may
keep raw string literals per the spec ("Test fixtures may keep string literals if the resulting
fixture is clearer").

- [ ] **Step 2: Add the legacy-spelling note to `coding-standards.md`**

Open `docs/development/coding-standards.md` and locate §"Typed enums for internal write-path
discriminators". Append a paragraph (each line ≤150 chars per `.markdownlint.json`):

```markdown
### Legacy on-disk spellings

`ActorType::Mqtt` returns `"uptrakit-mqtt"` (not `"mqtt"`) from `as_str()` for backwards
compatibility with rows written by the MQTT Service before the typed enum landed.

New code paths that classify Service-originated writes use
`ActorType::from_service_app_name(...)`, which collapses every non-MQTT Service binary
(including `"uptrakit-agent-ssh"` and the registration fallback `"unknown"`) to
`ActorType::Service` (`"service"`). The granular Service identity is recoverable via the row's
`actor_id` (the Service UUID) joined to `service.service_app_name`.
```

- [ ] **Step 3: Lint the docs**

Run: `markdownlint --config .markdownlint.json docs/development/coding-standards.md`
Expected: exit 0.

### Task 1.5: Verify R1 quality gates and commit

- [ ] **Step 1: Run the backend gate suite**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: all pass. No clippy `-- -D warnings` flag — workspace lints already enforce
`warnings = "deny"`.

- [ ] **Step 2: Commit R1**

```bash
git add -- \
  crates/ui/web-api-queries/src/queries/update_types.rs \
  crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs \
  docs/development/coding-standards.md
git commit -m "$(cat <<'EOF'
refactor(actor-type): consolidate ad-hoc actor strings into typed enum

Add `Service`, `SystemService`, `Mqtt`, `System` variants to `ActorType` so every
production-written `actor_type` value on `update_history` and `update_batches` has a canonical
typed origin. Closed `FromStr` (returns `Err` on unknown) replaces the implicit "raw string"
contract. `Mqtt::as_str() == "uptrakit-mqtt"` for backwards compatibility with rows written by
the MQTT Service before this refactor; document the legacy spelling in `coding-standards.md`.

`ActorType::from_service_app_name(...)` maps the Service-WS handler's open
`service_app_name` field to the typed enum: `"uptrakit-mqtt"` → `Mqtt`; everything else →
`Service`. Two production call sites in `service_ws/handler/update_tracking.rs` updated to
use the typed mapping.

Prepares for R4 (typed `actor_type` in `CreateUpdateRecordParams` / `TriggerUpdateParams` /
`CreateBatchParams`) — that refactor depends on this enum being closed and the production set
known.

Per `docs/development/coding-standards.md` §"Typed enums for internal write-path
discriminators": `ActorType` is internal and is not `#[non_exhaustive]`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3: Confirm working tree clean**

Run: `git status --short`
Expected: empty output.

---

## Phase 2 — R2: `find_hosts_with_any_tag` helper

This phase produces one commit. The function is callable from any tenant-scoped query context, is
N+1-safe in its own query shape, and returns `Vec<host::Model>` (matching the internal-query
convention of `host_tags.rs`).

**Binding rules:**

- `coding-standards.md` §"Tenant Isolation for Join Tables": `host_tag_assignment` has no
  `tenant_id`. Primary isolation comes from `TenantDb::find::<host::Entity>()`; the parent
  `host_tag.tenant_id` filter is added as belt-and-suspenders.
- `rust-idioms.md`: public Result-returning functions include a `# Errors` section.
- `testing.md`: in-memory SQLite + `Database::connect("sqlite::memory:")` + migrations. No tokio
  time APIs — no `start_paused`.

### Task 2.1: Add the helper function with closed empty-input contract

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/host_tags.rs:72` (insert after existing helpers,
  before `list_host_tags`)
- Test: same file (extend existing test module if present; otherwise add `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add a `#[cfg(test)] mod find_hosts_with_any_tag_tests` block at the end of `host_tags.rs`:

```rust
#[cfg(test)]
mod find_hosts_with_any_tag_tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{host, tenant};

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db).await.unwrap();
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set(name.to_string()),
            slug: Set(format!("slug-{id}")),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        id
    }

    async fn insert_host(db: &DatabaseConnection, tenant_id: Uuid, hostname: &str) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{id}")),
            hostname: Set(hostname.to_string()),
            friendly_name: Set(hostname.to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        id
    }

    async fn insert_tag(db: &DatabaseConnection, tenant_id: Uuid, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        host_tag::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            color: Set("#000000".to_string()),
            description: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        id
    }

    async fn assign_tag(db: &DatabaseConnection, host_id: Uuid, tag_id: Uuid) {
        host_tag_assignment::ActiveModel {
            host_tag_id: Set(tag_id),
            host_id: Set(host_id),
            assigned_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn empty_tag_ids_returns_empty() {
        let db = setup_db().await;
        let tenant_id = insert_tenant(&db, "t").await;
        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let result = find_hosts_with_any_tag(&tenant_db, &[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn single_tag_returns_assigned_hosts_only() {
        let db = setup_db().await;
        let tenant_id = insert_tenant(&db, "t").await;
        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        let h1 = insert_host(&db, tenant_id, "h1").await;
        let _h2 = insert_host(&db, tenant_id, "h2").await; // unassigned
        let tag = insert_tag(&db, tenant_id, "prod").await;
        assign_tag(&db, h1, tag).await;

        let result = find_hosts_with_any_tag(&tenant_db, &[tag]).await.unwrap();
        let ids: Vec<Uuid> = result.iter().map(|h| h.id).collect();
        assert_eq!(ids, vec![h1]);
    }

    #[tokio::test]
    async fn multi_tag_any_of_unions_hosts() {
        let db = setup_db().await;
        let tenant_id = insert_tenant(&db, "t").await;
        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        let h1 = insert_host(&db, tenant_id, "h1").await;
        let h2 = insert_host(&db, tenant_id, "h2").await;
        let _h3 = insert_host(&db, tenant_id, "h3").await; // no tags
        let t_a = insert_tag(&db, tenant_id, "a").await;
        let t_b = insert_tag(&db, tenant_id, "b").await;
        assign_tag(&db, h1, t_a).await;
        assign_tag(&db, h2, t_b).await;

        let result = find_hosts_with_any_tag(&tenant_db, &[t_a, t_b]).await.unwrap();
        let mut ids: Vec<Uuid> = result.iter().map(|h| h.id).collect();
        ids.sort();
        let mut expected = vec![h1, h2];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[tokio::test]
    async fn host_with_multiple_matching_tags_appears_once() {
        let db = setup_db().await;
        let tenant_id = insert_tenant(&db, "t").await;
        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        let h1 = insert_host(&db, tenant_id, "h1").await;
        let t_a = insert_tag(&db, tenant_id, "a").await;
        let t_b = insert_tag(&db, tenant_id, "b").await;
        assign_tag(&db, h1, t_a).await;
        assign_tag(&db, h1, t_b).await;

        let result = find_hosts_with_any_tag(&tenant_db, &[t_a, t_b]).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, h1);
    }

    #[tokio::test]
    async fn other_tenant_tag_excluded() {
        let db = setup_db().await;
        let tenant_a = insert_tenant(&db, "a").await;
        let tenant_b = insert_tenant(&db, "b").await;

        let host_a = insert_host(&db, tenant_a, "a-host").await;
        let tag_a = insert_tag(&db, tenant_a, "shared").await;
        assign_tag(&db, host_a, tag_a).await;

        let host_b = insert_host(&db, tenant_b, "b-host").await;
        let tag_b = insert_tag(&db, tenant_b, "shared").await;
        assign_tag(&db, host_b, tag_b).await;

        let tenant_db_b = TenantDb::new(db.clone(), tenant_b);
        // Pass tag_a (owned by tenant_a) while scoped to tenant_b — expect zero.
        let result = find_hosts_with_any_tag(&tenant_db_b, &[tag_a]).await.unwrap();
        assert!(result.is_empty(), "other-tenant tag must not match any host");
    }

    #[tokio::test]
    async fn deactivated_host_excluded() {
        let db = setup_db().await;
        let tenant_id = insert_tenant(&db, "t").await;
        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        let h1 = insert_host(&db, tenant_id, "h1").await;
        let tag = insert_tag(&db, tenant_id, "x").await;
        assign_tag(&db, h1, tag).await;

        // Soft-delete h1.
        let model = host::Entity::find_by_id(h1).one(&db).await.unwrap().unwrap();
        let mut active: host::ActiveModel = model.into();
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();

        let result = find_hosts_with_any_tag(&tenant_db, &[tag]).await.unwrap();
        assert!(result.is_empty(), "deactivated host must not appear");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test -p uptrakit-web-api-queries find_hosts_with_any_tag --no-run`
Expected: compile error — `find_hosts_with_any_tag` not in scope.

- [ ] **Step 3: Implement the helper**

Add to `host_tags.rs`, immediately above `list_host_tags`:

```rust
use sea_orm::{JoinType, RelationTrait};
use uptrakit_shared_db::entity::host;

/// Find every active Host that carries at least one of the given tags ("any-of" semantics).
///
/// Tenant isolation: this function relies on `TenantDb::find::<host::Entity>()` as the primary
/// filter (which injects `host.tenant_id = ?`). A secondary filter on
/// `host_tag.tenant_id = tenant_db.tenant_id()` is added as belt-and-suspenders, so a stray
/// `tag_id` from another tenant cannot match. Deactivated hosts (`host.deactivated_at IS NOT
/// NULL`) are excluded.
///
/// Empty `tag_ids` returns `Ok(vec![])` — never "all hosts in tenant". This is intentional:
/// the policy executor that will consume this helper must opt in to a specific tag set.
///
/// **N+1 advisory:** callers that intend to enumerate outdated items per host should consider
/// `find_outdated_hosts_for_item` when the item axis is known, to avoid running one candidate
/// query per host.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the underlying query fails. Tenant-isolation errors do not
/// surface — they manifest as empty results.
pub async fn find_hosts_with_any_tag(
    tenant_db: &TenantDb,
    tag_ids: &[Uuid],
) -> Result<Vec<host::Model>, sea_orm::DbErr> {
    if tag_ids.is_empty() {
        return Ok(vec![]);
    }

    tenant_db
        .find::<host::Entity>()
        .join(JoinType::InnerJoin, host::Relation::HostTagAssignment.def())
        .join(
            JoinType::InnerJoin,
            host_tag_assignment::Relation::HostTag.def(),
        )
        .filter(host_tag::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host_tag_assignment::Column::HostTagId.is_in(tag_ids.iter().copied()))
        .filter(host::Column::DeactivatedAt.is_null())
        .distinct()
        .all(tenant_db.db())
        .await
}
```

**Notes for the implementer:**

- `TenantDb::find` exists on `crate::tenant_db::TenantDb` (already imported in this file). The
  exact accessor for the inner UUID (`tenant_db.tenant_id()`) and DB handle (`tenant_db.db()`)
  should match what other functions in this file use — check `list_host_tags` for the canonical
  form and adjust if the names differ.
- `host::Entity` is added to the `use uptrakit_shared_db::entity::{...}` line at the top of the
  file (currently imports only `host_tag, host_tag_assignment`).
- `host::Relation::HostTagAssignment` is the existing `has_many` defined in `host.rs:36`. This is
  more idiomatic than `host_tag_assignment::Relation::Host.def().rev()` when starting from
  `host::Entity`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-web-api-queries find_hosts_with_any_tag`
Expected: 6 passed.

### Task 2.2: Verify R2 quality gates and commit

- [ ] **Step 1: Backend gate suite**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: all pass.

- [ ] **Step 2: Commit R2**

```bash
git add -- crates/ui/web-api-queries/src/queries/host_tags.rs
git commit -m "$(cat <<'EOF'
feat(host-tags): find_hosts_with_any_tag helper

Add `find_hosts_with_any_tag(tenant_db, tag_ids)` for finding every active Host in a tenant
that carries at least one of the supplied tags. Primary tenant isolation comes from
`TenantDb::find::<host::Entity>()`; a secondary `host_tag.tenant_id` filter is applied via the
JOIN as belt-and-suspenders so a stray other-tenant tag id cannot match.

Empty input returns `Ok(vec![])` — never "all hosts in tenant". Deactivated hosts are
excluded.

Prepares for the future `UpdatePolicy` executor's hot path. AllOf semantics are deferred to a
separate helper (`find_hosts_with_all_tags`) since the SeaORM shapes do not share structure.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 3 — R3: pluralize `categories` + add `plugin_type_ids` filter

This phase produces one commit. Both candidate-query helpers (`find_outdated_items_for_host`,
`find_outdated_hosts_for_item`) get two new optional filter axes, with a hard empty-slice contract
enforced by `debug_assert!` + `tracing::warn!` + early-return-empty.

**Binding rules:**

- `coding-standards.md` §"Typed enums for internal write-path discriminators".
- `rust-idioms.md`: prefer typed enums or newtypes over raw String mode flags.
- Existing pattern in this file: `exclude_item_ids: Option<&[Uuid]>` (borrowed slice). The new
  filters follow the same shape.

### Task 3.1: Replace `category_filter: Option<&str>` with `categories: Option<&[UpdateCategory]>`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_batches/candidates.rs:30-131` (helper +
  callers in tests) and `:135-225` (sibling helper)

- [ ] **Step 1: Add the failing test for `categories` plural shape**

The shared base fixture in `crates/ui/web-api-queries/src/queries/update_batches/mod.rs::tests::insert_base_fixture`
sets `update_category = "security"` on the fixture's `host_software_item` row. The test below
adds a second item with `update_category = "feature"`, then filters for `[Security, Bugfix]`
and asserts only the fixture (security) item is returned.

Append at the bottom of the existing `tests` module in `candidates.rs`:

```rust
#[tokio::test]
async fn find_outdated_items_filters_by_multiple_categories() {
    let db = setup_db().await;
    let f = insert_base_fixture(&db).await; // fixture item has update_category = "security"
    let now = OffsetDateTime::now_utc();

    // Add a second item with "feature" category.
    let item2_id = Uuid::now_v7();
    let pc2_id = Uuid::now_v7();
    software_item::ActiveModel {
        id: Set(item2_id),
        tenant_id: Set(f.tenant_id),
        name: Set("feat-app".to_string()),
        featured: Set(true),
        icon_url: Set(None),
        last_checked_at: Set(None),
        awaiting_restart_timeout: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    plugin_config::ActiveModel {
        id: Set(pc2_id),
        tenant_id: Set(f.tenant_id),
        name: Set("feat-plugin".to_string()),
        plugin_type: Set("releases_github".to_string()),
        config: Set(serde_json::json!({})),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    let hsi2_id = Uuid::now_v7();
    host_software_item::ActiveModel {
        id: Set(hsi2_id),
        host_id: Set(f.host_id),
        software_item_id: Set(item2_id),
        qualifier: Set(None),
        plugin_config_id: Set(Some(pc2_id)),
        package_identifier: Set(Some("feat-app".to_string())),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(Some("1.1.0".to_string())),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("feature".to_string()),
        deactivated_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(f.host_id),
        software_item_id: Set(item2_id),
        host_software_item_id: Set(hsi2_id),
        plugin_config_id: Set(Some(pc2_id)),
        plugin_type: Set("releases_github".to_string()),
        role: Set("execute_update".to_string()),
        ordinal: Set(0),
        package_identifier: Set("org/feat".to_string()),
        config: Set(None),
        execution_site: Set("auto".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();

    // Filter for security + bugfix — fixture (security) matches, item2 (feature) does not.
    let cats = [UpdateCategory::Security, UpdateCategory::Bugfix];
    let candidates = find_outdated_items_for_host(
        &db,
        f.tenant_id,
        f.host_id,
        Some(&cats),
        None, // plugin_type_ids
        None, // exclude_item_ids
    )
    .await
    .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].software_item_id, f.item_id);
}
```

**No `#[should_panic]` test for the empty-slice case.** `#[should_panic]` does not compose
reliably with `#[tokio::test]` — the runtime catches panics inside the async future via
`catch_unwind`, so the outer `#[should_panic]` wrapper can produce a false-pass. The
`debug_assert!` is its own contract assertion (panics in debug builds, no-ops in release); we
rely on the production `tracing::warn!` + early-return path as the testable behaviour, and
that is already exercised by every other test in the module (which all pass `None` or a
non-empty slice). If a future caller bug introduces `Some(&[])`, the debug-mode panic will
surface during local development and CI test runs as a test failure.

- [ ] **Step 2: Run to confirm tests fail to compile**

Run: `cargo test -p uptrakit-web-api-queries -- candidates --no-run`
Expected: compile error — `find_outdated_items_for_host` signature mismatch.

- [ ] **Step 3: Change the signature of `find_outdated_items_for_host`**

In `candidates.rs`, replace the existing signature and category-filter block (lines 30-54) with:

```rust
use uptrakit_shared_types::{PluginTypeId, UpdateCategory};

/// Find all outdated items for a host, optionally filtered by category and/or plugin source.
///
/// `categories`: `None` = no filter; `Some(&[..])` = items whose
/// `host_software_item.update_category` is in the slice. Passing `Some(&[])` is a caller bug —
/// `debug_assert!` panics in debug; production logs `tracing::warn!` and returns `Ok(vec![])`.
/// Validation at the HTTP boundary should reject empty lists before reaching this helper.
///
/// `plugin_type_ids`: `None` = no filter; `Some(&[..])` = items whose
/// `host_software_item_plugin.plugin_type` (role `execute_update`) matches. Same empty-slice
/// contract as `categories`.
///
/// # Errors
///
/// Returns a [`TriggerUpdateError`] if the host is missing/wrong tenant, or if a DB error occurs.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn find_outdated_items_for_host(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    categories: Option<&[UpdateCategory]>,
    plugin_type_ids: Option<&[PluginTypeId]>,
    exclude_item_ids: Option<&[Uuid]>,
) -> Result<Vec<BatchUpdateCandidate>> {
    if let Some(c) = categories {
        debug_assert!(
            !c.is_empty(),
            "categories: empty slice is a caller bug; pass None to disable filter"
        );
        if c.is_empty() {
            tracing::warn!("find_outdated_items_for_host called with empty categories slice");
            return Ok(vec![]);
        }
    }
    if let Some(p) = plugin_type_ids {
        debug_assert!(
            !p.is_empty(),
            "plugin_type_ids: empty slice is a caller bug; pass None to disable filter"
        );
        if p.is_empty() {
            tracing::warn!("find_outdated_items_for_host called with empty plugin_type_ids slice");
            return Ok(vec![]);
        }
    }

    // Verify host exists and belongs to tenant
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotFound))?;

    // Load all host_software_items for this host that have both versions set and differ
    let mut query = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::InstalledVersion.is_not_null())
        .filter(host_software_item::Column::LatestVersion.is_not_null());

    if let Some(cats) = categories {
        let strs: Vec<&str> = cats.iter().map(UpdateCategory::as_str).collect();
        query = query.filter(host_software_item::Column::UpdateCategory.is_in(strs));
    }

    let links = query.all(db).await.context_to()?;
    // ... rest of body unchanged through line 117 ...
```

Continue from line 56 onward unchanged, **but add a `plugin_type_ids` join filter when batch-
loading execute_update plugin assignments**. Replace the existing
`execute_plugin_item_ids` block (lines 75–85) with:

```rust
    // Batch-load execute_update plugin assignments for this host, optionally filtered by plugin
    // source.
    let mut plugin_query = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(link_ids))
        .filter(host_software_item_plugin::Column::Role.eq("execute_update"));

    if let Some(ptids) = plugin_type_ids {
        let strs: Vec<&str> = ptids.iter().map(PluginTypeId::as_str).collect();
        plugin_query = plugin_query.filter(host_software_item_plugin::Column::PluginType.is_in(strs));
    }

    let execute_plugin_item_ids: HashSet<Uuid> = plugin_query
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|p| p.software_item_id)
        .collect();
```

- [ ] **Step 4: Change the signature of `find_outdated_hosts_for_item`**

Apply the same shape to `find_outdated_hosts_for_item` (currently in `candidates.rs:135-225`).
Add the same two parameters in the same positions (after the existing `host_ids: Option<&[Uuid]>`).

Replace its signature and body filters analogously:

```rust
pub async fn find_outdated_hosts_for_item(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
    host_ids: Option<&[Uuid]>,
    categories: Option<&[UpdateCategory]>,
    plugin_type_ids: Option<&[PluginTypeId]>,
) -> Result<Vec<BatchUpdateCandidate>> {
    if let Some(c) = categories {
        debug_assert!(!c.is_empty(), "categories: empty slice is a caller bug; pass None to disable filter");
        if c.is_empty() {
            tracing::warn!("find_outdated_hosts_for_item called with empty categories slice");
            return Ok(vec![]);
        }
    }
    if let Some(p) = plugin_type_ids {
        debug_assert!(!p.is_empty(), "plugin_type_ids: empty slice is a caller bug; pass None to disable filter");
        if p.is_empty() {
            tracing::warn!("find_outdated_hosts_for_item called with empty plugin_type_ids slice");
            return Ok(vec![]);
        }
    }
    // ... rest of body with the same filter additions as in find_outdated_items_for_host
}
```

Add the `categories` filter on `host_software_item::Column::UpdateCategory` in the same query
build site. Add the `plugin_type_ids` filter on the `HostSoftwareItemPlugin::find()` query.

- [ ] **Step 5: Update existing tests in this file**

Five existing call sites in the test module use the old `Option<&str>` signature
(lines 256, 271, 361, 369, 382, 408, 423 per the current file). Update each:

```rust
// Before:
find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, None)
// After:
find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, None, None)

// Before:
find_outdated_items_for_host(&db, f.tenant_id, f.host_id, Some("security"), None)
// After:
find_outdated_items_for_host(
    &db,
    f.tenant_id,
    f.host_id,
    Some(&[UpdateCategory::Security]),
    None,
    None,
)
// (and similarly for "feature" → Some(&[UpdateCategory::Feature]))

// Before:
find_outdated_hosts_for_item(&db, f.tenant_id, f.item_id, None)
// After:
find_outdated_hosts_for_item(&db, f.tenant_id, f.item_id, None, None, None)
```

- [ ] **Step 6: Add `plugin_type_ids` positive/negative tests**

Append to the test module:

```rust
#[tokio::test]
async fn find_outdated_items_plugin_type_ids_matches() {
    let db = setup_db().await;
    let f = insert_base_fixture(&db).await;
    let ptids = [PluginTypeId::from_static("releases_github")];
    let candidates = find_outdated_items_for_host(
        &db,
        f.tenant_id,
        f.host_id,
        None,
        Some(&ptids),
        None,
    )
    .await
    .unwrap();
    assert_eq!(candidates.len(), 1);
}

#[tokio::test]
async fn find_outdated_items_plugin_type_ids_excludes_unmatched() {
    let db = setup_db().await;
    let f = insert_base_fixture(&db).await;
    let ptids = [PluginTypeId::from_static("releases_gitlab")];
    let candidates = find_outdated_items_for_host(
        &db,
        f.tenant_id,
        f.host_id,
        None,
        Some(&ptids),
        None,
    )
    .await
    .unwrap();
    assert!(candidates.is_empty());
}
```

- [ ] **Step 7: Update production call sites**

Three production layers reach `find_outdated_items_for_host` / `find_outdated_hosts_for_item`.
The boundary conversion from a single `Option<String>` category to
`Option<Vec<UpdateCategory>>` happens at the **route handler** (not in the action). Action
signatures change to accept the typed slice; callers thread it through.

**A. Route handler — `crates/ui/web-api/src/routes/update_batches.rs:140,158`**

The route reads `req.category_filter: Option<String>` and currently passes
`category_filter.as_deref()` (an `Option<&str>`) into the action. `HostBatchUpdateRequest.validate()`
guarantees the string parses as a known `UpdateCategory`. Convert to a typed one-element
`Vec<UpdateCategory>` here:

```rust
// Around line 140 (existing): let category_filter = req.category_filter.clone();
// Replace with:
let categories: Option<Vec<UpdateCategory>> = req
    .category_filter
    .as_deref()
    .map(|s| {
        vec![s.parse::<UpdateCategory>()
            .expect("Validate guarantees a known category")]
    });
let categories_slice = categories.as_deref();

// Around line 158: change the action call to pass categories_slice and None for plugin_type_ids.
// The action signature changes (see B).
```

Leave the existing audit-log key `"category_filter_present"` at lines 174 and 195 unchanged.
The key's value should now read `categories.is_some()` (a `bool`, same semantic as before). The
test assertion at line ~1158 keeps the same literal key — only the variable name changes in the
producing code:

```rust
"category_filter_present": categories.is_some(),
```

A future audit-schema migration can rename across all call sites uniformly; do not do it
piecemeal here.

**B. Action — `crates/ui/web-api/src/actions/update_batches.rs:40,43,290`**

Change the action signature from `category_filter: Option<&str>` to
`categories: Option<&[UpdateCategory]>`. Thread the slice through to the candidate query, plus
`None` for `plugin_type_ids` (the new axis is not yet exposed on the HTTP surface — that lands
with the future feature spec). The line-290 site (`find_outdated_hosts_for_item`) gains the same
two `None` arguments.

```rust
// Action fn signature (line ~40):
async fn trigger_host_batch(
    bctx: &BatchContext,
    host_id: Uuid,
    actor_type: ActorType,
    actor_id: &str,
    categories: Option<&[UpdateCategory]>,
    exclude_item_ids: Option<&[Uuid]>,
) -> ... {

// Inner call (line ~43):
let candidates = batch_queries::find_outdated_items_for_host(
    bctx.db,
    bctx.tenant_id,
    host_id,
    categories,
    None, // plugin_type_ids — exposed in a later HTTP surface
    exclude_item_ids,
)
.await?;
```

**B.2: `trigger_item_batch` line ~290.** This action does not accept `category_filter` from the
route (item rollouts target a single Software Item with a pinned `to_version`, not a category
filter). Its call to `find_outdated_hosts_for_item` gains two `None` arguments after the existing
`host_ids` arg. No signature change on `trigger_item_batch` itself:

```rust
let mut candidates = batch_queries::find_outdated_hosts_for_item(
    bctx.db,
    bctx.tenant_id,
    item_id,
    host_ids,
    None, // categories — not exposed on item-rollout surface
    None, // plugin_type_ids — exposed in a later HTTP surface
)
.await?;
```

**Audit-log key names** (`"category_filter_present"` at `routes/update_batches.rs:174,195` and
the matching test assertion at `~:1158`, plus `"category_filter_present"` in the service-WS
handler at lines 229/253/297): **do not rename**. The JSON key is part of an externally
visible audit-log shape; renaming half of the call sites and not the other introduces drift.
Leave both as-is for this refactor; a follow-up can rename consistently with a single audit
schema migration if desired.

**C. Service-WS handler — `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs:206-218`**

The current code derives `category_filter: Option<&str>` from `payload.security_only`. Convert
to a typed one-element Vec:

```rust
// Before:
let category_filter = if payload.security_only {
    Some("security")
} else {
    None
};
let outdated = match crate::queries::update_batches::find_outdated_items_for_host(
    state.db(),
    payload.tenant_id,
    payload.host_id,
    category_filter,
    None,
)
.await { ... }

// After:
let categories: Option<Vec<UpdateCategory>> = if payload.security_only {
    Some(vec![UpdateCategory::Security])
} else {
    None
};
let outdated = match crate::queries::update_batches::find_outdated_items_for_host(
    state.db(),
    payload.tenant_id,
    payload.host_id,
    categories.as_deref(),
    None, // plugin_type_ids
    None, // exclude_item_ids
)
.await { ... }
```

The `security_only=true` semantics must survive the migration — passing `None` here would be
a silent regression.

- [ ] **Step 8: Run tests to verify all pass**

Run: `cargo test -p uptrakit-web-api-queries candidates`
Run: `cargo test -p uptrakit-web-api`
Expected: all pass.

### Task 3.2: Verify R3 quality gates and commit

- [ ] **Step 1: Backend gate suite**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: all pass.

- [ ] **Step 2: Commit R3**

```bash
git add -- \
  crates/ui/web-api-queries/src/queries/update_batches/candidates.rs \
  crates/ui/web-api/src/actions/update_batches.rs \
  crates/ui/web-api/src/routes/update_batches.rs \
  crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs
git commit -m "$(cat <<'EOF'
refactor(candidate-queries): plural categories + plugin_type_ids filter

Replace `category_filter: Option<&str>` with `categories: Option<&[UpdateCategory]>` and add
`plugin_type_ids: Option<&[PluginTypeId]>` on `find_outdated_items_for_host` and
`find_outdated_hosts_for_item`. AND across axes. Empty `Some(&[])` on either filter is a
caller bug: `debug_assert!` panics in debug builds, production emits `tracing::warn!` and
returns `Ok(vec![])`. The HTTP `Validate` layer (deferred to feature spec) becomes the primary
defence once it lands.

Two production callers in `web-api/actions/update_batches.rs` and
`service_ws/handler/update_tracking.rs` updated to the new shape. The single-string parsing in
the HTTP path converts to a typed `Vec<UpdateCategory>` at the boundary.

Prepares for the future `UpdatePolicy` selector: per-policy AND-of (categories, plugin sources,
software items) over candidate discovery.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 4 — R4: typed `actor_type` in `CreateUpdateRecordParams` / `TriggerUpdateParams` / `CreateBatchParams`

This phase produces one commit. Requires R1 merged. Drops the `'a` lifetime on the three params
structs by making `actor_id: String` (owned) — same shape `to_version: String` already uses.

**Binding rules:**

- `coding-standards.md` §"Typed enums for internal write-path discriminators".
- `rust-idioms.md`: prefer typed enums or newtypes over raw String mode flags; avoid
  unnecessary lifetime constraints on params structs when owned `String` is already the norm.

### Task 4.1: Flip `CreateUpdateRecordParams` to typed actor

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:264-294` (struct) and the
  `Set(params.actor_type.to_string())` call (line ~981)

- [ ] **Step 1: Write the failing test**

The shared `pub(crate)` fixture is `crate::queries::update_batches::tests::insert_base_fixture`,
which returns a `Fixture` carrying `tenant_id`, `host_id`, `item_id`, and `service_id`. The
local `update_dispatch.rs::tests` module today uses its own private helpers
(`make_sqlite_db`, `insert_update_history_parents`) — switch this test to the shared fixture
via an explicit `use`. Add to the existing `tests` module in `update_dispatch.rs`:

```rust
#[tokio::test]
async fn create_update_record_accepts_typed_actor() {
    use crate::queries::update_batches::tests::{insert_base_fixture, setup_db};
    use crate::queries::update_types::ActorType;

    let db = setup_db().await;
    let f = insert_base_fixture(&db).await;
    let params = CreateUpdateRecordParams {
        tenant_id: f.tenant_id,
        host_id: f.host_id,
        item_id: f.item_id,
        host_software_item_id: None,
        to_version: "1.1.0",
        from_version: None,
        actor_type: ActorType::Mqtt,
        actor_id: f.service_id.to_string(),
        update_category: "feature",
        batch_id: None,
        initial_status: update_history::UpdateStatus::Pending,
        interactive: false,
    };
    let id = create_update_history_record(&db, &params).await.unwrap();
    let row = UpdateHistory::find_by_id(id).one(&db).await.unwrap().unwrap();
    assert_eq!(row.actor_type, "uptrakit-mqtt");
}
```

If `update_batches::tests` is not `pub(crate)`-visible from this module, mark its items
`pub(crate)` as a prerequisite (the `Fixture` struct, `setup_db`, and `insert_base_fixture` may
need the visibility bump). Verify by reading
`crates/ui/web-api-queries/src/queries/update_batches/mod.rs::tests` first.

- [ ] **Step 2: Run to confirm it fails to compile**

Run: `cargo test -p uptrakit-web-api-queries create_update_record_accepts_typed_actor --no-run`
Expected: compile error — `ActorType` not assignable to `&'a str`.

- [ ] **Step 3: Change the struct definition**

Replace lines 264–294 of `update_dispatch.rs` with:

```rust
/// Parameters for [`create_update_history_record`].
pub struct CreateUpdateRecordParams<'a> {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub item_id: Uuid,
    pub host_software_item_id: Option<Uuid>,
    pub to_version: &'a str,
    /// The currently installed version at the time the update was triggered.
    ///
    /// Populated from `host_software_items.installed_version` so the history
    /// record shows the "before" version even while the update is still
    /// pending or in progress.
    pub from_version: Option<String>,
    /// Who initiated the update (typed).
    pub actor_type: ActorType,
    /// Variant-dependent identifier (user UUID, API-token UUID, Service UUID, or empty string).
    pub actor_id: String,
    pub update_category: &'a str,
    /// Set when the update belongs to a batch.
    pub batch_id: Option<Uuid>,
    /// Initial status of the record. Non-batch callers always use
    /// [`update_history::UpdateStatus::Pending`]. Batch callers use
    /// [`update_history::UpdateStatus::Queued`] for non-first items on a
    /// host so that only one active record exists per host at a time.
    pub initial_status: update_history::UpdateStatus,
    /// Whether the update is dispatched in interactive mode (PTY allocated).
    pub interactive: bool,
}
```

And in `create_update_history_record` (search for `actor_type: Set(params.actor_type.to_string())`),
replace with:

```rust
actor_type: Set(params.actor_type.as_str().to_string()),
actor_id: Set(params.actor_id.clone()),
```

(Adjust surrounding `Set(...)` calls if the existing code passes `&str` for `actor_id` — change to
`params.actor_id.clone()`.)

Add to the imports at the top of `update_dispatch.rs`:

```rust
use crate::queries::update_types::ActorType;
```

- [ ] **Step 4: Run the focused test to confirm it passes**

Run: `cargo test -p uptrakit-web-api-queries create_update_record_accepts_typed_actor`
Expected: 1 passed.

### Task 4.2: Flip `TriggerUpdateParams` to typed actor

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_triggers.rs:58-73` (struct) and the
  construction of `CreateUpdateRecordParams` inside `trigger_update_for_host` (~line 138)

- [ ] **Step 1: Replace the struct definition**

```rust
/// Parameters for [`trigger_update_for_host`].
pub struct TriggerUpdateParams {
    pub tenant_id: Uuid,
    pub item_id: Uuid,
    pub host_id: Uuid,
    pub to_version: String,
    /// Who initiated the update (typed).
    pub actor_type: ActorType,
    /// Variant-dependent identifier (user UUID string, API token UUID string, Service UUID
    /// string, or empty string).
    pub actor_id: String,
    /// Optional release metadata supplied by the REST caller.
    /// `None` when triggered from a service or a scheduler.
    pub release_info: Option<ReleaseInfo>,
    /// When true, the agent allocates a PTY and keeps stdin open for forwarding.
    pub interactive: bool,
}
```

The struct drops the `'a` lifetime entirely. Update the function signature on
`trigger_update_for_host` to remove the lifetime — change `params: TriggerUpdateParams<'_>` to
`params: TriggerUpdateParams`.

Add the import at the top of `update_triggers.rs`:

```rust
use crate::queries::update_types::ActorType;
```

- [ ] **Step 2: Update the inner `CreateUpdateRecordParams` construction**

Inside `trigger_update_for_host`, the existing `build_record` closure (~line 132) passes
`actor_type: params.actor_type, actor_id: params.actor_id` — both `&str`. Change to:

```rust
let build_record = |initial_status| CreateUpdateRecordParams {
    tenant_id: params.tenant_id,
    host_id: params.host_id,
    item_id: params.item_id,
    host_software_item_id: Some(target.hsi_link.id),
    to_version: &params.to_version,
    from_version: target.hsi_link.installed_version.clone(),
    actor_type: params.actor_type,
    actor_id: params.actor_id.clone(),
    update_category: &target.hsi_link.update_category,
    batch_id: None,
    initial_status,
    interactive: resolved_interactive,
};
```

- [ ] **Step 3: Update test fixtures inside `update_triggers.rs::tests`**

Existing tests use `actor_type: ActorType::User.as_str()` and `actor_id: "user-1"`. Change every
one to:

```rust
actor_type: ActorType::User,
actor_id: "user-1".to_string(),
```

(Search: `cargo check -p uptrakit-web-api-queries 2>&1 | grep -E 'actor_type|actor_id'` to locate
all sites the compiler flags.)

- [ ] **Step 4: Confirm tests pass**

Run: `cargo test -p uptrakit-web-api-queries trigger_update`
Expected: all pass.

### Task 4.3: Flip `CreateBatchParams` to typed actor

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_batches/mod.rs:71-78` (struct) and line ~180
  (`Set(params.actor_type.to_string())` and `actor_type: params.actor_type` ripple at line ~215)

- [ ] **Step 1: Replace the struct definition**

```rust
/// Parameters for creating a batch update.
pub struct CreateBatchParams {
    pub tenant_id: Uuid,
    /// The batch category.
    pub batch_type: BatchType,
    /// Who initiated the batch (typed).
    pub actor_type: ActorType,
    pub actor_id: String,
}
```

Drop the `'a` lifetime from the struct and from `create_batch`'s `params: &CreateBatchParams<'_>`.

Add the import at the top:

```rust
use crate::queries::update_types::ActorType;
```

- [ ] **Step 2: Update the implementation**

Replace `actor_type: Set(params.actor_type.to_string())` (line ~180) with:

```rust
actor_type: Set(params.actor_type.as_str().to_string()),
actor_id: Set(params.actor_id.clone()),
```

The inner construction of `CreateUpdateRecordParams` (~line 215) that today reads
`actor_type: params.actor_type, actor_id: params.actor_id` becomes:

```rust
actor_type: params.actor_type,
actor_id: params.actor_id.clone(),
```

### Task 4.4: Sweep external callers of all three params structs

**Files (production):**

- `crates/ui/controller-core/src/update/controller.rs:103` — `TriggerUpdateParams { ... }`
- `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs:117,264` —
  `TriggerUpdateParams { ... }` and `CreateBatchParams { ... }`
- `crates/ui/web-api/src/actions/update_batches.rs:62,312` — `CreateBatchParams { ... }`

**Files (tests):**

- `crates/ui/web-api-queries/src/queries/update_triggers.rs::tests` — Already covered in Task 4.2
- Any other test fixtures the compiler flags

- [ ] **Step 1: Update `controller-core/src/update/controller.rs:103`**

Read the surrounding code first — `ControllerUpdateDispatcher::dispatch` translates `ActorInfo`
into the params shape. The `ActorInfo` struct already carries `actor_type: ActorType, actor_id:
String`. Replace any `.as_str()` call on `actor_type` with the typed enum directly, and pass
`actor_id` by clone:

```rust
TriggerUpdateParams {
    tenant_id: params.tenant_id,
    item_id: params.software_item_id,
    host_id: params.host_id,
    to_version: params.to_version,
    actor_type: params.actor.actor_type,
    actor_id: params.actor.actor_id.clone(),
    release_info: ..., // unchanged
    interactive: params.interactive,
}
```

- [ ] **Step 2: Update `service_ws/handler/update_tracking.rs:117 and 264`**

Both sites currently pass `actor_type: ActorType::from_service_app_name(service_app_name).as_str()`
(after Task 1.3). Drop the `.as_str()`:

```rust
// line 117 area:
TriggerUpdateParams {
    // ...
    actor_type: ActorType::from_service_app_name(service_app_name),
    actor_id: service_id.to_string(),
    // ...
}

// line 264 area:
let params = CreateBatchParams {
    tenant_id,
    batch_type: BatchType::HostUpdate,
    actor_type: ActorType::from_service_app_name(service_app_name),
    actor_id: service_id.to_string(),
};
```

(Verify the surrounding variable names: `service_id` may be named differently in scope — use
whatever the existing code passes.)

- [ ] **Step 3: Update `web-api/actions/update_batches.rs:62 and 312`**

Both sites construct `CreateBatchParams { actor_type: ActorType::User.as_str(), actor_id: ... }`.
Drop the `.as_str()`:

```rust
&batch_queries::CreateBatchParams {
    tenant_id,
    batch_type: BatchType::HostUpdate, // or ItemRollout, whichever the site uses
    actor_type: ActorType::User,
    actor_id: actor_id.to_string(), // already owned; ensure type matches
}
```

- [ ] **Step 4: Update MCP and any other callers the compiler flags**

Run: `cargo check --all-features 2>&1 | tail -50`

For every error: open the file, replace `ActorType::*.as_str()` with `ActorType::*` and
`&str` for `actor_id` with an owned `String` via `.to_string()` or `.to_owned()`.

Likely sites:

- `crates/ui/mcp/src/tools/update.rs` — already uses `ActorInfo::new(ActorType::ApiToken,
ctx.token_id.to_string())` per the earlier grep; verify it passes the typed actor through
  cleanly.

### Task 4.5: Verify R4 quality gates and commit

- [ ] **Step 1: Backend gate suite**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

Expected: all pass.

- [ ] **Step 2: Run remaining CI gates** (per `docs/development/quality-gates.md`)

```bash
python3 ci/check_plugin_semantic_boundary.py
bash ci/verify_no_security_audit.sh
bash ci/verify_typed_audit_actions.sh
bash ci/verify_handler_state_contract.sh
python3 ci/verify_db_access_policy.py
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: all pass.

- [ ] **Step 3: Commit R4**

```bash
git add -- \
  crates/ui/web-api-queries/src/queries/update_dispatch.rs \
  crates/ui/web-api-queries/src/queries/update_triggers.rs \
  crates/ui/web-api-queries/src/queries/update_batches/mod.rs \
  crates/ui/controller-core/src/update/controller.rs \
  crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs \
  crates/ui/web-api/src/actions/update_batches.rs \
  crates/ui/mcp/src/tools/update.rs
git commit -m "$(cat <<'EOF'
refactor(update-dispatch): typed actor_type in params structs

Change `CreateUpdateRecordParams.actor_type`, `TriggerUpdateParams.actor_type`, and
`CreateBatchParams.actor_type` from `&'a str` to `ActorType`. Make `actor_id: String` (owned)
so all three structs drop their `'a` lifetime entirely — matching the existing
`to_version: String` shape inside `TriggerUpdateParams`.

Every production call site (controller-core, service-WS handler, web-api batch actions, MCP)
passes the typed enum end-to-end. The `service_ws/handler/update_tracking.rs` site already used
`ActorType::from_service_app_name(...)` after R1; this commit drops the trailing `.as_str()`.

Final SeaORM write site converts with `actor_type.as_str().to_string()` per
`coding-standards.md` §"Typed enums for internal write-path discriminators".

Closes the per-host single-flight gate (no further refactor needed on the dispatch path for
the future `UpdatePolicy` executor).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Post-Plan Self-Review

**Spec coverage:**

- R1 → Phase 1 (Tasks 1.1–1.5). Inventory in 1.1 + 1.4. Closed enum + `FromStr` in 1.1. Mqtt
  legacy spelling preserved + doc note in 1.4. Tests in 1.1 + 1.2.
- R2 → Phase 2 (Tasks 2.1–2.2). `find_hosts_with_any_tag` only (no `TagMatchMode`). Tenant
  isolation is primary via `TenantDb::find`, with belt-and-suspenders filter on
  `host_tag.tenant_id`. Empty-input returns empty. N+1 advisory in doc comment. Tests cover
  empty, single, multi, dedup, other-tenant exclusion, deactivated-host exclusion.
- R3 → Phase 3 (Tasks 3.1–3.2). Plural `categories` and new `plugin_type_ids`. Empty `Some(&[])`
  is handled by `debug_assert!` (debug panic), `tracing::warn!` (production), and an early
  `Ok(vec![])` return. Tests cover positive and negative filtering; the empty-slice path is
  asserted by the `debug_assert!` contract itself rather than a fragile `#[should_panic]` +
  `#[tokio::test]` combination. Production callers swept at the route boundary
  (`routes/update_batches.rs:140`), action (`actions/update_batches.rs:40`), and the
  service-WS handler (`service_ws/handler/update_tracking.rs:206`).
- R4 → Phase 4 (Tasks 4.1–4.5). All three params structs flip `actor_type` to typed `ActorType`
  and `actor_id` to owned `String`. `TriggerUpdateParams` and `CreateBatchParams` drop their
  `'a` lifetime entirely (those were the only borrowed fields). `CreateUpdateRecordParams`
  retains `'a` for `to_version: &'a str` and `update_category: &'a str` — owning those is out
  of scope for R4. Production callers (controller-core, service-WS handler, web-api batch
  actions, MCP) swept.

**Placeholder scan:** No "TBD" / "implement later" / "similar to" — every code step shows the
exact code.

**Type consistency:** `ActorType::Mqtt.as_str() == "uptrakit-mqtt"` used consistently in 1.1,
1.4, and 4.4. `find_hosts_with_any_tag` signature consistent between Task 2.1 step 1 (test) and
step 3 (implementation). `plugin_type_ids` param order consistent in `find_outdated_items_for_host`
(3rd, 4th, 5th) and `find_outdated_hosts_for_item` (3rd, 4th, 5th, 6th: `host_ids`, then the new
two, then nothing).

**Idiom audit:**

- No task suggests silencing a lint or `#[allow(...)]` to make code compile.
- R3 `debug_assert!` is the idiomatic Rust contract-assertion shape (not a hand-rolled `if cfg!(debug_assertions)` block).
- R2 uses `host::Relation::HostTagAssignment.def()` (the canonical `has_many` from the host side)
  rather than `host_tag_assignment::Relation::Host.def().rev()` — matches `host.rs:36` directly.
- R3 imports `UpdateCategory` and `PluginTypeId` from `uptrakit_shared_types` — the typed-enum
  path the rest of the codebase already uses, not new wrappers.
- R4 conversions use `.as_str().to_string()` per the coding-standards rule, not `format!`.

No task asks the implementer to fight the framework or reinvent a primitive.

**Documentation deliverables:**

- `docs/development/coding-standards.md` — updated in Task 1.4 (legacy-spelling note).
- Public doc comments — `find_hosts_with_any_tag` (Task 2.1 step 3) has `# Errors`, body
  description, N+1 advisory, and tenant-isolation contract. `find_outdated_items_for_host` and
  `find_outdated_hosts_for_item` (Task 3.1 step 3) have updated `# Errors` and the empty-slice
  contract. `ActorType::from_service_app_name` (Task 1.2 step 3) has a body doc comment.
- No new ADR (spec §8: extends existing typed-enum pattern; no hard-to-reverse decision).
- No `CONTEXT.md` change (spec §8: no new domain terms).
- No OpenAPI / web-api-types change (spec §8: internal-only params structs; R3's HTTP boundary
  conversion handled at the call site, not the type).

All spec deliverables traced to plan tasks. No vague "polish" tasks.
