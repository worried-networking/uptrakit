# SSH Bootstrap Conflict Pre-check + Partial-Config Error Context — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** the three SSH-bootstrap **execute** paths (`bootstrap.rs`, `bootstrap_routeros.rs`,
`bootstrap_proxmox.rs`) + a doc note. No ADR, no deps, no wire.

## Problem

Audit `audit-2026-07-11` L876 (MEDIUM · stability · core-agent-ssh · verified): `bootstrap_execute` mutates the
**remote host** (creates the target user, deploys the SSH key, writes sudoers) **before** any host-name-conflict
check, deferring the uniqueness check to its last step (`save_host` → `host_ops::add_host`). On a name conflict
(a host added between the connect-review and execute, or a double-submit of execute) — or any DB failure —
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
  (`error.rs:29`); crate `Result<T> = Result<T, Report<Error>>` (`error.rs:60`). The rootcause context idiom:
  `.attach(…)` appends a printable note and **preserves** `current_context` (the error variant), so
  `HostNameConflict` stays matchable. (`rootcause` is pinned `0.13`; `.attach` is its only context-attach method —
  `attach_printable`/`change_context` are error-stack APIs and do not exist here.) **Precedent is composite, stated
  plainly:** no site in the workspace combines `.map_err(|e| e.attach(format!(…)))` on the propagated error today —
  `ui/cli/src/client.rs:57` and `ui/cli/src/commands/auth.rs:640` use `.map_err(|e| e.attach("…"))` with a **string
  literal**, and `web-api/src/oauth/canonical_url.rs:57` attaches a `format!` string to a **fresh** `report!(…)`
  (discarding `e`'s context). The proposed shape synthesizes the two known-good halves (attach-on-propagated-`e`
  wiring + `format!` content); `Report::attach<A: Display + Debug>` accepts a `String`, so the composition is
  sound — but this is its **first** instance in the workspace, not a copy of an existing one. **Render path
  verified against rootcause 0.13 source** (not assumed): `Report`'s Display *and* Debug both delegate to the
  default report formatter, whose `format_node_data` unconditionally iterates `report.attachments()` — and both
  consumers format with Display (`surface_runtime.rs:1719-1745` builds the surface error via
  `format!("bootstrap failed: {e}")`; the CLI's `main.rs:305` prints `eprintln!("Error: {e}")`), so the attached
  note reaches the operator on every surface. The enrichment is not a silent no-op.

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
(`:423`) before its remote connect. This closes the **common** window — a host added during the connect→execute
human-review gap (minutes) — **before any remote mutation**. (A concurrent double-submit of execute, if any UI
surface permits one, is closed by the same guard; the review gap alone carries the justification — the
double-submit scenario is a bonus, not the demonstrated premise.)

### 2. Enrich each `add_host` call site with partial-config context

At all three raw `add_host` call sites, attach the cleanup guidance via the crate's `.attach(...)` idiom,
preserving the underlying variant so a name conflict stays matchable:

```rust
// bootstrap.rs:694 (save_host call) and bootstrap_proxmox.rs:752 — user creation on these
// paths is existence-guarded (`id -u` check), so re-run genuinely reuses the account:
… .await
  .map_err(|e| e.attach(format!(
      "The remote host '{}' has been partially configured (user/account created, key deployed, \
       config written). If the name is now taken by another host, choose a different name and \
       remove the orphaned key/account from the remote; otherwise re-run bootstrap — the \
       existing account is detected and reused.",
      params.name)))?;

// bootstrap_routeros.rs:272 — path-specific message: RouterOS `/user add`/`/user group add`
// are NOT existence-guarded (bootstrap_routeros.rs:132-141), so there is no reuse path; the
// crate's own verify-error (:234-237) already documents the manual removal:
… .await
  .map_err(|e| e.attach(format!(
      "The router '{}' has been partially configured (uptrakit user/group created, key \
       imported). Remove them before retrying (`/user remove uptrakit; /user group remove \
       uptrakit`), and choose a different host name if this one is now taken.",
      params.name)))?;
```

The note is accurate at each site — the remote was already mutated by the time `add_host` runs. `.attach` keeps
`HostNameConflict` (or any DB error) as `current_context`.

### Residual window (stated honestly — not full closure)

The early recheck does **not** eliminate the TOCTOU: a host can still be inserted in the narrow window between
the recheck and `add_host` (the seconds of remote-mutation work), and a generic DB failure (pool/disk) at
`add_host` is always possible. Those residuals are exactly what change (2) covers — recheck removes the
large/common windows *before* mutation; enrichment surfaces cleanup guidance for the small residual *after*
mutation. Defense-in-depth, not a race-free guarantee.

**Worst case is three inert artifacts, not an exposed credential.** On the residual failure, the remote holds
(1) an `authorized_keys` entry whose matching private key was **generated in memory and dropped/zeroized** (never
persisted anywhere) — no one holds it, the entry grants access to nobody; (2) a created user account that is
**password-locked** (`useradd` with no `-p` leaves `!` in shadow — no password-auth login path even with
`PasswordAuthentication yes`); and (3) a sudoers drop-in **scoped to declared plugin commands, never `ALL`** (the
architecture invariant). All three residuals are inert. Recovery splits by variant — and the `.attach` wording
must not promise reuse unconditionally: on the **generic DB-failure** residual (pool/disk at `add_host`, no
competing row), re-running bootstrap passes the recheck and the remote user-detection reuses the account
(`docs/end-user/ssh-agent-bootstrap.md:401`) — idempotent recovery, the real reason a transaction/reservation fix
is not warranted. On the **conflict** residual (a foreign host claimed the name inside the residual window),
re-run with the same name fails fast at the new head recheck *before any remote work* — correct behavior, but no
reuse: the operator must pick a different name and remove the orphaned key/account manually. **RouterOS carve-out
(both variants):** the reuse claim is POSIX/Proxmox-only — their user creation is existence-guarded
(`bootstrap.rs:594-606`, `bootstrap_proxmox.rs:437-462` run `id -u` first), but RouterOS `/user add` /
`/user group add` are unconditional (`bootstrap_routeros.rs:132-141`), so *any* RouterOS re-run after a residual
failure hits "user exists" and needs manual `/user remove uptrakit; /user group remove uptrakit` first (the
path's own verify-error at `:234-237` already documents this) — hence the path-specific `.attach` message above.
The RouterOS residual artifacts are a router `/user` + `/user group` + imported ssh-key (no shell account, no
shadow entry), but the inert-credential conclusion is unchanged: the imported pubkey's private half was dropped,
so the leftover principal grants access to nobody. A security reviewer should read "orphaned key" as "dead pubkey
line plus a locked (or router-local) account," not "exposed credential."

## Tests

The bootstrap.rs `mod tests` today are **`ScriptedRemoteExecutor` mock-only — there is no DB-backed test there**.
The DB-test pattern must be **ported** (not "reused"). Port from **`host_ops.rs:341-381`** — its `setup_db()`
(tempdir + `crate::db::init_db`, which runs migrations so `ssh_hosts` exists), `test_encrypted_key()` (master-key
init + `register_column_aad`) and `add_params()` helpers are the closest precedent: same crate, and the module
that **owns** the `add_host`/`find_host` functions the new test exercises. (`surface_runtime.rs:2611+` carries the
same ~15 lines but is farther from the code under test — do not port from there.)

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
  `save_routeros_host_entry` `add_host` call (`:272`), using the **path-specific RouterOS message** (no reuse
  promise; names the `/user remove` cleanup). (Recheck already covered via the shared `bootstrap_execute` head.)
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
