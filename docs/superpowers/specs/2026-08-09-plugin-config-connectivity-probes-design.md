# Plugin Config-Test Connectivity: Real Credential Liveness Probes

- **Date:** 2026-08-09
- **Status:** Approved (owner round 2026-08-09)
- **Scope:** `crates/ui/web-api` (test_action handler, rate-limit map, audit), `crates/plugins/infrastructure/core` (descriptor seam),
  `crates/plugins/releases/{github,gitlab,forgejo,docker}` (probes), `crates/plugins/package-managers/npm` (declaration cleanup),
  `crates/shared/web-api-types`, `crates/shared/wire`, `crates/shared/agent-core`, docs + new ADR.

## Problem

The plugin config-test endpoint (`POST /api/v1/plugin-configs/test`) certifies dead credentials as valid. Verified 2026-08-09:

1. **Fake success.** `crates/ui/web-api/src/routes/plugin_configs/test_action.rs:187-196`: when the plugin's capability set contains
   `PluginCapability::ControllerSideFetchReleases`, the handler returns `success: true` / `"Plugin configuration is valid"` /
   `duration_ms: 0` without constructing the plugin or making any network call. An operator testing a revoked GitHub PAT sees
   "Config test passed."
2. **Agent side unimplemented.** `crates/shared/agent-core/src/config_test.rs:71-85`: `ConfigTestKind::Connectivity` falls into the
   not-yet-implemented catch-all arm.
3. **Masked-secret false negative (latent).** `test_action.rs` shallow-merges incoming config over the saved profile but never calls
   `restore_config_secrets` — the CRUD write path does (4 sites in `crates/ui/web-api-queries/src/queries/plugin_configs.rs:205,218,369,384`).
   The GET/edit path masks secrets to `"***"` unconditionally, so the moment a real probe exists, testing a saved profile from the UI
   would authenticate with the literal string `"***"` — a guaranteed false negative. This fix is a prerequisite and lands first.
4. **Docs describe behavior that does not exist.** `docs/development/plugin-guidelines.md:239,245-247` claims Connectivity performs a
   lightweight `fetch_releases()` call, keys the controller path on `HostRequirements::CONTROLLER_ONLY`, and lists Docker/Cargo/npm as
   Connectivity plugins. All three claims are wrong: no plugin is constructed, the branch keys on `ControllerSideFetchReleases`, Docker
   and Cargo declare no `config_test` at all, and npm declares `Connectivity` but routes agent-side where it is unimplemented.

## Decisions (owner-confirmed 2026-08-09)

| #   | Decision                                                                                                                                           |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Credential path allows inline (unsaved) config; merged config runs through `restore_config_secrets` so `"***"` restores the stored secret          |
| D2  | Rate limiting v1 = per-IP entry in the existing `RATE_LIMITS` path map; tenant-keyed limit + per-tenant in-flight cap are a registered follow-up   |
| D3  | A new ADR records the architecture: controller-only probes, no agent fallback, descriptor seam, no persisted results                               |
| D4  | v1 probes: GitHub, GitLab, Forgejo, **and Docker**; npm's stray `Connectivity` declaration is removed; Cargo gets typed not-supported              |
| D5  | Two-probe distinction: credential **liveness** is profile-scoped and is NOT a `fetch_releases()` call; reachability-for-an-item stays out of scope |
| D6  | Controller-only execution, explicitly no agent fallback (an agent fallback would launder private-network targets past `is_private_host`)           |
| D7  | Declaration seam is the existing descriptor machinery (`declare_plugin!` / `ConfigTestOps`); no new method on the `PluginConfig` trait             |
| D8  | Response carries typed allowlisted fields only; provider response bodies are never echoed; provider-side scope is not reported in v1               |
| D9  | No persisted validation results                                                                                                                    |

## Design

### Phase 0 — secrets-restore prerequisite (lands first)

