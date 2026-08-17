# PVE Legacy-User Migration Removal + Token Comments — Design

- **Date:** 2026-08-17
- **Status:** Draft (pending `/review-spec`)
- **Scope:** `crates/plugins/infrastructure/proxmox`, one test line in
  `crates/core/agent-ssh-runtime`, prose docs
- **ADR impact:** completion note in ADR-0044 § Migration; no new ADR

## Context

ADR-0044 (accepted 2026-08-17) introduced the shared `uptrakit@pve` user with per-tenant
`--privsep=1` API tokens (`tenant-{tenant_uuid}`), replacing the legacy per-tenant
`uptrakit-{tenant_uuid}@pve` users. To move existing installs across, the plugin shipped a
two-phase, cluster-scoped migration: phase 1 records the legacy user
(`set_legacy_pve_user`); phase 2 proves the new token on the node
(`prove_token_on_node`), waits for the controller ack marker
(`new_pve_plugin_config_id`), deletes the legacy user (`delete_pve_user`), and promotes
the cluster rows (`promote_cluster_rows`), with a recovery arm and a
`MAX_MIGRATION_ATTEMPTS = 5` poison guard.

The single live deployment has completed this migration (owner declaration, 2026-08-17).
The machinery is now dead weight: three extra DB columns, four credential-flow branches,
a dedicated outcome variant, and a body of tests and docs describing a flow that can
never run again. This spec removes it and, independently, adds a `--comment` to the
per-tenant API tokens for operator convenience.

## Goals

1. Remove all two-phase migration machinery from the proxmox plugin: code, DB columns,
   tests, and prose.
2. Collapse the ack-marker/operative column duality into a single
   `pve_plugin_config_id` column with plain-write semantics.
3. Add `--comment 'Uptrakit managed token (tenant {tenant_uuid})'` to both
   `pveum user token add` sites.
4. Update every doc that describes the migration; leave frozen historical records
   untouched.

## Non-goals

- No change to the credential flow's surviving branches (1–6), the per-cluster lock
  registry, the degraded-read guarded-create shape, or `prove_token_on_node` gating.
- No retrofit of comments onto already-created tokens in the sync path — a one-liner for
  the operator to run once covers the live install (see § Token comment).
- No rewrite of merged agent migrations — existing migration bodies stay byte-frozen;
  removal is a new appended migration.
- No new ADR: removing completed migration machinery executes ADR-0044's plan, it does
  not revise a decision.

## Design

### Column collapse (decision Q1: full collapse)

`proxmox_host_state` currently carries two config-id columns: `pve_plugin_config_id`
(operative) and `new_pve_plugin_config_id` (never-cleared ack marker). The duality
existed solely to protect the legacy operative id mid-migration. Verified: the only
production writers of the coalesce-fill setter `set_new_plugin_config_id` are the
controller-ack path (`plugin.rs` `on_plugin_config_reported`) and the Branch 4
reuse-persist site (`credential_flow.rs`); the sole production writer of the operative
column outside promotion passes `None` (`upsert_host_state` call sites in `plugin.rs`).
The operative column's consumer is `surface_actions.rs` (node/config → host map).

Post-collapse:

- Single column `pve_plugin_config_id`; `on_plugin_config_reported` and the Branch 4
  reuse-persist site write it directly via a plain single-column setter (rename
  `set_new_plugin_config_id` → `set_plugin_config_id`, drop the coalesce shape).
