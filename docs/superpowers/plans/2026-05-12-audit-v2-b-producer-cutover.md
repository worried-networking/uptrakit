# Semantic Audit Logs V2 — Plan B: Producer Cutover

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the staged producer migration under the V1→V2 deprecation shim: `AuditView` impls on every Stateful entity, migration of
~100 producer call sites to the V2 emitter API, removal of `emit_best_effort`, `correlation_id` threading at workflow heads, and the
additive wire-payload extension with ingress validation that rejects forwarded Stateful action types. Plan A made this compilable via
a `#[deprecated]` `emit_best_effort` shim that wraps `emit_event`. Plan B's per-entity commits (Tasks 5–23) each pass CI on their own.
Task 26 (removal of the shim) is the only commit that is structurally a breaking change. Reviewers should expect a sequence of small
entity-scoped PRs (one or a few entities each) followed by a final removal PR — not one ~100-site rewrite PR.

**Architecture:** Plan B proceeds entity-by-entity for Stateful actions (each entity gets an `AuditView` derive and its producer sites
migrate to `emit_stateful`) and route-by-route for Event actions (producer sites translate `emit_best_effort` → `emit_event`). The final
task removes `emit_best_effort` from `AuditEmitter`, which is the breaking-CI moment; the preceding tasks ensure no caller still uses
it. Wire-payload extension adds `correlation_id: Option<Uuid>` to `AuditEventPayload`; ingress validation rejects payloads whose
`action_type` resolves to `AuditActionKind::Stateful`.

**Tech Stack:** Rust workspace, sea-orm transactions (`begin_with_options` with `SqliteTransactionMode::Immediate` on read-then-write
paths), Axum handlers, WS message handlers. Source of truth: spec §"Emission Model", §"Wire protocol", §"`AuditCommitHook`".

