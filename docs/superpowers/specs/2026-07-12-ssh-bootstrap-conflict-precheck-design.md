# SSH Bootstrap Conflict Pre-check + Partial-Config Error Context — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** the three SSH-bootstrap **execute** paths (`bootstrap.rs`, `bootstrap_routeros.rs`,
`bootstrap_proxmox.rs`) + a doc note. No ADR, no deps, no wire.

## Problem

Audit `audit-2026-07-11` L876 (MEDIUM · stability · core-agent-ssh · verified): `bootstrap_execute` mutates the
**remote host** (creates the target user, deploys the SSH key, writes sudoers) **before** any host-name-conflict
check, deferring the uniqueness check to its last step (`save_host` → `host_ops::add_host`). On a name conflict
(double-submit of the execute step, or a host added between the connect-review and execute) — or any DB failure —
the in-memory Ed25519 private key is dropped: **the remote host is left holding an authorized key nobody
possesses, with no DB record**, and the error carries **no** partial-configuration context.

**Two of the three execute paths lack the recheck** (root-cause scope, per review). The generic/POSIX path
(`bootstrap.rs`) and the RouterOS path (`bootstrap_routeros.rs`, reached via the POSIX router) mutate the remote
then call `add_host` raw with **no execute-phase recheck**. The **Proxmox path already rechecks correctly** — it
folds the name check into a shared `load_and_validate_pve_host` step that both its connect *and* execute phases
call before any remote effect, so Proxmox is the **model** the POSIX path should have followed. All three,
however, call `add_host` **raw** (no partial-config error context) — so the enrichment applies to all three, the
recheck only to POSIX (which covers RouterOS).

## Verified current reality (byte-checked, 2026-07-12)

- **POSIX connect rechecks; POSIX execute does not.** `bootstrap_connect` (`bootstrap.rs:210-214`) does
  `if host_ops::find_host(db, &params.name).await?.is_some() { bail!(Error::HostNameConflict(params.name.clone())); }`
  right after validation; `bootstrap_execute` does not.
- **Proxmox rechecks in BOTH phases** (already correct). The check lives in the shared helper
  `load_and_validate_pve_host` (`bootstrap_proxmox.rs:164-183`, `find_host(db, &params.name)` → `bail!` at
  `:178-180`), which is called by `proxmox_bootstrap_connect` (`:285`) **and** `proxmox_bootstrap_execute`
  (`:423`) before any remote action. So the Proxmox execute path needs **no** recheck — only the enrichment below.
- **POSIX `bootstrap_execute`** (`bootstrap.rs:530`): head runs only `validate_bootstrap_inputs(&params)?` (`:537`),
  then key-gen (`:548`), SSH connect (`:573`), remote mutations, verify, and `save_host` **last** (`:694`).
  `save_host` (`:1432-1460`) calls `host_ops::add_host(...).await?` **raw**; the call site (`:694`) propagates raw.
  `bootstrap_execute` routes RouterOS hosts to `execute_bootstrap_routeros` at **`:577`** — after the head — so a
  recheck placed at the head covers RouterOS too.
- **RouterOS** `save_routeros_host_entry` (`bootstrap_routeros.rs:251`) calls `host_ops::add_host(...).await?`
  **raw** at `:272`, after RouterOS remote config. No enrichment.
- **Proxmox `proxmox_bootstrap_execute`** (`bootstrap_proxmox.rs:417`) is a **separate** function. Its body:
  `load_and_validate_pve_host(db, &params)` (`:423`, DB read **that already rechecks the name**, see above), then
  `connect_and_create_executors` (`:427`, the **remote** connect), key-gen (`:431`), guest setup (`:435+`), and
  `host_ops::add_host(...)` **raw** at `:752`. The recheck is present; only the `add_host` error enrichment is
  missing.
- **Verify-path decoration** already exists as the pattern to mirror (`bootstrap.rs:677-683`):
  `report!(Error::BootstrapVerification(format!("… The remote host has been partially configured (user created,
  key deployed, sudoers written). Manual cleanup may be required.", …)))`.