In `test_action.rs`, when `plugin_config_id` is present, after the existing shallow merge of `body.config` over `saved.config`, call
`state.plugin.plugin_ops.restore_config_secrets(&plugin_type_id, &mut merged, &saved.config)`. The saved row holds raw (unmasked)
config; restore maps every `"***"` field back to the stored secret while leaving genuinely new inline values untouched — identical
semantics to the CRUD update path. Without a `plugin_config_id` there is nothing to restore; a literal `"***"` then fails the probe
honestly as an invalid credential.

This also fixes the existing agent-side path, which today ships the merged config (with `"***"` clobbering the real secret) over the
wire.

### Probe seam: `ConfigTestOps` extension

Extend the `ConfigTestOps` static (emitted by `declare_plugin!`, wired via `descriptor.config_test` in
`crates/plugins/infrastructure/core/src/descriptor.rs`) with an optional controller-side probe:

```rust
pub struct ConfigTestOps {
    pub kinds: &'static [ConfigTestKind],            // existing
    pub connectivity_probe: Option<ConnectivityProbeFn>, // new
}

pub type ConnectivityProbeFn =
    fn(serde_json::Value) -> futures::future::BoxFuture<'static, ProbeOutcome>;
```

`declare_plugin!` gains an optional `connectivity_probe:` argument next to `config_test:`; omitting it yields `None` (additive,
feature-monotonic — ADR-0032 compliant). The probe fn deserializes the typed config itself (same pattern as the generated
`mask_secrets` / `restore_secrets` fn pointers) and returns a typed outcome.

`ProbeOutcome` lives in `uptrakit-plugin-infrastructure-core`:

```rust
#[non_exhaustive]
pub struct ProbeOutcome {
    pub success: bool,
    pub failure_reason: Option<ConfigTestFailureReason>,
    pub detail: Option<String>, // short, sanitized, plugin-authored; never a provider body
}
```

`ConfigTestFailureReason` is a `#[non_exhaustive]` enum in `crates/shared/types` (serde `snake_case`, `FromStr` + typed parse error
per coding standards): `invalid_credential`, `unreachable`, `timeout`, `provider_error`, `invalid_config`, `not_supported`.

Rejected seams:

- **Role trait** — roles are per `(host, software_item)` assignments; a liveness probe is profile-scoped, so the model does not fit.
- **Plugin-type match in the handler** — violates the plugin semantic-boundary CI gate (`check_plugin_semantic_boundary.py`).
- **New `PluginConfig` trait method** — the trait is sync and I/O-free and is compiled into the agent; an async HTTP method there
  collides with the additive-feature rules (D7).

### Handler routing

For requested test kind `connectivity` (after validation, dangerous-pattern gate, and Phase 0 restore):

1. **Descriptor has a probe** → run controller-side: wrap the probe future in `tokio::time::timeout(Duration::from_secs(10))`
   (timeout ⇒ `failure_reason: timeout`), measure wall-clock duration, map `ProbeOutcome` onto the response. HTTP 200 for both
   success and failure outcomes (a failed credential is a successful test run).
2. **Controller-side plugin (`ControllerSideFetchReleases`) without a probe** (Cargo) → HTTP 200, `success: false`,
   `failure_reason: not_supported`. This replaces today's fake success.
3. **Otherwise** → agent dispatch exactly as today (`ConfigTestProxy`, 30 s timeout). The agent's unsupported arm keeps returning a
   typed error (see wire change below).

Recorded limitation (goes in the ADR): dispatch keys on the plugin capability set, not on per-assignment `execution_site` — the
existing capability-vs-execution_site divergence in the test path is retained, not widened.

### Response and wire types

- `TestPluginConfigResponse` (`crates/shared/web-api-types/src/plugin_config_test.rs`) gains
  `failure_reason: Option<ConfigTestFailureReason>` — additive, typed, allowlisted. `output`/`error` remain short human-readable
  strings; provider response bodies are never copied into either (no-secrets-in-logs analogue, D8).
