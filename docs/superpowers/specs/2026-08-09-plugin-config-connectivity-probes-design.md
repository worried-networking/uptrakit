# Plugin Config-Test Connectivity: Real Credential Liveness Probes

- **Date:** 2026-08-09
- **Status:** Approved (owner round 2026-08-09)
- **Scope:** `crates/ui/web-api` (test_action handler, rate-limit map, audit), `crates/plugins/infrastructure/core` (descriptor seam),
  `crates/plugins/releases/{github,gitlab,forgejo}` (probes), `crates/plugins/package-managers/npm` (declaration cleanup),
  `crates/shared/web-api-types`, frontend result copy, docs + new ADR. No wire or agent-core changes.

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
   declares no `config_test` at all, Cargo declares `config_test: [VersionDetection, UpdateCommandValidation]` (no `Connectivity`), and
   npm declares `Connectivity` but routes agent-side where it is unimplemented. The doc comment on `ConfigTestKind::Connectivity`
   (`crates/shared/types/src/config_test_kind.rs:19`) makes the same wrong `fetch_releases` claim.

## Decisions (owner-confirmed 2026-08-09)

| #   | Decision                                                                                                                                                                                                                                                                                                                                                    |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Credential path allows inline (unsaved) config; merged config runs through `restore_config_secrets` so `"***"` restores the stored secret                                                                                                                                                                                                                   |
| D2  | Rate limiting v1 = per-IP entry in the existing `RATE_LIMITS` path map; tenant-keyed limit + per-tenant in-flight cap are a registered follow-up                                                                                                                                                                                                            |
| D3  | A new ADR records the architecture: controller-only probes, no agent fallback, descriptor seam, no persisted results                                                                                                                                                                                                                                        |
| D4  | v1 probes: GitHub, GitLab, Forgejo; npm's stray `Connectivity` declaration is removed; Cargo AND Docker get typed not-supported (Docker dropped from v1 in review round 2, 2026-08-09: `DockerConfig` has no registry field — the registry derives from item-scoped `package_identifier`, which D5 forbids; registry-field + probe is a deferred follow-up) |
| D5  | Two-probe distinction: credential **liveness** is profile-scoped and is NOT a `fetch_releases()` call; reachability-for-an-item stays out of scope                                                                                                                                                                                                          |
| D6  | Controller-only execution, explicitly no agent fallback (an agent fallback would launder private-network targets past `is_private_host`)                                                                                                                                                                                                                    |
| D7  | Declaration seam is the existing descriptor machinery (`declare_plugin!` / `PluginDescriptor`; `ConfigTestOps` unchanged — nesting rejected in review); no new method on the `PluginConfig` trait                                                                                                                                                           |
| D8  | Response carries typed allowlisted fields only; provider response bodies are never echoed; provider-side scope is not reported in v1                                                                                                                                                                                                                        |
| D9  | No persisted validation results                                                                                                                                                                                                                                                                                                                             |

## Design

### Phase 0 — secrets-restore prerequisite (lands first)

In `test_action.rs`, when `plugin_config_id` is present, after the existing shallow merge of `body.config` over `saved.config`, call
`state.plugin.plugin_ops.restore_config_secrets(&plugin_type_id, &mut merged, &saved.config)`. The saved row holds raw (unmasked)
config; restore maps every `"***"` field back to the stored secret while leaving genuinely new inline values untouched — identical
semantics to the CRUD update path. Without a `plugin_config_id` there is nothing to restore; a literal `"***"` then fails the probe
honestly as an invalid credential.

This also fixes the existing agent-side path, which today ships the merged config (with `"***"` clobbering the real secret) over the
wire.

### Probe seam: descriptor extension

Config-test kind metadata stays where it is: `ConfigTestOps` (emitted by `declare_plugin!` via `__declare_config_test_static!`,
wired through `descriptor.config_test` in `crates/plugins/infrastructure/core/src/descriptor.rs:119-125`) is unchanged
(verbatim):