**Quality gates (run after each entity-sweep task and as the final task):** `cargo fmt --all`, `cargo check --no-default-features
--features db-sqlite`, `cargo check --all-features`, `cargo clippy --all-targets --no-default-features --features db-sqlite --
-D warnings`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`, `cargo deny check`,
`markdownlint --config .markdownlint.json '**/*.md'`.

---

## File structure

The bulk of edits is across `crates/ui/web-api/src/routes/**`, `crates/ui/web-api-queries/src/queries/**`,
`crates/core/controller-runtime/src/**`, `crates/core/scheduler-runtime/src/executors/**`,
`crates/core/agent-runtime/src/lib.rs`, `crates/core/agent-ssh-runtime/src/lib.rs`. Specific files per entity are inventoried during
each entity-sweep task.

| File                                                                                                                              | Status           | Responsibility                                                                      |
| --------------------------------------------------------------------------------------------------------------------------------- | ---------------- | ----------------------------------------------------------------------------------- |
| `crates/web-api-queries/src/queries/plugin_configs.rs` (and entity `Model`s under `crates/shared/db/src/entity/plugin_config.rs`) | modify           | Derive `AuditView`; convert producer to `emit_stateful`                             |
| `crates/web-api-queries/src/queries/notification_channels.rs`, `notification_rules.rs`                                            | modify           | Same pattern, plus three-layer email merge unaffected                               |
| `crates/web-api-queries/src/queries/host_tags.rs`, `hosts.rs`                                                                     | modify           | Same pattern                                                                        |
| `crates/web-api-queries/src/queries/services.rs`                                                                                  | modify           | Plus correlation_id threading on enrollment / merge / freeze                        |
| `crates/web-api-queries/src/queries/software_items.rs`, `software_item_assignments.rs`, `update_history.rs`                       | modify           | Same pattern; batch dispatch threads correlation_id                                 |
| `crates/web-api-queries/src/queries/users.rs`, `api_tokens.rs`, `enrollment_tokens.rs`, `oidc_providers.rs`                       | modify           | Same pattern                                                                        |
| `crates/web-api-queries/src/queries/global_settings.rs`, `tenant_settings.rs`, `scheduled_tasks.rs`                               | modify           | Same pattern                                                                        |
| `crates/web-api-queries/src/queries/software_ignore.rs`, `discovery_allowlist.rs`                                                 | modify           | Same pattern                                                                        |
| `crates/web-api-queries/src/queries/instance_plugins.rs`                                                                          | modify           | Same pattern                                                                        |
| `crates/web-api/src/routes/auth.rs`, `oidc_auth.rs`, `device_auth.rs`, `middleware/require_auth.rs`                               | modify           | Auth events stay Event-class; rename `emit_best_effort` → `emit_event`              |
| `crates/web-api/src/routes/service_ws/handler/**`                                                                                 | modify           | Event-class emissions through `emit_event`; service-forwarded path gains validation |
| `crates/core/controller-runtime/src/scheduler/**`                                                                                 | modify           | Scheduler executors thread `with_correlation(...)`; rename emit calls               |
| `crates/core/agent-runtime/src/lib.rs`, `crates/core/agent-ssh-runtime/src/lib.rs`                                                | modify           | Runtime Event emissions use `emit_event`                                            |
| `crates/shared/wire/src/lib.rs`                                                                                                   | modify           | Extend `AuditEventPayload` with `correlation_id: Option<Uuid>` (additive)           |
| `crates/web-api/src/routes/service_ws/handler/audit.rs` (controller-side ingress)                                                 | modify or create | Reject forwarded Stateful action types with a `tracing::warn!`                      |
| `crates/shared/audit-log/src/emitter.rs`                                                                                          | modify           | Remove `emit_best_effort` (final task)                                              |

---

## Task 1: Branch

- [ ] **Step 1:** `git checkout -b feat/audit-v2-producer-cutover` from the head of Plan A's `feat/audit-v2-foundation`.

---

## Task 2: Wire payload extension (additive)

**Files:**

- Modify: `crates/shared/wire/src/lib.rs`

The V1 spec defined `AuditEventPayload` as additive over the wire (services may forward audit events). V2 adds one optional field.

- [ ] **Step 1: Write failing test**

  Append to existing `wire` crate tests:

  ```rust
  #[test]
  fn audit_event_payload_round_trips_correlation_id() {
      let id = uuid::Uuid::now_v7();
      let p = AuditEventPayload {
          action_type: "software.update.finalized".parse().expect("registered"),
          tenant_id: None,
          target_type: None,
          target_id: None,
          target_display: None,
          outcome: "success".into(),
          details_json: None,
          request_id: None,
          correlation_id: Some(id),
      };
      let bytes = serde_json::to_vec(&p).expect("serialize");
      let back: AuditEventPayload = serde_json::from_slice(&bytes).expect("deserialize");
      assert_eq!(back.correlation_id, Some(id));
  }

  #[test]
  fn audit_event_payload_omits_correlation_id_for_v1_services() {
      // V1 services serialise the payload without the field; controller must accept it.
      let bytes = b"{\"action_type\":\"auth.login\",\"tenant_id\":null,\"target_type\":null,\"target_id\":null,\"target_display\":null,\"outcome\":\"success\",\"details_json\":null,\"request_id\":null}";
      let p: AuditEventPayload = serde_json::from_slice(bytes).expect("compat");
      assert_eq!(p.correlation_id, None);
  }
  ```

- [ ] **Step 2: Run test (expected fail — field missing)**

  Run: `cargo test -p uptrakit-wire audit_event_payload_round_trips_correlation_id`
  Expected: FAIL.

- [ ] **Step 3: Add the field**

  In `crates/shared/wire/src/lib.rs` `AuditEventPayload`:

  ```rust
  #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
  pub struct AuditEventPayload {
      // … existing fields …
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub correlation_id: Option<uuid::Uuid>,
  }
  ```

  `#[serde(default)]` ensures V1 payloads without the field deserialise. `skip_serializing_if` keeps the wire format compact when absent.

- [ ] **Step 4: Run test**

  Run: `cargo test -p uptrakit-wire`. Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/wire/src/lib.rs
  git commit -m "feat(audit-v2): AuditEventPayload gains optional correlation_id"
  ```

---

## Task 3: Wire-ingress rejection of forwarded Stateful action types

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/audit.rs` (or wherever V1 ingests `ServiceMessage::AuditEvent`)

- [ ] **Step 1: Locate the V1 ingress**

  Run: `grep -rn 'ServiceMessage::AuditEvent\|AuditEventPayload' crates/ui/web-api/src/routes/service_ws crates/core 2>/dev/null`
  Identify the handler that receives `AuditEventPayload` from connected services.

- [ ] **Step 2: Write failing integration test**

  Use the existing service-ws test harness. Send a payload with `action_type = "plugin_config.update"` (Stateful) and assert: no audit
  row is written; a `tracing::warn!` event is captured; the service connection remains open.

  The test must NOT combine `#[tokio::test(start_paused = true)]` with the SQLx-backed `audit_row_count().await` call —
  `docs/development/testing.md` exempts DB tests from paused time. Use real time and a deterministic completion signal
  (`harness.wait_for_inbound_drain().await`) instead of `tokio::time::advance`.

  ```rust
  #[tokio::test]
  async fn forwarded_stateful_action_is_rejected_with_warning() {
      let harness = ServiceWsTestHarness::start().await;
      let payload = AuditEventPayload {
          action_type: "plugin_config.update".parse().expect("registered"),
          tenant_id: Some(harness.tenant_id()),
          target_type: Some("plugin_config".into()),
          target_id: Some(uuid::Uuid::now_v7().to_string()),
          target_display: None,
          outcome: "success".into(),
          details_json: None,
          request_id: None,
          correlation_id: None,
      };
      let logs = harness.capture_tracing();
      harness.send_service_message(ServiceMessage::AuditEvent(payload)).await;
      // Wait for the controller to drain the inbound queue. The harness exposes a
      // semaphore the ingress handler releases after processing each message — no
      // wall-clock sleep, no paused-time `advance`.
      harness.wait_for_inbound_drain().await;

      assert_eq!(harness.audit_row_count().await, 0);
      assert!(logs.warnings_contain("forwarded Stateful action"));
      assert!(harness.service_connection_alive().await);
  }
  ```

  If the existing harness does not yet expose `wait_for_inbound_drain`, add it as part of this step — the harness's inbound
  ingress already increments a counter on each handled message; expose a `tokio::sync::Notify` that fires when the counter
  matches `sent_count`.

- [ ] **Step 3: Run test (expected fail)**

  Run: `cargo test -p uptrakit-web-api forwarded_stateful_action_is_rejected_with_warning -- --ignored`
  Expected: FAIL (currently accepted).

- [ ] **Step 4: Implement the ingress check**

  In the handler:

  ```rust
  async fn handle_audit_event(ctx: &ServiceWsCtx, payload: AuditEventPayload) {
      if payload.action_type.kind() == AuditActionKind::Stateful {
          tracing::warn!(
              service_id = %ctx.service_id(),
              action_type = %payload.action_type,
              "rejecting forwarded Stateful audit event; service-side stateful emission is not supported"
          );
          return;
      }
      // existing controller-side enrichment (actor attribution, tenant scope override) + emit_event …
  }
  ```

  The check uses `AuditActionType::kind()` (added in Plan A). The connection remains open by design.

- [ ] **Step 5: Run test**

  Run: same command. Expected: PASS.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/ui/web-api/src/
  git commit -m "feat(audit-v2): reject forwarded Stateful audit events at wire ingress"
  ```

---

## Task 4: Entity sweep procedure (read first)

This task is a **procedure**, not a code change. Subsequent tasks apply this procedure entity by entity.

For each Stateful entity that appears as a target type in the registry (full list in spec §"Initial catalog classification"):

1. **Derive `AuditView` on the SeaORM `Model`** in `crates/shared/db/src/entity/<entity>.rs`:

   ```rust
   #[derive(/* existing derives */, uptrakit_audit_log::AuditView)]
   #[audit(target_type = "<canonical_target_type>")]
   pub struct Model { /* fields */ }
   ```

   - Add `#[audit(skip)]` to internal fields (rowid surrogates, denormalised FK shadows, secrets stored as plain `String`).
   - Auto-skip handles `created_at`/`updated_at`/`deleted_at`/`deactivated_at`.
   - For entities with `EncryptedString` fields: the derive silently excludes them (no `Serialize` impl) — verify the secret-leak
     regression test in this entity's test module references the relevant field name and asserts the projection does not contain its
     plaintext or ciphertext.
   - For entities with a user-controlled `config_json: String` field (`plugin_config::Model`, `instance_plugin_setting::Model`):
     `#[audit(project_with = "mask_config_secrets_str")]` — applies the V1 secret masker. Import the helper in scope.