- `TestPluginConfigResultPayload` (`crates/shared/wire/src/payloads.rs`) gains optional `failure_reason` (serialized as the same
  snake_case string; `WireValidate` impl updated with a length limit in `limits.rs`). The agent's unsupported arm
  (`agent-core/src/config_test.rs`) sets `failure_reason: not_supported`; the controller passes it through to the REST response.
  Additive wire change ⇒ `./scripts/regen-asyncapi.sh` + `asyncapi.yaml` committed; `docs/api/wire-protocol.md` updated.
- REST contract change ⇒ `./scripts/regen-api.sh`; commit `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`.

### Per-plugin probes

Each probe lives in its plugin crate and reuses that crate's existing client construction — `build_plugin_http_client` with
`SsrfMode::Strict` (`SsrfSafeResolver::new()`), WebPKI TLS, no redirects (Docker blob client excepted, unchanged) — plus the config
validators' existing https-only + `is_private_host` rejection. No new client posture is introduced anywhere; in particular the
generic path cannot inherit Proxmox's permissive resolver or SMTP's no-resolver, because there is no generic path — every probe is
plugin-authored behind the descriptor fn pointer.

| Plugin  | With credential                                                                                                                                                                                                                                                                         | Anonymous (no token)                                                      |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| github  | `GET {api_base}/user` with `Authorization: Bearer` — 2xx ⇒ success; 401/403 ⇒ `invalid_credential`                                                                                                                                                                                      | `GET {api_base}/meta` — any HTTP response ⇒ success (base URL live)       |
| gitlab  | `GET {base}/api/v4/personal_access_tokens/self` with `PRIVATE-TOKEN` — 2xx ⇒ success; 401 ⇒ `invalid_credential`; 403 ⇒ success (token live but scope-limited; scope not reported in v1)                                                                                                | `GET {base}/api/v4/version` — any HTTP response ⇒ success                 |
| forgejo | `GET {base}/api/v1/user` with `Authorization: token` — 2xx ⇒ success; 401/403 ⇒ `invalid_credential`                                                                                                                                                                                    | `GET {base}/api/v1/version` — any HTTP response ⇒ success                 |
| docker  | `docker login` flow: `GET {registry}/v2/` → `WWW-Authenticate` challenge → token request against the realm with Basic/Bearer creds and **no scope** → 200 token ⇒ success; 401 ⇒ `invalid_credential`. Reuses `registry.rs`/`auth.rs` machinery including `is_private_host(realm_host)` | `GET {registry}/v2/` — any HTTP response (200 or 401 challenge) ⇒ success |

Cross-cutting mapping: connect/DNS/TLS errors ⇒ `unreachable`; provider 5xx ⇒ `provider_error`; handler timeout ⇒ `timeout`.
Anonymous probes validate base-URL liveness only — a 401 from an unauthenticated meta endpoint means the host is reachable and is a
success.

### Declaration changes

- `releases/docker`: add `config_test: [ConfigTestKind::Connectivity]` + `connectivity_probe`.
- `releases/{github,gitlab,forgejo}`: keep `config_test: [Connectivity]`, add `connectivity_probe`.
- `package-managers/npm`: remove `ConfigTestKind::Connectivity` from its `config_test` list (nothing implements it;
  `VersionDetection` stays). Sweep predicate for the removal: `rg -n 'Connectivity' crates/plugins/package-managers/npm` must return
  no matches afterwards.
- `package-managers/cargo`: untouched — a connectivity request now yields typed `not_supported` instead of fake success.

### Security hardening (ships with the probe, same milestone)