```rust
pub struct ConfigTestOps {
    /// Which test kinds this plugin can handle. Non-empty.
    pub supported_kinds: &'static [ConfigTestKind],
    /// Default when the caller doesn't specify a kind. Must be in `supported_kinds`.
    pub default_kind: ConfigTestKind,
}
```

The probe itself is a new optional `PluginDescriptor` field: `pub connectivity_probe: Option<&'static dyn ConnectivityProbe>`.
The trait definition follows the workspace's `async_trait` role-trait idiom (`roles.rs`; `async-trait` is already a workspace
dependency). The **storage** is deliberately novel: existing descriptor extension points store constructor fn pointers producing
`Arc<dyn Trait>` from a `&CatalogConfig` at catalog-build time, but the probe needs no build-time config — a `&'static dyn`
trait object fits, and the ADR records this storage choice. The `Send + Sync` supertraits are load-bearing, not style: without
them the containing static cannot satisfy `Sync` (E0277).

```rust
#[async_trait]
pub trait ConnectivityProbe: Send + Sync {
    /// Deserializes the typed config itself (same pattern as the generated
    /// `mask_secrets` / `restore_secrets` fn pointers) and probes the provider.
    async fn probe(&self, config: serde_json::Value) -> ProbeOutcome;

    /// Resolves the host the probe will contact (scheme + host, no path/query),
    /// from config alone, WITHOUT performing I/O. Called by the handler BEFORE
    /// the probe runs so the audit event has the target even when the probe
    /// times out (a timeout produces no `ProbeOutcome`).
    fn target_host(&self, config: &serde_json::Value) -> Option<String>;
}
```

