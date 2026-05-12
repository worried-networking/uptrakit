# Semantic Audit Logs V2 — Plan E: Documentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land every documentation deliverable enumerated in the spec: the new ADR for transactional stateful emit, and rewrites of
the four `audit-logs.md` files (development / security / end-user / api), plus targeted updates to `AGENTS.md` and `ARCHITECTURE.md`.
`CONTEXT.md` stays unchanged — V2 introduces no new domain terms.

**Architecture:** Each documentation deliverable is its own task and its own commit, scoped so reviewers can read the diff
independently. Each file follows the project's existing markdown discipline (line length 150; prettier-managed; tables/code blocks
ignored by markdownlint's line-length rule). The new ADR follows the numbering and shape of the existing six ADRs.

**Tech Stack:** markdown; prettier for formatting; markdownlint per `.markdownlint.json`. Spec is the source of truth.

**Quality gate:** `markdownlint --config .markdownlint.json '**/*.md'`, plus a manual reading pass for technical accuracy and
spec-alignment.

---

## File structure

| File                                                 | Status                            | Responsibility                                                                                       |
| ---------------------------------------------------- | --------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `docs/adr/0007-audit-stateful-transactional-emit.md` | create                            | New ADR: synchronous in-tx stateful emit + Stateful/Event kind split                                 |
| `docs/development/audit-logs.md`                     | modify (rewrite producer section) | Two emit paths, AuditView, action kinds, catalog workflow, coverage tool, correlation_id             |
| `docs/security/audit-logs.md`                        | modify                            | V2 evidence-integrity guarantees; storage sizing; V1→V2 cutover note + optional pre-migration export |
| `docs/end-user/audit-logs.md`                        | modify                            | State tab usage; correlation_id filter + copy button; what's excluded (V3 deferred items)            |
| `docs/api/audit-logs.md`                             | modify                            | DTO additions (`action_kind`, `before_snapshot`, `after_snapshot`, `correlation_id`); new filters    |
| `AGENTS.md`                                          | modify                            | Audit subsystem summary update (Stateful/Event split, transactional emit, catalog)                   |
| `ARCHITECTURE.md`                                    | modify                            | V2 flow diagram producer → tx-bound emit → DB row + post-commit journald multiplex                   |
| `docs/superpowers/follow-up-audit-2026-05-12.md`     | create                            | Optional follow-up audit log referencing this plan's landing                                         |

---

## Task 1: Branch

- [ ] `git checkout -b feat/audit-v2-docs` from Plan D's branch.

---

## Task 2: New ADR — `0007-audit-stateful-transactional-emit.md`

**Files:** `docs/adr/0007-audit-stateful-transactional-emit.md`

- [ ] **Step 1: Read the existing ADR shape**

  ```bash
  head -40 docs/adr/0006-instance-scoped-plugins.md
  ```

  Mirror the front-matter (title, status, date, deciders) and section headings (Context, Decision, Consequences, Alternatives,
  References).

