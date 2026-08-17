# PVE Legacy-User Migration Removal + Token Comments — Design

- **Date:** 2026-08-17
- **Status:** Draft (pending `/review-spec`)
- **Scope:** `crates/plugins/infrastructure/proxmox`, `crates/plugins/infrastructure/core`
  (`InfraPluginContext`), `crates/shared/wire` (one optional `ServiceSettings` field),
  `crates/ui/web-api` (`routes/service_ws/connection.rs`),
  `crates/core/controller-runtime` (embedded settings), `crates/core/agent-ssh-runtime`
  (instance-host plumbing + one test line), prose docs
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
per-tenant API tokens for operator convenience — one naming both the managing Uptrakit
instance (its FQDN) and the tenant, so an operator reading `pveum user token list` on a
node shared between deployments can tell whose token it is.

## Goals

1. Remove all two-phase migration machinery from the proxmox plugin: code, DB columns,
   tests, and prose.
2. Collapse the ack-marker/operative column duality into a single
   `pve_plugin_config_id` column with plain-write semantics.
3. Add `--comment 'Uptrakit managed token ({instance_host}, tenant {tenant_uuid})'` to
   both `pveum user token add` sites, falling back to
   `'Uptrakit managed token (tenant {tenant_uuid})'` when the instance host is unknown.
4. Plumb the controller's instance host (FQDN) to the SSH agent's infra-plugin call
   sites: a new optional `ServiceSettings` field sourced from the existing global
   `oauth.canonical_host` setting.
5. Update every doc that describes the migration; leave frozen historical records
   untouched.

## Non-goals

- No change to the credential flow's surviving branches (1–6), the per-cluster lock
  registry, the degraded-read guarded-create shape, or `prove_token_on_node` gating.
- No retrofit of comments onto already-created tokens in the sync path — a one-liner for
  the operator to run once covers the live install (see § Token comment).
- No new instance-URL setting, no UI, and no change to `oauth.canonical_host` semantics
  or validation — that work belongs to `uptrakit-spec-2026-08-11-cimd-relying-party`
  (see § Dependencies). This spec only reads the setting and hardens the value at the
  shell boundary.
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
  an ack or Branch 4 reuse-persist overwrite a _non-NULL_ operative id; post-collapse a
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
     can be the _legacy_ plugin-config id with `legacy_pve_user` never set. The
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

### Instance host on the wire (decision Q6: canonical host via `ServiceSettings`)

The agent has no notion of the Uptrakit deployment's FQDN today — nothing in the
agent-local SQLite DB, nothing in `service.json`/`ServiceIdentityState`, nothing in
`ServiceSettingsPayload`. `<state_dir>/discovery.json` records a URL only on the mDNS
path (never when `--url` is passed), so it is not a source. The FQDN therefore comes
from the controller, sourced from the one setting that already means "this instance's
public host": the global `oauth.canonical_host`
(`SettingKey::OauthCanonicalHost`, a bare host with optional port).

Wire (`crates/shared/wire/src/payloads.rs`):

- `ServiceSettingsPayload` gains `#[serde(default, skip_serializing_if =
"Option::is_none")] pub instance_host: Option<String>` plus a
  `with_instance_host(...)` builder, matching the existing `tenant_id`/`trust_domain`
  optional-field shape. Wire-field name is deliberately neutral — `oauth.canonical_host`
  keeps its storage key (CIMD rejected a key rename), but nothing about this field is
  OAuth-specific.
- `WireValidate for ServiceSettingsPayload` (`wire_validate_impls.rs`:993) adds
  `check_string_len(host, MAX_SHORT_STRING_LEN, "instance_host")` for the `Some` case —
  no new limit constant.
- Additive and backward-compatible: older services ignore the field, older controllers
  omit it and the agent falls back to the tenant-only comment.

Controller:

- `crates/ui/web-api/src/routes/service_ws/connection.rs` — `send_service_settings`
  (which holds `&AppState`) loads the raw setting via
  `uptrakit_web_api_auth::settings_store::load_global_setting_raw(state.db(),
SettingKey::OauthCanonicalHost.as_str())` once per WS connect and passes it into
  `build_service_settings`, which sets the field when non-empty (same
  `if !x.is_empty()` shape the `trust_domain` arm already uses). A read failure logs at
  debug and yields `None` — it must never fail a service connection.
- `crates/core/controller-runtime/src/embedded/mod.rs` — `embedded_service_settings`
  takes the same value; the caller (`add()`, :265) has `state.db()` in scope, so the
  embedded SSH agent gets the same host as an external one.

Agent (`crates/core/agent-ssh-runtime`):

- `SshAgentSettings` (lib.rs:317) gains `instance_host: Option<String>` and **loses
  `Copy`** (keeps `Clone`, `Debug`, `Default`); `handler.rs` `on_settings` (:113) fills
  it from `settings.instance_host`.
- `RuntimeSessionState` (lib.rs:323) gains the same field, written in `apply_settings`
  next to `tenant_id` — session-scoped, refreshed on every `ServiceSettings`.
- `SurfaceRuntimeContext` (surface_runtime.rs:1014, built at runtime_support.rs:376 from
  `session_state`) carries it, which covers the fully-populated `InfraPluginContext` at
  surface_runtime.rs:1093 and gives the spawn helpers something to capture.
- `BootstrapParams` (bootstrap.rs:111) gains `instance_host: Option<String>`, set in
  `parse_bootstrap_params` (surface_runtime.rs:1717) from a new
  `BootstrapConnectArgs`/`BootstrapExecuteArgs` field — **not** from the request-params
  JSON: the value is a controller session fact, and reading it from the UI-supplied
  params would let a surface caller choose the string that lands in a shell command.
  This covers both `InfraPluginContext` sites in `bootstrap.rs` (:395, :1279), where
  `params` is already in scope.
- `sync_connect`/`sync_execute` (sync.rs:277, :423) receive it and set it on the
  `InfraPluginContext` at sync.rs:547. `sync_execute` is already at six parameters, so
  the added value goes in via a small args struct rather than a seventh parameter
  (clippy `too_many_arguments`).
- `spawn_post_report_hooks_impl` (runtime_support.rs:127) reads it from `session_state`
  for the `InfraPluginContext` at :144.

Plugin core (`crates/plugins/infrastructure/core/src/agent_infra.rs`:232):

- `InfraPluginContext` gains `pub instance_host: Option<&'a str>`, documented as "the
  Uptrakit instance's public host as configured on the controller; unvalidated operator
  input — sanitize before use". Adding the field touches the four production literals
  above plus the plugin-crate test literals (`plugin.rs`:773/820/872/903,
  `credential_flow.rs`:548).

### Token comment (decisions Q4, Q6)

Both `pveum user token add` sites in `pve_setup.rs` (:462 provisioning, :519
regenerate) gain a `--comment`; `create_pve_api_credentials` and
`regenerate_pve_api_token` take the instance host as a new `Option<&str>` parameter,
passed by `credential_flow.rs` from `ctx.instance_host`. The comment is built by one
shared helper:

```text
Some("uptrakit.example.com") -> Uptrakit managed token (uptrakit.example.com, tenant <uuid>)
None                         -> Uptrakit managed token (tenant <uuid>)
```

The user add site (:453) keeps `--comment 'Uptrakit managed user'` unchanged: that user
is shared by all tenants, `pveum user add` runs once and is never rewritten, so an
instance-dependent string there would only ever describe whichever deployment created
the user first.

**Sanitization is mandatory, not decorative.** The value crosses a shell boundary inside
a single-quoted argument, and it is operator-entered free text: `CanonicalUrlConfig::new`
accepts `user@example.com` and `example.com/app` today, and nothing rejects a `'`.
`pve_setup.rs` gets a private `sanitize_instance_host(&str) -> Option<String>` that
accepts only `A-Za-z0-9`, `.`, `-`, `:`, `[`, `]` (the last two for bracketed IPv6
literals), rejects empty and anything longer than 253 characters, and returns `None`
otherwise — an unusable value degrades to the tenant-only comment rather than
propagating into a command string. The tenant UUID stays shell-safe by construction
(UUID formatting), same posture as the existing `token_id` interpolation.