**The probe is a top-level `PluginDescriptor` field, not a `ConfigTestOps` member.** Review compiled a reproduction of the
nested alternative and hit the macro_rules sibling-optional-group wall ("attempted to repeat an expression containing no syntax
variables…" — the same repetition-driver problem `macros.rs:165` documents), which would force a real restructuring of
`__declare_config_test_static!`, per-combination helper arms to stay `#[allow]`-free under `warnings = "deny"`, and const-assert
gymnastics for kind-consistency. A separate top-level `pub connectivity_probe: Option<&'static dyn ConnectivityProbe>` on
`PluginDescriptor` avoids all of it: `declare_plugin!` gains one ordinary independently-optional section (the existing pattern for
optional descriptor fields), `ConfigTestOps` and `__declare_config_test_static!` stay byte-identical (`default_kind = first
listed kind` unchanged), and omission yields `None` — additive by construction, independent of feature flags (ADR-0032's
feature-monotonicity rule is not in play; no `cfg` involved).

Each plugin implements the trait on a unit struct and passes `&MyProbe` (rvalue static promotion coerces to
`&'static dyn ConnectivityProbe`, no `const` ceremony).

**Consistency validation moves to catalog admission** (ordinary code in the registry crate's existing validation pass, not
const-eval): a descriptor with `connectivity_probe` but no `config_test`, or without `ConfigTestKind::Connectivity` in
`supported_kinds`, fails catalog build at startup (fail-fast) and is covered by a registry test over `all_descriptors()`.

`ProbeOutcome` lives in `uptrakit-plugin-infrastructure-core`:

```rust
#[non_exhaustive]
pub struct ProbeOutcome {
    pub success: bool,
    pub failure_reason: Option<ConfigTestFailureReason>,
    pub detail: Option<String>, // short, sanitized, plugin-authored; never a provider body
}
```

`ConfigTestFailureReason` lives in `crates/shared/types` and is defined via the `wire_safe_enum!` macro
(`uptrakit-shared-macros`) — mandatory here per the coding-standards wire-safe rule, which covers enums returned as JSON strings
in REST responses (`TestPluginConfigResponse`; no wire payload carries it — see Response types). Variants: `invalid_credential`,
`unreachable`, `timeout`, `provider_error`, `invalid_config`, `not_supported`, plus the macro-generated `Other(String)`
forward-compat catch-all (an older peer receiving a future reason degrades gracefully instead of failing deserialization). The
macro provides the snake_case serde forms and `FromStr`.

Rejected seams:

- **Role trait** — roles are per `(host, software_item)` assignments; a liveness probe is profile-scoped, so the model does not fit.
- **Raw fn pointer returning `BoxFuture`** — zero workspace precedent (`BoxFuture` is unused across `crates/`); the codebase's
  idiom for pluggable async behavior is `async_trait` trait objects, adopted above.
- **Plugin-type match in the handler** — violates the plugin semantic-boundary CI gate (`check_plugin_semantic_boundary.py`).
- **New `PluginConfig` trait method** — the trait is sync and I/O-free and is compiled into the agent; an async HTTP method there
  collides with the additive-feature rules (D7).

### Handler routing

Today's `is_controller_side` short-circuit (`test_action.rs:187-196`) fires on capability alone, **before** `test_kind` is ever
parsed — a `version_detection` request against a `ControllerSideFetchReleases` plugin also gets the fake success and never reaches
an agent. The fix restructures the handler: parse `test_kind` first (existing string→`ConfigTestKind` mapping), delete the
capability short-circuit entirely, then route (after validation, dangerous-pattern gate, and Phase 0 restore).

**Kind defaulting is per-plugin, from the descriptor.** When the request omits `test_kind` — which the sole production caller
does: `frontend/src/routes/settings/PluginConfigsTab.svelte:521-527` sends only `plugin_type`/`config`/`plugin_config_id`, no
`test_kind`, no `host_id` — the handler resolves the default via `config_test_info(&plugin_type_id).default_kind`
(`plugin_ops.rs:117-120`, dead code today; this wires it). There is **no** global fallback kind: a plugin that declares no
`config_test` has nothing to default to and nothing declared to run — it returns `not_supported` (routing rule 1). That is a
stated behavior change: today such plugins either got the fake success (controller-side) or were agent-dispatched with the
hardcoded `"version_detection"` literal; after this change the descriptor is authoritative and undeclared means unsupported.
`TestPluginConfigRequest`'s doc comment (`plugin_config_test.rs:30-34`) already gestures at auto-detection, but
its capability-based three-bullet description is wrong under the new mechanism (it promises `connectivity` for _all_
controller-side plugins; Cargo's `default_kind` is `VersionDetection`) — that comment is rewritten to the descriptor-based rule
(listed under Documentation corrections). A single global default would misroute the primary UI flow for
github/gitlab/forgejo (whose `default_kind` is `Connectivity`, first listed) into agent dispatch and a spurious
`400 host_id is required`.

**The descriptor is the single routing authority** — explicit kinds are gated the same way defaults are resolved:

1. **Resolved kind ∉ `supported_kinds`, or no `config_test` declared at all** → HTTP 200, `success: false`,
   `failure_reason: not_supported`. This replaces today's fake success for the whole class of `ControllerSideFetchReleases`
   plugins without a probe (Cargo, Docker, and any current or future sibling — the class is asserted by a registry test over
   `all_descriptors()`, never hand-listed in prose) and also stops undeclared kinds (e.g. `version_detection` against github,
   which declares only `Connectivity`) from dead-ending in a spurious `400 host_id is required` agent dispatch.
2. **Kind is `connectivity` and the descriptor has a probe** → run controller-side: wrap the probe future in
   `tokio::time::timeout(Duration::from_secs(10))` (timeout ⇒ `failure_reason: timeout`), measure wall-clock duration, map
   `ProbeOutcome` onto the response. HTTP 200 for both success and failure outcomes (a failed credential is a successful test run).
3. **Kind is `connectivity`, declared, but no probe** → HTTP 200, `success: false`, `failure_reason: not_supported`. Connectivity
   is controller-only by design (D6); a declared-but-probeless Connectivity never dispatches to an agent. (Unreachable for v1
   plugins after npm's cleanup; defensive arm, kept because catalog admission validates probe⇒declared, not declared⇒probe.)
4. **Any other case** (declared non-`connectivity` kinds) → agent dispatch (`ConfigTestProxy`, 30 s timeout). Behavior change:
   non-`connectivity` declared kinds against controller-side plugins (Cargo `version_detection`) previously got the fake success
   and now dispatch to the agent like any other plugin; the agent answers per its `handle_config_test` support matrix. The
   agent's unsupported arm stays as-is (string error, defense-in-depth) — after this gate the controller never dispatches a kind
   the descriptor doesn't declare, so the typed `not_supported` verdict is controller-authoritative and no wire change is needed.

Recorded limitation (goes in the ADR): dispatch keys on the plugin capability set, not on per-assignment `execution_site` — the
existing capability-vs-execution_site divergence in the test path is retained, not widened.

### Response and wire types

- `TestPluginConfigResponse` (`crates/shared/web-api-types/src/plugin_config_test.rs`) gains
  `failure_reason: Option<ConfigTestFailureReason>` — additive, typed, allowlisted. `output`/`error` remain short human-readable
  strings; provider response bodies are never copied into either (no-secrets-in-logs analogue, D8). **Every failure outcome
  populates `error`** with a human-readable rendering of `failure_reason` (+ truncated `detail`) — the frontend already renders
  `testResult.error ?? generic` (`PluginConfigsTab.svelte:529-533`), so leaving `error` empty would collapse all typed reasons
  into one generic string.
- `ProbeOutcome.detail` sanitization is a mechanism, not a comment: the handler hard-truncates `detail` at 200 chars at the seam
  (char-boundary-safe — `String::truncate` on a non-boundary byte index panics, violating the no-`panic!` rule; truncate on
  `char_indices`), test-covered with an over-long plugin-supplied detail, and `detail` is excluded from audit-event details
  entirely.
- Adding the `connectivity_probe` field breaks every hand-written `PluginDescriptor` literal (`PluginDescriptor` has no `Default`
  and is not `#[non_exhaustive]`) — the plan must sweep literal construction sites, notably the registry's `test_support.rs`
  fixtures.
- **No wire change.** `TestPluginConfigResultPayload` is untouched: the descriptor gate means the controller never dispatches an
  undeclared kind, so `failure_reason` is REST-only and controller-produced. (Adding a wire `failure_reason` whose only producer
  would be the agent's now-unreachable unsupported arm was cut as over-engineering in review; it can be added additively if the
  agent ever gains real reasons.) `wire_safe_enum!` still applies to `ConfigTestFailureReason` — the coding-standards wire-safe
  rule covers enums returned as JSON strings in REST responses.
- Failure copy for 403 is pinned: "the provider rejected the request (403) — the credential may be live but blocked by SSO, org
  policy, or scope" — not a retry prompt, not `invalid_credential`.
- REST contract change ⇒ `./scripts/regen-api.sh`; commit `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`.

### Per-plugin probes

Each probe lives in its plugin crate and reuses that crate's existing client construction — `build_plugin_http_client` with
`SsrfMode::Strict` (`SsrfSafeResolver::new()`), WebPKI TLS, no redirects — plus the config
validators' existing https-only + `is_private_host` rejection. No new client posture is introduced anywhere; in particular the
generic path cannot inherit Proxmox's permissive resolver or SMTP's no-resolver, because there is no generic path — every probe is
plugin-authored behind the descriptor's `ConnectivityProbe` trait object. The three v1 probes differ only in path, auth header,
and marker field, so their impls may share a parameterized executor helper in `infrastructure-core`
(`{credentialed_path, anonymous_path, auth_header, marker}`), each invoked with the plugin's **own** client — the status mapping
is then tested once centrally with thin per-plugin tests on top, and the trait seam stays open for Docker's future token flow.

| Plugin  | With credential                                      | Anonymous (no token)        |
| ------- | ---------------------------------------------------- | --------------------------- |
| github  | `GET {api_base}/user` with `Authorization: Bearer`   | `GET {api_base}/meta`       |
| gitlab  | `GET {base}/api/v4/user` with `PRIVATE-TOKEN`        | `GET {base}/api/v4/version` |
| forgejo | `GET {base}/api/v1/user` with `Authorization: token` | `GET {base}/api/v1/version` |

GitLab uses `/api/v4/user` (stable across old self-hosted instances), **not** `/personal_access_tokens/self` (404 on older
GitLab, which the status mapping must never read as a credential verdict).

**Status mapping is uniform across providers and total** (every status has a row — no fall-through to an unspecified outcome):

| Credentialed-probe status           | Outcome                                                                                                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2xx with provider marker (below)    | `success: true`                                                                                                                                    |
| 2xx without provider marker         | `failure_reason: provider_error` (URL answered but is not the expected provider — a typo'd base URL must not certify green)                        |
| 401                                 | `failure_reason: invalid_credential`                                                                                                               |
| 403                                 | `failure_reason: provider_error` (indeterminate — GitHub App/installation tokens 403 on `/user`; secondary rate limits; scope-blocked live tokens) |
| any other status (404, 3xx, 5xx, …) | `failure_reason: provider_error`, status class in `detail`                                                                                         |
| connect/DNS/TLS error               | `failure_reason: unreachable`                                                                                                                      |
| handler timeout                     | `failure_reason: timeout`                                                                                                                          |

**Provider markers** — a 2xx is a success only if the size-capped response body parses as JSON and contains the marker field:
github `/user` ⇒ `login`, `/meta` ⇒ `ssh_key_fingerprints`; gitlab `/api/v4/user` ⇒ `username`, `/api/v4/version` ⇒ `version`;
forgejo `/api/v1/user` ⇒ `login`, `/api/v1/version` ⇒ `version`. Without this, any reachable https host answering 2xx would pass,
and a non-provider host answering 401 would misdiagnose a healthy credential as dead.

Anonymous probes validate base-URL liveness: 2xx requires the version/meta marker as above; 401/403 (auth-required instance)
counts as reachable ⇒ success; connect/DNS/TLS errors and timeout fail; any other status ⇒ `provider_error`. **Accepted
residual:** a 401 carries no marker-checkable body, so a typo'd non-provider host that answers 401 passes an anonymous test —
the anonymous success copy therefore claims only "URL is reachable" (provider identity is verified only on 2xx); no credential
is at stake on this arm.

The marker check reads at most 64 KiB of the response body (cap the read, then parse).

**A liveness pass is not a scope guarantee.** A live token with wrong scopes or org access still passes `GET .../user`. The
response and UI copy must say what was tested: success text is "Credential is live" with an explicit "does not verify scope or
repository access" caveat (frontend deliverable below); scope verification stays out of scope (D8).

### Declaration changes

- `releases/{github,gitlab,forgejo}`: keep `config_test: [Connectivity]`, add `connectivity_probe`.
- `releases/docker`: untouched (no `config_test`) — any test request now yields typed `not_supported` instead of fake success.
  The registry-field + probe follow-up is deferred (see below).
- `package-managers/npm`: remove `ConfigTestKind::Connectivity` from its `config_test` list (nothing implements it;
  `VersionDetection` stays). Sweep predicate for the removal: `rg -n 'Connectivity' crates/plugins/package-managers/npm` must return
  no matches afterwards.
- `package-managers/cargo`: untouched — a connectivity request now yields typed `not_supported` instead of fake success.

### Security hardening (ships with the probe, same milestone)

- **Authorization** — unchanged: `CanTriggerPluginConfigs` (`plugin-configs:trigger`), not admin.
- **Audit** — the `audit-catalog.toml` skip for
  `uptrakit_web_api::routes::plugin_configs::test_action::test_plugin_config` (rationale `"test probe; no state change, no
AuditEntry emit"` — false once a real probe exists) is
  replaced by an Event-class action `plugin_config.test.triggered` registered in `action_type.rs`, emitted via `emit_event` on every
  test invocation (controller-probe, not-supported, and agent-dispatch paths alike). Target: plugin type + optional plugin-config id;
  details: `test_kind`, outcome, `failure_reason`, and — for controller probes — the **probe target host** (scheme + host only, no
  path/query/credentials): the endpoint becomes an outbound-request primitive, and the audit trail must be able to answer "what did
  the controller connect to". No snapshots (Event class forbids them); no config or secret material and no `detail` in details.
- **Rate limit (v1, D2)** — add `/api/v1/plugin-configs/test` to the `RATE_LIMITS` map in
  `crates/ui/web-api/src/middleware/rate_limit.rs`: 10 requests / 60 s per IP, `fail_closed: true` (matches the login tier). 429
  with `Retry-After` comes from the existing middleware. Known properties of the mechanism, accepted for v1: the middleware
  silently skips limiting when the `ClientIp` extension is absent (`rate_limit.rs:198-201`), and behind a reverse proxy the
  per-IP key makes 10/60 s effectively a shared cap for all UI users. Tenant-keyed limiting and a per-tenant in-flight cap are
  deliberately deferred (follow-up below); the `ConfigTestProxy` pending map stays unbounded in v1. Recorded residual (goes in
  the ADR): with inline config (D1), any
  holder of `plugin-configs:trigger` can direct authenticated GETs at arbitrary non-private https hosts and observe status
  class/timing — a new outbound-request oracle this endpoint did not previously have; the per-IP limit is the weaker control for
  that vector, which is why the audit event records the target host and why the tenant-keyed follow-up exists.
- **No persistence** — probe outcomes are returned to the caller and audited; nothing is written to plugin-config rows (D9).

### Documentation corrections

`docs/development/plugin-guidelines.md`:

- Connectivity table row rewritten: profile-scoped credential **liveness** probe (provider self-endpoint), explicitly not a
  `fetch_releases()` call.
- Controller-side test path paragraphs rewritten: dispatch predicate is the descriptor (`supported_kinds` gate +
  `connectivity_probe`), no plugin instantiation via `NoopCommandExecutor` for this path. No hand-maintained plugin list — the
  doc points at `all_descriptors()` and the registry class-guard test (per the AGENTS.md anti-inventory rule; review found the
  would-be list already stale on arrival: the `skills` plugin also fake-passes today).
- Link the new ADR.

`crates/shared/types/src/config_test_kind.rs:19`: the doc comment on the `Connectivity` variant currently reads "Test connectivity
for controller-side plugins (`fetch_releases`)" — same wrong claim; rewrite to the liveness-probe semantics.

`crates/shared/web-api-types/src/plugin_config_test.rs:30-34`: the `test_kind` field's doc comment describes capability-based
defaulting ("`connectivity` for controller-side plugins", …) — stale under the descriptor mechanism; rewrite to the
`default_kind`-from-descriptor rule (no fallback: plugins without `config_test` are `not_supported`).

Per the common-mistakes ledger: the rewritten section must not assert mechanisms unverified against code — every claim in the new
text names the code path that implements it, and the plan must include a verify-against-source step for any sentence carried over
from the old text.

## New ADR

Created with `adrs new "Controller-only connectivity probes for plugin config tests"` (never hand-numbered). Records: the two-probe
distinction (D5), controller-only execution with explicitly no agent fallback and the SSRF-laundering rationale (D6), the
top-level `PluginDescriptor.connectivity_probe` seam (`&'static dyn` storage choice) and rejected alternatives (D7; note the
`ConfigTestOps`-nesting shape was itself rejected during review — `ConfigTestOps` ships unchanged), no persisted results (D9),
the outbound-oracle residual, and the retained capability-vs-execution_site divergence as a known limitation. ADR sections must
avoid `adrs doctor` hard-fail tokens (no ellipsis-as-placeholder, no TODO).

## Testing

Success and failure paths per the testing standard; no real sleeps; no upstream-crate behavior tests.

**Plugin crates** (probe logic, per crate, against local mock HTTP servers following each crate's existing HTTP test pattern):

- valid credential ⇒ `success: true` (marker present)
- 2xx **without** the provider marker ⇒ `provider_error` — locks the typo'd-base-URL rule
- 401 ⇒ `invalid_credential`; 403 ⇒ `provider_error` (indeterminate, NOT `invalid_credential`)
- unlisted status (404) ⇒ `provider_error` — locks the total-mapping rule
- connect error ⇒ `unreachable`; 5xx ⇒ `provider_error`
- anonymous config ⇒ reachability semantics (401 counts as success; 2xx needs the version/meta marker)
- `target_host()` resolves scheme+host from config without I/O

Constraint verified at plan time: the Strict resolver blocks localhost, so probes must go through each crate's injectable
client-construction seam exactly as the existing `fetch_releases` tests do. If a crate's seam is not injectable, making it so is in
scope for that crate.

**web-api (TestApp harness — mandatory for endpoint tests):**

- **Regression:** saved profile + masked `"***"` form value ⇒ probe receives the stored secret. Fixture: a `test-support`-feature
  descriptor in the registry crate (precedent: `test_support.rs` + the static-capture pattern in `controller_fetch.rs` tests) whose
  `ConnectivityProbe` impl records the config it receives for assertion.
- revoked/invalid credential ⇒ HTTP 200, `success: false`, `failure_reason: invalid_credential`
- valid credential ⇒ `success: true` with real (non-zero) `duration_ms`
- **explicit** `connectivity` against a cargo-shaped fixture (declares other kinds, not `Connectivity`) ⇒ `success: false`,
  `failure_reason: not_supported`; a no-`config_test` fixture (docker-shaped) ⇒ `not_supported` for **any** kind — the two
  fake-success regression gates. (An omitted kind against the cargo shape defaults to `VersionDetection` ⇒ agent dispatch, per
  the bullet below — not `not_supported`.)
- explicit kind ∉ `supported_kinds` (e.g. `version_detection` against a Connectivity-only fixture) ⇒ `not_supported`, no agent
  dispatch — locks the descriptor-as-single-authority rule
- over-long plugin-supplied `detail` ⇒ truncated to 200 chars in the response, absent from the audit event
- probe timeout ⇒ `failure_reason: timeout` (paused-time test via `start_paused = true` + `tokio::time::advance`; note: only if the
  test uses no real DB — DB-backed tests must not use `start_paused`, so the timeout test isolates the timeout wrapper. **Known
  coverage gap, accepted:** the handler's end-to-end timeout wiring is not exercised under the DB-backed harness)
- registry: catalog-admission validation test — descriptor with `connectivity_probe` but no `config_test`/no `Connectivity` in
  `supported_kinds` fails catalog build; class-guard over `all_descriptors()` asserts every probe-bearing descriptor declares
  `Connectivity`
- non-`connectivity` kind against a controller-side plugin ⇒ agent dispatch attempted (400 "no agent" in the harness without a
  connected service), not fake success — locks the routing restructure
- omitted `test_kind` + probe-bearing plugin ⇒ routes to the probe (descriptor `default_kind` resolution), no `host_id` required —
  locks the UI flow
- audit Event `plugin_config.test.triggered` emitted per invocation (assert via the test audit backend)
- rate limit: 11th request within the window ⇒ 429 (harness test; plan must first confirm the TestApp harness populates the
  `ClientIp` extension — the middleware silently skips limiting without it, so an unpopulated harness makes this test pass
  vacuously green-side and unable to produce the 429)

**agent-core:** untouched — the existing `unsupported_test_kind_returns_error` test keeps covering the defensive unsupported arm;
no wire or agent change ships.

**Ledger-mandated dry-runs:** every done-when grep in the implementation plan (e.g. the npm sweep, the fake-success string removal
`rg -n '"Plugin configuration is valid"' crates/ui/web-api` ⇒ empty) must be dry-run against both the pre-change corpus (non-empty)
and a synthetic post-change state before being trusted.

## Acceptance criteria

1. Saved profile + masked `"***"` form value ⇒ probe authenticates with the stored secret (regression test above).
2. Revoked/invalid credential ⇒ `success: false` with typed `failure_reason`.
3. Valid credential ⇒ `success: true` with real measured `duration_ms`.
4. A `connectivity` request never dispatches to an agent (descriptor gate); the agent's existing unsupported arm and its test stay
   unchanged as defense-in-depth. Undeclared kinds return typed `not_supported` from the controller.
5. Explicit `connectivity` against a plugin without a probe, and any kind against a plugin without `config_test`, return typed
   `not_supported` — `"Plugin configuration is valid"` no longer appears in `crates/ui/web-api`.
6. `plugin-guidelines.md` matches shipped behavior; new ADR merged.
7. Audit Event emitted per test invocation (with probe target host on controller probes); catalog skip removed;
   `cargo xtask audit-coverage-check` passes.
8. Rate limit enforced on the endpoint (harness test).
9. All regen gates clean: `regen-api.sh`, `regen-adr-toc.sh --check`, plus the standard quality gates (no asyncapi change).

## Deliverables (docs)

| File                                                                | Change                                                                                           |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `docs/adr/NNNN-*.md` (new, via `adrs new`)                          | probe architecture decision                                                                      |
| `docs/adr/README.md`                                                | regenerated via `scripts/regen-adr-toc.sh` (never hand-edited)                                   |
| `docs/development/plugin-guidelines.md`                             | Connectivity semantics + controller-side path corrected; points at descriptors, no hand-list     |
| `crates/ui/web-api/openapi.json`, `frontend/src/lib/api/generated/` | regenerated                                                                                      |
| `crates/shared/audit-log/audit-catalog.toml`                        | skip → `plugin_config.test.triggered`                                                            |
| `frontend/src/routes/settings/PluginConfigsTab.svelte`              | result copy: render `failure_reason`; success = "credential is live" + no-scope-guarantee caveat |

No new external dependencies; `async-trait` is already a workspace dependency (root `Cargo.toml`). One new intra-workspace
manifest edge: `crates/shared/types/Cargo.toml` gains `uptrakit-shared-macros` (`workspace = true`; already registered in
`[workspace.dependencies]`) for `wire_safe_enum!`.

## Deferred / out of scope

- Tenant-keyed rate limit + per-tenant in-flight cap on the test endpoint (registered follow-up; v1 is per-IP only — the weaker
  control against the outbound-oracle vector recorded in the ADR).
- Docker connectivity probe: requires an explicit `registry` field on `DockerConfig` (`is_private_host`-validated) plus the
  `docker login` challenge/token flow and an identity-bound success check (scope-less tokens are issued to anonymous callers on
  ghcr.io and others); dropped from v1 (review round 2, 2026-08-09).
- Reachability-for-an-item probes (needs `HostRuntime` + `package_identifier`; different feature).
- Provider-side scope/permission reporting in the response (D8).
- Resolving the capability-vs-execution_site divergence in the test path (recorded limitation in the ADR).
- Cargo/npm connectivity probes (crates.io is anonymous; npm routes agent-side).
- Bounding the `ConfigTestProxy` pending-request map (follows with the in-flight cap).

## Standards-snapshot conformance notes

- No new `PluginConfig` obligations for agent-compiled code; descriptor change is additive, involves no feature gating, and
  leaves ADR-0032 contribution-monotonicity untouched.
- `ConfigTestFailureReason` via `wire_safe_enum!` (wire-safe `Other(String)` rule — REST JSON-string enum); no
  `unwrap`/`panic!`; `rootcause` at boundaries.
- `SecretString` fields untouched; probe never logs or echoes secrets; audit details exclude config material.
- SSRF posture unchanged: Strict resolver + `is_private_host` per plugin; no agent fallback (D6).
- Endpoint tests on the TestApp harness; time-dependent tests paused; no upstream-crate behavior tests.
- OpenAPI params/body via existing `Validated<TestPluginConfigRequest>`; no inline param lists.
- Conventional Commits; ADR via `adrs` CLI only.