- Symbols: `host_ops::find_host(db, name_or_id) -> Result<Option<Model>>` (`host_ops.rs:117`; tries `Uuid::parse`
  first, else `Column::Name.eq` — identical call semantics to the existing connect-phase checks, no new risk);
  `add_host` does `bail!(Error::HostNameConflict(params.name))` on its UNIQUE-name check; `Error::HostNameConflict`
  (`error.rs:29`); crate `Result<T> = Result<T, Report<Error>>` (`error.rs:60`). The rootcause context idiom is
  `.attach(format!(…))` on a `Report<Error>` (used at `ui/cli/src/client.rs:57`, `ui/cli/src/commands/auth.rs:640`,
  `web-api/src/oauth/canonical_url.rs:57`) — it appends a printable note and **preserves** `current_context` (the
  error variant), so `HostNameConflict` stays matchable. (`rootcause` is pinned `0.13`; `.attach` is its only
  context-attach method — `attach_printable`/`change_context` are error-stack APIs and do not exist here.)

## Approach (chosen — recheck-early + enrich-error across all three paths, YAGNI)

### 1. Early conflict recheck in `bootstrap_execute` (POSIX + RouterOS)

Add the same 3-line guard the POSIX connect phase uses, at the `bootstrap_execute` head — right after
`validate_bootstrap_inputs(&params)?` (`bootstrap.rs:537`), before key-gen (`:548`). This precedes the RouterOS
routing branch (`:577`), so it **covers both POSIX and RouterOS**:

```rust
if host_ops::find_host(db, &params.name).await?.is_some() {
    bail!(Error::HostNameConflict(params.name.clone()));
}
```

No shared helper — a 3-line guard at few call sites is below the repo's reuse-extraction threshold. **Proxmox
needs no recheck** — `proxmox_bootstrap_execute` already runs this check via `load_and_validate_pve_host`
(`:423`) before its remote connect. This closes the **common** windows — double-submit of execute, and a host
added during the connect→execute human-review gap (minutes) — **before any remote mutation**.

### 2. Enrich each `add_host` call site with partial-config context

At all three raw `add_host` call sites, attach the cleanup guidance via the crate's `.attach(...)` idiom,
preserving the underlying variant so a name conflict stays matchable:

```rust
// bootstrap.rs:694 (save_host call), bootstrap_routeros.rs:272, bootstrap_proxmox.rs:752
… .await
  .map_err(|e| e.attach(format!(
      "The remote host '{}' has been partially configured (user/account created, key deployed, \
       config written). Manual cleanup may be required.", params.name)))?;
```

The note is accurate at each site — the remote was already mutated by the time `add_host` runs. `.attach` keeps
`HostNameConflict` (or any DB error) as `current_context`.

### Residual window (stated honestly — not full closure)

The early recheck does **not** eliminate the TOCTOU: a host can still be inserted in the narrow window between
the recheck and `add_host` (the seconds of remote-mutation work), and a generic DB failure (pool/disk) at
`add_host` is always possible. Those residuals are exactly what change (2) covers — recheck removes the
large/common windows *before* mutation; enrichment surfaces cleanup guidance for the small residual *after*
mutation. Defense-in-depth, not a race-free guarantee.

**Worst case is a dangling no-op pubkey, not an exposed credential.** On the residual failure, the remote holds
an `authorized_keys` entry whose matching private key was **generated in memory and dropped/zeroized** (never
persisted anywhere) — so no one holds it; the orphaned entry grants access to nobody. That is why a
transaction/reservation fix is not warranted: there is no leaked secret to protect, only a stale public-key line
for the operator to remove (which the enriched error tells them about). A security reviewer should read "orphaned
key" as "dead pubkey line," not "exposed credential."

## Tests

The bootstrap.rs `mod tests` today are **`ScriptedRemoteExecutor` mock-only — there is no DB-backed test there**.
The DB-test pattern must be **ported** (not "reused"): `crate::db::init_db(tempdir)` (runs migrations so
`ssh_hosts` exists) plus `uptrakit_crypto::init_master_key(...)` + `register_column_aad(...)` so `EncryptedString`
works — the same ~15 lines used by `surface_runtime.rs:2611+` / its `test_encrypted_key()` helper (`:2660-2669`).

