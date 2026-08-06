# agent-ssh Surface Runtime: Decomposition + Typed Error Taxonomy — Design

- **Date:** 2026-08-06
- **Source:** audit-2026-07-11, MEDIUM `core-agent-ssh`, verified
- **Status:** Spec approved pending owner review. One spec, **two strictly-ordered plans**:
  Plan A (module decomposition) then Plan B (typed error taxonomy).
- **Crate:** `uptrakit-agent-ssh-runtime` (`crates/core/agent-ssh-runtime`)
- **Scope note:** the audit brief said "do not perform the split here". The owner overrode this on
  2026-08-06 (decision D3v2): the split and the taxonomy ship as one combined effort, because no
  live split work item exists to sequence behind — the 2026-04-17 decomposition plan is complete
  *except* its agent-ssh Task 3, whose unwired parallel implementation was deleted as dead code
  (commit `47738f589`, 2026-06-05).

## 1. Problem

`surface_runtime.rs` is a 3380-line monolith mixing registration, dispatch, parameter parsing,
bootstrap/sync workflow spawning, audit classification, and response construction. Inside it,
three coupled defects in the error path:

1. **Stringly-typed errors.** `operations/sync.rs` is the only operations module returning bare
   `std::result::Result<_, String>` (`establish_session`, `detect_and_persist_sudo_state`,
   `sync_connect`, `sync_execute`), contrary to the crate's own typed model in `src/error.rs`
   (`Error` enum + `pub type Result<T> = std::result::Result<T, Report<Error>>` + `report!`/`bail!`,
   used throughout `operations/bootstrap.rs`). Error chains are flattened via
   `format!("...: {e}")` — the exact anti-pattern `docs/development/error-handling.md` bans, and
   none of that doc's approved `Result<T, String>` exceptions (clap parsers — Pattern 14; thin HTTP
   validation helpers — Pattern 15) applies to these multi-hundred-line orchestration functions.
2. **Classification by message matching.** `classify_validation_failure` and
   `classify_surface_mutation_outcome` re-derive audit semantics by matching message
   prefixes/literals ("missing required field", "invalid target", "no password provided",
   "host not found"). Rewording any producer message in `parse_bootstrap_params`,
   `build_sync_auth_override`, or `handle_remove_host` silently reclassifies audit events with no
   compile-time or test failure. This has **already happened**: `sync.rs` emits
   `"host '{host_id}' not found"`, which never matches the classifier's literal
   `"host not found"` — the `sync-execute` → `denied`/`host_not_found` arm is unreachable in
   production, and the test that claims to pin it
   (`sync_execute_missing_host_maps_to_denied_audit_event`) constructs the matching string
   synthetically instead of driving the real producer.
3. **Undifferentiated wire error code.** `make_surface_error_response` hardcodes
   `SurfaceActionErrorCode::InvalidRequest` for every failure, including DB errors and SSH
   failures, so consumers (the web-api HTTP mapping in `crates/ui/web-api/src/routes/surfaces.rs`)
   cannot distinguish caller error from system error.

## 2. Decisions (owner, 2026-08-06)

| # | Decision | Choice |
| --- | --- | --- |
| D1 | sync-execute missing-host classification | **Activate the intended arm**: typed `NotFound` maps to `denied`/`host_not_found` for `sync-execute`, matching the classification table the code already declares (and matching `hosts` DELETE). This is the single deliberate audit-classification change; everything else is preserved exactly. |
| D2 | Wire error-code remap | **In scope.** System-side failures (storage, SSH, internal) switch from `InvalidRequest` to `InternalError`. Value-level change only; all consumers already match all 8 `SurfaceActionErrorCode` variants exhaustively, both directions wire-compatible. |
| D3v2 | Split relationship | **One combined effort** (supersedes the initial "sequence after split" answer, revised after fact-check). Plan A performs the decomposition; Plan B lands the taxonomy into the decomposed layout. This spec's Plan A supersedes the unexecuted agent-ssh portion (Task 3) of `docs/superpowers/plans/2026-04-17-surface-runtime-decomposition.md`. |

## 3. Plan A — decompose `surface_runtime.rs` (behavior-preserving, layout only)

### 3.1 Target layout

`surface_runtime.rs` becomes the module root (`mod` declarations + the small shared glue), with
crate-private submodules under `src/surface_runtime/` grouped by responsibility:

- `registration` — surface/interaction descriptors, capability declarations, form adapters
  (the plan may subdivide as the 2026-06 deleted attempt did if single files grow unwieldy).
