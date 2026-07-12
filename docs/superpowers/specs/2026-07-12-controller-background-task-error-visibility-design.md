# Controller Background-Task Error Visibility — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/core/controller-runtime/src/tasks.rs` — two polling loops. No ADR, no deps, no wire change.

## Problem

Audit `audit-2026-07-11` L894 (MEDIUM · stability · core-controller · verified): two controller background
polling loops silently swallow errors via `let Ok(...) = … else { continue; }`, disabling their function forever
with **zero observability** — inconsistent with every other arm of the same loops, which log at `error`. One of
them also does **blocking file IO on a tokio runtime worker**.

## Verified current reality (byte-checked, 2026-07-12)

- **`tasks.rs:303-305`** (`spawn_ca_reload` loop, interval `CA_RELOAD_INTERVAL` = 30s, `durations.rs:17`):

  ```rust
  let Ok(db_version) = crate::pki::load_ca_version(&db).await else {
      continue;
  };
  ```

  The error is discarded. If this query starts failing persistently (pool exhaustion, disk error, schema drift
  after a bad migration), **cross-instance CA-rotation detection is disabled forever with no log**. Every other
  failure in the *same* loop logs at error: `load_managed_ca_state` (`:313`), `to_snapshot` (`:322`),
  `crl_manager.update_ca` (`:328`) — all `tracing::error!(error = ?e, …)`.
  `load_ca_version` signature: `pub(crate) async fn load_ca_version(db: &DatabaseConnection) -> Result<i64>`
  (`pki.rs:642`) — a `Report`-based `Result`, so `error = ?e` logs fine.

- **`tasks.rs:483-485`** (`spawn_server_cert_renewal` loop, interval `SERVER_CERT_RENEWAL_CHECK_INTERVAL` = 24h,
  `durations.rs:20`):

  ```rust
  let cert_path = pki_path.join("server.crt");   // pki_path: PathBuf
  let Ok(cert_pem) = std::fs::read_to_string(&cert_path) else {
      continue;
  };
  ```

  The error is discarded **and** this is a **blocking `std::fs`** call on an async runtime worker. An
  unreadable/missing `server.crt` permanently disables auto-renewal with no log. Siblings in the same loop log at
  error: `KeyPair::from_pem` (`:501`), `Issuer::from_ca_cert_pem` (`:509`).

- Grep-confirmed these are the **only** two `let Ok(...) else { continue }` / `std::fs::` sites in `tasks.rs` — no
  other silent-swallow siblings to fold in.
- `tokio::fs` is already used idiomatically in this crate: `server.rs:186` (`tokio::fs::read`),
  `boot/init/installation_id.rs:20` (`tokio::fs::read_to_string`), `service_host/builtins.rs:330`
  (`tokio::fs::metadata`). Switching the cert read to `tokio::fs::read_to_string(&cert_path).await` matches
  precedent.

## Approach (chosen — KISS, log-before-continue, no new machinery)

Convert each silent `else { continue }` into a logged `Err` arm, and fix the blocking read. Mechanical:
`let Ok(x) = … else { continue }` → `match … { Ok(x) => x, Err(e) => { <log>; continue } }`.

### 1. `spawn_ca_reload` (`tasks.rs:303`)

```rust
let db_version = match crate::pki::load_ca_version(&db).await {
    Ok(v) => v,
    Err(e) => {
        tracing::error!(error = ?e, "CA reload: failed to query CA version from database; retrying next interval");
        continue;
    }
};
```

Level `error` — matches every other arm of *this* loop (`:313`/`:322`/`:328` all `error!`), which likewise
`continue` on failure. Local consistency (a uniform loop, no `warn`/`error` mix within one `match` chain) is the
maintainable choice. `error = ?e` matches the `Report<PkiError>`-typed siblings (`load_ca_version` returns
`Result<i64> = Result<_, Report<PkiError>>`, `pki.rs:115`; `Report`'s Debug is the idiomatic render).

### 2. `spawn_server_cert_renewal` (`tasks.rs:483`)

Two changes — un-block the read **and** log:

```rust
let cert_pem = match tokio::fs::read_to_string(&cert_path).await {
    Ok(pem) => pem,
    Err(e) => {
        tracing::error!(error = %e, path = %cert_path.display(),
            "server cert renewal: failed to read server.crt; skipping this cycle");
        continue;
    }
};
```

- **`error = %e` (Display), not `?e`:** the read returns a plain `std::io::Error`, not a `Report` — `%e` matches
  both `logging.md`'s stated default ("`%e` for most errors; `?e` only when Display is insufficient") and the
  loop's own non-`Report` sibling `rcgen::Error` at `:501`/`:510` (which use `%e`). `path = %cert_path.display()`
  matches the crate's field style (`boot/reload.rs:224`).
- **`tokio::fs::read_to_string(&cert_path).await` is behavior-identical** to the `std::fs` call (same
  `io::Result<String>`, ENOENT→`Err`, UTF-8 and symlink-follow semantics) but non-blocking on the worker. This is
  the IO-**call** idiom already used at `installation_id.rs:20` / `server.rs:186`. **Add `"fs"` explicitly to
  `controller-runtime`'s `tokio` features** — it currently arrives only via feature-unification (three existing
  `tokio::fs` sites compile today), which is a latent fragility (a future build that drops the unifying crate
  would break compile). Declaring the feature this crate directly uses is correct hygiene and cheap insurance.