- [ ] **Step 2: Write the ADR**

  Required sections, content drawn from the spec:
  - **Context**: V1 used a single async `emit_best_effort` path. V2 must guarantee that stateful audit rows commit or roll back
    atomically with the mutation they describe. Async best-effort cannot give that guarantee on crash.
  - **Decision**:
    1. Split audited actions into two compile-time-enforced classes: `Stateful` (entity transition; requires before/after snapshot
       pair) and `Event` (workflow fact; forbidden from carrying snapshots). Enforcement via a typestate builder
       (`AuditEntry<K>` + `Builder<K, B, A>`).
    2. Stateful emission writes the audit row through `emit_stateful(&tx, &hook, entry)` inside the caller's `DatabaseTransaction`.
       The DB row is the canonical record. Journald multiplex is buffered in an `AuditCommitHook` and flushed by the caller
       immediately after `tx.commit()` succeeds; on rollback or any pre-commit error the hook is dropped without flushing.
    3. Event emission keeps V1's fire-and-forget dispatcher path (`emit_event`).
  - **Operator-observable reliability difference**:
    - Stateful rows: committed-or-not-present. The mutation and the audit row share one transaction.
    - Event rows: best-effort. May be delayed or missing on crash.
    - This difference must be surfaced to operators in `docs/security/audit-logs.md`.
  - **Latency budget**: target ≤5 ms P99 regression added to a typical mutation transaction on Postgres. SQLite is dominated by the
    existing `BEGIN IMMEDIATE` write lock; the audit INSERT is folded into the same lock window. A benchmark in
    `crates/shared/audit-log/benches/` measures the actual regression at landing and is referenced from this ADR.
  - **Consequences**:
    - Every state-changing producer call site is reshaped (~100 sites in Plan B).
    - `BEGIN IMMEDIATE` becomes load-bearing for snapshot capture, not just for the existing read-then-write paths.
    - V1 audit rows are dropped (no schema migration retains them); see `docs/security/audit-logs.md` cutover section for the
      optional pre-migration export.
    - Wire-forwarded Stateful action types are rejected at controller ingress.
  - **Alternatives considered**:
    - All-async (V1 default). Rejected: cannot give committed-or-not-present guarantee.
    - SeaORM lifecycle hook auto-emit. Rejected: handler-level outcome (denied, partial, validation_failed) and actor attribution
      cannot be derived at the model layer.
    - Optional snapshots everywhere. Rejected: makes V2's coverage promise hollow.
    - Per-action-kind retention. Deferred to V3 (compliance scope).
  - **References**:
    - Spec `docs/superpowers/specs/2026-05-11-semantic-audit-logs-v2-design.md`
    - V1 spec `docs/superpowers/specs/2026-04-17-semantic-audit-logs-design.md`
    - V1 coding standard for `BEGIN IMMEDIATE` (in `docs/development/coding-standards.md`)

- [ ] **Step 3: Format + commit**

  ```bash
  npx prettier --write docs/adr/0007-audit-stateful-transactional-emit.md
  markdownlint --config .markdownlint.json docs/adr/0007-audit-stateful-transactional-emit.md
  git add docs/adr/0007-audit-stateful-transactional-emit.md
  git commit -m "docs(adr): 0007 audit-stateful-transactional-emit"
  ```

---

## Task 3: Rewrite `docs/development/audit-logs.md`

**Files:** `docs/development/audit-logs.md`

Producer section is completely replaced. Section list:

- [ ] **Step 1: Sections to write (in order)**
  1. **Overview** — what an audit log is in uptrakit V2; the two action kinds.
  2. **The two emit paths**:
     - `emit_event(entry)` — fire-and-forget; Event-class actions.
     - `emit_stateful(&tx, &hook, entry)` — synchronous DB write inside the caller's transaction; Stateful-class actions; require
       `BEGIN IMMEDIATE`-opened transaction; pair with `AuditCommitHook::flush_after_commit()` after `tx.commit()`.
  3. **The `AuditView` derive macro** — usage, attributes (`#[audit(target_type = ...)]`, `#[audit(skip)]`, `#[audit(include)]`,
     `#[audit(project_with = "<fn>")]`, `#[audit(id_field = ...)]`, `#[audit(display_field = ...)]`), auto-skip allowlist, secret
     handling via type system primitives (`EncryptedString`, `MaskedUrl`, `MaskedEmail`).
  4. **Action-kind classification rule** — how to classify a new action as Stateful or Event; borderline guidance from spec.
  5. **Catalog workflow** — how to add a new state-changing site to `crates/shared/audit-log/audit-catalog.toml` (either `action`
     or `skip = "<reason>"`).
  6. **The `audit-coverage-check` tool** — what it checks, when it runs, how to interpret a failure.
  7. **`correlation_id` threading** — when to mint, when to thread, how to use `AuditEmitter::with_correlation(id)` in scoped
     handlers.
  8. **Test ergonomics** — `event_test_stub` / `stateful_test_stub`; secret-leak regression test pattern.
  9. **Don't** — banned patterns: no parallel `target: "security_audit"` tracing; no raw `action_type` string literals outside the
     registry, tests, fixtures, and migrations; no service-supplied snapshots (rejected at ingress).

