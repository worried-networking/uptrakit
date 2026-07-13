# Honest Config Hot-Reload — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — two HIGH findings, one mechanism: "NATS config hot-reload is a
silent no-op that leaks a connection and reports success" and "TLS cert/key-path and HTTPS-addr reloads report
'Applied' but have no runtime effect".

## Problem

Every watch receiver returned by the reloadable constructors in `boot/reload.rs` is bound to an underscore and
dropped (`_tls_rx`, `_https_rx`, `_pki_rx`, `_zeroconf_rx`, `_plugin_rx`, `_embedded_rx`), violating the
documented Per-Section Watch Pattern ("inject receivers at construction time"). Consequences:

- **NATS:** `NatsReloadable::apply()` connects a real new `async_nats::Client`, `tx.send` fails (zero
  subscribers — `receiver()` has no call sites), the new client is dropped/disconnected, `health_check()` borrows
  whatever `tx` currently holds — the **freshly-connected new client, orphaned from every real consumer** (alive,
  flushes fine) — and passes, and the coordinator emits `SYSTEM_CONFIG_RELOAD_APPLIED`. The live consumers
  (`NotificationService`, `EventBroadcaster`, `BatchProgressBroadcaster`) hold a static clone captured once at
  boot. An operator migrating NATS servers sees "applied" while delivery silently stays on (or loses) the old
  server — and every unrelated file reload touching `[nats]` reconnects-and-discards a client (the leak).
- **TLS / HTTPS-addr:** the rustls `ServerConfig` is built once at boot, the HTTPS socket is pre-bound and never
  rebinds; `TlsSnapshotReloadable`/`HttpsListenerReloadable`/`PkiListenerReloadable` `apply()` only republish
  snapshots nobody reads. `reexec/triage.rs` checks only `db.url`, `master_key`, `log.path`,
  `embedded_services` — while the operator runbook *claims* `network.https_addr`, `network.pki_addr`, and
  `tls.trust_domain` force reexec (doc/code drift, verified). An operator rotating an expiring external cert gets
  audited success and the old cert served indefinitely. The `draining` flags in the two listener reloadables are
  never set outside tests — dead scaffolding for an unimplemented drain/rebind.

Two facts discovered during design that shape the fix:

1. **A cert hot-swap mechanism already ships.** `ControllerServerCertResolver` wraps `ArcSwap<CertifiedKey>`,
   implements `ResolvesServerCert`, is wired into the live rustls config, and is already swap-called from two
   production paths (manual renewal API, auto-renewal task). TLS leaf-cert reload does not need reexec — it needs
   the third caller of an existing, proven mechanism.
2. **DB-sourced reloads never reach the reexec hook — and for nats/network settings, never fire at all.**
   `ReloadSource::DbBump` skips `triage::decide` entirely. Deeper (verified during review): the
   `PUT /api/v1/global-settings/nats` and `/network` routes call `upsert_global_setting_raw`, which never bumps
   `settings_version`; the config reconciler (the only DbBump emitter) hardcodes its sections to
   `audit`/`registration`; and `sections_to_deltas` has no `nats`/`network`/`tls` arms. So the DB write path for
   these sections currently produces **no reload cycle whatsoever** — the validate-reject gates below fire on
   file-sourced reloads today and future-proof the DbBump path if that wiring ever lands. Stated as a known gap,
   not silently claimed as covered. The codebase's own answer for honest rejection is the validate-reject
   precedent: `DbPoolReloadable::validate()` rejects `db.url` changes with "requires reexec" even though triage
   also catches them — defense in depth.

## Approach

Principle: **apply() must never report success while changing nothing.** Per subsystem, in order of preference:
wire into an existing live-swap mechanism; else force reexec (file-sourced) + validate-reject (both sources);
never keep a republish-to-nobody path.

### NATS — validate-reject + triage; delete the connect machinery