- `dispatch` — `handle_surface_action_request` / `handle_surface_request_internal`, the
  `(interaction_id, method)` arm table, and the `spawn_*` workflow launchers
  (`spawn_bootstrap_connect/execute`, `spawn_sync_connect/execute`, `spawn_infra_plugin_action`)
  plus their `run_*`/handler bodies.
- `params` — `parse_bootstrap_params`, `build_sync_auth_override`, `resolve_sync_auth`.
- `audit` — `classify_validation_failure`, `classify_surface_mutation_outcome`,
  `emit_surface_mutation_audit`, `build_surface_mutation_details`, and the response constructors
  (`make_surface_error_response`, `make_surface_success_response`). Plan B rewires this module.
- `tests` — the existing `mod tests` moves with the code it exercises.

Exact fn-to-file assignment is plan work, decided by reading the code, under these invariants:
the crate's public API is unchanged (submodules are `pub(crate)`/private; `lib.rs` keeps
`pub mod surface_runtime;`); zero behavior change; no `#[allow]` additions.

### 3.2 Guards against the June failure mode

The previous attempt built a parallel `surface_runtime/` directory that was never reachable (no
`mod` wiring; the monolith stayed authoritative) and was later deleted wholesale. Plan A must:

1. **Convert in place, never in parallel.** Each step moves code out of the monolith and wires the
   `mod` declaration in the same commit; the monolith shrinks monotonically. At no point do two
   copies of a function exist.
2. **Every commit compiles and is green** on the crate's test suite (workspace `warnings = "deny"`
   makes an unwired file's dead code invisible, not loud — reachability is proven by the `mod`
   chain, not by lint silence).
3. **Test-count assertion:** record the crate's test count before the split; after the final move,
   the count must be ≥ the baseline (guards against tests silently dropped with an unwired file).
4. **Orphan check at the end:** every `.rs` file under `src/` reachable from `lib.rs` via the
   `mod` chain (the June artifact was exactly this class of defect).

## 4. Plan B — typed error taxonomy

### 4.1 Reuse the crate error model — no parallel error enum

The audit brief suggested "a small typed error enum (Validation, NotFound, SshFailure,
StorageFailure)". The crate already ships the error carrier: `src/error.rs` `Error` (15 variants
including `HostNotFound`, `InvalidInput`, `Database`, `SshConnection`, `SshAuth`, `SshCommand`)
with `Report<Error>` conversions. Introducing a second error enum would duplicate it. Instead:

- **`operations/sync.rs` migrates to `crate::error::{Error, Result}`.** All four functions return
  `Result<T>` (i.e. `Report<Error>`), using `report!`/`bail!`/`.context_to::<Error>()` — never
  `Report::new()`. Existing string messages map to variants; representative mapping (exact
  per-line selection is plan work, constrained by the classification matrix in § 4.3):
  - `"database error: {e}"`, `"failed to update sudo state: {e}"` → `Error::Database` (via existing
    `impl_report_conversion!`)
  - `"host '{host_id}' not found"` → `Error::HostNotFound`
  - `"SSH connection failed: {e}"` → `Error::SshConnection` (existing russh conversion)
  - command-execution failures (root/sudo detection, helper install, sudoers write, docker group)
    → `Error::SshCommand` / `Error::Io` as appropriate
  - `"host '{name}' has no stored key fingerprint"`, `"sudo is not available for user ..."` →
    **new variant `Error::PreconditionFailed(String)`** (host/environment state precondition;
    neither validation nor SSH transport). New-variant check: `Error` is `pub` and not
    `#[non_exhaustive]` — the implementation plan MUST grep the workspace for cross-crate `match`
    sites on this enum (expected: none outside the crate; `uptrakit-agent-ssh` is a thin CLI shell
    per ADR-0005) before adding the variant.
  - `"auth override provided but neither password nor private key set"` → `Error::InvalidInput`.
    Reachability note: via the surface path this inner guard is dead — `build_sync_auth_override`
    validates password/key presence before `sync_connect`/`sync_execute` are called (their only
    callers are `spawn_sync_connect` / `spawn_sync_execute`). Classifying it as Validation is
    semantically right and changes no reachable behavior.
- **In-crate `Result<_, String>` helpers migrate too**: `parse_bootstrap_params`,
  `build_sync_auth_override` (→ `Error::InvalidInput`), `resolve_sync_auth`,
  `handle_remove_host`, `handle_list_hosts`. These strings are classification inputs today, so
  error-handling.md Pattern 15 (display-only strings) does not cover them.

Per error-handling.md, the module-wide `Result` alias covers **all** functions in each touched
module, read-only ones included.

### 4.2 One classification point