- Reuse evidence (Branch 4's cluster-wide scan) reads `pve_plugin_config_id` instead of
  the ack marker. A stored operative id **is** reuse evidence — its only writers are the
  ack path and a prior confirmed reuse.
- `surface_actions.rs` keeps working unchanged.
- **Intended behavior change** (not semantics-preserving): today's coalesce never lets
  an ack or Branch 4 reuse-persist overwrite a *non-NULL* operative id; post-collapse a
  plain write does. On a duplicate-config install where cluster peers disagree, the
  Branch 4 `ids` max-arm result is written onto the local row (peers converge instead
  of staying frozen), and a controller re-report with a changed config id updates
  `surface_actions.rs`'s map instead of leaving it stale. Both are improvements and are
  covered by the renamed multi-id reuse test (see § Test changes).

### Code removal inventory (verified against source, 2026-08-17)

`crates/plugins/infrastructure/proxmox/src/`:

- **`agent/credential_flow.rs`** — remove Branch 7 (phase 1 legacy record, :339),
  Branch 8 (phase 2 prove-then-delete, :384), Branch 9 (recovery arm, :356), the
  Branches 7–9 degraded-skip block (:323), Branch 10 (`MigrationPending` precedence,
  :426), `cluster_migration_attempts_max` (:445), `MAX_MIGRATION_ATTEMPTS` (:34), and
  the `MigrationPending` outcome variant (:61). Branches 1–6, `cluster_lock`, and the
  degraded guarded create stay.
- **`agent/db_ops.rs`** — remove `set_legacy_pve_user` (:83), `promote_cluster_rows`
  (:137), `increment_migration_attempts` (:165); collapse `set_new_plugin_config_id`
  (:107) into `set_plugin_config_id` (plain write to `PvePluginConfigId`). Edit
  `upsert_host_state`: its insert-branch `ActiveModel` literal (:63–:65) sets the three
  dropped fields (`legacy_pve_user`, `new_pve_plugin_config_id`, `migration_attempts`)
  and loses those initializers; the stale doc comment above the collapsed setter
  (~:101–:105, referencing `set_new_plugin_config_id`/`promote_cluster_rows`) is
  rewritten, as is the preserve-on-None comment inside `upsert_host_state` (:41–:46) —
  it names both dead setters and its "must not wipe an already-migrated row's operative
  config id" justification dissolves once the parameter is dropped (below). Additionally drop the `pve_plugin_config_id` parameter from
  `upsert_host_state` entirely — every production caller passes `None`, and removing
  the parameter makes "only the ack path and Branch 4 reuse-persist write the operative
  column" structural instead of conventional (test callers adapt). Keep `wipe_all`,
  `find_pve_hosts`, `upsert_host_state` (edited as above), pending-match functions.
- **`agent/entity.rs`** — drop fields `legacy_pve_user`, `new_pve_plugin_config_id`,
  `migration_attempts`.
- **`agent/plugin.rs`** — remove the `sync_step_previews` migration step line
  "Advance legacy-user migration when pending" (:55); remove the whole
  `if state.legacy_user.is_some()` preview-action block in `probe_host` (:96–:101,
  guard included — the `legacy_user` field it reads is deleted); simplify the outcome
  mapping `Reused | MigrationPending` → `Reused` (:251); `on_plugin_config_reported`
  (:287) calls the renamed plain setter.
- **`pve_setup.rs`** — remove `LEGACY_PVE_USERNAME_PREFIX` (:36), the `legacy_user`
  field on the check-state struct (:60), the legacy-user probe in `check_pve_state`
  (~:298–:336: the second `pveum user list` remote call, its JSON parse, and the
  matching logic — removes one remote round-trip per sync/bootstrap), and
  `delete_pve_user` (:587). Decision Q2: no vestigial legacy-user detection or warning
  remains.
- **`agent/migration.rs`** — bodies of the four existing migrations stay byte-frozen
  (ledger rule: merged migrations are append-only). Append
  `m20260817_000001_drop_pve_migration_columns`. `up()` opens with a single
  `manager.has_column("proxmox_host_state", "new_pve_plugin_config_id")` check
  (same helper already used in this crate's `controller_migration.rs`, no raw SQL) and
  early-returns `Ok(())` when absent — a ledger/schema skew state (cf. the
  plugin-agent-migrations-not-running incident) then no-ops instead of failing the
  whole agent migration run; no per-statement guards needed. Otherwise:
  1. Fold with one unconditional `sea_query` UPDATE: `SET pve_plugin_config_id =
     new_pve_plugin_config_id`. The ack column is the only trustworthy source — every
     legitimate post-ADR-0044 operative write also set the ack column
     (`set_new_plugin_config_id` writes both; `promote_cluster_rows` only runs after
     an ack exists; production `upsert_host_state` callers pass `None`), while a
     pre-ADR-0044 operative value (the `m20260308_000001` backfill from `ssh_hosts`)
     can be the *legacy* plugin-config id with `legacy_pve_user` never set. The
     unconditional overwrite makes "stored ⇒ ack-derived" true of all pre-existing
     data. Side effect on a skewed install: a legacy-only operative id is cleared, so
     `surface_actions.rs`'s node/config→host map goes empty until the next credential
     flow repopulates it and the flow falls through to create/regenerate — the safe
     direction. No-op on the live deployment (promotion completed) and fresh installs.
  2. Drop `migration_attempts`, `new_pve_plugin_config_id`, `legacy_pve_user` — as
     three separate `alter_table` calls (SQLite single-alteration limit), matching the
     existing `AddPveMigrationColumns::down()` loop.

  The existing `ProxmoxHostState` iden enum already carries the needed variants.
  `down()` re-adds the three columns (data not restored — acceptable for an agent-local
  forward-only store, same posture as existing `down()` bodies).

`crates/core/agent-ssh-runtime/src/runtime_support.rs`:

- Test-only raw-SQL probe at :691 reads `new_pve_plugin_config_id` (approved raw-SQL
  exception, cross-crate table) — switch to `pve_plugin_config_id`; the inline
  exception comment stays.

### Token comment (decision Q4)

Both `pveum user token add` sites in `pve_setup.rs` (:462 provisioning, :519
regenerate) gain `--comment 'Uptrakit managed token (tenant {tenant_uuid})'`. The user
add site (:453) already carries `--comment 'Uptrakit managed user'`. No sync-path
retrofit of existing tokens; the operator runs once on the live node:

```sh
pveum user token modify uptrakit@pve 'tenant-<tenant_uuid>' \
  --privsep 1 --comment 'Uptrakit managed token (tenant <tenant_uuid>)'
```

`--privsep 1` is passed explicitly: whether a partial `token modify` preserves or
resets unsupplied schema-defaulted fields is PVE-version-dependent, and a reset would
silently flip the token to inheriting the shared user's full permissions. Explicit
`--privsep 1` is idempotent when preserved and corrective when not, matching the
`--privsep=1` both `token add` sites already pass.

The tenant UUID must be shell-safe by construction (UUID formatting) — same posture as
the existing `token_id` interpolation.

### Test changes

- **Delete** (migration-only): `phase1_records_legacy_and_keeps_it_alive`,
  `legacy_stored_without_ack_marker_never_deletes`, the six `phase2_*` tests
  (`phase2_prove_then_delete_promotes_on_success`,
  `phase2_promotes_both_rows_in_a_multi_node_cluster`,
  `phase2_delete_failure_increments_attempts_and_pends`,
  `phase2_delete_failure_at_cap_reports_stuck`,
  `phase2_excludes_peer_row_with_null_node_name`,
  `phase2_standalone_write_scope_isolated`),
  `recovery_arm_promotes_when_marker_known`,
  `failed_creation_outranks_a_pending_migration` (credential_flow);
  `migration_setters_roundtrip_and_promotion_retains_ack_marker` (db_ops, replaced by a
  plain-setter roundtrip); `check_pve_state_token_and_legacy_coexist` (pve_setup).
- **Adapt** to single-column semantics: `reuse_bare_operative_id_without_ack_marker_is_not_reused`
  inverts — a bare operative id is now valid reuse evidence (rename accordingly);
  `reuse_with_ack_marker_and_confirmed_token_reuses`,
  `reuse_multiple_ack_markers_uses_max` (also renamed — it no longer reads ack
  markers; its rewrite additionally asserts the peers-disagree result is written back
  over the local row's differing pre-existing operative id, pinning the intended
  plain-write overwrite from § Column collapse), `reuse_standalone_peer_isolation`,
  `reuse_persists_peer_evidence_id_onto_flow_hosts_own_row`,
  `reuse_dead_token_evidence_falls_through_to_create` — rewritten against
  `pve_plugin_config_id`. `regenerate_on_ack_loss` survives as the
  evidence-missing-but-token-present Branch 6 regenerate case. Degraded tests
  (`degraded_read_never_fires_destructive_arms`, `degraded_create_uses_regenerate_shape`)
  keep their create-shape assertions, drop `MigrationPending` assertions.
- **plugin.rs tests** (:1059–:1152): `on_plugin_config_reported` assertions flip to the
  operative column (positional-scan regression coverage stays).
- **pve_setup tests**: token-add tests additionally assert the `--comment` flag;
  `check_pve_state_fresh_cluster`, `check_pve_state_user_no_token`,
  `check_pve_state_read_failure_is_err`, and
  `check_pve_state_tolerates_stderr_noise_before_json` adapt — they construct the
  check-state struct with `legacy_user: None` and lose that initializer (and any
  scripted second `pveum user list` response) when the field drops.
- Success and failure paths stay covered per testing standards; scripted-executor
  harness unchanged.

### Data-migration correctness

On the live deployment the migration completed, so `promote_cluster_rows` already copied
the ack marker into the operative column (and the ack marker is never cleared) — the
unconditional fold is a no-op there, and fresh installs never had the columns populated.
The fold exists for defense in depth on any skewed install: the ack column is the only
trustworthy source, because a pre-ADR-0044 operative value can be the legacy
plugin-config id — any COALESCE-style preservation of the operative value would turn a
legacy id into false reuse evidence, and a `legacy_pve_user`-based predicate misses
rows backfilled before that column existed. Post-collapse a stored
`pve_plugin_config_id` always means "ack-derived", including for pre-existing data.

## Dependencies (cross-cycle sweep, 2026-08-17)

Swept all open spec/plan epics for overlap with the in-scope files
(`crates/plugins/infrastructure/proxmox/src/*`, `agent-ssh-runtime/runtime_support.rs`):

- `uptrakit-spec-2026-08-16-pve-node-queue-gate` — controller-side update-queue
  promotion (`crates/shared/db`, promoters, sweeper); no file overlap. Not wired.
- `uptrakit-spec-2026-08-06-agent-ssh-surface-error-taxonomy` — `surface_runtime.rs` +
  `operations/sync.rs` in agent-ssh-runtime; no overlap with `runtime_support.rs`'s one
  test line. `PveCredentialOutcome` does not leak outside the plugin crate (verified by
  workspace grep). Not wired.
- `uptrakit-spec-2026-07-12-ssh-bootstrap-conflict-precheck` — bootstrap execute paths;
  no overlap. Not wired.

No hard dependencies, no soft relations.

## Documentation deliverables

Implementation must update (verified hit list):

1. `docs/adr/0044-shared-pve-user-with-per-tenant-privilege-separated-api-tokens.md` —
   short completion note in § Migration ("completed on the single live deployment;
   machinery removed", dated, pointing at this spec). The note keeps the manual
   cleanup one-liner (`pveum user delete 'uptrakit-<tenant_uuid>@pve'`) so an operator
   with a straggler legacy user retains written instructions after `delete_pve_user`
   and the end-user cleanup subsection are gone. History stays intact — no rewrite
   (decision Q5).
2. `docs/architecture/ssh-agent.md` — § Legacy-user migration (two-phase,
   cluster-scoped), lines 366–403, replaced with a one-paragraph historical note
   pointing at ADR-0044.
3. `docs/development/proxmox-plugin.md` — migration bookkeeping rows/sections (row 57,
   row 216, and the full "PVE Cluster Deduplication and Legacy Migration" subsection at
   219–244 including its header) removed; credential-flow branch description updated to
   the surviving branches; token-comment behavior documented.
4. `docs/end-user/proxmox.md` — legacy-identity-model note block (lines 197–204), the
   full "Migrating to the Shared PVE Identity Model" section (lines 255–373, all
   subsections), and the "Cleaning up a never-fully-migrated legacy user" subsection
   under Deprovisioning (lines 415–431, tied to the removed `delete_pve_user` /
   `LEGACY_PVE_USERNAME_PREFIX`) removed; token comment mentioned as observable
   behavior.
5. `docs/end-user/ssh-agent-bootstrap.md` — migration mention (lines ~432–437)
   simplified to the shared-user model description.
6. `docs/development/proxmox-bootstrap.md` — line ~107 rephrased (the shared-user model
   is no longer "this migration introduces").
7. `CONTEXT.md`, `docs/development/error-handling.md` — **no change**: their "two-phase"
   hits are unrelated (config-reload validate-then-apply; error-decision table).
8. In-crate rustdoc/comments on surviving code — `credential_flow.rs` module `//!` doc
   (:1–:16, includes the cluster-lock rationale), the Branch 4 evidence comment
   (:201–:202, "A bare `pve_plugin_config_id` never satisfies reuse" — inverted by this
   design), the Branch 4 persist comment (:233–:238, coalesce/legacy-row semantics
   being removed), `CredentialFlowOutput::degraded`'s doc, and `wipe_all`'s doc all
   describe two-phase mechanics; rewrite to the post-removal flow. While editing
   Branch 4, also fix the misleading "using newest" wording on the peers-disagree
   `ids.max()` arm — it picks the lexicographic max of UUID strings, not the newest
   (keep `sort_unstable` + `dedup` and take `ids.pop()`, or document the arm as
   arbitrary-but-deterministic).
9. `docs/superpowers/specs/*` historical specs — bodies **untouched** (frozen records).
   Sole exception: `2026-08-16-pve-bootstrap-refactor-design.md` still reads
   `**Status:** Design (pending plan)` although it was implemented; correct that one
   status line (e.g. "Implemented; migration machinery since removed — see
   2026-08-17-pve-migration-removal-design.md") so its load-bearing rationale for the
   ack marker is not mistaken for current design.

The standards-snapshot Binding Rule describing the two-phase migration derives from
ADR-0044 / ssh-agent.md / proxmox-plugin.md and dissolves at the next snapshot refresh
once deliverables 1–3 land — removal is the intent of this spec, not a rule violation.

`docs/end-user/` edits trigger the `( cd website && zola check )` gate (AGENTS.md,
2026-08-17).

## Quality gates

Standard Rust gates (fmt, check minimal + all-features, clippy both, tests,
`cargo deny check` not needed — no dependency changes), `markdownlint`,
`bash ci/verify_no_new_cfg_not_feature.sh`, `python3 ci/verify_no_orphan_modules.py`,
and `( cd website && zola check )` for the end-user doc edits. No wire, REST, OpenAPI,
or audit-catalog surface is touched (verified: no PVE/proxmox entries in
`audit-catalog.toml`; `PveCredentialOutcome` is crate-internal).

## Alternatives considered

- **Keep the ack/operative duality, delete only branches 7–9** — rejected: the duality's
  only purpose was protecting the legacy operative id mid-migration; keeping it would
  preserve dead complexity and a misleading invariant.
- **Vestigial legacy-user warning in `check_pve_state`** — rejected (decision Q2):
  single-deployment reality makes it unreachable; dead detection code drifts.
- **Retrofit token comments in the sync path** — rejected (decision Q4): a recurring
  write to cover a one-time backfill on one node; the manual one-liner is cheaper and
  leaves the sync path untouched.
- **New ADR for the removal** — rejected (decision Q5): executing ADR-0044's migration
  plan to completion is not a new architectural decision.

## Deferred / out of scope

- Any change to controller-side plugin-config handling.
- Comment backfill automation for pre-existing tokens (manual one-liner suffices).
