# Proxmox Host Bootstrapping Refactor — Design

**Date:** 2026-08-16
**Status:** Design (pending plan)
**Supersedes:** `docs/superpowers/specs/2026-07-12-proxmox-verify-tls-key-fix-design.md` (and its plan
`docs/superpowers/plans/2026-07-13-proxmox-verify-tls-key-fix.md`) — both PVE config emit sites are rewritten
here, absorbing that fix.

## Problem

PVE credential provisioning during SSH host bootstrap has an identity model that fights PVE's own semantics and
several verified defects:

1. **Per-tenant PVE users** `uptrakit-{tenant_uuid}@pve` (`pve_setup.rs:247-253`, `pve_user_realm`
   `pve_setup.rs:301-303`) with a single token `uptrakit` created `--privsep=0` (`pve_setup.rs:472`, `:524`) and
   ACLs granted to the user. First `uptrakit-*@pve` match decides cluster ownership
   (`check_pve_token_exists`, `pve_setup.rs:310-359`): a second tenant is blocked (`OwnedByOtherTenant`), and
   with multiple tenants the answer is iteration-order-dependent.
2. **The "non-destructive" connect phase mutates the cluster.** `gather_remote_host_info` →
   `detect_infra_plugins` (`operations/bootstrap.rs:363`, `:375-425`) calls
   `HostLifecycle::on_host_bootstrapped` (`:407`), which runs `pveum role add/modify`, `pveum user add`,
   `pveum user token add`, `pveum acl modify`, then discards the resulting credentials. Abandoning at the review
   step orphans a PVE user + token with nothing recorded controller-side. The execute phase calls
   `on_host_bootstrapped` a second time (`collect_infra_results`, `bootstrap.rs:1213-1250`), relying on
   idempotency to converge.
3. **The `pve_setup` skip action is cosmetic.** `bootstrap.rs:645-655`: PVE provisioning is skipped only when
   `configure_sudoers` is _also_ skipped; skipping `pve_setup` alone does nothing distinct.
4. **`verify_ssl` / `verify_tls` key mismatch.** Both emit sites (`agent/plugin.rs:128`, `:215`) report
   `"verify_ssl": true`; `ProxmoxConfig` deserializes `verify_tls` (`config.rs:34`) with no serde alias — the
   reported value is silently dropped (previously specced separately; absorbed here).
5. **`verify_pve_privileges` is dead code** (`pve_setup.rs:109-216` — locator hint; zero callers
   workspace-wide), while `docs/architecture/ssh-agent.md` describes sync as "verifying PVE privileges".
6. **Plugin-config naming is per-host** (`pve-{host_id}`, `agent/plugin.rs:123-130`), so N cluster nodes
   bootstrapped before reconciliation yield N configs; `reconcile_pve_config` resolves by `max()` + warning.
7. **No deprovisioning documentation exists** anywhere (workspace grep: zero host-lifecycle hits for
   deprovision/teardown/userdel); removing a host leaves the PVE user, token, roles, ACLs, ssh user, sudoers
   drop-in, and helper scripts behind forever, undocumented.
8. **Test gap:** no test drives any `pveum` command builder against a scripted executor, and no test covers
   `on_host_bootstrapped`/`on_host_synced` flows, despite `ScriptedRemoteExecutor` existing
   (`operations/bootstrap.rs`, `operations/sudoers.rs` test modules).