- **Authorization** — unchanged: `CanTriggerPluginConfigs` (`plugin-configs:trigger`), not admin.
- **Audit** — the `audit-catalog.toml` skip for
  `uptrakit_web_api::routes::plugin_configs::test_action::test_plugin_config` (rationale "no state change, no I/O" — now false) is
  replaced by an Event-class action `plugin_config.test.triggered` registered in `action_type.rs`, emitted via `emit_event` on every
  test invocation (controller-probe, not-supported, and agent-dispatch paths alike). Target: plugin type + optional plugin-config id;
  details: `test_kind`, outcome, `failure_reason`. No snapshots (Event class forbids them); no config or secret material in details.
- **Rate limit (v1, D2)** — add `/api/v1/plugin-configs/test` to the `RATE_LIMITS` map in
  `crates/ui/web-api/src/middleware/rate_limit.rs`: 10 requests / 60 s per IP (matches the login tier). 429 with `Retry-After` comes
  from the existing middleware. Tenant-keyed limiting and a per-tenant in-flight cap are deliberately deferred (follow-up below);
  the `ConfigTestProxy` pending map stays unbounded in v1.
- **No persistence** — probe outcomes are returned to the caller and audited; nothing is written to plugin-config rows (D9).

### Documentation corrections

`docs/development/plugin-guidelines.md`:

- Connectivity table row rewritten: profile-scoped credential **liveness** probe (provider self-endpoint), explicitly not a
  `fetch_releases()` call.
- Controller-side test path paragraphs rewritten: dispatch predicate is the descriptor's `connectivity_probe` (with the
  `ControllerSideFetchReleases`-without-probe ⇒ `not_supported` rule), no plugin instantiation via `NoopCommandExecutor` for this
  path, correct plugin list (github, gitlab, forgejo, docker; cargo/npm not supported).
- Link the new ADR.

Per the common-mistakes ledger: the rewritten section must not assert mechanisms unverified against code — every claim in the new
text names the code path that implements it, and the plan must include a verify-against-source step for any sentence carried over
from the old text.

## New ADR

Created with `adrs new "Controller-only connectivity probes for plugin config tests"` (never hand-numbered). Records: the two-probe
distinction (D5), controller-only execution with explicitly no agent fallback and the SSRF-laundering rationale (D6), the
`ConfigTestOps` descriptor seam and rejected alternatives (D7), no persisted results (D9), and the retained
capability-vs-execution_site divergence as a known limitation. ADR sections must avoid `adrs doctor` hard-fail tokens
(no ellipsis-as-placeholder, no TODO).

## Testing

Success and failure paths per the testing standard; no real sleeps; no upstream-crate behavior tests.

