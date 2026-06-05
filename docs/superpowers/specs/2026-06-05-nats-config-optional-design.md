# NATS Config Section Optional at Runtime

**Date:** 2026-06-05
**Status:** Approved

## Problem

Two bugs in `crates/shared/config-reload/src/config/` force the `[nats]` TOML section to
be present even when the operator has no NATS server:

1. `RuntimeConfig.nats: NatsConfig` (`mod.rs:53`) has no `#[serde(default)]` → serde
   hard-requires the section → `missing field 'nats'` on startup.
2. `NatsConfig::validate()` (`nats.rs:37`) treats an empty URL as an error
   (`"nats.url is empty"`) → even an empty `[nats]` section fails config validation.

## Desired Behaviour

| Config state                                    | Result                                      |
| ----------------------------------------------- | ------------------------------------------- |
| `[nats]` section absent                         | No startup error (TOML parse succeeds)      |
| `[nats]` present, no `url`                      | No startup error (validation passes)        |
| `[nats]` present, valid URL                     | NATS enabled (unchanged)                    |
| `[nats]` present, valid URL, server unreachable | Startup fails hard (unchanged, intentional) |

**Reconciliation caveat (pre-existing):** The NATS URL is persisted to the DB on first
run and the DB value wins on subsequent starts. Removing `[nats]` from TOML disables NATS
only when no URL has been previously persisted to the DB. This is existing behaviour,
not changed by this fix. To clear a previously-configured NATS URL, use the admin
settings API or clear the `nats.url` key in the DB directly.

## Fix

### 1. `crates/shared/config-reload/src/config/mod.rs` — two changes

**a)** Add `#[serde(default)]` to the `nats` field, consistent with every other optional
section in the same struct (`db`, `tls`, `audit`, `log`, `zeroconf`,
`embedded_services`):

```rust
#[serde(default)]
pub nats: NatsConfig,
```

**b)** In `RuntimeConfig::validate()`, guard the `nats.validate()` call so it is skipped
when NATS is not configured (empty URL = disabled):

```rust
if !self.nats.url.is_empty() {
    self.nats.validate()?;
}
```

This keeps `NatsConfig::validate()` semantically correct ("if populated, this URL is
valid") without splitting the disabled-path logic across callers.

### No changes to `nats.rs` or `reload/nats.rs`

`NatsConfig::validate()` stays as-is: errors on empty URL. `NatsReloadable::validate()`
continues to delegate to `NatsConfig::validate()`, which naturally rejects any hot-reload
attempt to clear the URL — preserving the existing "can't hot-disable NATS" invariant
without code changes. The existing `nats_validate_rejects_empty_url` test (line 205 of
`reload/nats.rs`) continues to pass.

**Edge case:** Whitespace-only `url` (e.g. `url = "   "`) is not treated as empty — it
propagates to the NATS client and fails hard at connection time. This is acceptable;
trimming is not added here.

**Known gap (out of scope):** Hot-_enabling_ NATS (adding `nats.url` to a live config
when NATS was disabled at startup) is silently ignored — no `NatsReloadable` is
registered, so the reload coordinator drops the delta with no warning. This is a
pre-existing limitation; a `tracing::warn!` for this case is deferred.

## Invariants Preserved

- Configured URL + unreachable server → startup still fails hard (no change to
  `controller-runtime/src/lib.rs`).
- `nats` feature compiled out → `RuntimeConfig.nats` field still parsed from TOML (no
  feature gate on the field) so configs with `[nats]` remain forward-compatible. All
  runtime paths are already `#[cfg(feature = "nats")]` gated.
- `#[cfg(not(feature = "nats"))]` not used anywhere — compliant with additive-only
  feature flag rule.

## Tests

`NatsConfig::validate()` is unchanged, so its existing tests in `reload/nats.rs` continue
to pass unchanged.

Add one new test to `crates/shared/config-reload/` covering the new guard in
`RuntimeConfig::validate()`:

| Test                                         | What it covers                                                                                                                          | Expected                                              |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `runtime_validate_skips_nats_when_url_empty` | `NatsConfig::default().validate()` still errors, but `RuntimeConfig::validate()` does not propagate that error when `nats.url` is empty | No `ConfigReloadError::Validate("nats.url is empty")` |

Concretely: construct a `RuntimeConfig` with an otherwise-valid state (or call only the nats
branch in isolation) and assert the nats guard skips validation when `url == ""`. The
simplest form is a direct unit test on `RuntimeConfig::validate()` with a stub config that
has a valid `master_key`, `network`, `tls`, and an empty `nats.url`.

Note: serde `#[serde(default)]` field mechanics are not tested — testing upstream derive
behavior is forbidden by `docs/development/testing.md`. The config-parsing change is
exercised by compilation and the runtime validate test above.

## Docs Impact

None. `ARCHITECTURE.md` references `NatsAccess` (service credential capability), not the
controller TOML config section. `CONTEXT.md` requires no new terms.

## Out of Scope

NATS connection resilience — if `nats.url` is set but the server is unreachable, startup
continues to fail hard. That is a separate concern and requires its own retry/reconnect
design.