- Gut `NatsReloadable`: delete the watch channel, the `apply()` client-connect (the leak), and `health_check()`'s
  old-client borrow. Keep a minimal reloadable whose `validate()` rejects any `nats.url` change with
  "nats.url change requires restart" (exact shape of `DbPoolReloadable::validate()`;
  `EmbeddedServicesReloadable::health_check()`'s documented `Ok(())` no-op is the precedent for the gutted
  methods). Any file-sourced change fails loudly at validate time — no side effects, coordinator emits FAILED
  not APPLIED.
- **Register it unconditionally.** Today the reloadable is pushed only when NATS is configured at boot
  (`if let (Some(nats), Some(url)) …`) — an unconfigured-NATS deployment registers nothing, so a `[nats]` delta
  matches zero reloadables and the coordinator vacuously reports APPLIED (the same bug class via absent
  registration). The gutted gate needs no client or URL handle — it is a pure prior-vs-new compare — so the
  conditional registration goes away with the connect machinery.
- Add `nats.url` to `reexec/triage.rs::decide()` (one line, mirroring `db.url` which also exists in both
  places). Direction note: `triage.rs` lives in controller-runtime, which already owns it — no
  `uptrakit-config-reload` → controller-runtime import is introduced (the forbidden direction). All changes stay
  in the existing `reload/` modules and `boot/reload.rs` wiring — no new boot phase.
- **Sequencing, stated plainly (contrarian):** for file-sourced reloads the coordinator runs triage *before*
  building deltas (`config-reload/src/coordinator/state_machine.rs` `process_request`, Sighup/FileWatch arm —
  the load-bearing ordering this whole section depends on; if a refactor ever reorders it, the gates below
  change meaning) — a triage hit reexecs and validate never runs. So the operator UX for a file-sourced
  `nats.url` edit is an **automatic graceful reexec** (the same UX as `db.url`), not a reload failure. The
  validate-reject gate is the backstop for delta sources that bypass triage — today that path is unreachable for
  nats (the known DbBump gap above), so mark the gate `// unreachable until DbBump wires nats sections` rather
  than calling it defense in depth. Same triage-preemption logic is why the addr gates below deliberately get
  **no** triage entry.
- Reject message wording: "nats.url change requires reexec" — matching the `db.url` precedent and the actual
  triage behavior (a file-sourced edit reexecs); "restart" is reserved for the listener-addr case below, which is
  deliberately NOT reexec-able.
- Feature-gate clarity: "register unconditionally" removes only the **runtime** `if let (Some(nats), Some(url))`
  gate; the `#[cfg(feature = "nats")]` block around the push stays (with the feature compiled out there is no
  type to register — additive-only, unchanged).
- This matches `docs/development/nats-integration.md`'s stated policy (URL hot-reload intentionally unsupported) —
  the code finally agrees with the doc. Explicit line item: remove the `#[expect(dead_code)]` on `mod nats` in
  `reload/mod.rs` — its reason string ("unused until Task 14") is already stale (the reloadable is conditionally
  wired today), and after this change nothing dead remains.

**Rejected:** wiring the receiver into `NatsTransport` and live-swapping clients in
`NotificationService`/broadcasters — three consumers captured statically at boot with no swap seam; building one
is real machinery for a change the project documents as restart-only. Reexec-triage-only — leaves the DbBump path
claiming success.

### TLS cert/key — real hot-reload via the shipped resolver

- `TlsSnapshotReloadable::apply()` calls `ControllerServerCertResolver::swap_cert()` with the newly loaded
  cert/key (the external-cert branch `pki::load_external_cert` provides the load path), becoming the third caller
  of the proven mechanism. `revert()` swaps back the pre-apply `CertifiedKey` (Reloadable contract: apply
  snapshots pre-apply state). The reloadable needs the resolver handle at construction
  (`PkiRuntime.server_cert_resolver` is available at wire-up time in `boot/reload.rs`).