**Plugin crates** (probe logic, per crate, against local mock HTTP servers following each crate's existing HTTP test pattern):

- valid credential ⇒ `success: true`
- 401 ⇒ `invalid_credential`; gitlab 403 ⇒ success
- connect error ⇒ `unreachable`; 5xx ⇒ `provider_error`
- anonymous config ⇒ reachability semantics (401 counts as success)
- docker: challenge→token flow, `is_private_host(realm)` rejection

Constraint verified at plan time: the Strict resolver blocks localhost, so probes must go through each crate's injectable
client-construction seam exactly as the existing `fetch_releases` tests do. If a crate's seam is not injectable, making it so is in
scope for that crate.

**web-api (TestApp harness — mandatory for endpoint tests):**

- **Regression:** saved profile + masked `"***"` form value ⇒ probe receives the stored secret (fixture descriptor with an injected
  probe fn asserting on the config it receives)
- revoked/invalid credential ⇒ HTTP 200, `success: false`, `failure_reason: invalid_credential`
- valid credential ⇒ `success: true` with real (non-zero) `duration_ms`
- controller-side plugin without probe (cargo-shaped fixture) ⇒ `success: false`, `failure_reason: not_supported` — the fake-success
  regression gate
- probe timeout ⇒ `failure_reason: timeout` (paused-time test via `start_paused = true` + `tokio::time::advance`; note: only if the
  test uses no real DB — DB-backed tests must not use `start_paused`, so the timeout test isolates the timeout wrapper if needed)
- audit Event `plugin_config.test.triggered` emitted per invocation (assert via the test audit backend)
- rate limit: 11th request within the window ⇒ 429 (harness test)

**agent-core:** existing `unsupported_test_kind_returns_error` extended to assert the typed `failure_reason: not_supported` on the
wire payload.

**Ledger-mandated dry-runs:** every done-when grep in the implementation plan (e.g. the npm sweep, the fake-success string removal
`rg -n '"Plugin configuration is valid"' crates/ui/web-api` ⇒ empty) must be dry-run against both the pre-change corpus (non-empty)
and a synthetic post-change state before being trusted.

## Acceptance criteria

1. Saved profile + masked `"***"` form value ⇒ probe authenticates with the stored secret (regression test above).
2. Revoked/invalid credential ⇒ `success: false` with typed `failure_reason`.
3. Valid credential ⇒ `success: true` with real measured `duration_ms`.
4. Agent-side Connectivity request still returns a typed not-supported error (now with `failure_reason: not_supported` end-to-end).
5. Controller-side plugin without a probe returns typed `not_supported` — `"Plugin configuration is valid"` no longer appears in
   `crates/ui/web-api`.
6. `plugin-guidelines.md` matches shipped behavior; new ADR merged; `docs/api/wire-protocol.md` covers the new wire field.
7. Audit Event emitted per test invocation; catalog skip removed; `cargo xtask audit-coverage-check` passes.
8. Rate limit enforced on the endpoint (harness test).
9. All regen gates clean: `regen-api.sh`, `regen-asyncapi.sh`, `regen-adr-toc.sh --check`, plus the standard quality gates.

## Deliverables (docs)

| File                                                                | Change                                                                |
| ------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `docs/adr/NNNN-*.md` (new, via `adrs new`)                          | probe architecture decision                                           |
| `docs/adr/README.md`                                                | regenerated via `scripts/regen-adr-toc.sh` (never hand-edited)        |
| `docs/development/plugin-guidelines.md`                             | Connectivity semantics + controller-side path + plugin list corrected |
| `docs/api/wire-protocol.md`                                         | `TestPluginConfigResultPayload.failure_reason` documented             |
| `crates/shared/wire/asyncapi.yaml`                                  | regenerated                                                           |
| `crates/ui/web-api/openapi.json`, `frontend/src/lib/api/generated/` | regenerated                                                           |
| `crates/shared/audit-log/audit-catalog.toml`                        | skip → `plugin_config.test.triggered`                                 |

No new external dependencies; `futures` (BoxFuture) is already a workspace dependency.

## Deferred / out of scope

- Tenant-keyed rate limit + per-tenant in-flight cap on the test endpoint (registered follow-up; v1 is per-IP only).
- Reachability-for-an-item probes (needs `HostRuntime` + `package_identifier`; different feature).
- Provider-side scope/permission reporting in the response (D8).
- Resolving the capability-vs-execution_site divergence in the test path (recorded limitation in the ADR).
- Cargo/npm connectivity probes (crates.io is anonymous; npm routes agent-side).
- Bounding the `ConfigTestProxy` pending-request map (follows with the in-flight cap).

## Standards-snapshot conformance notes

- No new `PluginConfig` obligations for agent-compiled code; descriptor change is additive and feature-monotonic (ADR-0032).
- Typed errors + `FromStr` for `ConfigTestFailureReason`; no `unwrap`/`panic!`; `rootcause` at boundaries.
- `SecretString` fields untouched; probe never logs or echoes secrets; audit details exclude config material.
- SSRF posture unchanged: Strict resolver + `is_private_host` per plugin; no agent fallback (D6).
- Endpoint tests on the TestApp harness; time-dependent tests paused; no upstream-crate behavior tests.
- OpenAPI params/body via existing `Validated<TestPluginConfigRequest>`; no inline param lists.
- Conventional Commits; ADR via `adrs` CLI only.