- [ ] **Step 2: Write the file**

  Each section has concrete code examples (drawn from the spec) and concrete file paths. No placeholders. Reference the spec only
  for the "Why" — the developer doc is operational.

- [ ] **Step 3: Format + commit**

  ```bash
  npx prettier --write docs/development/audit-logs.md
  markdownlint --config .markdownlint.json docs/development/audit-logs.md
  git add docs/development/audit-logs.md
  git commit -m "docs(audit-v2): rewrite docs/development/audit-logs.md for V2 emit paths and catalog"
  ```

---

## Task 4: Update `docs/security/audit-logs.md`

**Files:** `docs/security/audit-logs.md`

Operator-facing. Sections to add or rewrite:

- [ ] **Step 1: Sections**
  1. **Evidence-integrity properties (V2)**:
     - Stateful rows: committed-or-not-present. The mutation and the audit row share one transaction.
     - Event rows: best-effort, may be delayed or missing on crash.
     - Journald is a mirror, not canonical; the DB row is the audit-of-record.
  2. **Snapshot retention storage math**:
     - Worst-case row size ~32 KB (two 16 KB snapshots).
     - Worked example: 100 stateful mutations/day × 16 KB × 90 days = ~144 MB/tenant for stateful rows.
     - Operator dial: `audit_log.retention_days` setting (single global value); lower it if storage pressure surfaces.
  3. **V1→V2 cutover note**:
     - V1 audit rows are dropped by the V2 migration.
     - Deployments with compliance posture (SOC 2, ISO 27001, customer DPA referencing audit history): export V1 rows before
       running the migration. Optional pre-migration export commands:
       - Postgres: `pg_dump --table audit_logs --table system_audit_logs <db> > audit-v1-backup.sql`
       - SQLite: `sqlite3 <path> ".dump audit_logs" ".dump system_audit_logs" > audit-v1-backup.sql`
     - Store the dump outside the database with the retention period your compliance posture requires.
  4. **What's excluded from V2 audit**: link to the catalog's `skip` entries; explain that GET handlers / heartbeats /
     lifecycle bookkeeping / cache writes are intentionally not audited (per spec).
  5. **Trust boundary**: snapshots always sourced from controller's authoritative DB read; service-supplied stateful audits are
     rejected.

- [ ] **Step 2: Write + format + commit**

  ```bash
  npx prettier --write docs/security/audit-logs.md
  markdownlint --config .markdownlint.json docs/security/audit-logs.md
  git commit -am "docs(audit-v2): operator-facing security/audit-logs covers V2 guarantees and cutover"
  ```

---

## Task 5: Update `docs/end-user/audit-logs.md`

**Files:** `docs/end-user/audit-logs.md`

Dashboard-facing. Concrete UI flows:

- [ ] **Step 1: Sections**
  1. **The State tab** — when it appears (only on Stateful rows), what it shows (key-value diff with added/removed/changed
     highlights), how to read the colors (`--color-success` added, `--color-danger` removed, `--color-warning` changed,
     `--text-muted` unchanged — but describe in human terms, not token names).
  2. **Correlation ID filter** — what a correlation ID is, how to paste one to filter, the "copy correlation ID" button on each
     row.
  3. **What's excluded** — V3 deferred features called out explicitly: workflow timeline view, per-entity audit history view,
     analytics dashboards.
  4. **Reading Stateful vs Event rows** — visual marker in the row (existing `action_kind` chip if added, else explain via the
     presence of the "State" tab in the detail drawer).

- [ ] **Step 2: Write + format + commit**

  ```bash
  npx prettier --write docs/end-user/audit-logs.md
  markdownlint --config .markdownlint.json docs/end-user/audit-logs.md
  git commit -am "docs(audit-v2): end-user audit-logs page documents State tab + correlation_id filter"
  ```