### Rate-limiting: rejected (YAGNI)

The finding says "ideally rate-limited or on first occurrence." **Rejected** as over-engineering:

- **Cert renewal (24h interval):** at most one log/day — zero spam risk; plain per-occurrence log is correct.
- **CA reload (30s interval):** a *persistent* failure logs ~120×/hour — but a persistently-failing CA-version
  query means the DB is broken, a genuine incident the operator *wants* surfaced. 120 lines/hour is far under
  journald's default rate limit (`RateLimitBurst`≈10000 per `RateLimitIntervalSec`≈30s per service), so it is
  signal, not noise. This is exactly the cadence the loop's existing `error!` siblings already accept (they too
  `continue` and would log every 30s on persistent failure). Adding a `bool`/counter edge-trigger (and its test)
  to suppress logs *during a real outage* is complexity for negative value.

**Considered-and-deferred alternative:** edge-triggered "log on failure-onset + recovery only." Add *only* if log
volume is later shown to be a problem — not now.

### Resilience posture (why log-and-continue is right here)

Both loops refresh *detection* against an already-loaded state, not active crypto: a persistent `load_ca_version`
failure stalls detection of a CA *rotation*, but the controller keeps serving TLS on the CA it booted with — no
key is lost. A failed cert read only defers *renewal*. So log-and-continue is a **bounded degradation made
visible**, not a silent crypto failure — and it matches the posture the existing `error!` siblings in both loops
already take for the same failure class. The `error!` level introduces **no new alert class**: siblings already
emit `error!` for this failure mode, so anything scanning for `error!` fires on it today. Escalating to a
supervisor/health-signal would be a larger change across all arms of both loops — out of scope here.

## Tests

**No new unit test.** This change adds **no new branching logic**: it converts two silent `else { continue }` into
logged `Err(e) => { error!; continue }` (pure observability) plus one blocking→async IO-mode swap
(behavior-preserving). Per the repo testing decision-table, tracing output is not unit-tested and `tokio::fs` is
an upstream dep whose behavior we do not test. Both loops are `tokio::spawn`'d infinite polling loops that are
impractical to unit-test without refactoring the whole task. The only extractable "unit" would be the *rejected*
edge-trigger state — do **not** add it. Do **not** add `start_paused` (no tokio-time assertion is introduced).

## Verification

- `cargo check --all-features` / `cargo clippy --all-targets --all-features` clean — the added `.await` compiles;
  no blocking `std::fs::` remains in `tasks.rs` (grep confirms zero after the change).
- Grep `tasks.rs` for `let Ok(` + `else {` and `std::fs::` → both patterns gone from these two loops; every
  poll-loop error arm now logs.
- Manual: confirm the two `error!` lines carry the error and (for the cert read) the path.

## Deliverables

- `crates/core/controller-runtime/src/tasks.rs` — the two logged `Err` arms + the `std::fs` → `tokio::fs` swap.
- `crates/core/controller-runtime/Cargo.toml` — add `"fs"` to the `tokio` features list (declare the feature the
  crate now directly depends on, instead of relying on feature-unification).

### Documentation deliverables

- **No doc impact.** Internal background-task observability; no API, config, or externally-observable behavior
  surface. `docs/development/logging.md` documents the logging *policy* (journald/stdout, no secrets), which this
  change already satisfies — it does not enumerate per-task log lines, so nothing to update there. (Explore to
  confirm no controller-tasks doc enumerates these loops' logging expectations; if one does, add a one-line note.)
- **No ADR** (bug fix). **No wire/OpenAPI/frontend change, no new dependency** — `tokio` is already a workspace
  dep; the only manifest change is enabling its existing `"fs"` feature on `controller-runtime` (feature-enable,
  not a new crate).

## Alternatives considered

- **Edge-triggered / rate-limited logging** — rejected (deferred): adds state + a test to suppress logs during a
  real DB outage; negative value at 30s/24h cadences with journald de-dup. See § Rate-limiting.
- **Log at `warn` instead of `error`** — rejected: `warn` is defensible crate-wide (`reload/reconciler.rs:54`
  uses `warn!` for an identical "DB poll failed, retry next tick" event), but every existing arm of *both* target
  loops uses `error!` and equally `continue`s, so `warn` would introduce a `warn`/`error` mix within one `match`
  chain. Local consistency wins; led with `error!`.
- **Keep `std::fs` but wrap in `spawn_blocking`** — rejected: `tokio::fs::read_to_string` is the idiomatic,
  already-used-in-crate primitive for a single small read; `spawn_blocking` is heavier and unprecedented here.

## Out of scope

Other unspecced immediate-Medium findings in different subsystems (core-agent-ssh L876, core-mqtt-scheduler L911,
plugins-infra L1042, ui-cli-surface-proxy L1093/1110/1126, web-api-routes L1226) — separate specs. No change to
poll intervals, to the CA-rotation/renewal **logic** (only logging + the blocking-read fix are added), and no
retry/backoff/rate-limit machinery or new background tasks.