No sync-path retrofit of existing tokens; the operator runs once on the live node:

```sh
pveum user token modify uptrakit@pve 'tenant-<tenant_uuid>' \
  --privsep 1 --comment 'Uptrakit managed token (<instance_host>, tenant <tenant_uuid>)'
```

`--privsep 1` is passed explicitly: whether a partial `token modify` preserves or
resets unsupplied schema-defaulted fields is PVE-version-dependent, and a reset would
silently flip the token to inheriting the shared user's full permissions. Explicit
`--privsep 1` is idempotent when preserved and corrective when not, matching the
`--privsep=1` both `token add` sites already pass.

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
- **pve_setup tests**: token-add tests assert both `--comment` forms — instance host
  present (`Some("uptrakit.example.com")` ⇒ host and tenant in the string) and absent
  (`None` ⇒ tenant-only); a table test on `sanitize_instance_host` covers accept (bare
  host, `host:port`, IPv4, `[::1]:8443`) and reject (`'`, `;`, space, `user@host`,
  `host/path`, empty, 254 chars) with the rejected cases asserted to produce the
  tenant-only comment (failure path);
  `check_pve_state_fresh_cluster`, `check_pve_state_user_no_token`,
  `check_pve_state_read_failure_is_err`, and
  `check_pve_state_tolerates_stderr_noise_before_json` adapt — they construct the
  check-state struct with `legacy_user: None` and lose that initializer (and any
  scripted second `pveum user list` response) when the field drops.
- **Instance-host plumbing**: `build_service_settings` sets `instance_host` when the
  setting is non-empty and leaves it `None` when unset/empty (no serde-roundtrip test —
  plain derive, per testing standards); `WireValidate` rejects an over-long
  `instance_host` and accepts `None`; an agent-side test asserts `apply_settings` stores
  the host on `RuntimeSessionState` and that a later `ServiceSettings` without the field
  clears it (stale-value regression).
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

**Soft relation (not blocking): `uptrakit-spec-2026-08-11-cimd-relying-party`**, in
particular `uptrakit-plan-2026-08-11-cimd-a-mcp-opt-in` (Tasks A2 inversion, A3
canonical-host shape validation, A6 Global Settings "Instance" card). The instance host
this spec reads is `oauth.canonical_host`, and until Plan A lands:

- setting it also boots the MCP OAuth authorization server (`resolve_mcp_enabled`
  auto-enables when the canonical host is set), so an operator cannot set it _only_ to
  stamp PVE token comments;
- its stored shape is loosely validated (`user@example.com`, `example.com/app` pass),
  which is why sanitization lives on the agent side of the boundary regardless;
- there is no Global Settings home for it outside the MCP Access tab.

Consequence, accepted deliberately: on a deployment with the canonical host unset the
token comment ships in its tenant-only form and gains the FQDN with no code change once
the host is set. That fallback is what keeps this a soft relation — this spec is
implementable and shippable today, and neither side edits the other's files (CIMD:
`oauth/`, `routes/settings_oauth.rs`, `web-api-types`; here:
`routes/service_ws/connection.rs`). Implementation order does not matter; if CIMD Plan A
lands first, nothing here changes.

No hard dependencies.

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
   pointing at ADR-0044; plus one line in the settings/session description: the agent
   caches the controller-supplied instance host per session and hands it to infra
   plugins, never persisting it.
3. `docs/development/proxmox-plugin.md` — migration bookkeeping rows/sections (row 57,
   row 216, and the full "PVE Cluster Deduplication and Legacy Migration" subsection at
   219–244 including its header) removed; credential-flow branch description updated to
   the surviving branches; token-comment behavior documented, including where the
   instance host comes from and the tenant-only fallback.