---

## Task 6: Update `docs/api/audit-logs.md`

**Files:** `docs/api/audit-logs.md`

API consumer reference.

- [ ] **Step 1: Document the DTO additions**

  Response DTO fields:
  - `action_kind: "stateful" | "event"` (always present)
  - `before_snapshot: object | null` (present iff `action_kind === "stateful"`)
  - `after_snapshot: object | null` (present iff `action_kind === "stateful"`)
  - `correlation_id: string | null` (UUID; present when the row participates in a multi-step workflow)

- [ ] **Step 2: Document filters**
  - `?correlation_id=<uuid>` — exact match.
  - `?action_kind=stateful|event` — exact match on the kind label.

- [ ] **Step 3: Examples**

  Include one `curl` example per filter and one `curl` example of the response shape for a Stateful row showing both snapshots.

- [ ] **Step 4: Format + commit**

  ```bash
  npx prettier --write docs/api/audit-logs.md
  markdownlint --config .markdownlint.json docs/api/audit-logs.md
  git commit -am "docs(audit-v2): API reference covers V2 fields and filters"
  ```

---

## Task 7: Update `AGENTS.md`

**Files:** `AGENTS.md`

Targeted edit, not a rewrite. Update the audit subsystem summary.

- [ ] **Step 1: Locate the existing audit summary**

  Run: `grep -n -i audit AGENTS.md`. Identify the paragraph(s) discussing the audit pipeline.

- [ ] **Step 2: Replace with V2 summary**

  Concise, ≤25 lines:
  - Stateful vs Event split; typestate enforcement.
  - Two emit paths (`emit_stateful` in-tx, `emit_event` async).
  - `AuditView` derive macro for entity snapshots.
  - `audit-catalog.toml` + `audit-coverage-check` CI gate.
  - V3 deferred list (one-liner per item).
  - Banned patterns: no parallel `target: "security_audit"` tracing; no raw `action_type` literals; no service-forwarded
    Stateful events.

- [ ] **Step 3: Format + commit**

  ```bash
  npx prettier --write AGENTS.md
  markdownlint --config .markdownlint.json AGENTS.md
  git commit -am "docs(audit-v2): update AGENTS.md audit subsystem summary for V2"
  ```

---

## Task 8: Update `ARCHITECTURE.md`

**Files:** `ARCHITECTURE.md`

- [ ] **Step 1: Locate the existing audit architecture section**

  Run: `grep -n -i audit ARCHITECTURE.md`.

- [ ] **Step 2: Replace with V2 flow**

  Replace the existing flow block with the V2 flow:

  ```text
  HTTP handler
    └─ begin tx (BEGIN IMMEDIATE)
         ├─ SELECT before
         ├─ perform mutation
         ├─ SELECT after
         ├─ audit_emitter.emit_stateful(&tx, &hook, AuditEntry::<verb>(&before, &after).…build()?)
         └─ tx.commit().await?
    └─ hook.flush_after_commit().await   // post-commit journald mirror (best-effort)

  Service-forwarded audit events (Event-class only):
    Service → AuditEventPayload (wire) → controller-side ingress → action-kind check
       ├─ Stateful action type → rejected with warning, dropped
       └─ Event action type    → controller-side enrichment → emit_event → async dispatcher
  ```

  Plus a sentence on the kind taxonomy and how `correlation_id` ties workflows together.

- [ ] **Step 3: Format + commit**

  ```bash
  npx prettier --write ARCHITECTURE.md
  markdownlint --config .markdownlint.json ARCHITECTURE.md
  git commit -am "docs(audit-v2): ARCHITECTURE.md V2 flow diagram"
  ```

---

## Task 9: Optional follow-up audit doc

**Files:** `docs/superpowers/follow-up-audit-2026-05-12.md`

The project keeps follow-up audit notes for spec landings (see `docs/superpowers/follow-up-audit-2026-05-03.md`). Capture: what
shipped, deferred items, anything noteworthy that emerged during implementation but didn't change the spec.