2. **Locate the producer site** in `crates/ui/web-api-queries/src/queries/<entity>.rs` (and any direct callers in
   `crates/ui/web-api/src/routes/<entity>.rs`).

3. **Rewrite the producer**:
   - Before: `audit_emitter.emit_best_effort(AuditEntry::builder(AuditActionType::<X>).target(...).actor_user(user_id, display).outcome(Success).build()?);`
   - After (Stateful):

     ```rust
     // Open or use the existing read-then-write transaction with BEGIN IMMEDIATE.
     let tx = db.begin_with_options(sea_orm::TransactionOptions {
         sqlite_transaction_mode: Some(sea_orm::SqliteTransactionMode::Immediate),
         ..Default::default()
     }).await?;
     let before = <Entity>::find_by_id(target_id).one(&tx).await?.ok_or(...)?;
     // … perform the mutation against &tx …
     let after = <Entity>::find_by_id(target_id).one(&tx).await?.ok_or(...)?;
     let hook = audit_emitter.commit_hook();
     audit_emitter.emit_stateful(
         &tx,
         &hook,
         AuditEntry::<entity>_update(&before, &after)
             .actor_user(user_id, user_display)
             .correlation_id_opt(req_correlation_id)
             .request_id_opt(request_id)
             .outcome(AuditOutcome::Success)
             .build()?,
     ).await?;
     tx.commit().await?;
     hook.flush_after_commit().await;
     ```