- Bucketing, resolved during review (no runtime investigation left): `tls.cert_path`/`tls.key_path` (leaf) = hot
  swap via the resolver; `tls.trust_domain` is a real, independently stored TOML-only field with no DB-write
  route — restart bucket by construction (validate-reject + triage). If a `tls.*` delta can change CA/trust
  material beyond the leaf, the same apply also swaps the client verifier (`DynamicClientVerifier::swap`, also
  existing). No `tls.*` field may stay in the republish-to-nobody bucket. Note: cert/key-path hot-reload is
  **file-source only** — no DB route exists for these fields; document it that way.
- **Phase split (contrarian-driven; stash removed on review):** `validate()` does the full load — parse cert,
  parse key, verify the key matches the leaf (building the `CertifiedKey` surfaces this) — **and discards the
  result**. Today's metadata-only probe would let a mid-rotation write race (cert written, key not yet) through
  to apply, swapping in a mismatched pair that breaks every handshake while fingerprint health-check still
  passes; a full parse in validate rejects that before ANY subsystem's apply runs (the coordinator's
  all-validate-then-all-apply atomicity is why the full check belongs here, not only in apply). Read-only file
  loads in `validate()` match the crate's existing validate behavior (metadata probes, bind probes); "no state
  mutation" in the trait contract holds — which is also why validate does NOT stash the prepared `CertifiedKey`
  for apply: the trait gives no ordering guarantee that the next call on `&self` is the matching `apply()`, no
  sibling reloadable carries validate→apply state, and a stale stash consumed by a mismatched apply is a new bug
  class. Instead `apply()` **re-loads and re-verifies** the pair itself (cheap double disk read) and only then
  swaps — a mid-gap file change fails apply cleanly BEFORE any mutation (honest ApplyFailed, no broken
  handshakes). `apply()` stashes the pre-apply `CertifiedKey` in memory (that stash is apply-internal state for
  revert — the documented Reloadable snapshot pattern every sibling uses); `revert()` restores it via the same
  in-process store — never re-reads disk (an `ArcSwap` swap is state restoration, not I/O).
- `health_check()` asserts the resolver now serves the new cert (leaf fingerprint compare) — this proves **the
  swap took**, not pair validity; validity is already guaranteed twice upstream (validate's full parse, apply's
  re-verify before the swap) — health_check performs no third validation. Requires a small new accessor on
  `ControllerServerCertResolver` (e.g. `current() -> Arc<CertifiedKey>`) — it exposes only `resolve()`/`swap()`
  today; named here so the implementer doesn't discover it mid-flight.
- Boot ordering verified: `identity.pki.server_cert_resolver` (re-verified twice during review — `Identity.pki:
  PkiFields` at `boot/identity/mod.rs:46-48`, `server_cert_resolver` inside `PkiFields` at `:32`; one reviewer
  wrongly "corrected" this to a flat field) exists before `reload::wire()` runs and `&identity` is already
  passed in — handing the resolver to `TlsSnapshotReloadable::new()` is a signature change only. In
  controller-runtime `Identity` the resolver is a plain `Arc` (always present); `AppState.server_cert_resolver`
  is an `Option` with a `reload_from_config` fallback branch in the manual-renew handler — the controller binary
  always wires `Some`, so that fallback is for other embedders; assert (do not assume) this when wiring.