New classification unit inside the `audit` submodule from Plan A:

```rust
/// Closed set — matched exhaustively on purpose; a new kind must pick a row
/// in every context of the classification matrix. Crate-private, NOT
/// #[non_exhaustive] (follows the closed verdict-enum precedent).
enum SurfaceFailureKind {
    Validation,
    NotFound,
    Storage,
    Ssh,
    Internal,
}

/// The audited surface operation the failure occurred in.
enum SurfaceAuditContext {
    HostsRemove,      // interaction "hosts", DELETE
    BootstrapExecute, // interaction "bootstrap-execute"
    SyncExecute,      // interaction "sync-execute"
}

struct SurfaceFailureClass {
    wire_code: SurfaceActionErrorCode,
    outcome: AuditOutcome,                  // typed, from uptrakit-audit-log
    reason_code: Option<&'static str>,
}
```

Two total functions, each a single exhaustive `match` (no wildcard arms — `Error` and
`SurfaceFailureKind` are own-crate enums; a new `Error` variant then **fails compilation** until a
kind is chosen, which is the compile-time guarantee the audit asked for):

1. `fn failure_kind(report: &Report<Error>) -> SurfaceFailureKind` — exhaustive over `Error`:
   `InvalidInput` → `Validation`; `HostNotFound` → `NotFound`; `Database` → `Storage`;
   `SshConnection` | `SshAuth` | `SshCommand` | `HostKeyMismatch` | `KeyGeneration` → `Ssh`;
   `Io` | `Directory` | `Crypto` | `Enrollment` | `HostNameConflict` | `UnsupportedKeyType` |
   `BootstrapVerification` | `PreconditionFailed` → `Internal`.
   (`HostNameConflict` and `UnsupportedKeyType` are arguably caller errors, but today they classify
   as `failed` — preservation wins; re-kinding later is a one-line, compile-checked, audit-visible
   change, which is the point of the taxonomy.)
2. `fn classify(kind: SurfaceFailureKind, ctx: SurfaceAuditContext) -> SurfaceFailureClass` — the
   full matrix in § 4.3.

`AuditOutcome` comes from `uptrakit-audit-log` (already a dependency; `lib.rs` imports
`RuntimeAuditEmitter`). Its `as_str()` produces exactly the five strings the classifier hardcodes
today (`success`/`denied`/`validation_failed`/`failed`/`partial`), so the wire payload
(`AuditEventPayload.outcome: String`) is stringified from the typed value at the send boundary
only. The `reason_code` literals stay `&'static str` constants, now co-located with `classify` as
named consts instead of scattered inline literals.

### 4.3 Classification matrix (normative)

Wire code is context-independent: `Validation` and `NotFound` → `InvalidRequest` (caller error);
`Storage`, `Ssh`, `Internal` → `InternalError` (system error).

| kind \ context | HostsRemove | BootstrapExecute | SyncExecute |
| --- | --- | --- | --- |
| Validation | `validation_failed` / `invalid_request` | `validation_failed` / `invalid_request` | `validation_failed` / `invalid_request` |
| NotFound | `denied` / `host_not_found` | `failed` / `bootstrap_failed` † | `denied` / `host_not_found` **(D1 — the one classification change)** |
| Storage | `failed` / `storage_error` | `failed` / `bootstrap_failed` | `failed` / `sync_failed` |
| Ssh | `failed` / `storage_error` † | `failed` / `bootstrap_failed` | `failed` / `sync_failed` |
| Internal | `failed` / `storage_error` | `failed` / `bootstrap_failed` | `failed` / `sync_failed` |

† Cells unreachable today (bootstrap creates hosts, so `NotFound` cannot occur; `HostsRemove` does
no SSH). They are still defined — fail-closed to the context's existing fallback — because the
matrix must be total. Every cell equals current production behavior except the marked D1 cell.

Success-path classification (`success` and the `bootstrap-proxmox-guest` `partial` derivation from
`result.failed > 0`) is untouched.

### 4.4 Response construction and audit emission

- `make_surface_error_response(request_id, message)` splits into:
  - `surface_error_response(request_id, code, message)` — explicit-code constructor for the
    non-`Report` call sites that stay caller-errors (`"unknown surface"`, unknown-interaction),
    which keep `InvalidRequest`;
  - a typed path `surface_error_from_report(request_id, report, kind)` (exact signature is plan
    work) used by every `Err(Report<Error>)` site, taking `wire_code` from the classification.
