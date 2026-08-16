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
3. **The `pve_setup` skip action is cosmetic — and the sudoers skip is half-cosmetic.** `bootstrap.rs:645-655`:
   PVE provisioning is skipped only when `configure_sudoers` is _also_ skipped; skipping `pve_setup` alone does
   nothing distinct. Mirror bug on the sudoers side: `setup_sudoers_and_plugins` writes the base sudoers file
   unconditionally (`:1335-1337`); `skip_sudoers` only nulls the returned value after the write.
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
- `pveum user token list '<user>' --output-format json` behavior confirmed: absent user → "no such user" on
  stderr + exit 255; existing user → JSON token array + exit 0. The `check_pve_state` non-zero-exit ⇒
  user-absent degradation is therefore live-verified, not assumed.
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
3. Token creation, gated on `PveCredentialState` (mirrors today's `create_or_reuse_pve_credentials`
   branching — required for multi-node clusters, where a second node's run hits an already-created token
   whose secret is unrecoverable):
   - `our_token_exists == false` → `pveum user token add 'uptrakit@pve' 'tenant-{uuid}' --privsep=1
--output-format json` — secret parsed by the existing `parse_token_value`;
   - `our_token_exists == true` + a local or cluster-peer config referencing it → reuse that config, no
     token command. Reuse is **agent-local convergence only** (a peer node cannot re-report a config whose
     secret only the creating node ever saw); the controller-side convergence mechanism is idempotent
     reporting under the per-cluster name (§ 5). No tenant filter is needed on these lookups: the SSH agent
     is a **tenant service** — one enrolled agent, one tenant, one agent-local DB (`ServiceSettings` tenant,
     `agent-ssh-runtime/src/lib.rs:525-527`), so cross-tenant rows cannot coexist in `proxmox_host_state`.
     The one real hazard is **re-enrollment to a different tenant** with stale local state; the guard for
     that is a uniform one: when `apply_settings` observes a tenant change, the agent wipes its
     Proxmox-plugin local state (`proxmox_host_state`, `proxmox_pending_matches`) — one guard at the rebind
     site instead of per-table tenant columns implying a multi-tenancy the agent does not have;
   - `our_token_exists == true` + no config anywhere → `regenerate_pve_api_token` (the stored secret is
     gone; remove + re-add). This arm is today's recovery semantic carried forward unchanged (current code
     takes it via `OwnedByTenant` + no local config); it is reached only when local + peer state genuinely
     hold no config reference, which post-upgrade rows (still carrying their legacy
     `pve_plugin_config_id`) do not trigger.
4. `ensure_pve_acls` rewritten to grant at **both levels** (idempotent, both re-runnable):
   - **user ceiling** (tenant-independent, same four `(path, role)` pairs as today):
     `pveum acl modify {path} --users 'uptrakit@pve' --roles {role}`;
   - **per-token** (selects within the ceiling): `pveum acl modify {path} --tokens
'uptrakit@pve!tenant-{uuid}' --roles {role}`.

   Effective token privileges = intersection(user grants, token grants) = exactly the four pairs (see
   Verified PVE facts). Widening any role's privileges later requires only the role definition change —
   both grant levels reference roles by name.