- **Owner split, external vs managed certs (contrarian Critical — resolved in scope):** `tls.cert_path`/`key_path`
  exist only in external-cert deployments (`has_external_tls_cert`), where the auto-renew task is already gated
  OFF (`boot/serve.rs` `if !ca.has_external_tls_cert`) — but the manual-renew API
  (`routes/server_cert.rs::renew_server_certificate`) is NOT gated: it mints an **internal-CA** cert into
  `pki_path/server.{crt,key}` and unconditionally swaps the resolver, silently replacing the operator's external
  cert with one from a different trust chain (and a later file reload flips back). Same illusory-success class
  this spec exists to kill, adjacent code — **in scope**: the manual-renew handler rejects when
  `has_external_tls_cert` ("server certificate is externally managed; rotate the files and reload instead").
  Placement (contrarian pass-2): the guard is an early return in the **outer** `renew_server_certificate`, NOT
  inside `_inner` — the handler funnels every `_inner` error through a catch-all that hardcodes
  `INTERNAL_SERVER_ERROR` + `AuditOutcome::Failed`; only an outer early-return yields the intended 409-class
  response and a semantically-correct rejection audit outcome.
  Consequence: in external-cert mode the reload path is the resolver's **single writer by construction**
  (auto-renew gated off, manual renew now rejected), which is what makes `revert()`'s in-memory restore
  unconditionally correct — no concurrent writer can be clobbered. State this ownership invariant in the
  reloadable's doc comment; a compare-and-swap guard in revert is NOT added (no second writer exists to race).

### HTTPS/PKI listener addresses — validate-reject only; delete drain scaffolding

