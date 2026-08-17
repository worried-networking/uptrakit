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
  (:107) into `set_plugin_config_id` (plain write to `PvePluginConfigId`). Keep
  `wipe_all`, `find_pve_hosts`, `upsert_host_state`, pending-match functions.
- **`agent/entity.rs`** — drop fields `legacy_pve_user`, `new_pve_plugin_config_id`,
  `migration_attempts`.
- **`agent/plugin.rs`** — remove the preview action line "Migrate legacy per-tenant PVE
  user to the shared uptrakit@pve scheme" (:98); simplify the outcome mapping
  `Reused | MigrationPending` → `Reused` (:251); `on_plugin_config_reported` (:287)
  calls the renamed plain setter.
- **`pve_setup.rs`** — remove `LEGACY_PVE_USERNAME_PREFIX` (:36), the `legacy_user`
  field on the check-state struct (:60) and its matching logic (:326–:336), and
  `delete_pve_user` (:587). Decision Q2: no vestigial legacy-user detection or warning
  remains.
- **`agent/migration.rs`** — bodies of the four existing migrations stay byte-frozen
  (ledger rule: merged migrations are append-only). Append
  `m20260817_000001_drop_pve_migration_columns`: first fill the operative column via a
  `sea_query` UPDATE setting `pve_plugin_config_id =
COALESCE(pve_plugin_config_id, new_pve_plugin_config_id)` (`Func::coalesce` — no raw
  SQL needed), then drop `migration_attempts`, `new_pve_plugin_config_id`,
  `legacy_pve_user`. The existing `ProxmoxHostState` iden enum already carries the
  needed variants. `down()` re-adds the three columns (data not restored — acceptable
  for an agent-local forward-only store, same posture as existing `down()` bodies).

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
  --comment 'Uptrakit managed token (tenant <tenant_uuid>)'
```

The tenant UUID must be shell-safe by construction (UUID formatting) — same posture as
the existing `token_id` interpolation.

### Test changes

- **Delete** (migration-only): `phase1_records_legacy_and_keeps_it_alive`,
  `legacy_stored_without_ack_marker_never_deletes`, the five `phase2_*` tests,
  `recovery_arm_promotes_when_marker_known`,
  `failed_creation_outranks_a_pending_migration` (credential_flow);
  `migration_setters_roundtrip_and_promotion_retains_ack_marker` (db_ops, replaced by a
  plain-setter roundtrip); `check_pve_state_token_and_legacy_coexist` (pve_setup).
- **Adapt** to single-column semantics: `reuse_bare_operative_id_without_ack_marker_is_not_reused`
  inverts — a bare operative id is now valid reuse evidence (rename accordingly);
  `reuse_with_ack_marker_and_confirmed_token_reuses`,
  `reuse_multiple_ack_markers_uses_max`, `reuse_standalone_peer_isolation`,
  `reuse_persists_peer_evidence_id_onto_flow_hosts_own_row`,
  `reuse_dead_token_evidence_falls_through_to_create` — rewritten against
  `pve_plugin_config_id`. `regenerate_on_ack_loss` survives as the
  evidence-missing-but-token-present Branch 6 regenerate case. Degraded tests
  (`degraded_read_never_fires_destructive_arms`, `degraded_create_uses_regenerate_shape`)
  keep their create-shape assertions, drop `MigrationPending` assertions.
- **plugin.rs tests** (:1059–:1152): `on_plugin_config_reported` assertions flip to the
  operative column (positional-scan regression coverage stays).
- **pve_setup tests**: token-add tests additionally assert the `--comment` flag.
- Success and failure paths stay covered per testing standards; scripted-executor
  harness unchanged.

### Data-migration correctness

On the live deployment the migration completed, so `promote_cluster_rows` already copied
the ack marker into the operative column — the COALESCE fill is a no-op there. It exists
for defense in depth: any row where only the ack marker was ever written (ack received,
promotion never ran) keeps its evidence. Fresh installs never had the columns populated.

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
   machinery removed", dated, pointing at this spec). History stays intact — no
   rewrite (decision Q5).
2. `docs/architecture/ssh-agent.md` — § Legacy-user migration (~lines 344–391) replaced
   with a one-paragraph historical note pointing at ADR-0044.
3. `docs/development/proxmox-plugin.md` — migration bookkeeping rows/sections (rows 57,
   216, 226–242) removed; credential-flow branch description updated to the surviving
   branches; token-comment behavior documented.
4. `docs/end-user/proxmox.md` — legacy-model + two-phase migration section
   (~lines 199–280) removed; token comment mentioned as observable behavior.
5. `docs/end-user/ssh-agent-bootstrap.md` — migration mention (lines ~432–437)
   simplified to the shared-user model description.
6. `docs/development/proxmox-bootstrap.md` — line ~107 rephrased (the shared-user model
   is no longer "this migration introduces").
7. `CONTEXT.md`, `docs/development/error-handling.md` — **no change**: their "two-phase"
   hits are unrelated (config-reload validate-then-apply; error-decision table).
8. `docs/superpowers/specs/*` historical specs — **untouched** (frozen records).

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