4. **Update or add a producer test** that asserts:
   - Exactly one audit row written after the mutation commits.
   - The row's `before_snapshot` and `after_snapshot` reflect the entity state.
   - Forcing a transaction rollback drops the audit row.
   - The row does not contain plaintext secrets (assert against any `EncryptedString` field name).

5. **Run** `cargo test -p uptrakit-web-api-queries <entity>::tests` and `cargo clippy --all-targets --all-features -- -D warnings`.

6. **Commit** with `feat(audit-v2): convert <entity> producer to emit_stateful`.

The same procedure applies to runtime mutations in `agent-runtime` and `agent-ssh-runtime` — see Task 13.

The procedure for **Event** actions is simpler:

- Replace `emit_best_effort(AuditEntry::builder(...).build()?)` with `emit_event(AuditEntry::<verb>().…build()?)` where `<verb>` is the
  macro-generated constructor (e.g. `AuditEntry::auth_login()`).
- Apply `correlation_id` from the surrounding context if the workflow has one.
- No transaction changes — Event remains async fire-and-forget.

---

## Tasks 5–18: Per-entity Stateful sweep

Apply the procedure from Task 4 to each entity below, one task per entity. Each task ends with a successful `cargo test
--all-features` and a Conventional Commit. The order is chosen to land smaller entities first so the derive-macro shape can be
validated before tackling the large entities.

| Task    | Entity                    | Notes                                                                                                                                                           |
| ------- | ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Task 5  | `host_tag`                | Plain CRUD; smallest blast radius for first conversion                                                                                                          |
| Task 6  | `software_ignore`         | Plain CRUD                                                                                                                                                      |
| Task 7  | `discovery_allowlist`     | Plain CRUD                                                                                                                                                      |
| Task 8  | `api_token`               | Plain CRUD; `token_hash` field is plain string — `#[audit(skip)]` it                                                                                            |
| Task 9  | `enrollment_token`        | Same as api_token                                                                                                                                               |
| Task 10 | `user`                    | Plain CRUD; password fields are `EncryptedString` (auto-excluded)                                                                                               |
| Task 11 | `oidc_provider`           | client_secret is `EncryptedString`                                                                                                                              |
| Task 12 | `global_setting`          | One row per key; snapshot covers the value JSON                                                                                                                 |
| Task 13 | `tenant_setting`          | Same shape as global_setting                                                                                                                                    |
| Task 14 | `scheduled_task`          | Mutations only; trigger remains Event                                                                                                                           |
| Task 15 | `notification_channel`    | Three-layer email config; SMTP defaults handled by V1 merger — snapshot captures the per-channel `config_json` (encrypted parts excluded via `EncryptedString`) |
| Task 16 | `notification_rule`       | Plain JSON body                                                                                                                                                 |
| Task 17 | `plugin_config`           | `config_json: String` — use `#[audit(project_with = "mask_config_secrets_str")]`                                                                                |
| Task 18 | `plugin_type_settings`    | Same as plugin_config — `mask_config_secrets_str` projector                                                                                                     |
| Task 19 | `instance_plugin_setting` | `config_json: Option<String>` — same projector                                                                                                                  |
| Task 20 | `service`                 | Lifecycle: approve/reject/deactivate/update mutate row state; `merge` stays Event                                                                               |
| Task 21 | `service_config`          | Stored config rows; deliver remains Event                                                                                                                       |
| Task 22 | `software_item`           | CRUD + assign/unassign — multiple Stateful actions on the same entity                                                                                           |
| Task 23 | `host`                    | Update/deactivate; discover remains Event                                                                                                                       |

