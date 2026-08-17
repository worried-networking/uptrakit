# 0044 — Shared PVE user with per-tenant privilege-separated API tokens

Date: 2026-08-17

## Status

Accepted

## Context

The Proxmox VE infrastructure plugin (`crates/plugins/infrastructure/proxmox`) provisions PVE API credentials
during agent-ssh host bootstrap and sync (`on_host_bootstrapped`/`on_host_synced` in
`crates/plugins/infrastructure/proxmox/src/agent/plugin.rs`) so the controller can drive the Proxmox plugin's REST
calls without an operator ever handing Uptrakit a standing PVE password. The identity model predating this branch
created one PVE user per tenant (`uptrakit-{tenant_uuid}@pve`) — a scheme this branch replaces.

Several properties of Proxmox's own access-control model and of Uptrakit's multi-tenant, agent-driven provisioning
shape the replacement:

- PVE realms split into `@pve` (cluster-wide, stored in `/etc/pve/user.cfg`, replicated across every node) and
  `@pam` (node-local, backed by the node's own PAM stack). Only `@pve` users can hold API tokens; a token can never
  authenticate through PAM.
- `pveum user token add … --privsep=1` creates a privilege-separated token whose effective permission set is the
  **intersection** of the owning user's ACL grants and the token's own ACL grants, not their union and not the
  token's grants alone. This intersection rule is enforced by `pve-access-control`'s `RPCEnvironment.pm`
  `permissions()` — the same routine every PVE API call runs through to compute what a caller is allowed to do.
- Cluster membership is dynamic and self-reported: a node may join or leave a cluster, and `pvesh get
/cluster/status` is the only reliable source of the current cluster name and node set
  (`crates/plugins/infrastructure/proxmox/src/pve_setup.rs::detect_pve_cluster_nodes`,
  `detect_pve_cluster_name`).
- Multiple agent-ssh processes can bootstrap or sync different nodes of the same cluster concurrently — the
  cluster's `user.cfg` is a single shared resource with no cross-process coordination beyond what `pveum` itself
  provides.
- The agent process never persists a token secret to disk once it has been reported to the controller
  (`report_plugin_config`/`ReportPluginConfig` flow) — only an opaque plugin-config id comes back on the wire, via
  `HostLifecycle::on_plugin_config_reported` (`crates/plugins/infrastructure/core/src/roles.rs`), correlated to the
  originating host through the request/response pending-ack map (`PendingConfigReport`,
  `crates/core/agent-ssh-runtime/src/runtime_support.rs`). A migration step that fails after the legacy identity is
  gone but before the agent has durably recorded the new one must therefore be recoverable without ever reading the
  old secret back.

## Decision

### One cluster-wide `@pve` user, one token per tenant

A single PVE user, `uptrakit@pve` (`pve_setup::PVE_USER`), is created once per cluster and shared by every tenant
that manages hosts in that cluster. Each tenant gets its own API token on that user, named `tenant-{tenant_uuid}`
(`pve_setup::pve_token_id`), created with `pveum user token add … --privsep=1`
(`pve_setup::create_pve_api_credentials`, `regenerate_pve_api_token`). The user itself is never given a password —
it exists purely as a token-holding identity, and PVE's own realm split enforces that: an `@pve` user's tokens are
the only credential surface Uptrakit ever presents to the API.

### Privileges are a user-level ceiling plus per-token grants

`pve_setup::ensure_pve_acls` grants each of four `(path, role)` pairs — `UptrakitAudit` on `/`, `UptrakitProtection`
on `/vms` and `/storage`, `UptrakitScaling` on `/vms` — at **both** grant levels: once via `pveum acl modify …
--users 'uptrakit@pve'` and once via `--tokens 'uptrakit@pve!tenant-{tenant_uuid}'`. Both grants are load-bearing
under PVE's privilege-separated intersection rule (`RPCEnvironment.pm::permissions()`, `pve-access-control`): a
token's own ACL entries can never grant anything the user-level ceiling doesn't also grant, and a user-level ACL
with no matching token-level grant confers nothing to that token either. Every tenant's token is therefore capped
by the same user-wide ceiling; per-tenant differentiation is purely a property of what each tenant's own token is
additionally granted, which today is identical for every tenant (see Consequences).

### Config naming and node-identity handling

The per-cluster plugin config is named `pve-{cluster_name}` when `pvesh get /cluster/status` reports one, or
`pve-{node_name}-{host_id[..8]}` on a standalone node (`credential_flow::run_locked`, branch 3). The host-id suffix
on the standalone form exists because Proxmox's default hostname for a freshly installed node is literally `pve` —
not unique across independent standalone installs — so the config name must disambiguate on something the operator
did not choose. A clustered config, by contrast, keys purely on the cluster name.

## Rejected alternatives

### Continuing the pam/pve realm split as a per-tenant boundary

The predecessor scheme (`uptrakit-{tenant_uuid}@pve`, one user per tenant) was rejected going forward in favor of
the shared user because per-tenant PVE users do not compose with cluster membership: a `@pve` user is inherently
cluster-wide (replicated to every node via `/etc/pve/user.cfg`), so creating one per tenant multiplies user objects
without multiplying isolation — every tenant's user still lived in the same cluster-wide `user.cfg`, visible to and
manageable by the same PVE administrators, with no `@pam` boundary available to lean on (tokens cannot authenticate
via PAM at all, so a `@pam` per-tenant identity was never an option in the first place). The shared-user model
makes explicit what was already true: identity is cluster-scoped, and tenant separation happens at the token layer,
not the user layer.

### Tenant exclusivity (one PVE user per tenant, revisited)

A stricter design would keep one PVE user per tenant so a compromised tenant's credential can never even carry
another tenant's grants in principle, regardless of Proxmox's intersection semantics. This was rejected for this
branch: `check_pve_state` reads `pveum user token list` but matches its entries only against this tenant's own
token id (`crates/plugins/infrastructure/proxmox/src/pve_setup.rs:280-283`, whose own comment notes "no
cross-tenant scanning") — other tenants' tokens are left untouched not because they are scanned for and skipped,
but because nothing in the read or the credential flow's write path ever inspects or matches against them
(`coexisting_tenant_tokens_untouched` test, `crates/plugins/infrastructure/proxmox/src/agent/credential_flow.rs`).
The per-token grant surface, not a separate user per tenant, is treated as the isolation boundary going forward.
Full tenant-exclusive users remain available as a future option if the per-token model proves insufficient, but
nothing in this branch forecloses it, since PVE's user/token model composes either way.

## Migration

### Cluster-scoped prove-then-delete

`check_pve_state` detects a legacy `uptrakit-{tenant_uuid}@pve` user still present on the cluster
(`pve_setup::check_pve_state`, matching against `pveum user list`). Migration proceeds in two phases, tracked per
cluster-scoped row set (`cluster_rows` in `credential_flow::run_locked`, the set of `proxmox_host_state` rows
belonging to this host's own row plus any peer sharing a detected cluster node name):

- **Phase 1 — record.** The first run that observes the legacy user stores it on every cluster row
  (`db_ops::set_legacy_pve_user`), without touching it. This is deliberately non-destructive: recording must
  survive a crash or reconnect with no side effect beyond bookkeeping.
- **Phase 2 — prove, then delete.** "Prove" is two-part, not just a controller ack. First, immediately after the
  new shared-user token is created or regenerated (`credential_flow::run_locked`'s create and regenerate branches),
  the flow calls `pve_setup::prove_token_on_node` (`crates/plugins/infrastructure/proxmox/src/pve_setup.rs:551`)
  over the existing SSH session: it presents the freshly issued token as an `Authorization: PVEAPIToken=…` header
  against the node's own `https://localhost:8006/api2/json/version` endpoint and requires an HTTP 200. If that
  proof fails — a non-200 response, a non-zero curl exit, or `curl` missing on the node entirely (exit 127, handled
  as a distinct case) — the outcome is `PveCredentialOutcome::Failed` and the flow stops there; it does not fall
  through to touching the legacy user (`credential_flow.rs:264` and `:303`, in the `match` arms on
  `prove_token_on_node`'s result). Only once that on-node proof succeeds, and separately once the controller has
  ack'd the new token's plugin-config id back to the agent (`new_pve_plugin_config_id`, the ack marker — see
  below), does the flow call `pve_setup::delete_pve_user` on the legacy identity and, only on success, promote
  every cluster row's operative `pve_plugin_config_id` to the new id and clear the legacy marker
  (`db_ops::promote_cluster_rows`). A delete failure does not lose the migration: it increments a per-row attempt
  counter (`migration_attempts`, capped for reporting purposes at `MAX_MIGRATION_ATTEMPTS = 5`, though retries
  continue past that cap) and the outcome is reported as `MigrationPending` (or `"migration STUCK after N
attempts"` in the summary line once the cap is passed), never silently dropped.
- **Recovery.** If a legacy marker is stored but a later, successful read shows the legacy user already gone
  (deleted out-of-band, or by a peer node's own flow), the flow reconciles state directly — promoting cluster rows
  if ack evidence exists, or simply clearing the stale marker otherwise — without attempting a redundant delete.

### A distinct, never-cleared ack marker

The phase-2 gate does not fire on a bare `pve_plugin_config_id` alone. `new_pve_plugin_config_id` is a **separate**
column, written only once the controller has confirmed receipt of the new config
(`HostLifecycle::on_plugin_config_reported` → `db_ops::set_new_plugin_config_id`), and the migration flow itself
never clears it once set — not even `promote_cluster_rows`, which promotes the operative id but leaves the ack
marker in place as a permanent record that this cluster has confirmed the new identity at least once. The one
exception outside the migration flow is a deliberate tenant-rebind wipe: `db_ops::wipe_all`
(`crates/plugins/infrastructure/proxmox/src/agent/db_ops.rs:272`), called from `HostLifecycle::on_tenant_changed`,
deletes every `proxmox_host_state` row — ack marker included — because a rebind means the row's prior migration
history belongs to a different tenant and must not be reused
(`legacy_stored_without_ack_marker_never_deletes`,
`reuse_bare_operative_id_without_ack_marker_is_not_reused` in `credential_flow.rs`'s test module). This closes a
narrow but real failure mode: a bare `pve_plugin_config_id` can be set by paths that never round-tripped through
the controller (for example the coalesce-fill on reuse), so gating deletion of the legacy user on it alone would
risk deleting the only surviving identity before the replacement was durably confirmed anywhere but this one
agent's memory.

### Regenerate-on-ack-loss

Because the agent persists no token secret, the only way to recover a token whose secret has been lost locally
(ack marker missing, but `check_pve_state` confirms the token itself still exists on the cluster) is to remove and
recreate it — `credential_flow::run_locked`'s branch 6 calls `pve_setup::regenerate_pve_api_token`, which does
`pveum user token remove` followed by a fresh `pveum user token add --privsep=1`, rather than attempting to recover
a value PVE itself never lets a caller read back after creation. This is the same shape used when a degraded
`check_pve_state` read forces the flow to fall back to a guarded add-user-then-regenerate path instead of the
unguarded create path, so a flaky read never risks a "token already exists" failure from blindly calling create.

## Consequences

### Every tenant has identical effective privileges today

Because every tenant's token is granted the same four `(path, role)` pairs against the same user-level ceiling,
there is currently no privilege differentiation between tenants sharing a PVE cluster — isolation comes from each
tenant holding an independently revocable token (deleting one tenant's token, or narrowing its own ACL grants,
never touches another tenant's), not from disjoint permission sets. Narrowing per-token grants below the shared
ceiling is possible later without a schema change, since the token-level ACL entries are already granted
per-tenant, independently of the user-level ones.

### The shared user is a deliberate single point of failure

`uptrakit@pve` is one identity every tenant on a cluster depends on. If its user-level ACLs are ever revoked or
misconfigured out-of-band, every tenant's token loses effective privileges simultaneously under the intersection
rule, regardless of what each token's own grants say. The sync path is the accepted mitigation, not a full defense:
`on_host_synced` re-runs `ensure_pve_privileges` (both roles and ACLs) whenever this run's outcome was `Reused` or
`MigrationPending` under a healthy (non-degraded) state read, so drift on the shared user's ceiling self-repairs on
the next sync rather than requiring an operator to notice and re-provision by hand.

### The plugin config carries the provisioning node's `api_url`

`create_pve_api_credentials`/`regenerate_pve_api_token` resolve `api_url` from `hostname -f` on whichever node ran
the flow (`pve_setup::resolve_pve_api_url`), not from a cluster-level concept of an API endpoint — PVE has none, by
design every node in a cluster exposes the same `/api2/json` surface. The controller's Proxmox plugin therefore
talks to one specific node's API by address, even though the credentials it uses are cluster-wide; a node rename or
address change on the provisioning node specifically (not any other cluster member) is what would invalidate that
URL.

### Cross-agent `pveum` serialization relies on PVE, not on Uptrakit

`credential_flow::run_credential_flow` takes a process-global, per-cluster-name `tokio::sync::Mutex`
(`CLUSTER_LOCKS`) before running any `pveum`-mutating step, which serializes concurrent flows **within one agent
process**. It does not, and cannot, serialize a multi-tenant cluster bootstrapped by separate SSH-agent processes
(for example two independent `uptrakit-agent-ssh` invocations, or an agent-ssh process racing an operator's manual
sync) — cross-process serialization for `user.cfg` mutations is left entirely to PVE's own locking around
`pveum`/`/etc/pve/user.cfg`, which every `pveum` invocation already goes through regardless of which process issued
it.

### Config naming collision risk is accepted, not solved

Because the per-cluster config name keys on the detected cluster name (`pve-{cluster_name}`), two distinct PVE
clusters that happen to share a cluster name inside the same tenant would collide on the same config row — nothing
in the naming scheme disambiguates them. This is an accepted residual risk: PVE cluster names are operator-chosen
at cluster-creation time and expected to be site-unique in practice, unlike the standalone-node case, where the
default hostname `pve` is genuinely **not** unique across independent installs and therefore does get a host-id
suffix specifically to avoid the collision that would otherwise be common, not rare.

### Credential work is confined to privileged sessions

`pveum` never appears among the sudo commands the Proxmox plugin declares for the unprivileged agent user —
`collect_pve_sudo_commands` (`crates/plugins/infrastructure/proxmox/src/agent/plugin.rs`) only ever contributes
`pct exec`/`qm guest exec`/`qm guest cmd` entries for guest management, never a `pveum` entry. That means `pveum`
mutation can only ever _succeed_ in a session that is already root by construction — host bootstrap, which
`docs/security/sudoers-management.md` documents as requiring a root SSH session, or a sync explicitly run with a
root-auth override (`build_sync_auth_override` in `crates/core/agent-ssh-runtime/src/surface_runtime.rs` defaults
its override username to `"root"`) — never via the unprivileged agent user's ordinary `NOPASSWD` sudo allowlist,
since no allowlist entry exists to grant it. It is not gated to those sessions on the _invocation_ side: an
ordinary sync run with the default `auth_method` of `"stored"` (`build_sync_auth_override`, `surface_runtime.rs:1880,1883`,
returns `None` for that case, so no override is applied) still reaches `HostLifecycle::on_host_synced`
(`crates/plugins/infrastructure/proxmox/src/agent/plugin.rs:196`), invoked whenever the sync runs the
`infra_sync` action and this host's stored state has `is_pve_node = true`
(`crates/core/agent-ssh-runtime/src/operations/sync.rs:537-587`, gated by
`!skip_actions.contains(ACTION_INFRA_SYNC)` and then per-bundle by `report.has_infra_state(db, host.id)`) —
neither gate is an auth-override check, so it invokes `pveum` through the credential flow regardless of session
privilege, and simply fails there if the session isn't root. This keeps the highest-privilege PVE operation (creating users,
granting cluster-wide ACLs) unable to _succeed_ outside sessions that are already root, rather than adding it to
the set of commands an unprivileged agent process can invoke via `NOPASSWD` sudo.
