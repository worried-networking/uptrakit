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
  the OLD client and passes, and the coordinator emits `SYSTEM_CONFIG_RELOAD_APPLIED`. The live consumers
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
2. **DB-sourced reloads never reach the reexec hook.** `ReloadSource::DbBump` (the path `PUT /api/v1/settings/*`
   uses) skips `triage::decide` entirely — so adding fields to triage alone would fix TOML edits while the
   documented primary UX stayed silently broken. The codebase's own answer is the validate-reject precedent:
   `DbPoolReloadable::validate()` rejects `db.url` changes with "requires reexec" even though triage also catches
   them — defense in depth covering both sources.

## Approach

Principle: **apply() must never report success while changing nothing.** Per subsystem, in order of preference:
wire into an existing live-swap mechanism; else force reexec (file-sourced) + validate-reject (both sources);
never keep a republish-to-nobody path.

### NATS — validate-reject + triage; delete the connect machinery

- Gut `NatsReloadable`: delete the watch channel, the `apply()` client-connect (the leak), and `health_check()`'s
  old-client borrow. Keep a minimal reloadable whose `validate()` rejects any `nats.url` change with
  "nats.url change requires restart" (exact shape of `DbPoolReloadable::validate()` /
  `EmbeddedServicesReloadable::validate()`). This makes both TOML- and DB-sourced changes fail loudly at validate
  time — no side effects, coordinator emits FAILED not APPLIED.
- Add `nats.url` to `reexec/triage.rs::decide()` (one line, file-sourced defense in depth, mirroring `db.url`
  which also exists in both places).
- This matches `docs/development/nats-integration.md`'s stated policy (URL hot-reload intentionally unsupported) —
  the code finally agrees with the doc. The `#[expect(dead_code)]` on the module goes away with the dead halves.

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
- Scope check during implementation: if a `tls.*` delta can change CA/trust material (not just the leaf), the
  same apply must also swap the client verifier (`DynamicClientVerifier::swap`, also existing); if trust material
  turns out to be immutable at runtime (`tls.trust_domain` structural), that field instead joins the
  validate-reject + triage set below. Leaf cert/key = hot swap; structural trust = restart. Map each `tls.*`
  field to exactly one of the two buckets — no field may stay in the republish-to-nobody bucket.
- `health_check()` becomes meaningful: assert the resolver now serves the new cert (compare leaf fingerprint).

### HTTPS/PKI listener addresses — reexec + validate-reject; delete drain scaffolding

- Add `network.https.addr` and `network.pki.addr` to `triage::decide()` — this also fixes the existing doc/code
  drift (runbook already promises it; reexec preserves the listen sockets via the FD-passing path, and an addr
  change simply binds fresh in the new process).
- `HttpsListenerReloadable::validate()` / `PkiListenerReloadable::validate()` reject addr changes ("requires
  restart") for the DbBump path, same precedent as above.
- Delete the never-set `draining` flags, their test-only setters, and the pre-bind probe (probing an addr whose
  change is now rejected is dead work). The reloadables shrink to validate-gates; their watch channels go away.

### The remaining dropped receivers — ruled in/out with evidence

- `ZeroconfReloadable`: same illusory class (republish-only, `_zeroconf_rx` dropped). Same minimal fix:
  validate-reject changes ("requires restart"), delete the channel. One-liner, closes the class.
- `EmbeddedServicesReloadable`: already validate-rejects topology changes (the shipped precedent) — its dropped
  receiver is harmless; leave as is.
- `PluginCatalogReloadable`: documented V1 no-op by design (parent spec §10.6) — out of scope.
- `DbPoolReloadable` / `AuditDispatcherReloadable`: **not** illusory — both have live production subscribers
  (config reconciler; `audit_log_filter_rx` through AppState). Untouched.
- `RuntimeConfigChannels`/`RuntimeConfigReceivers` (shared config-reload crate): boot-seeded; no evidence the
  coordinator re-pushes on apply and no production consumer of the nats/tls receivers found. Verify during
  implementation; if confirmed boot-seeded-only, note it in the module doc — removing it is a separate cleanup,
  not this spec.

## Tests

1. **NATS:** validate() rejects a url change (both delta sources — construct deltas as the coordinator would for
   Sighup and DbBump); triage `decide()` unit test gains the `nats.url` case; regression: a reload touching
   `[nats]` no longer opens any NATS connection (assert via a mock/unreachable URL — apply must not attempt a
   connect).
2. **TLS:** end-to-end-ish integration: build the resolver + reloadable, apply a delta with a fresh self-signed
   cert pair, assert `resolver.resolve()` serves the new leaf (fingerprint compare); revert restores the old one.
   This is the wiring test class whose absence let the no-ops ship (per-reloadable unit tests all passed while
   nothing was connected).
3. **Listeners:** validate() rejects addr change; triage gains https/pki addr cases; draining-flag tests deleted
   with the flag.
4. **Coordinator behavior unchanged** for genuinely reloadable sections (existing coordinator tests keep
   passing).
5. No `start_paused` on anything touching real sockets/certs; no tokio-time APIs added (snapshot rule).

## Documentation deliverables

- `docs/end-user/operator-runbook-reload.md`: reconcile the reexec-forcing field list with the now-true code
  (https/pki addr, trust_domain if bucketed structural, nats.url added; cert/key-path documented as genuinely
  hot-reloadable now).
- `docs/development/nats-integration.md`: URL-change section points at the validate-reject + restart behavior.
- `docs/development/coding-standards.md` Reloadable-trait section: add the rule distilled from this bug class —
  a Reloadable whose apply() has no live consumer must not exist; validate-reject or wire a real subscriber.
- Doc comments on the gutted/rewired reloadables.
- No new ADR: no new architecture — this makes existing documented behavior (runbook, nats doc, ADR-0008 reexec)
  actually true.

## Out of scope / deferred

- Live NATS client swapping in NotificationService/broadcasters (restart-only per policy).
- In-place listener rebind with drain (the deleted scaffolding's original ambition; reexec covers addr changes).
- Making `ReloadSource::DbBump` flow through the reexec hook (validate-reject covers the honesty requirement; a
  DB-driven auto-restart is a product decision, not a bug fix).
- `RuntimeConfigChannels` removal (verify-and-note only).
- `PluginCatalogReloadable` V1 no-op (documented design).
- The scheduler/other-binary reload paths (audit scoped this to the controller).