4. `docs/end-user/proxmox.md` — legacy-identity-model note block (lines 197–204), the
   full "Migrating to the Shared PVE Identity Model" section (lines 255–373, all
   subsections), and the "Cleaning up a never-fully-migrated legacy user" subsection
   under Deprovisioning (lines 415–431, tied to the removed `delete_pve_user` /
   `LEGACY_PVE_USERNAME_PREFIX`) removed; token comment mentioned as observable
   behavior, with the note that the instance FQDN appears only when the instance's
   canonical host is configured.
5. `docs/end-user/ssh-agent-bootstrap.md` — migration mention (lines ~432–437)
   simplified to the shared-user model description.
6. `docs/development/proxmox-bootstrap.md` — line ~107 rephrased (the shared-user model
   is no longer "this migration introduces").
7. `docs/api/wire-protocol.md` § `ServiceSettingsPayload` Fields (table at :1043) — one
   row for `instance_host` (optional; the controller's `oauth.canonical_host`, absent
   when unset), plus `crates/shared/wire/asyncapi.yaml` regenerated via
   `./scripts/regen-asyncapi.sh` and committed (CI gates on the
   `asyncapi_yaml_is_up_to_date` golden test).
8. `CONTEXT.md`, `docs/development/error-handling.md` — **no change**: their "two-phase"
   hits are unrelated (config-reload validate-then-apply; error-decision table).
9. In-crate rustdoc/comments on surviving code — `credential_flow.rs` module `//!` doc
   (:1–:16, includes the cluster-lock rationale), the Branch 4 evidence comment
   (:201–:202, "A bare `pve_plugin_config_id` never satisfies reuse" — inverted by this
   design), the Branch 4 persist comment (:233–:238, coalesce/legacy-row semantics
   being removed), `CredentialFlowOutput::degraded`'s doc, and `wipe_all`'s doc all
   describe two-phase mechanics; rewrite to the post-removal flow. While editing
   Branch 4, also fix the misleading "using newest" wording on the peers-disagree
   `ids.max()` arm — it picks the lexicographic max of UUID strings, not the newest
   (keep `sort_unstable` + `dedup` and take `ids.pop()`, or document the arm as
   arbitrary-but-deterministic).
10. `docs/superpowers/specs/*` historical specs — bodies **untouched** (frozen records).
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
and `( cd website && zola check )` for the end-user doc edits. The wire change adds
`./scripts/regen-asyncapi.sh` with `crates/shared/wire/asyncapi.yaml` committed (gated
by the `asyncapi_yaml_is_up_to_date` golden test). No REST, OpenAPI, or audit-catalog
surface is touched — the instance host rides an existing message and an existing
setting, no endpoint changes (verified: no PVE/proxmox entries in
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
- **FQDN from the agent's own `--url` host** (thread `CommonServiceArgs::parsed_url()`
  through `AgentSshRuntimeSupport` → `SurfaceRuntimeContext`/`BootstrapParams`) —
  rejected (decision Q6): it records whatever address that agent happens to dial (LAN
  IP, VPN name, split-horizon alias), which is exactly the ambiguity the comment is
  meant to resolve; the controller-hosted embedded SSH agent has no `--url` at all; and
  it would fork "the instance's address" into a second, agent-local notion.
- **A new dedicated instance-URL setting** — rejected (decision Q6): duplicates
  `oauth.canonical_host`, which CIMD is already turning into the general instance
  setting; two settings that must agree is worse than one that is briefly MCP-coupled.
- **Hard-blocking this spec on CIMD Plan A** — rejected (decision Q6): the tenant-only
  fallback makes the FQDN a pure enrichment, so the migration removal (the bulk of the
  work, and the part with a schema change) ships independently.

## Deferred / out of scope

- Any change to controller-side plugin-config handling.
- Comment backfill automation for pre-existing tokens (manual one-liner suffices).
- Any other consumer of the new `instance_host` wire field (notifications, MQTT,
  agent-side link building) — added here for the token comment only.