## Decisions (settled with owner, 2026-08-16)

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| D1  | Single cluster-wide PVE user **`uptrakit@pve`**, always. No `@pam` registration, no per-tenant users. The originally proposed pam/pve split was dropped: PVE users are cluster-wide while bootstrap is per-node, and API tokens never authenticate via PAM, so pam registration buys nothing and creates a mixed-cluster identity ambiguity.                                                                                                                                                                                                                                                                       |
| D2  | **Per-tenant API tokens** on that user: token id `tenant-{tenant_uuid}`, created with explicit `--privsep=1`. ACLs granted at **two levels**: the four `(path, role)` pairs to the **user** (a fixed ceiling — PVE computes a privsep token's effective privileges as the **intersection** of user and token grants, so a user with zero ACLs would zero out every token) and the same pairs to **each token** (`pveum acl modify <path> --tokens 'uptrakit@pve!tenant-{uuid}' --roles <role>`). The user gets no password (token-only identity). Amended 2026-08-16 after source-verifying the intersection rule. |
| D3  | **Multi-tenant coexistence**: tenants share a cluster, each with its own token + ACL grants. `OwnedByOtherTenant` blocking and the first-match ownership scan are removed.                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| D4  | **Connect phase becomes read-only.** New read-only probe hook on `HostLifecycle`; provisioning moves exclusively to the execute phase (single `on_host_bootstrapped` call per bootstrap).                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D5  | **`pve_setup` becomes an independently skippable execute action.**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D6  | **Two-phase migration** from the legacy per-tenant-user scheme, driven by sync.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| D7  | **Per-cluster plugin-config naming**: `pve-{cluster_name}`, standalone fallback `pve-{node_name}`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D8  | `verify_ssl` → `verify_tls` emit fix absorbed into the rewritten emit sites.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| D9  | Delete dead `verify_pve_privileges`; idempotent `ensure_*` repair (rewritten for per-token grants) is the sync-time mechanism; docs updated from "verifies" to "ensures/repairs".                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| D10 | **Deprovisioning: documentation only.** No teardown code.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D11 | Test coverage per § Testing. Guest bootstrap (`bootstrap_proxmox.rs`) tests are out of scope.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| D12 | Bootstrap path emits a user-visible summary line when PVE setup is skipped for lack of `tenant_id` (parity with the sync path's summary).                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

Alternatives rejected during grilling: pam/pve split by bootstrap mode (mixed-cluster ambiguity, no functional
gain — D1); keeping tenant exclusivity (perpetuates order-dependent ownership, blocks legitimate coexistence —
D3); privsep=0 shared-privilege tokens (no isolation between tenants — D2); one-shot migration (mid-run failure
leaves config pointing at a deleted user — D6); automated deprovision code (unreachable-host and partial-failure
handling is its own project — D10).

## Verified PVE facts (source-checked 2026-08-16, `proxmox/pve-access-control` @ master)

- ACLs for tokens are first-class: `PVE/API2/ACL.pm` `tokens` parameter (format `pve-tokenid-list`), stored
  per-token in the ACL tree separately from user grants. `pveum acl modify <path> --tokens <full-tokenid>
--roles <role>` is the CLI form.
- Token id grammar: `PVE/AccessControl.pm:280` `token_subid_regex = realm_regex`; `PVE/Auth/Plugin.pm:34`
  `realm_regex = [A-Za-z][A-Za-z0-9.\-_]+`. `tenant-{uuid}` (starts with letter, 43 chars) is valid.
- `privsep` **defaults to 1** (`PVE/API2/User.pm` `token-privsep` standard option, `default => 1`): "Restrict
  API token privileges with separate ACLs (default)". Current code opts out with `--privsep=0`; the new scheme
  aligns with the PVE default but still passes `--privsep=1` explicitly.
- `pveum user delete` prunes the deleted user's ACL entries (`delete_user_acl`, `PVE/AccessControl.pm:1020-1031`)
  and its tokens (tokens live inside the user's `user.cfg` entry). Migration therefore needs no separate ACL/token
  cleanup for the legacy user.
- **Privsep tokens cannot exceed their owning user.** `PVE/RPCEnvironment.pm` `permissions()` doc comment
  (:133-135): "priv-separated: permissions for owning user are calculated and **intersected** with those of
  token"; implementation at :98-107 filters token privs to those the user also holds, per path.
  (`AccessControl.pm::roles()` :1792-1794 explicitly does NOT filter — only the `$rpcenv->permissions()` check
  does.) Consequence: the shared user must carry the four `(path, role)` grants as a ceiling; per-token grants
  select within it.

## Live verification (owner cluster `uk-home1`, PVE 9.2.10, 2026-08-16 — read-only)

Checked over SSH against the owner's 5-node cluster (nodes thinkstation1/nuc1/optiplex1-3):

- `pveum help user token add` shows `--privsep <boolean> (default=1)`; `pveum help acl modify` shows the
  `--tokens <string>` parameter — both spec mechanisms present in the deployed CLI, and `pveum acl list`
  already shows working `type: "token"` grants on this cluster (unrelated `root@pam!ansible` etc.).
- A legacy managed user `uptrakit-{tenant_uuid}@pve` exists with token `uptrakit` at `privsep: 0` — the
  migration path (§ 4) will fire on this deployment.
- **Legacy residue grant found:** besides the four current `(path, role)` grants, the legacy user carries a
  stale `PVEAuditor` on `/` from the pre-custom-roles scheme. Phase 2's `pveum user delete` removes all of the
  user's grants including such residue — migration must delete the **user**, never enumerate-and-remove
  individual grants, precisely so unknown residue cannot survive.
- `pvesh get /cluster/status --output-format json` cluster row: `{"id":"cluster","name":"uk-home1",
"type":"cluster"}` — the `name` extraction key for § 5 confirmed; this deployment's config name becomes
  `pve-uk-home1`.
- An unrelated manually-created token named `uptrakit` exists on a third-party user
  (`…@pocket-id!uptrakit`) — the new exact-name checks (`uptrakit@pve`, `uptrakit-{our_uuid}@pve`) cannot
  collide with it; the deleted first-match prefix scan could have been confused by adjacent naming, another
  reason it dies.
- `pveum acl list --output-format json` entry shape (`type`/`ugid`/`roleid`/`path`/`propagate`) recorded as
  the fixture shape for scripted-executor tests.

## Design

### 1. Identity and naming (`pve_setup.rs`)

Constants become:

```rust
const PVE_USER: &str = "uptrakit@pve";
const PVE_TOKEN_PREFIX: &str = "tenant-";
```

`pve_user_realm(tenant_id)` is deleted; new `pve_token_id(tenant_id) -> String` returns
`tenant-{tenant_uuid}`, and `pve_full_token_id(tenant_id)` returns `uptrakit@pve!tenant-{tenant_uuid}`.
The stored `api_token` config value keeps the existing `USER@REALM!TOKENID=SECRET` wire format
(`uptrakit@pve!tenant-{uuid}=SECRET`), which already passes `is_valid_pve_token` unchanged.

Provisioning sequence (execute phase, idempotent):

1. `ensure_pve_roles` — unchanged role definitions (`UptrakitAudit`, `UptrakitProtection`, `UptrakitScaling`)
   via `pveum role add … 2>/dev/null; pveum role modify …`.
2. `pveum user add 'uptrakit@pve' --comment 'Uptrakit managed user' 2>/dev/null || true` — no password ever
   set; a PVE-realm user without a password cannot authenticate interactively, tokens are the only credential.
3. `pveum user token add 'uptrakit@pve' 'tenant-{uuid}' --privsep=1 --output-format json` — secret parsed by
   the existing `parse_token_value`.
4. `ensure_pve_acls` rewritten to grant at **both levels** (idempotent, both re-runnable):
   - **user ceiling** (tenant-independent, same four `(path, role)` pairs as today):
     `pveum acl modify {path} --users 'uptrakit@pve' --roles {role}`;
   - **per-token** (selects within the ceiling): `pveum acl modify {path} --tokens
'uptrakit@pve!tenant-{uuid}' --roles {role}`.

   Effective token privileges = intersection(user grants, token grants) = exactly the four pairs (see
   Verified PVE facts). Widening any role's privileges later requires only the role definition change —
   both grant levels reference roles by name.

`regenerate_pve_api_token` targets `uptrakit@pve!tenant-{uuid}` (`token remove … || true` then `token add`)
and **re-runs `ensure_pve_acls` afterwards** — token deletion may prune that token's ACL entries, so re-granting
after regeneration is mandatory, not belt-and-braces.

`check_pve_token_exists` is replaced by `check_pve_state(executor, tenant_id) -> PveCredentialState`, a
struct (not an enum — "our token exists" and "legacy user present" co-occur during migration phase 1→2):

```rust
pub struct PveCredentialState {
    pub user_exists: bool,          // uptrakit@pve present in `pveum user list`
    pub our_token_exists: bool,     // tenant-{uuid} present in `pveum user token list 'uptrakit@pve'`
                                    // (non-zero exit ⇒ user absent ⇒ false)
    pub legacy_user: Option<String>, // uptrakit-{our_tenant_uuid}@pve if present (drives migration;
                                    // other tenants' legacy users ignored — no cross-tenant scanning)
}
```

`PveTokenStatus`, `OwnedByOtherTenant`, and the first-match loop are deleted.

### 2. Connect phase becomes read-only (`infrastructure/core` + `bootstrap.rs`)

`HostLifecycle` (`crates/plugins/infrastructure/core/src/roles.rs:817`) gains a read-only probe:

```rust
async fn probe_host(
    &self,
    ctx: &InfraPluginContext<'_>,
    executor: &dyn RemoteExecutor,
    host_id: Uuid,
    host_name: &str,
) -> Result<InfraProbeResult>;
```

`InfraProbeResult { detected: bool, planned_actions: Vec<String> }` — must carry enough for the review step
to display "will create PVE API token"; the plan may extend the struct (`#[non_exhaustive]` per coding
standards) but not reduce it. Proxmox is the sole implementor (workspace grep confirms one
`impl HostLifecycle`); the method nevertheless gets a default implementation returning
`InfraProbeResult { detected: false, planned_actions: vec![] }` so the trait stays additive for future infra
plugins.

- `detect_infra_plugins` (`bootstrap.rs:375-425`) calls `probe_host` instead of `on_host_bootstrapped`. The
  proxmox probe runs only `detect_pve_node` (`command -v pveversion`), `detect_pve_node_name`, and
  `check_pve_state` — no `pveum` mutation of any kind.
- `on_host_bootstrapped` keeps its provisioning role and is called exactly once, from `collect_infra_results`
  in the execute phase. The double-run disappears.
- The bootstrap wizard's "non-destructive connect" documentation claim becomes true.

### 3. Independent `pve_setup` skip (`bootstrap.rs`)

`setup_sudoers_and_plugins` splits into its two concerns; the execute phase gates each on its own flag:

```rust
let sudoers_content = if skip_sudoers { None } else { generate_and_install_sudoers(…).await? };
let infra_results = if skip_pve { Vec::new() } else { collect_infra_results(…).await? };
```

Review-step planned actions already include the `pve_setup` action id (`bootstrap.rs:508`); its description is
updated to name the real effect ("create PVE user `uptrakit@pve` + per-tenant API token").

### 4. Two-phase migration from the legacy scheme

New agent-local migration (contributed via the plugin's `agent_migrations`,
`proxmox/src/agent/migration.rs`) adds `legacy_pve_user: Option<String>` to `proxmox_host_state`.

- **Phase 1** (any sync or bootstrap that finds `LegacyUser(name)` for our tenant): provision the new scheme in
  full (user, token, per-token ACLs — roles are shared and unchanged), emit the `PluginConfigReport` with the
  new token, store `legacy_pve_user = Some(name)`. The legacy user keeps working until phase 2 — no window
  where the stored config references dead credentials.
- **Phase 2** (subsequent sync, when `pve_plugin_config_id` is `Some` — i.e. the controller confirmed the
  config via the existing `on_plugin_config_reported` callback — and `legacy_pve_user` is `Some`):
  `pveum user delete '{legacy_user}'` (prunes its token and ACLs per verified PVE behavior — including
  pre-custom-roles residue such as the stale `PVEAuditor` on `/` observed on the live cluster; delete the
  user, never enumerate individual grants), clear `legacy_pve_user`. Deletion failure logs a warning and
  retries next sync; it never blocks the sync.

Fresh bootstraps on clean clusters never enter migration. `reconcile_pve_config` keeps its cross-node
convergence role; with per-cluster naming (next section) its `max()` disagreement path becomes a rare
safety net rather than the expected path.

### 5. Per-cluster config naming

The reported config name becomes `pve-{cluster_name}` where `cluster_name` is the `name` of the
`type == "cluster"` row of `pvesh get /cluster/status` (the command `detect_pve_cluster_nodes` already parses);
standalone nodes (no cluster row) fall back to `pve-{node_name}`. `find_or_create_default_plugin_config`'s
`(tenant_id, plugin_type, name)` idempotency key then deduplicates cluster nodes naturally. Existing
`pve-{host_id}`-named rows are left alone (rename is not attempted; the migration path reports under the new
name and `reconcile_pve_config` converges nodes onto the confirmed id).

### 6. Emit-site fixes (`agent/plugin.rs`)

Both `PluginConfigReport` emit sites (`agent/plugin.rs:123-130`, `:210-217`) are rewritten to serialize a
`ProxmoxConfig` value (`serde_json::to_value(ProxmoxConfig { api_url, api_token, verify_tls: true,
node_filter: vec![] })`) instead of hand-built `json!` maps — the struct is the schema, so the
`verify_ssl`/`verify_tls` class of drift becomes unrepresentable. This absorbs and supersedes the standing
2026-07-12 fix spec.

### 7. Deletions and hygiene

- `verify_pve_privileges` (and the test-local copies of its ACL predicates) deleted.
- `PveTokenStatus::OwnedByOtherTenant` and first-match scan deleted.
- Sole remaining sentinel semantics: `check_pve_state` list-command failures degrade to the all-absent state
  (`user_exists: false, our_token_exists: false, legacy_user: None`, matching today's NotFound degradation),
  letting the creation attempt surface the real failure.
- Bootstrap-without-`tenant_id` gains a user-visible summary line ("PVE detected; API credential setup skipped:
  no tenant context"), matching the sync path's existing summary emission.

## Security notes

- Per-token ACLs (`--privsep=1`) mean a leaked tenant token grants only Uptrakit's three custom roles on the
  four granted paths — the intersection of its own grants and the user ceiling; it can never be widened beyond
  the ceiling by a token-level grant alone, and it never exposes another tenant's token.
- `uptrakit@pve` never gets a password; the only authentication paths to it are the per-tenant tokens.
- Token secrets continue to flow only through the existing `PluginConfigReport` → encrypted `plugin_configs`
  path (`api_token` is `.sensitive()`, `config.rs:97`); no new logging of secrets (existing tracing rules
  apply).
- Legacy-user deletion in phase 2 removes the old shared-privilege (`privsep=0`) token from the cluster.

## Testing

All new tests use `ScriptedRemoteExecutor`-style scripted executors (existing harness pattern in
`operations/bootstrap.rs` and `operations/sudoers.rs` test modules) — no live PVE. Fixture token values must
satisfy `is_valid_pve_token` (`USER@REALM!TOKENID=SECRET` — e.g.
`uptrakit@pve!tenant-0193…=secret`), per the recorded fixture-validity mistake.

1. **Command-builder tests** (`pve_setup.rs`): scripted-executor coverage for `ensure_pve_roles`,
   `ensure_pve_acls` (asserts BOTH grant levels: `--users 'uptrakit@pve'` ceiling AND `--tokens
'uptrakit@pve!tenant-{uuid}'` per-token form — dropping either level must fail the test, since a missing
   ceiling zeroes every token via the intersection rule), user + token creation with
   `--privsep=1`, `regenerate_pve_api_token` (asserts ACL re-grant runs after token re-add), `check_pve_state`
   (field matrix: fresh cluster, user-no-token, token present, legacy user present alongside token,
   non-zero-exit degradation), cluster-name extraction (cluster row present / standalone).
2. **Flow tests** (`agent/plugin.rs`): `on_host_bootstrapped` fresh-cluster provisioning; coexisting-tenant
   path (user exists, other tenants' tokens present — ours added, theirs untouched); migration phase 1
   (legacy user detected → new scheme provisioned, `legacy_pve_user` stored, legacy user NOT deleted);
   migration phase 2 (config confirmed → `pveum user delete` issued, state cleared); phase-2 deletion failure
   (warn + state retained); missing-`tenant_id` skip with summary line.
3. **Connect read-only regression** (`bootstrap.rs` / probe): scripted executor that **fails the test on any
   `pveum` command whose subcommand is not `list`/`get`** during the probe — a red-checkable guard (revert the
   probe split and the test fails), not a pin.
4. **Skip independence** (`bootstrap.rs`): `skip_actions = {"pve_setup"}` runs sudoers but no infra
   provisioning; `{"configure_sudoers"}` runs infra but writes no sudoers.
5. **Emit-site round-trip**: serialize-then-deserialize the reported config into `ProxmoxConfig`, assert
   `verify_tls` survives (red-checkable: reverting to the `json!`/`verify_ssl` form fails it).
6. **Agent migration test**: new column migration applies on top of the existing chain (follow the in-file
   `SchemaManager` shape used by existing agent migrations; do not add tip-relative `down(Some(1))` tests —
   recorded migration-test mistake).

Success and failure paths covered per binding rule (AGENTS.md "Cover new logic with tests").

## Documentation deliverables

From a repo-wide sweep (`PVEAuditor`, `VM.Monitor`, `pveum`, `uptrakit.*@pve`, `privsep`,
`verify_pve_privileges`, per-tenant-user prose, `pve-{host` naming) — every hit below is a deliverable, per
file and per clause:

1. **New ADR** via `adrs new "Shared PVE user with per-tenant privilege-separated API tokens"` — records D1–D3
   (identity model change), the pam-split rejection, and the migration strategy. Never hand-allocate the number.
2. `docs/development/proxmox-bootstrap.md` — rewrite privilege chain: new user/token/ACL model, `--privsep=1`,
   per-token grants; already-stale `useradd -m -s /bin/bash` and `PVEAuditor`/`VM.Monitor` content corrected
   against code in the same pass.
3. `docs/architecture/ssh-agent.md` — bootstrap flow section (connect now read-only, probe semantics), sync
   section ("verifies PVE privileges" → "ensures/repairs per-token ACLs"), migration two-phase description,
   file-tree entry if `pve_setup.rs` items renamed.
4. `docs/end-user/ssh-agent-bootstrap.md` — PVE section: new identity model, `pve_setup` now independently
   skippable, review step shows planned PVE actions, `PVEAuditor` mention corrected.
5. `docs/end-user/proxmox.md` — auto-provisioning section (new user/token naming, coexistence), manual-setup
   section reviewed for consistency (manual `root@pam!uptrakit` examples remain valid — manual tokens are
   user-supplied), **new Deprovisioning section** (D10): per-tenant token removal
   (`pveum user token remove 'uptrakit@pve' 'tenant-{uuid}'` — the user-level ceiling grants remain, inert,
   until last-tenant cleanup), last-tenant cleanup (`pveum user delete
'uptrakit@pve'` — removes the ceiling grants and any remaining tokens, `pveum role delete` × 3),
   legacy-scheme cleanup (`pveum user delete
'uptrakit-{tenant}@pve'` — also removes pre-custom-roles residue grants like a stale `PVEAuditor`),
   host-side cleanup (`userdel -r`, `/etc/sudoers.d/uptrakit-{user}`, installed
   helper scripts, `authorized_keys` entries), controller-side plugin-config deletion. Cross-linked from
   `docs/end-user/ssh-agent-host-management.md` (host-removal section states what is NOT cleaned up remotely
   and points here).
6. `docs/end-user/ssh-agent-host-management.md` — add the "removal does not deprovision" clause + link (sweep
   hit).
7. `docs/development/proxmox-plugin.md` — credential-model section updated (sweep hit).
8. `docs/security/sudoers-management.md` — reviewed for PVE-user mentions (sweep hit; expected no-op —
   confirm, don't assume).
9. Tracker: mark the superseded 2026-07-12 verify-tls spec/plan rows per § Supersession below.

No wire-type changes (`PluginConfigReport` shape untouched) ⇒ no `asyncapi.yaml` regen. No REST contract
changes ⇒ no `regen-api.sh`. No controller-DB migration (agent-local only).

## Out of scope / deferred

- Guest bootstrap (`operations/bootstrap_proxmox.rs`) test coverage — explicitly excluded by owner (round 1).
- Automated deprovisioning code (teardown on host removal) — docs only (D10).
- The stub `ReleaseFetcher`/`UpdateExecutor` roles on the proxmox descriptor.
- Renaming existing `pve-{host_id}` plugin-config rows.
- Any change to the three custom role definitions' privilege sets.
- Multi-controller/cluster-wide coordination beyond the existing `reconcile_pve_config` mechanism.

## Supersession

The standing tracker section "Proxmox verify_ssl → verify_tls Config-Key Fix" (spec NOT_STARTED, plan NEW) is
fully absorbed by § 6. On registration of this spec, annotate that section as superseded by this spec rather
than deleting it silently; final disposition (delete vs. keep annotated) is the owner's call at review time.

## Snapshot conformance

Binding rules touched, all satisfied: agents unprivileged/outbound-only (unchanged); no shell injection (all
new command strings interpolate only UUIDs and PVE-validated names; command builders tested); no secrets in
logs (token secret only in config report path); typed errors via `thiserror`/`rootcause` (existing
`ProxmoxError` boundary); no `#[allow]`; tests cover success+failure; wire docs unaffected (no wire change);
ADR via `adrs` CLI; `FromStr` rule not triggered (no new string-to-type parse surface — state enum is
internal); feature flags untouched. No new external dependencies (⇒ no version pins needed).