Each task follows the procedure exactly; no per-task code listings are repeated here.

---

## Task 24: Event-action producer sweep

**Files:**

- Modify across `crates/ui/web-api/src/routes/auth.rs`, `oidc_auth.rs`, `device_auth.rs`, `middleware/require_auth.rs`,
  `crates/core/scheduler-runtime/src/executors/audit_log_cleanup.rs`, `crates/core/agent-runtime/src/lib.rs`,
  `crates/core/agent-ssh-runtime/src/lib.rs`, `crates/ui/web-api-queries/src/queries/services.rs` (merge, certificate, enrollment),
  `crates/ui/web-api-queries/src/queries/software_*.rs` (workflow-fact actions), `crates/ui/web-api/src/surface_proxy.rs`.

Event actions stay async fire-and-forget. The migration is mechanical:

- [ ] **Step 1: Inventory call sites**

  Run: `grep -rn 'emit_best_effort\|AuditEntry::builder' crates/ 2>/dev/null > /tmp/audit-callers.txt; wc -l /tmp/audit-callers.txt`
  Expect a list of approximately 70–90 lines after the Stateful sweep tasks above have already migrated their sites.

- [ ] **Step 2: Convert each remaining site to `emit_event` + named constructor**

  Example diff:

  ```rust
  // Before:
  audit_emitter.emit_best_effort(
      AuditEntry::builder(AuditActionType::AUTH_LOGIN)
          .actor(AuditActorType::User, Some(user_id))
          .actor_display_opt(Some(display))
          .outcome(AuditOutcome::Success)
          .request_id_opt(req_id)
          .build()?,
  );

  // After:
  audit_emitter.emit_event(
      AuditEntry::auth_login()
          .actor_user(user_id, display)
          .outcome(AuditOutcome::Success)
          .request_id_opt(req_id)
          .build()?,
  );
  ```

  Mechanical rules:
  - `AuditEntry::builder(AuditActionType::X)` → `AuditEntry::<x_snake_case>()`.
  - `emit_best_effort(...)` → `emit_event(...)`.
  - Preserve every existing builder method call — they all still exist on the new builder.
  - If the surrounding handler already has a `correlation_id` in context (e.g. from `with_correlation`), append
    `.correlation_id_opt(corr_id)`.

- [ ] **Step 3: Compile after each crate**

  Run: `cargo check -p <crate>` after editing each crate; expect green.

- [ ] **Step 4: Commit per crate**

  ```bash
  git commit -m "feat(audit-v2): convert <crate> Event producers to emit_event"
  ```

---

## Task 25: correlation_id threading at workflow heads

Workflow heads are call sites that initiate multi-step audited workflows. Each head mints `Uuid::now_v7()` and threads it through to
downstream emit sites via `AuditEmitter::with_correlation(id)`.

Sites (each is a small task):

- [ ] **Task 25.1 — Batch update dispatch**: `crates/ui/web-api/src/routes/software_updates.rs` (or wherever
      `software.batch_update.triggered` is emitted). Mint `correlation_id`; pass into the dispatched per-host trigger events;
      forward via wire payload to the service so `software.update.started/finalized` inherit it.
- [ ] **Task 25.2 — OIDC flow chain**: `crates/ui/web-api/src/routes/oidc_auth.rs`. Mint at `auth.oidc.authorize`; thread through
      `auth.oidc.callback`, `auth.oidc.exchange`, and the eventual `auth.login` if any.