- At each in-crate `Err` site the classification is computed **once** and flows to both the wire
  response and the audit emit — the audit path no longer re-parses `response.error.message`.
  `emit_surface_mutation_audit` gains a typed classification parameter
  (`Option<SurfaceFailureClass>` or equivalent): `Some` for in-crate failures (hosts DELETE,
  bootstrap-execute, sync-execute), `None` for the plugin-origin path (§ 4.5) and success paths,
  which keep their current derivation. Every `Err` path that emits an audit event today must still
  emit one — the refactor must not insert early returns ahead of existing emission points.
- Audit emission mechanism is unchanged: wire `ServiceMessage::AuditEvent(AuditEventPayload)` over
  `bg_tx`, same `action_type` mapping (`hosts` → `host.deactivate`; `bootstrap-execute` |
  `sync-execute` | `bootstrap-proxmox-guest` → `host.update`), `reason_code` still injected into
  `details_json`. No new emission sites, no `audit-catalog.toml` changes. (The audit brief's
  "stays on the typed V2 API" constraint is honored in the applicable sense: this crate's surface
  audit path is wire-forwarded Events — allowed by the V2 rules, which ban only service-forwarded
  *Stateful* events — and outcome typing now flows through `AuditOutcome`.)
- **Error message text stops being load-bearing.** Wire messages become
  `Report<Error>` display renderings; minor wording changes versus today's literals are acceptable
  and expected (e.g. `"host not found: {id}"` instead of `"host 'id' not found"`). No test may pin
  message text for classification purposes; message assertions are limited to presence/context.

### 4.5 Explicit residual: plugin-origin responses

`bootstrap-proxmox-guest` (and any future plugin-handled interaction reaching the `_` dispatch
arm) produces its `SurfaceActionResponse` inside the proxmox plugin crate, which hardcodes
`InvalidRequest` for **all** its failures. Classifying those by wire code would flip
non-validation plugin failures to `validation_failed` — a behavior change outside this spec's
producers. Therefore the plugin-origin path keeps the existing message-based classification
(`classify_surface_mutation_outcome` retained, reduced to: success/partial logic + the plugin-path
failure arms — `bootstrap-proxmox-guest` and the `_` → `failed`/`unclassified_error` fallthrough;
the `hosts`/`bootstrap-execute`/`sync-execute` arms are deleted as dead once callers pass typed
classifications). `classify_validation_failure` survives only for this path, with a doc comment
naming the residual and pointing at the deferred item (§ 8). Rewording messages inside the proxmox
plugin still silently reclassifies — unchanged risk, now documented instead of latent.

### 4.6 Behavior-change inventory (complete)

1. **D1**: real sync-execute against a missing host: audit `failed`/`sync_failed` →
   `denied`/`host_not_found`.
2. **D2**: wire `SurfaceActionError.code` for system-side failures: `InvalidRequest` →
   `InternalError`. Affected classes: DB failures in list/remove/sync/bootstrap paths, SSH
   failures, sudoers/helper-script/docker-group failures, `"failed to list hosts"`. Downstream:
   web-api's code→HTTP-status mapping moves these to the 5xx group. No doc pins per-code
   semantics for these paths (verified — only `CloseReason::InternalError` appears in
   `docs/api/wire-protocol.md`), and no payload *shape* changes ⇒ no asyncapi/openapi regen.
3. Wire error message wording may change (§ 4.4). Classification and audit fields are independent
   of it by construction.

Everything else — audit outcomes, reason codes, action types, emission sites, success/partial
logic, plugin-path classification, and all of Plan A — byte-identical in behavior.

## 5. Testing

All in-crate, using the existing real-DB test setup already used by the `remove_host_*` tests.
Both success and failure paths per docs/development/testing.md.

**Plan A:** no new behavior tests — the existing suite moves with the code and must stay green at
every commit; plus the test-count and orphan-check guards from § 3.2.

**Plan B:**

1. **Matrix pin (table test).** One table-driven test asserting `classify(kind, ctx)` over **every
   cell** of § 4.3 — wire code, `AuditOutcome`, reason code. This replaces message-literal pinning
   as the classification contract.
2. **Kind pin.** Unit test asserting `failure_kind` for a representative `Report<Error>` of each
   `Error` variant (the exhaustive `match` itself is the structural guarantee; the test documents
   the mapping and catches accidental re-kinding).