- **Primary (load-bearing, DB-only, no network):** insert a host named `"dup"` (via `host_ops::add_host` after the
  master-key setup), then call `bootstrap_execute` with `params.name = "dup"` + valid minimal params (e.g.
  `use_ssh_agent: true`, valid POSIX usernames so `validate_bootstrap_inputs` passes) pointing at an unreachable
  SSH target; assert it returns `Error::HostNameConflict("dup")`. Because the recheck precedes the SSH connect,
  it bails **without any network** — proving the conflict is rejected before remote mutation.
- **Enrichment** is error-message decoration mirroring the existing (untested) verify-error decoration at `:677`;
  assert-if-cheap that the returned error is still `HostNameConflict` and its printable chain carries the note. No
  SSH mocking. No `start_paused` (no tokio-time API added). (No Proxmox recheck test — Proxmox already rechecks;
  no change to test there.)

## Deliverables

- `crates/core/agent-ssh-runtime/src/operations/bootstrap.rs` — recheck at the `bootstrap_execute` head;
  `.attach` enrichment at the `save_host` call site; the primary DB-backed test (with the ported setup).
- `crates/core/agent-ssh-runtime/src/operations/bootstrap_routeros.rs` — `.attach` enrichment at the
  `save_routeros_host_entry` `add_host` call (`:272`). (Recheck already covered via the shared `bootstrap_execute`
  head.)
- `crates/core/agent-ssh-runtime/src/operations/bootstrap_proxmox.rs` — `.attach` enrichment at the `add_host`
  call (`:752`) **only**. (Recheck already present via `load_and_validate_pve_host` — do not add a duplicate.)

**No 4th path.** Every production `add_host` caller was checked: `host_cli.rs:109` uses a user-supplied key file
(no in-memory-generated key to orphan, no remote mutation) and `surface_runtime.rs:2682` is test-only. No
re-bootstrap/re-enroll/host-sync path shares the mutate-then-`add_host` shape. The three execute paths are the
exhaustive set.

### Documentation deliverables

- `docs/architecture/ssh-agent.md` — note that the POSIX/RouterOS execute path now rechecks the name at its
  **start**, before remote mutation (matching what Proxmox already does), and that a residual post-mutation save
  failure returns cleanup guidance. (`:203` documents the connect-phase check;
  `:419`/`docs/end-user/ssh-agent-bootstrap.md:398-419` describe the partial-config/duplicate-credential risk —
  update to reflect the earlier rejection reduces it.)
- **No ADR** (bug fix). **No wire/OpenAPI/frontend/dependency change** — `Error::HostNameConflict` (variant) is
  unchanged, and grep confirms **no HTTP/UI consumer matches this crate's `HostNameConflict` variant or message
  text** (zero `HostNameConflict` references in `crates/ui`/`controller-runtime`), so appending an `.attach` note
  breaks no parsing. Internal robustness + error context only.

## Alternatives considered

- **Name-reservation scheme** (insert a "pending" host row before remote mutation to atomically claim the name) —
  **rejected**: larger change; introduces its own partial-state (a reserved row dangling if the remote phase then
  fails), and relocates rather than eliminates cleanup.
- **Persist the generated key before `add_host` for retry-reuse** — **deferred**: adds a secret-file-on-disk
  lifecycle for a narrow benefit; the enriched error already tells the operator to remove the one orphaned key.
- **Extract a shared `ensure_name_available(db, name)` helper** — **rejected**: a 3-line guard at a handful of
  sites is below the repo's reuse-extraction threshold (cross-plugin pipeline shapes, not local guard clauses);
  adds an indirection + `# Errors` contract to maintain.
- **Enrich inside a shared `add_host` wrapper** — rejected: `add_host` is a generic persist helper called from
  many non-bootstrap sites; the "remote was configured" note is only true in the bootstrap-execute flows, so it
  belongs at those call sites (mirroring where the verify-path decoration lives).

## Out of scope

Other unspecced immediate-Medium findings in different subsystems (core-mqtt-scheduler L911, plugins-infra
L1042/L1052, ui-cli-surface-proxy L1105/L1122/L1138, web-api-routes L1226) — separate specs. No change to the
remote-mutation ordering or the user-creation/sudoers logic, no name-reservation redesign, no key-persistence, no
new abstraction. (The three bootstrap execute paths ARE all in scope here — the root-cause fix covers the whole
bug class, not one instance.)