- [ ] **Task 25.3 — Service enrollment**: `crates/core/controller-runtime/src/enrollment/*`. Mint at enrollment start; thread through
      certificate issuance, `service.enrollment.completed`, and the first `service.approve` emission.
- [ ] **Task 25.4 — Scheduler job**: `crates/core/controller-runtime/src/scheduler/mod.rs` and each executor's `run()`. Mint at
      executor entry; all audit events spawned by the job inherit via `audit_emitter.with_correlation(id)`.

Each sub-task adds a test that walks the chain and asserts every audit row shares the same `correlation_id` value.

Final commit:

```bash
git commit -m "feat(audit-v2): thread correlation_id through batch dispatch / OIDC / enrollment / scheduler"
```

---

## Task 26: Remove `emit_best_effort` and the deprecation shim

**Files:**

- Modify: `crates/shared/audit-log/src/emitter.rs`

By this point every call site has migrated. The deprecation shim from Plan A Task 14 Step 2 can be deleted.

- [ ] **Step 1: Confirm no remaining callers**

  Run: `grep -rn 'emit_best_effort' crates/ 2>/dev/null`
  Expected: empty (only the definition itself in `emitter.rs`).

- [ ] **Step 2: Remove the method**

  Delete:

  ```rust
  #[deprecated(...)]
  #[doc(hidden)]
  pub fn emit_best_effort(&self, entry: AuditEntry<Event>) { self.emit_event(entry); }
  ```

- [ ] **Step 3: Run full quality gates**

  ```bash
  cargo fmt --all
  cargo check --all-features
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  ```

  Expected: green.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/shared/audit-log/src/emitter.rs
  git commit -m "refactor(audit-v2)!: remove AuditEmitter::emit_best_effort (use emit_event/emit_stateful)"
  ```

  The `!` after the scope follows `docs/development/commit-messages.md`: breaking change.

---

## Task 27: Secret-leak regression test pass

**Files:** spread across producer test modules.

Add or extend a regression test in each Stateful entity's producer-test module that:

1. Constructs an entity with a known-secret value (the encrypted column's plaintext) and a known-non-secret value.
2. Emits a stateful audit row.
3. Reads back `before_snapshot` and `after_snapshot`.
4. Asserts neither the plaintext nor the raw `db_value` ciphertext is present anywhere in the serialised JSON.

Representative entities to cover: `plugin_config` (config_json with secret fields), `notification_channel` (SMTP config), `user` (
password), `oidc_provider` (client_secret), `api_token` (token_hash).

Each test follows the same shape and lives alongside the entity's existing tests.

Commit:

```bash
git commit -m "test(audit-v2): regression tests against secret leakage through audit snapshots"
```

---

## Task 28: Final quality gates + push

- [ ] **Step 1: Full quality gate suite**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  cargo deny check
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Plus the conditional gates from `CLAUDE.md` if enrollment/wire/service lifecycle changed:

  ```bash
  docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
  cargo test -p uptrakit-integration-tests -- --ignored
  ```

  Expected: green.

- [ ] **Step 2: Push branch**

  ```bash
  git push -u origin feat/audit-v2-producer-cutover
  ```

---

## Spec coverage check (Plan B scope)

This plan delivers:

- Spec §"Emission Model" — every Stateful producer migrated to `emit_stateful` inside a `BEGIN IMMEDIATE` transaction; every Event
  producer migrated to `emit_event` (Tasks 4–24).
- Spec §"`AuditCommitHook`" — caller-flush pattern applied at every Stateful producer (Tasks 4–23).
- Spec §"Wire protocol" — additive `correlation_id` field; ingress rejection of forwarded Stateful action types (Tasks 2, 3).
- Spec §"`correlation_id`" — threaded through batch dispatch, OIDC, enrollment, scheduler (Task 25).
- Spec §"Why service-supplied snapshots are rejected" — enforced at ingress (Task 3).
- `AuditEmitter::emit_best_effort` removal — coordinated breaking change (Task 26).
- Secret-leak regression coverage (Task 27).

Deferred to Plan C: catalog file + static-analysis CI gate.
Deferred to Plan D: frontend State tab + correlation_id filter, CLI rendering.
Deferred to Plan E: documentation deliverables + new ADR.