- [ ] **Step 1: Write a short follow-up note**

  Sections: shipped (per plan A–E), deferred (V3), test coverage summary, known caveats.

- [ ] **Step 2: Commit**

  ```bash
  git add docs/superpowers/follow-up-audit-2026-05-12.md
  git commit -m "docs(audit-v2): follow-up audit note"
  ```

---

## Task 10: Update auto-memory notes

**Files:** memory files under `~/.claude/projects/-Users-andreyyantsen-Development-uptrakit/memory/`

The spec (§"Auto-memory note") mandates that the implementation plan updates the user's project memory once the V2 emitter is
wired in. V1 memory notes about `emit_best_effort` and the V1 `AuditEntry` shape must be replaced or supplemented with V2 emit-path
guidance so future sessions reach for the correct API.

- [ ] **Step 1: Inventory memory entries that reference the V1 audit subsystem**

  Run: `ls ~/.claude/projects/-Users-andreyyantsen-Development-uptrakit/memory/`. Open `MEMORY.md` and grep its referenced files
  for `emit_best_effort`, `AuditEntry::builder`, "semantic audit", "security_audit".

- [ ] **Step 2: Replace or update the V1-shaped entries with V2 guidance**

  Each affected memory file gets a focused update covering:
  - `AuditEntry<K>` typestate + per-action constructors (`AuditEntry::auth_login()`, `AuditEntry::plugin_config_update(&before, &after)`, etc.).
  - Two emit paths: `emit_stateful(&tx, &hook, entry)` for Stateful (synchronous, in-tx) vs `emit_event(entry)` for Event
    (async fire-and-forget).
  - `AuditCommitHook::flush_after_commit()` after `tx.commit()`.
  - `BEGIN IMMEDIATE` requirement for read-then-write transactions that capture snapshots.
  - Banned patterns: no `emit_best_effort` (removed), no parallel `target: "security_audit"`, no service-forwarded Stateful events.
  - Pointer to `docs/development/audit-logs.md` as the producer doc and `audit-catalog.toml` as the source-of-truth for coverage.

- [ ] **Step 3: Update the `MEMORY.md` index if any memory file was renamed**

  The index already lists topics; keep entries one-line under ~150 chars.

- [ ] **Step 4: Confirm no V1 references remain**

  Run: `grep -rn 'emit_best_effort\|AuditEntry::builder(' ~/.claude/projects/-Users-andreyyantsen-Development-uptrakit/memory/`
  Expected: no matches.

  Memory updates do not produce a git commit (memory lives outside the repo).

---

## Task 11: Final markdownlint + push

- [ ] **Step 1:**

  ```bash
  npx prettier --write 'docs/**/*.md' 'AGENTS.md' 'ARCHITECTURE.md'
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: clean.

- [ ] **Step 2:**

  ```bash
  git push -u origin feat/audit-v2-docs
  ```

---

## Spec coverage check (Plan E scope)

This plan delivers every documentation deliverable from spec §"Documentation deliverables":

- New ADR `0007-audit-stateful-transactional-emit.md` (Task 2).
- Rewritten `docs/development/audit-logs.md` (Task 3).
- Rewritten `docs/security/audit-logs.md` with V1→V2 cutover and optional pre-migration export (Task 4).
- Rewritten `docs/end-user/audit-logs.md` (Task 5).
- Rewritten `docs/api/audit-logs.md` (Task 6).
- Updated `AGENTS.md` audit summary (Task 7).
- Updated `ARCHITECTURE.md` V2 flow (Task 8).

`CONTEXT.md` is unchanged by design — V2 introduces no new domain vocabulary; "stateful action", "event action", "snapshot",
"correlation_id" are implementation vocabulary that lives in `docs/development/audit-logs.md`, not in the domain glossary.

Plan E completes the V2 rollout. After Plans A through E land, the V2 audit subsystem is fully shipped per the spec.