3. **Real-producer end-to-end tests** (the fix for the synthetic-test flaw; each drives the actual
   handler so a producer-side change fails the test):
   - sync-execute with a nonexistent host id (DB-only, no SSH needed) → audit
     `denied`/`host_not_found`, wire `InvalidRequest`. **Replaces**
     `sync_execute_missing_host_maps_to_denied_audit_event` (which hand-builds the message and
     currently pins a fiction).
   - bootstrap-execute with a validation failure driven through
     `handle_surface_action_request` (e.g. missing `target`) → `validation_failed` /
     `invalid_request`, wire `InvalidRequest`.
   - keep `remove_host_missing_host_emits_denied_audit_event` (already real-path).
   - one system-error real-path test where cheaply stageable; where a real DB/SSH fault cannot be
     staged through production paths, the matrix test is the pin (the typed kind closes the
     message-rewording channel that made synthetic tests vacuous).
4. **Previously untested arms** gain coverage: `failed`/`storage_error`,
   `failed`/`bootstrap_failed`, and `failed`/`sync_failed` via the matrix test;
   `failed`/`unclassified_error` (plugin-path fallthrough, outside the matrix) via a test driving
   the retained legacy classifier with an unknown interaction id.
5. Existing plugin-path tests (`bootstrap_proxmox_guest_*`) remain unchanged and must stay green.
6. No `tokio::time` usage is added ⇒ no `start_paused` requirement.

Gates (per docs/development/quality-gates.md, run scoped to the touched crate plus workspace where
required): `cargo fmt`, `cargo clippy --all-targets` (both feature worlds), `cargo test`,
`cargo xtask audit-coverage-check` (audit-emitting code is touched), `cargo deny check` unaffected
(no new dependencies).

## 6. Sequencing

Plan A strictly before Plan B (B rewires symbols A relocates). Both belong to this spec; no other
work item is a prerequisite. The 2026-04-17 decomposition plan's Task 3 is superseded by Plan A
and must not be executed separately.

## 7. Documentation deliverables

- **This spec** (`docs/superpowers/specs/2026-08-06-agent-ssh-surface-error-taxonomy-design.md`).
- **Rustdoc**: module-level docs on each new `surface_runtime` submodule (Plan A); rustdoc on
  `SurfaceFailureKind`, `SurfaceAuditContext`, `classify`, `failure_kind`, and the retained legacy
  classifier (residual note per § 4.5) — the normative matrix lives in code form; the "document
  everything" invariant is satisfied in-code.
- **No ADR**: no new architectural rule is introduced — the change applies the existing
  error-handling standard and audit outcome vocabulary; the decomposition is internal layout. D1
  is a bug fix recorded here and in the Conventional Commit; D2 changes no documented contract
  (verified above).
- **No changes** to README/CONTEXT.md/ARCHITECTURE.md, `docs/api/wire-protocol.md`,
  `asyncapi.yaml` (no wire *type* changes), OpenAPI artifacts (no HTTP endpoint changes), or
  `docs/development/error-handling.md` (the crate becomes conforming; no new exception is added).

## 8. Out of scope / deferred

- **Typed request structs** for surface params (`BootstrapRequest`/`SyncHostRequest` from the
  2026-04-17 design's Task 3) — not needed by the taxonomy; params keep flowing as
  `serde_json::Map` for now.
- **Proxmox plugin typed errors**: the plugin crate's `make_error_response` hardcodes
  `InvalidRequest`; typing plugin-origin failures (and then classifying plugin responses by wire
  code instead of message matching) is a follow-up item. Until then the § 4.5 residual stands.
- **Audit coverage for `sync-connect`, `bootstrap-connect`, and `hosts` GET** — these emit no
  audit events today; adding events means new `audit-catalog.toml` entries and is a scope change,
  not behavior preservation.
- **Centralized reason-code registry** in `uptrakit-audit-log` (reason codes remain crate-local
  consts; nothing else in the workspace consumes them today).

## 9. Alternatives considered

- **New standalone surface-error enum carrying failure data** (the audit brief's literal
  suggestion): rejected — duplicates the crate's existing `Error`/`Report<Error>` model; the
  taxonomy is a *classification* of errors, not a second error carrier.
- **Classify plugin-origin responses by `SurfaceActionErrorCode`**: rejected for now — the proxmox
  plugin sends `InvalidRequest` for all failures, so code-based classification would reclassify
  non-validation plugin failures as `validation_failed`, violating behavior preservation
  (deferred, § 8).
- **Strictly preserve `failed`/`sync_failed` for sync missing-host** (pure behavior preservation):
  rejected by owner (D1) — the current behavior is a string-match bug; the classifier's own table
  already declares the `denied` arm, and `hosts` DELETE behaves that way.
- **Taxonomy independent of the split** and **taxonomy sequenced behind a separately-owned
  split**: both rejected by owner (D3v2) in favor of the combined effort, after fact-checking
  showed no live split work item exists.