`regenerate_pve_api_token` targets `uptrakit@pve!tenant-{uuid}` (`token remove … || true` then `token add`)
and **re-runs `ensure_pve_acls` afterwards** — defensive: whether `pveum user token remove` prunes that token's
ACL entries is not source-verified (only whole-user deletion's pruning is, see Verified PVE facts), and the
re-grant is idempotent and cheap, so it runs unconditionally either way. If the re-grant itself fails, the
whole credential operation is treated as failed (outcome `Failed`, no config report) — a token is never
reported with unconfirmed privileges; the next sync retries via the `our_token_exists == true` + no-config
regenerate arm.

`check_pve_token_exists` is replaced by `check_pve_state(executor, tenant_id) -> PveCredentialState`, a
struct (not an enum — "our token exists" and "legacy user present" co-occur during migration phase 1→2):

```rust
#[non_exhaustive] // new-struct default per coding standards
pub struct PveCredentialState {
    pub user_exists: bool,          // uptrakit@pve present in `pveum user list`
    pub our_token_exists: bool,     // tenant-{uuid} present in `pveum user token list 'uptrakit@pve'`
                                    // ("no such user"/exit 255 = VERIFIED user absent — live-checked;
                                    // any other command failure => the fn returns Err, never a state)
    pub legacy_user: Option<String>, // uptrakit-{our_tenant_uuid}@pve if present (drives migration;
                                    // other tenants' legacy users ignored — no cross-tenant scanning)
}
```

(Same-crate construction is unaffected by `#[non_exhaustive]`; the struct is not part of a cross-crate
constructor surface.)

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
standards) but not reduce it. **Consumption path (must be wired, or the field is computed and discarded):**
`RemoteHostInfo` (`bootstrap.rs:304-309`) currently carries only `pve_detected: bool` — it gains the probe's
`planned_actions` (replacing or alongside the bool), and `build_bootstrap_actions`' `pve_setup` branch
(`bootstrap.rs:~506-517`, today a hardcoded description with `commands: vec![]`) renders them into the
review-step action entry. Proxmox is the sole implementor (workspace grep confirms one
`impl HostLifecycle`), so the method is **required — no default implementation**. A `detected: false` default
would let a future plugin implement `on_host_bootstrapped` without `probe_host`: invisible in the review step
yet still provisioning in the execute phase. Honest limits of the guard: connect and execute are separate SSH
sessions with no plan payload carried between them, so the execute phase **re-runs detection** (cheap
`command -v` + list commands) and gates provisioning on the execute-time result — the two phases agree
because they run the same read-only detection, not because a flag is transported. `probe_host` returning
`Err` in the connect phase degrades to not-detected (today's semantics); in the execute phase a detection
error on a host the review step showed as PVE-detected is a **loud bootstrap-action failure**, never a
silent skip — a silent skip would suppress the `is_pve_node` upsert and sudo grants, recreating the
unrecoverable-state bug § 3 exists to prevent.

- `detect_infra_plugins` (`bootstrap.rs:375-425`) calls `probe_host` instead of `on_host_bootstrapped`. The
  proxmox probe runs only `detect_pve_node` (`command -v pveversion`), `detect_pve_node_name`, and
  `check_pve_state` — no `pveum` mutation of any kind.
- `on_host_bootstrapped` keeps its provisioning role and is called exactly once, from `collect_infra_results`
  in the execute phase. The double-run disappears.
- The bootstrap wizard's "non-destructive connect" documentation claim becomes true.

### 3. Independent `pve_setup` skip (`bootstrap.rs`)

`setup_sudoers_and_plugins` splits into its two concerns, **preserving the infra→sudoers sudo-command merge**
(today: base sudoers written at `bootstrap.rs:1335-1337`, then `collect_infra_results` at `:1339`, then
`merge_infra_sudo_commands` at `:1341-1343` **rewrites** the file to append PVE's `pct exec` / `qm guest exec`
grants — a naive two-independent-lets split would silently drop those grants). New order: infra first, one
sudoers write consuming its `sudo_commands`:

**`skip_pve` gates only credential provisioning — never detection or sudo collection.** Detection state
(`upsert_host_state` with `is_pve_node`/node name) and `collect_pve_sudo_commands` (`pct exec` /
`qm guest exec` grants) must run regardless of the skip: gating them would (a) leave the host permanently
unrecognized as a PVE node — `on_host_synced` early-returns without an `is_pve_node` row and nothing else
ever creates one, making the skip unrecoverable short of re-bootstrapping (strictly worse than today's
cosmetic skip), and (b) silently break guest bootstrap by dropping its sudo grants. The skip flag is
therefore threaded INTO the infra call as "provision credentials or not", not used to suppress the call:

```rust
// always runs: detection upsert + sudo collection; credentials gated by skip_pve inside
let infra_results = collect_infra_results(…, provision_credentials: !skip_pve).await?;
let sudoers_content = if skip_sudoers {
    None
} else {
    // single write: base entries + infra_results[].sudo_commands merged before rendering
    generate_and_install_sudoers(…, &infra_results).await?
};
```

(Exact parameter plumbing — flag on `InfraPluginContext` vs. an argument — is a plan-time choice; the
contract is fixed: skip ⇒ no `pveum user/token/acl/role` mutation, everything else unchanged. A user
skipping `pve_setup` to supply a manual `root@pam` token keeps a fully functional PVE host.)

This also fixes the sudoers twin of Problem #3: today the base sudoers file is written unconditionally inside
`setup_sudoers_and_plugins` (`:1335-1337`) and `skip_sudoers` only nulls the _returned_ value — the restructure
makes the skip actually suppress the write, deliberately (see Testing #4), not incidentally.

Review-step planned actions already include the `pve_setup` action id (`bootstrap.rs:508`); its description is
updated to name the real effect ("create PVE user `uptrakit@pve` + per-tenant API token").

### 4. Two-phase migration from the legacy scheme

New agent-local migration (contributed via the plugin's `agent_migrations`,
`proxmox/src/agent/migration.rs`) adds **two** columns to `proxmox_host_state`: `legacy_pve_user:
Option<String>` and `new_pve_plugin_config_id: Option<String>` — **plus the matching `Model` fields in
`proxmox/src/agent/entity.rs`** (SeaORM `DeriveEntityModel` requires them) and updates to every
constructor/upsert of that Model (`agent/db_ops.rs`, any `ActiveModel`/struct literals in `agent/plugin.rs`
and tests; same-task `cargo check --workspace --all-targets` guards the literal-construction blast radius).

**Why a distinct `new_pve_plugin_config_id` column is load-bearing:** on the live deployment every
`proxmox_host_state` row _already_ has `pve_plugin_config_id = Some(<legacy pve-{host_id} config>)` from the
original bootstrap — a phase-2 gate keyed on "`pve_plugin_config_id` is `Some`" is satisfied _before
migration even starts_ and would delete the legacy user in the same sync that first stored
`legacy_pve_user`, killing the token the live host mappings still reference. Destructive migration must
never key on a pre-existing field.

**Report↔host correlation is a prerequisite:** `on_plugin_config_reported` (`agent/plugin.rs:300-325`)
today stamps "the first PVE host without a config ID" — its `request_id` parameter is unused
(`_request_id`). During migration no host is "without a config ID" (all hold the legacy id), so the new
config id would never land and `reconcile_pve_config` would re-affirm the legacy id forever. **The seam is
runtime-side, with no struct change:** both report send sites already hold the host id
(`send_infra_plugin_reports(bg_tx, host_id, …)` at `surface_runtime.rs:~1899`; the sync task resolves
`host_id` before `sync_execute`). There is no pending-ack bookkeeping today, so the plan adds one: an
`Arc<parking_lot::Mutex<HashMap<request_id, (host_id, plugin_type)>>>` constructed alongside the infra
bundles and threaded into both the surface-runtime context and the ack-handling support struct (guards
dropped before any `.await`). The runtime stamps entries at send time from the host id **it already holds**
(a plugin-supplied id could disagree with the running task — mismatch stays unrepresentable), records
`plugin_type` so an ack is delivered only to the owning plugin (today `runtime_support.rs:~296` fans acks
out to every infra bundle — a future second `HostLifecycle` impl's ack must not write proxmox state), and
**removes entries on failure acks too** (the current handler consumes only success acks, which would leak
map entries). The reworked callback is `on_plugin_config_reported(db, config_id, host_id)`; it writes
`new_pve_plugin_config_id` on that host's row. The positional scan is deleted for ALL paths. `PluginConfigReport`,
the wire payload, and asyncapi are all untouched. In-memory pending state lost to an agent restart is covered
by the regenerate rule below, so no pending-request column is needed.

**Ack-loss rule (regenerate, never "re-emit"):** reports are fire-and-forget; a dropped WS frame or restart
loses the ack. The new token's secret exists only in the provisioning run's memory and in the controller's
encrypted store — **the agent persists no secret and can never re-send the same report**. When a sync
observes `legacy_pve_user: Some` and **no cluster row** holds `new_pve_plugin_config_id` (cluster-scoped,
matching the phase gates) while the new-scheme token exists, it **regenerates** (`token remove` + `token
add` — a fresh secret), proves on-node, and reports again; the controller row is idempotent on
`(tenant_id, plugin_type, name)`, so the churn is one bounded update per lost ack, never a permanent block.

**Concurrency: per-cluster lock, in scope (not an assumption):** sync executions are `tokio::spawn`ed per
surface request (`surface_runtime.rs:~1464-1475`) — two nodes of one cluster CAN sync concurrently, and
`pveum token add/remove` against one shared `user.cfg` from two sessions races. The credential + migration
section of the flow runs under an agent-local async mutex keyed by cluster name (a
`Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>`-style registry or equivalent; plan picks the shape).
No controller involvement.

**Cluster-scoped, not per-row:** the legacy user, the new token, and the per-cluster config are all
cluster-wide objects; per-row gating would let node A's delete break nodes B–E, whose rows still reference
legacy config ids. Phases are therefore evaluated against the whole cluster (all `proxmox_host_state` rows
whose `pve_node_name` appears in the current `/cluster/status` membership — rows with NULL `pve_node_name`
are outside the cluster set and never gate or receive promotion):

- **Phase 1** (first sync/bootstrap of any cluster node that finds `legacy_user: Some(name)`): provision the
  new scheme in full (user, token, per-token ACLs — roles shared and unchanged); **prove on-node** — the
  proof runs over the existing SSH executor against the node itself
  (`curl -sk -H 'Authorization: PVEAPIToken=uptrakit@pve!tenant-{uuid}={secret}'
https://localhost:8006/api2/json/version`): same transport as everything else, no agent-side HTTP, no
  DNS/TLS/SSRF coupling, and it proves the token itself (`-k` is sound here — the proof targets token
  validity on localhost, not endpoint trust); on success, report under the per-cluster name and store
  `legacy_pve_user = Some(name)` on every cluster row. Legacy credentials keep working throughout.
- **Ack**: correlated callback writes `new_pve_plugin_config_id` (the reporting host's row). **This column,
  once written by a correlated ack, is never cleared** — it is the durable "new-scheme confirmed" marker;
  promotion copies it into `pve_plugin_config_id`, and migration completion clears only `legacy_pve_user`.
- **Phase 2** (a later sync of any cluster node) requires ALL of: `legacy_pve_user` stored; some cluster row
  holds `new_pve_plugin_config_id`; and **non-secret evidence read successfully in this run** — the agent
  holds no token secret, so the proof is structural: `pveum user token list 'uptrakit@pve'` contains
  `tenant-{uuid}` AND `pveum acl list` shows both grant levels (the live proof of the secret itself happened
  once, at phase 1, before the report; the correlated ack proves the controller stored it). Then, as one
  step: **promote all cluster rows** (`pve_plugin_config_id = new id`), `pveum user delete '{legacy_user}'`
  (prunes its token and ACLs per verified PVE behavior — including pre-custom-roles residue such as the
  stale `PVEAuditor` on `/` observed live; delete the user, never enumerate grants), clear `legacy_pve_user`
  on all cluster rows. Any step failing → warn + retry next sync; a bounded attempt counter (the
  `MAX_MATCH_ATTEMPTS` pattern from the pending-match drain) escalates the summary line after repeated
  failures so "stuck" is distinguishable from "pending" — the legacy credential stays intact until the new
  one is confirmed, so stuck means stuck-but-working.
- **Recovery arm** (legacy user **verified absent** — successful `pveum user list` without it, never a
  degraded read — while `legacy_pve_user` is stored — e.g. phase 2 half completed or an operator deleted it
  manually): if a new config id is known anywhere in the cluster → promote all rows + clear
  `legacy_pve_user`; otherwise fall through to the normal § 4b branches (token exists →
  reconcile/regenerate; absent → provision fresh). No row is ever left pointing at a deleted credential
  without a path forward.

**Read integrity gates every destructive/terminal step:** `check_pve_state` returns
`Result<PveCredentialState>` — a state struct is produced only from successful list reads (the one
live-verified special case: `pveum user token list` failing with "no such user"/exit 255 IS a verified
"shared user absent"). Any other command failure is an `Err`: the provisioning/creation path may degrade it
to a creation attempt (today's semantics), but phase 2, the recovery arm, and promotion all require `Ok` —
a transient SSH hiccup must never read as "legacy user gone" and strand a live `privsep=0` token with the
migration state wiped.

**The reuse arm requires positive new-scheme evidence — unconditionally, not just during migration:** § 1
step 3's reuse arm counts only a peer row's `new_pve_plugin_config_id` (the never-cleared ack marker) as "a
config referencing the new token". A bare `pve_plugin_config_id` never satisfies it: on pre-upgrade rows it
is the legacy id (non-creator nodes would silently stay legacy), and in the split-agent case — two SSH
agents managing disjoint node sets of one cluster — agent 2 never migrates, so after agent 1's phase 2 its
rows reference a dead token; with this rule agent 2's next sync finds no new-scheme evidence, regenerates,
and reports under the same per-cluster name, converging both agents onto the same controller row.

**Aftermath (documented, not automated):** controller-side legacy `pve-{host_id}` config rows still exist
after migration, holding a dead token; the deprovisioning/migration doc directs deleting them and
re-selecting the per-cluster config in any surface flow that had the old one picked.

Fresh bootstraps on clean clusters use the same correlation path (`new_pve_plugin_config_id` written by the
ack, promoted immediately — no legacy user to delete). `reconcile_pve_config` keeps its cross-node
convergence role; with per-cluster naming (§ 5) its `max()` disagreement path becomes a rare safety net.

**Nothing drives sync automatically** — it is a user-triggered surface interaction. The migration therefore
progresses one step per manual sync (typically two syncs of any one cluster node) and a never-synced
deployment stays on the legacy scheme indefinitely, which is harmless. The end-user doc gets a short
migration runbook: sync once (provisions + reports), sync again (promotes + deletes legacy), what the
pending/stuck summary lines look like.

**Rollout sequencing (plan directive):** the reversible fixes (read-only probe, skip semantics, emit-site
fix, dead-code deletion, docs, test harness) land before the identity/migration flip; the migration is the
last change to ship, so every supporting mechanism (correlation, on-node proof, regenerate-on-ack-loss,
per-cluster lock) is already in place and tested when the first destructive `pveum user delete` can fire.

### 4b. Rewritten sync flow (`on_host_synced`)

Today's sync (`agent/plugin.rs:142-294`) gates steps 2-3 entirely on `PveTokenStatus::OwnedByTenant` — that
enum dies, so the flow is respecified (this is the highest-risk call-site rewrite in the spec, not an
implementation detail):

1. Detect node name; upsert host state (unchanged).
2. No `tenant_id` in context → summary line + return. Deliberate behavior change: today sync has no early
   return here (only the credential logic deeper in gates on tenant, warn-only) — step 1's upsert still runs
   before the return, so node-name detection is preserved.
3. `check_pve_state`. The per-tenant token is the ownership signal; there is no user-level ownership gate
   anymore.
4. Branch on the state struct:
   - `our_token_exists == true` → `reconcile_pve_config` to converge `pve_plugin_config_id` across peers; if
     no config exists anywhere (local or peers) → `regenerate_pve_api_token` (secret unrecoverable) + report;
     then `ensure_pve_acls` (both levels — the sync-time repair, D9). The regenerate arm is today's recovery
     semantic unchanged, and post-upgrade rows still carry their legacy config reference, so upgrading alone
     never triggers it.
   - `our_token_exists == false` → full provisioning sequence (§ 1) + report. Covers both a fresh cluster
     and a token someone deleted out-of-band.
5. Migration bookkeeping: `legacy_user: Some(name)` from step 3 → store it (all cluster rows). Phase 2 fires
   only under the § 4 cluster-scoped gate (legacy stored + correlated `new_pve_plugin_config_id` on some
   cluster row + successfully-read structural evidence), and the § 4 recovery arm handles a legacy user
   verified absent.
6. Summary lines report which arm ran (provisioned / reused / regenerated / migrated / skipped), feeding the
   same accumulator the sync path has today (`lines`, `agent/plugin.rs:157`).

`on_host_bootstrapped` (execute phase) shares steps 3-6 through a common helper so bootstrap and sync cannot
drift; its outcome feeds `BootstrapInfraResult.summary_lines` (§ 7).

### 5. Per-cluster config naming

The reported config name becomes `pve-{cluster_name}` where `cluster_name` is the `name` of the
`type == "cluster"` row of `pvesh get /cluster/status` (the command `detect_pve_cluster_nodes` already parses);
standalone nodes (no cluster row) fall back to `pve-{node_name}`. `find_or_create_default_plugin_config`'s
`(tenant_id, plugin_type, name)` idempotency key then deduplicates cluster nodes naturally. Existing
`pve-{host_id}`-named rows are left alone (rename is not attempted; the migration path reports under the new
name and `reconcile_pve_config` converges nodes onto the confirmed id).

Two properties of the cluster-scoped config, stated so they are deliberate rather than discovered:

- **`api_url` is the provisioning node's FQDN** (`resolve_pve_api_url` → `https://{fqdn}:8006`). One
  cluster-wide config means the integration talks to that one node; if it goes down, the integration is down
  until the user edits the config's URL (any cluster node serves the full API). Reuse-arm nodes never
  re-report, so the URL does not flip between nodes on every sync; only regeneration re-points it. A
  failover endpoints list is a deliberate non-goal here (see Out of scope) — the end-user doc states the
  single-endpoint behavior and the manual-edit remedy.
- **Cluster-wide `pveum` work is serialized by an in-scope per-cluster lock** (§ 4 Concurrency). Sync
  executions are `tokio::spawn`ed per surface request — concurrent syncs of two cluster nodes are possible
  today, so this is a requirement, not an assumption.

### 6. Emit-site fixes (`agent/plugin.rs`)

Both `PluginConfigReport` emit sites (`agent/plugin.rs:123-130`, `:210-217`) are rewritten to serialize a
`ProxmoxConfig` value (`serde_json::to_value(ProxmoxConfig { api_url, api_token, verify_tls: true,
node_filter: vec![] })`) instead of hand-built `json!` maps — the struct is the schema, so the
`verify_ssl`/`verify_tls` class of drift becomes unrepresentable. This absorbs and supersedes the standing
2026-07-12 fix spec.

Two clarifications carried from review: (a) because `verify_tls` has `#[serde(default = "default_true")]`
and no `deny_unknown_fields`, a **round-trip test cannot go red** on the old emit form (the dropped
`verify_ssl` key falls back to the same `true`) — the test must assert on the serialized **key set**
(`verify_ssl` absent, `verify_tls` present), see Testing #5; (b) the emitted VALUE stays hardcoded `true` —
whether auto-provisioned configs should report `verify_tls: false` for PVE's default self-signed certs
remains the deferred value question from the superseded spec (migration is unaffected: the § 4 proof runs
on-node with `curl -k`, independent of this setting; users with self-signed certs edit the config, as the
end-user doc already describes).

### 7. Deletions and hygiene

- `verify_pve_privileges` (and the test-local copies of its ACL predicates) deleted.
- `PveTokenStatus::OwnedByOtherTenant` and first-match scan deleted.
- Read-failure semantics are split by consequence (§ 4 Read integrity): `check_pve_state` returns
  `Result<PveCredentialState>`; the provisioning/creation path may treat `Err` as "attempt creation"
  (today's NotFound degradation), while phase 2, the recovery arm, and promotion hard-require `Ok` — a
  degraded read must never look like "verified absent" to a destructive step.
- Bootstrap-without-`tenant_id` gains a user-visible summary line ("PVE detected; API credential setup skipped:
  no tenant context"). Carrier: `BootstrapInfraResult` (`agent_infra.rs:265`, currently exhaustive with no
  summary field) gains `summary_lines: Vec<String>` mirroring `SyncInfraResult.summary_lines`
  (`agent_infra.rs:300-301`); the struct derives `Default`, but any literal construction sites must be updated
  in the same task (workspace-wide `cargo check --all-targets` guard), and the bootstrap execute path forwards
  the lines into the wizard's result output the same way sync forwards its summary. The plumbing below it:
  the credential helper returns a typed outcome (`Provisioned` / `Reused` / `Regenerated` /
  `SkippedNoTenant` / `MigrationPending` / `Failed` — exact variant set at plan time, `#[non_exhaustive]`)
  instead of today's ambiguous `(None, None)` tuple, and `on_host_bootstrapped` maps outcomes to summary
  lines. `Failed` covers the currently warn-only failure branches (invalid tenant-id format, `pveum` list
  failure, regeneration failure) so every credential-path outcome — not just the missing-tenant skip — is
  visible in the flow summary.

## Security notes

- Per-token ACLs (`--privsep=1`) mean a leaked tenant token grants only Uptrakit's three custom roles on the
  four granted paths — the intersection of its own grants and the user ceiling; it can never be widened beyond
  the ceiling by a token-level grant alone, and it never exposes another tenant's token. **Honest framing:**
  since every tenant today receives the identical four grants, effective privileges match what the legacy
  privsep=0 scheme granted — the change does not shrink any tenant's blast radius now. What it buys:
  per-tenant revocation handles, per-tenant secrets, and the structural room to narrow one tenant's token
  grants below the ceiling later without touching the others.
- The user ceiling is a single shared object: deleting the user's grants (or the user) silently zeroes every
  tenant's token — no auth error, just empty API results. The deprovisioning doc therefore gates the
  last-tenant `pveum user delete` on an explicit emptiness check (`pveum user token list 'uptrakit@pve'`
  shows no remaining `tenant-*` tokens), and sync's `ensure_pve_acls` re-repairs a deleted ceiling on the
  next run.
- `uptrakit@pve` never gets a password; the only authentication paths to it are the per-tenant tokens.
- Token secrets continue to flow only through the existing `PluginConfigReport` → encrypted `plugin_configs`
  path (`api_token` is `.sensitive()`, `config.rs:97`); no new logging of secrets (existing tracing rules
  apply).
- Legacy-user deletion in phase 2 removes the old shared-privilege (`privsep=0`) token from the cluster.

## Testing

All new tests use a scripted `RemoteExecutor` test double — no live PVE. The double is **extracted into
`uptrakit-command` behind an additive `test-support` feature** (the trait's home crate,
`crates/shared/command/src/remote_executor.rs`; `test-support` follows the established repo idiom —
`controller-runtime`, `tracing-init`, `service-sdk` all ship one) rather than minting a third private copy of
the pattern already duplicated in `operations/bootstrap.rs` and `operations/sudoers.rs` test modules. This
spec's tests consume the shared double; migrating those two existing private copies onto it is a follow-up,
not in scope. **Not a verbatim lift**: the existing copies use `std::sync::Mutex` — tolerable only because
they are `#[cfg(test)]`-local; a `test-support` feature module is library code, so the shared double must use
`parking_lot::Mutex` per the synchronous-locks-in-async rule. Fixture token values must
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
   **phase-2 gate matrix (cluster-scoped)** — no delete while `new_pve_plugin_config_id` is `None`
   everywhere even though every pre-existing `pve_plugin_config_id` is `Some` (red-checkable: keying the
   gate on the old column fails it — pins the exact live-deployment hazard), no delete while the structural
   evidence read (token present + both grant levels) fails or errors, delete + **promotion of ALL cluster
   rows** when the gate holds (red-checkable: per-row
   promotion leaves a sibling row on the legacy id and fails the assertion); phase-2 failure (warn + state
   retained + attempt counter escalates the summary past the threshold); **recovery arm** — legacy user
   absent while `legacy_pve_user` stored → promote-if-known / fall-through-if-not, no delete issued;
   **regenerate-on-ack-loss** — stored legacy + NO cluster row holding `new_pve_plugin_config_id` + token
   present regenerates (fresh secret), proves on-node, and reports again (red-checkable: dropping the
   trigger leaves the ack loss permanent; a "re-emit the same report" impl is unimplementable — the agent
   persists no secret); **read-integrity gating** — a failed (non-"no such user") list read must NOT fire
   the recovery arm or phase 2 (red-checkable: degrading `Err` to all-absent wipes migration state on a
   transient hiccup); **per-cluster lock** — two concurrent syncs of same-cluster nodes serialize the
   credential section (red-checkable with a scripted executor that flags interleaved `pveum` writes);
   **reuse-arm evidence rule (unconditional)** — a bare `pve_plugin_config_id` never satisfies the reuse
   check, only a peer's never-cleared `new_pve_plugin_config_id` does (covers both pre-upgrade rows and the
   split-agent cluster); `on_plugin_config_reported` correlation (ack for host X writes
   `new_pve_plugin_config_id` on X's row, not "first host without a config id" — red-checkable against the
   positional scan); missing-`tenant_id` skip with summary line; **tenant-rebind wipe** — `apply_settings`
   observing a changed tenant clears `proxmox_host_state`/`proxmox_pending_matches` (guards the stale-state
   hazard that replaced the abandoned per-table tenant column).
3. **Connect read-only regression** (`bootstrap.rs` / probe): scripted executor that **fails the test on any
   `pveum` command whose subcommand is not `list`/`get`** during the probe — a red-checkable guard (revert the
   probe split and the test fails), not a pin.
4. **Sudoers/infra composition** (`bootstrap.rs`): (a) **neither skipped** — final sudoers content contains
   the PVE-contributed `pct exec` / `qm guest exec` entries (red-checkable: dropping the merge into
   `generate_and_install_sudoers` fails it — this guards the regression a naive split would ship);
   (b) `skip_actions = {"pve_setup"}` runs sudoers but no infra
   provisioning; (c) `{"configure_sudoers"}` runs infra but writes no sudoers — (c) fixes a live bug (today
   the base file write is unconditional and the skip only nulls the returned value), so it must be asserted
   on the file-write path, not just the return value.
5. **Emit-site key-set assertion**: assert the serialized report config's key set directly — `verify_ssl`
   absent, `verify_tls` present (a value round-trip CANNOT go red against the old form: the serde default
   heals the dropped key to the same `true`, see § 6). Red-checkable: reverting to the `json!`/`verify_ssl`
   emit fails the key-set check.
6. **Agent migration test**: new column migration applies on top of the existing chain (follow the in-file
   `SchemaManager` shape used by existing agent migrations — note this multi-struct single-file layout is a
   deliberate plugin-local exception to the shared-db one-file-per-migration convention, not the rule to copy
   elsewhere; do not add tip-relative `down(Some(1))` tests —
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
5. `docs/end-user/proxmox.md` — auto-provisioning section (new user/token naming, coexistence, and the
   single-endpoint behavior: the auto-provisioned config's `api_url` points at the node that created the
   token; if that node is down, edit the config's URL to any live cluster node), manual-setup
   section reviewed for consistency (manual `root@pam!uptrakit` examples remain valid — manual tokens are
   user-supplied), **new Deprovisioning section** (D10): per-tenant token removal
   (`pveum user token remove 'uptrakit@pve' 'tenant-{uuid}'` — the user-level ceiling grants remain, inert,
   until last-tenant cleanup), last-tenant cleanup (**gated on an explicit emptiness check** — `pveum user
   token list 'uptrakit@pve'` must show no remaining `tenant-*` tokens, because deleting the shared user
   zeroes every remaining tenant's token silently; then `pveum user delete
'uptrakit@pve'` — removes the ceiling grants and any remaining tokens, `pveum role delete` × 3),
   legacy-scheme cleanup (`pveum user delete
'uptrakit-{tenant}@pve'` — also removes pre-custom-roles residue grants like a stale `PVEAuditor`),
   host-side cleanup (`userdel -r`, `/etc/sudoers.d/uptrakit-{user}`, installed
   helper scripts, `authorized_keys` entries), controller-side plugin-config deletion, **plus a migration
   runbook subsection** (§ 4: sync once to provision + report, sync again to promote + delete legacy; what
   the pending vs. stuck summary lines look like; aftermath — delete the dead legacy `pve-{host_id}`
   configs controller-side and re-select the per-cluster config where the old one was picked; a never-synced
   deployment stays on the legacy scheme, harmless; **split-agent clusters** — when two SSH agents manage
   disjoint node sets of one cluster, sync each agent's nodes after migration so the second agent
   regenerates onto the shared per-cluster config). Cross-linked from
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
- Migrating the two existing private scripted-executor copies (`bootstrap.rs`/`sudoers.rs` test modules) onto
  the new shared `uptrakit-command` test-support double (this spec creates the shared double and uses it for
  all NEW tests; rewriting existing green tests is a follow-up).
- A failover endpoints list on `ProxmoxConfig` (cluster-scoped config keeps a single `api_url` pinned to the
  provisioning node; manual URL edit is the documented remedy — § 5).

## Supersession

The standing tracker section "Proxmox verify_ssl → verify_tls Config-Key Fix" (spec NOT_STARTED, plan NEW) is
fully absorbed by § 6. On registration of this spec, annotate that section as superseded by this spec rather
than deleting it silently; final disposition (delete vs. keep annotated) is the owner's call at review time.

## Snapshot conformance

Binding rules touched, all satisfied: agents unprivileged/outbound-only (unchanged); no shell injection (all
new command strings interpolate only UUIDs and PVE-validated names; command builders tested); no secrets in
logs (token secret only in config report path); typed errors via `thiserror`/`rootcause` (existing
`ProxmoxError` boundary); no `#[allow]`; tests cover success+failure; wire docs unaffected (no wire change);
ADR via `adrs` CLI; `FromStr` rule not triggered (no new string-to-type parse surface — state struct is
internal); feature flags additive only (new `test-support = []` on `uptrakit-command`). The § 4 migration
proof runs **on-node over the existing SSH executor** (curl against `https://localhost:8006`) — the SSH
agent opens no new outbound HTTP connection, so the `SsrfSafeResolver` rule is not newly triggered; the
proof's secret travels only inside the SSH session and is never logged (command string carries the token —
the executor's tracing must log the command name/length, not the full line, per existing no-secrets-in-logs
handling of token-bearing `pveum` commands). No new external dependencies (⇒ no version pins needed).