Contrarian review killed the reexec route for addr changes, with code evidence: the reexec child uses the
inherited socket **unconditionally** when FDs are present (`boot/listeners.rs` — the configured addr is consulted
only in the fresh-bind branch), so an addr-change reexec would keep serving the **old** address — the exact
honesty bug this spec exists to kill, relocated. Making reexec rebind on addr delta (compare the inherited
socket's `local_addr` to the configured addr, drop and fresh-bind on mismatch) is real machinery with a
new failure mode (child fails to bind → controller down instead of failed reload). Therefore:

- `HttpsListenerReloadable::validate()` / `PkiListenerReloadable::validate()` **reject** addr changes with
  "listener address change requires a full controller restart". No triage entry for addr fields — which keeps
  this gate *live* for file-sourced reloads (triage would preempt validate; see sequencing note below). The old
  process keeps serving; the operator restarts deliberately.
- The runbook's claim that addr changes force reexec was never implemented (doc drift) — the doc deliverable
  corrects it to "requires full restart" instead of making the false claim true in a way the FD path can't honor.
- In-place rebind-with-drain or reexec-with-rebind stays deferred (listed in Out of scope) until someone actually
  needs runtime addr changes; an integration test proving the child binds the *new* addr is the admission ticket.
- Delete the never-set `draining` flags, their test-only setters, and **both** probe users — the pre-bind probe
  in `validate()` and the TCP-connect liveness probe in `health_check()` (a validate-only gate controls no
  socket, so the health probe is dead work too) — which orphans `reload/probe.rs` (`pick_probe_addr`) entirely;
  delete the module. The reloadables shrink to validate-gates; their watch channels go away.

### The remaining dropped receivers — ruled in/out with evidence

- `ZeroconfReloadable`: same illusory class (republish-only, `_zeroconf_rx` dropped). Same minimal fix:
  validate-reject changes ("requires restart"), delete the channel. One-liner, closes the class.
- `EmbeddedServicesReloadable`: already validate-rejects topology changes (the shipped precedent) — its dropped
  receiver is harmless; leave as is.
- `PluginCatalogReloadable`: documented V1 no-op by design (parent spec §10.6) — out of scope.
- `DbPoolReloadable` / `AuditDispatcherReloadable`: **not** illusory — both have live production subscribers
  (config reconciler; `audit_log_filter_rx` through AppState). Untouched.
- `RuntimeConfigChannels`/`RuntimeConfigReceivers` (shared config-reload crate): **live infrastructure, not
  removable** (resolved during review — AppState carries `config_receivers` in production, e.g. middleware).
  The nats/tls receivers specifically appear unread; harmless boot-seeded values. No action in this spec.

## Tests

1. **NATS:** validate() rejects a url change (file-sourced delta, as the coordinator constructs it — the DbBump
   path is unreachable for nats today, per the known gap above); the reloadable is registered without NATS
   configured and still rejects (unconditional-registration regression); triage `decide()` unit test gains the
   `nats.url` case; regression: a reload touching `[nats]` no longer opens any NATS connection (assert via a
   mock/unreachable URL — apply must not attempt a connect).
2. **TLS:** end-to-end-ish integration: build the resolver + reloadable, apply a delta with a fresh self-signed
   cert pair, assert `resolver.current()` returns the new leaf (fingerprint compare — `resolve()` takes a
   `ClientHello` that cannot be constructed outside a live handshake; `current()` is the new accessor this spec
   adds); revert restores the old one.
   This is the wiring test class whose absence let the no-ops ship (per-reloadable unit tests all passed while
   nothing was connected).
3. **Listeners:** validate() rejects addr change with the "requires full restart" message (live for file-sourced
   reloads — no triage preemption for addr fields, by design); draining-flag and probe tests deleted with the
   scaffolding.
4. **Manual-renew guard:** `renew_server_certificate` returns the rejection (409-class error) when
   `has_external_tls_cert`; unchanged behavior when managed (TestApp harness). This endpoint change requires
   `./scripts/regen-api.sh` (new error response documented in OpenAPI) — commit the regenerated spec + client.
5. **Coordinator behavior unchanged** for genuinely reloadable sections (existing coordinator tests keep
   passing).
6. No `start_paused` on anything touching real sockets/certs; no tokio-time APIs added (snapshot rule).

## Documentation deliverables

- `docs/end-user/operator-runbook-reload.md`: reconcile with the now-true code — `nats.url` and
  `tls.trust_domain` join the reexec-forcing list; `network.https.addr`/`network.pki.addr` are corrected from
  "forces reexec" (never implemented) to "requires full controller restart"; cert/key-path documented as
  hot-reloadable **via file reload only** (no DB route). Also fix the field-path spelling drift while there: the
  runbook writes flat `network.https_addr`; the config struct is nested `network.https.addr`.
- `docs/development/nats-integration.md`: URL-change section points at the validate-reject + restart behavior.
- `docs/development/coding-standards.md` Reloadable-trait section: add the rule distilled from this bug class —
  a Reloadable whose apply() has no live consumer must not exist; validate-reject or wire a real subscriber.
- Doc comments on the gutted/rewired reloadables.
- No new ADR: no new architecture — this makes existing documented behavior (runbook, nats doc, ADR-0008 reexec)
  actually true.

## Out of scope / deferred

- Live NATS client swapping in NotificationService/broadcasters (restart-only per policy).
- Runtime listener address changes entirely — both in-place rebind-with-drain (the deleted scaffolding's
  ambition) and reexec-with-rebind (would need addr-delta detection + fresh-bind fallback in the FD-inheritance
  path, plus an integration test that the child binds the new addr). Addr changes are restart-only until a real
  need appears.
- Making `ReloadSource::DbBump` flow through the reexec hook (validate-reject covers the honesty requirement; a
  DB-driven auto-restart is a product decision, not a bug fix).
- **HTTP-layer false-success on the settings PUT routes — named, accepted, deferred.** Two distinct gaps this
  spec deliberately does not conflate: (1) the coordinator DbBump path not emitting nats/network/tls deltas —
  fine to leave (no delta ⇒ no false APPLIED audit event); (2) `PUT /api/v1/global-settings/{nats,network}`
  returning 200 OK for a persisted setting with **zero runtime effect and no restart signal** — the original
  bug's class relocated to the HTTP boundary, live for the most common operator entry point. This spec's honesty
  guarantee is therefore **file-source-scoped**; the HTTP-layer fix (reject with "requires restart" or a
  `restart_required: true` response field on those routes) touches web-api routes + OpenAPI regen + frontend and
  is a separate follow-up spec — it must not silently fall off the backlog.
- `RuntimeConfigChannels` removal (verify-and-note only).
- `PluginCatalogReloadable` V1 no-op (documented design).
- The scheduler/other-binary reload paths (audit scoped this to the controller).
