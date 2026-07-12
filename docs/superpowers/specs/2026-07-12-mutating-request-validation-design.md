# Mutating Request Types Bypass `Validate` — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/shared/web-api-types/src/*` (add `Validate` impls + unit tests),
`crates/ui/web-api/src/routes/*` (wire validation into the handlers that skip it entirely),
`ci/verify_mutating_requests_validated.sh` (+ allowlist) + its wiring. No new deps, no wire change.

## Problem

Audit `audit-2026-07-11` L1246 (MEDIUM · stability · effort M · web-api-routes · verified): some mutating (Update /
PATCH / action) HTTP request handlers extract their body with plain `Json<T>` and **never validate it** — neither via
the `Validated<T>` extractor (`extract.rs:302-322`, the only extractor that calls `body.validate()`) nor via a manual
`body.validate()` call. So the format/length invariants the `Create` counterparts enforce are silently skipped.

Worst case: `update_provider` (`oidc_providers.rs:430`) takes `Json<UpdateOidcProviderRequest>` and writes
`name`/`slug`/`issuer_url`/`client_id` straight into the row. A PATCH can set `issuer_url` to `"ftp://x"` or `""`,
`slug` to `"UPPER/../x"`, or `name`/`client_id` to `""` — breaking OIDC login for **every** user of that provider at
discovery time. Tellingly, `create_provider` **does** validate and emits a `ValidationFailed` audit event
(`oidc_providers.rs:164`); `update_provider` even emits `ValidationFailed` for its manual field checks
(`oidc_providers.rs:569`) but **never calls `body.validate()`** — the format invariants are simply absent on the update
path. The defensive `provider.issuer_url.is_empty()` guard at `oidc_providers.rs:929` is a hint the hole is known.

**Root cause:** there is **no enforcement** that a mutating request body is validated on the request path. The
coding-standard rule exists ("HTTP request types accepting user input must implement `Validate`") but nothing checks
it, so the gap recurs. Fixing only the audit-named instances is symptom-patching; the fix must also close the class.

## Verified current reality (grounded 2026-07-12)

- **`Validate` trait** (`validation.rs:18-21`): `fn validate(&self) -> Result<(), ValidationError>`. **`Validated<T>`**
  (`extract.rs:302-322`) invokes it and returns `error_response(StatusCode::BAD_REQUEST, e.to_string())` on failure —
  a **generic** 400 with **no audit event and no entity context**.
- **This repo has two legitimate validation-invocation patterns for mutating handlers, not one:**
  1. **`Validated<T>` extractor** — generic 400, no audit; used where validation-failure is not audited.
  2. **Plain `Json<T>` + manual `body.validate()` that emits a domain-specific `AuditOutcome::ValidationFailed` audit
     event, then returns 400** — the **dominant** pattern for auditable-entity mutations. Grounded at ~21 sites
     (`host_tags`, `notifications`, `services/*`, `system_services`, `users` roles/profile, `scheduler`,
     `settings_access`, `enrollment_tokens`, `system_enrollment_tokens`, `access_presets`, batch handlers, …), each
     emitting the entity's own action (`HOST_TAG_UPDATE`, `NOTIFICATION_CHANNEL_UPDATE`, `SERVICE_UPDATE`,
     `USER_UPDATE`, `SCHEDULED_TASK_UPDATE`, …) with `outcome(ValidationFailed)`.

  **The generic `Validated<T>` extractor cannot reproduce pattern 2** — it has no entity/action context and runs before
  the handler body. So pattern 2 is **not** a deletable anti-pattern; deleting those manual blocks in favor of
  `Validated<T>` would **silently drop `ValidationFailed` audit coverage** (a security-audit regression against the
  repo's audit invariant). **These ~21 handlers are already correct and are OUT OF SCOPE.**

- **The actual bug = handlers that validate *not at all*** (`Json<*Request>` body, and the fn body contains **no**
  `.validate()` call). Grounded via a per-`fn` scan of `crates/ui/web-api/src/routes/`. Two groups:

  **Group A — the `*Request` type has no `Validate` impl at all** (absent from the `impl Validate for …` set in
  `web-api-types`). The 9 audit-named types plus 5 the audit missed:

  | Type | Handler | Fields → invariant |
  | --- | --- | --- |
  | `UpdateOidcProviderRequest` | `oidc_providers.rs:430` | `Option` name/slug/logo_url/issuer_url/client_id/… |
  | `UpdateHostRequest` | `hosts.rs:128` | `friendly_name: Option<String>` |
  | `UpdatePluginConfigRequest` | `plugin_configs/crud.rs:474` | `name: Option<String>`, `config`, `enabled` |
  | `TriggerUpdateRequest` | `software_items/updates.rs:44` | `to_version: String`, `release_info`, `interactive` |
  | `AssignHostsRequest` | `software_items/host_assignments.rs:56` | `host_assignments: Vec<HostSoftwareAssignment>` |
  | `UpdateHostAssignmentRequest` | `software_items/host_assignments.rs:433` | `role`, `ordinal:i32`, 3 mutually-exclusive config sources, `package_identifier`, `execution_site` |
  | `UpdateUserActiveRequest` | `users.rs:735` | `is_active: bool` |
  | `MergeAgentRequest` | `services/merge.rs:50` | `source_id: Uuid` |
  | `CreateDiscoveryAllowlistEntryRequest` | `discovery_allowlist.rs:96,450` | `plugin_type: PluginTypeId` |
  | `UpdateAgentCertificateSettingsRequest` | `settings_agent_certs.rs:100` | swept-in — plan inventories |
  | `MergeSoftwareItemsExecuteRequest` | `software_items/merge.rs:63` | swept-in |
  | `InvokeSurfaceInteractionRequest` | `surfaces.rs:210` | swept-in |
  | `OidcExchangeRequest` | `oidc_auth.rs:1351` | swept-in |
  | `OidcCompleteRegistrationRequest` | `oidc_auth.rs:1421` | swept-in |

  **Group B — the type *has* a `Validate` impl but the handler never calls it** (plain `Json`, no `Validated`, no
  manual `.validate()`): `DeviceAuthApproveRequest` (`device_auth.rs:72`), `UpdateSoftwareItemRequest`
  (`software_items/crud.rs:271`). Fix is wiring-only; no new impl.

- **No CI gate** enforces this. But the repo **does** have a per-handler grep-gate idiom to model on:
  `ci/verify_handler_state_contract.sh` uses `perl -0777` to extract full `async fn` spans and inspect each; sibling
  gates (`verify_no_inline_query_params.sh`, `verify_typed_audit_actions.sh`) ship `*_allowlist.txt` companions.
- **`to_version` reaches a shell command** — `apt` `update.rs:68` builds `format!("{package_identifier}={to_version}")`
  (`validate_version(to_version)` at `:51`). So a length cap on `to_version` is grounded defense-in-depth, not
  speculative.

## Approach (chosen — validate the bypass handlers using the repo's own idiom + a per-fn CI gate; root-cause, YAGNI)

### Part 1 — add `Validate` to the Group A types (the missing rules)

Each Group A type gets an `impl Validate`. Update types are `Option`-wrapped (PATCH semantics): **apply each rule only
when the field is `Some`.**

`Some("")` **must fail for required, non-clearable fields.** Justification: `name`/`slug`/`issuer_url`/`client_id` map
to non-`Option` DB columns the update writes unconditionally (`Set(...)`) — there is no "clear to empty" semantics, so
`Some("")` is always malformed, never an intentional clear. (A genuinely *clearable* optional string would need a typed
patch enum — `Set`/`Clear`/`Keep` — not `Some("")`; **no such field exists among these**, so that idiom is out of
scope, noted only so a future clearable field is not modeled as `Some("")`.)

- **`UpdateOidcProviderRequest`**: `name` `Some`→non-empty; `slug` `Some`→non-empty, len ≤ 64, regex
  `^[a-z0-9][a-z0-9-]*$`; `issuer_url` `Some`→non-empty **and** `http://`|`https://`; `client_id` `Some`→non-empty.
  Booleans/`scopes`/`role_claim_path`/`logo_url`/`client_secret` carry no `Create` rule → none here.
- **`UpdatePluginConfigRequest`**: `name` `Some`→non-empty. (Redundant with the query-layer
  `PluginConfigError::EmptyName`→400, but harmless boundary defense-in-depth; `config` stays opaque.)
- **`UpdateHostRequest`**: `friendly_name` `Some`→non-empty. Length cap only if the host name DB column bounds it.
- **`TriggerUpdateRequest`**: `to_version` non-empty **and** length-capped via
  `uptrakit_shared_types::command_validation::validate_command_length` (grounded: interpolated into an `apt` command).
- **`AssignHostsRequest`**: `host_assignments` non-empty **and** len ≤ 100 (mirror the `BatchActionRequest` cap). If
  `HostSoftwareAssignment` carries string fields (`package_identifier`, `execution_site`), give it its own `Validate`
  and iterate.
- **`UpdateHostAssignmentRequest`**: enforce the **mutual exclusivity** its field comments document — exactly one of
  `plugin_config_id`/`plugin_config`/`plugin_type` (no "exactly one of N" helper exists — code the count inline:
  `[a.is_some(), b.is_some(), c.is_some()].into_iter().filter(|x| *x).count() == 1`); `execution_site` `Some`→known
  value if a shared type defines the set; `package_identifier` `Some`→non-empty; nested `plugin_config` `Some`→its
  `CreatePluginConfigRequest::validate()`. See Behavior changes for the 404→400 shift. **No `ordinal` rule** — `ordinal:
  i32` has no DB constraint and no downstream break on negatives, so a `>= 0` check would be invented policy, not a
  parity fix; omitted (YAGNI).
- **Typed-only — `UpdateUserActiveRequest` (`bool`), `MergeAgentRequest` (`Uuid`),
  `CreateDiscoveryAllowlistEntryRequest` (`PluginTypeId`)**: no checkable invariant in the pure `web-api-types` crate
  (the `PluginTypeId` capability check needs `PluginCatalog`, stays handler-side). Documented `Ok(())` impl:
  `// No format/length invariants beyond field types; capability/existence checks are handler-side.`
- **5 swept-in types**: minimal impl — documented `Ok(())` where no obvious invariant, or a minimal presence/length
  rule where a field plainly needs one (e.g. an auth `code`/`state` string non-empty). Plan inventories per type; rich
  per-field design is reserved for the 9 audit-named types. (`MergeSoftwareItemsPreviewRequest` was swept out — it is a
  read-only dry-run, not a mutation; it is the seeded gate-allowlist entry, see Part 3.)

**Keep the defensive `oidc_providers.rs:929` `is_empty()` guard** — it protects reads of legacy rows written before
validation existed; orthogonal to input validation.

### Part 2 — wire validation into the bypass handlers (Groups A + B), matching each handler's local idiom

For each of the ~17 bypass handlers, invoke validation using the pattern its **sibling** mutations already use:

- **Auditable-entity mutations whose Create/sibling handler emits `ValidationFailed`** (e.g. `update_provider` beside
  `create_provider`; `update_host`; `update_plugin_config`; `update_user_active` beside `update_user_roles`;
  `merge_agent`; host-assignment + merge handlers) → add a manual `body.validate()` that emits the entity's
  `ValidationFailed` audit event on failure, then 400 — **consistent with the sibling**, preserving audit coverage.
- **Handlers with no `ValidationFailed`-audit family** (plan determines per handler; likely `trigger_update`,
  `device_auth` approve, the OIDC exchange/register bodies, surface invoke) → switch to the `Validated<T>` extractor
  (simplest; generic 400 is acceptable where no sibling audits validation failure).

The plan classifies each of the ~17 handlers by inspecting its sibling pattern and picks per handler. **Do not touch
the ~21 already-validated Bucket-B1 manual-validate handlers** — they are correct.

### Part 3 — the CI gate (root-cause regression guard)

Add `ci/verify_mutating_requests_validated.sh`, modeled on `ci/verify_handler_state_contract.sh` (perl `-0777` extracts
each `async fn` span under `crates/ui/web-api/src/routes/`). **Invariant enforced:** every handler whose signature takes
a typed `*Request` body via **`Json` or `Form`** must **invoke validation** — i.e. the fn body contains a `.validate()`
call, **or** the body is extracted via `Validated<…Request>` instead. Fail otherwise. `Form` is included because OAuth
token/device handlers (`token.rs:57,238`, `device_authorization.rs:40`) take `Form<*Request>` bodies — a `Json`-only
gate would silently miss them (contrarian MAJOR).

**Canonical detection pattern (used for both the one-off enumeration *and* the gate) — match all four binding shapes so
neither over- nor under-counts:** namespace-optional extractor + either binding style —
`(axum::)?(extract::)?(Json|Form)\s*<\s*([A-Za-z0-9_]+Request)\s*>` for the type-position form, plus the destructured
form `(Json|Form)\s*\(\s*[a-z_]+\s*\)\s*:\s*(Json|Form)\s*<\s*…Request\s*>`. The gate applies this to each extracted fn
span, then checks the span for `.validate()` or a `Validated<` extraction of the same type.

Ship a `ci/verify_mutating_requests_validated_allowlist.txt` companion (repo idiom) for any typed `*Request` body that is
legitimately non-mutating, **seeded with `MergeSoftwareItemsPreviewRequest`** (`software_items/merge.rs` preview handler
— a read-only dry-run, no state change, so no validation required). `command -v perl`/`rg` guard as siblings.

**Scope of the guarantee (stated honestly):** the gate covers typed `Json`/`Form` bodies whose type ends in `Request`.
It does **not** cover raw-`Bytes`/`serde_json::Value` bodies (no type to key on) nor mutating bodies not named
`*Request`. Two documented consequences: (1) `Json<ConsentDecision>` (`oauth/consent.rs:202`, already validates) evades
detection by name — so document in the gate header + coding standard that **mutating request bodies must be named
`*Request`**; (2) the OAuth DCR (`Json`) and token/device (`Form`) handlers keep their **manual** `.validate()` calls —
the gate *accepts* them (it enforces "validation invoked", not a specific extractor), which is required: those handlers
return RFC-shaped `oauth_400` error bodies (not the generic `Validated<T>` 400) and order rate-limiting before
validation, both of which `Validated<T>` would break.

Wire the gate into `.github/workflows/ci.yml`, `AGENTS.md` quick-start (Rust block), and
`docs/development/quality-gates.md` (canonical) in the **same commit** (quality-gate-authoring invariant). Husky
pre-commit inclusion optional (grep-only, fast).

## Behavior changes (call out — not silent)

- **Group B now validates** (`DeviceAuthApproveRequest`, `UpdateSoftwareItemRequest`): inputs previously accepted
  unvalidated are now subject to their existing `Validate` rules → possible new 400s. Correct (that is the bug); note
  in changelog + handler test.
- **`UpdateHostAssignmentRequest` mutual exclusivity: 404 → 400.** Today illegal config-source combinations surface as
  404 (not-found on the unresolved source) or are silently resolved by the query layer's fixed precedence — `plugin_type`
  always wins over a supplied `plugin_config_id`/`plugin_config` (web-api-queries `host_assignments.rs:492-544`,
  precedence at `:516`; **not** order-dependent "first-wins"); enforcing "exactly one" in `Validate` returns **400** — a
  stricter, more correct contract that rejects the ambiguous combination at the boundary instead of silently dropping a
  field. Note in OpenAPI + test.
- **No audit regression.** Bucket B1 is untouched; the newly-wired bypass handlers *gain* `ValidationFailed` audit
  coverage (pattern-1 handlers gain a generic 400 with no audit, matching their sibling family which also does not
  audit validation failures).

## Tests

- **Unit (`web-api-types`, the 9 rich/moderate types):** valid input passes; each violation returns
  `Err(ValidationError)` — `Some("")` name; slug `"UPPER"`/`"-x"`/65-char/`"a/b"`; `issuer_url` `Some("ftp://x")`,
  `Some("")`; `client_id` `Some("")`; `to_version` `""` and over-length; `host_assignments` empty and 101-length;
  mutual-exclusivity (zero set, two set). Assert `field` where it disambiguates. **Omitted (`None`) fields pass** (PATCH
  keep-semantics).
- **Typed-only + swept-in `Ok(())`:** one `validate()`→`Ok(())` smoke test each.
- **Handler (TestApp harness):** `PATCH /api/v1/oidc-providers/{id}` with `issuer_url:"ftp://x"` → 400 **and** assert a
  `ValidationFailed` audit event is emitted (proves the manual-validate+audit wiring, not just a bare 400); one
  `UpdateHostAssignmentRequest` two-sources-set → 400 (proves the 404→400 shift).
- **Gate self-check:** the gate exits 0 after the fix; add a negative fixture (temporarily add a `Json<XRequest>`
  handler with no `.validate()` → gate exits non-zero) to prove it bites.
- No `start_paused` (no `tokio::time`). **Do not test** the `Validated` extractor machinery or serde deserialization
  (framework behavior).

## Deliverables

- `crates/shared/web-api-types/src/*` — 14 `impl Validate` (9 rich + 5 minimal) + `HostSoftwareAssignment` if it carries
  string fields; unit tests for the rich/moderate ones.
- `crates/ui/web-api/src/routes/*` — wire validation into the ~17 bypass handlers (Groups A+B), each matching its
  sibling idiom (manual `body.validate()` + `ValidationFailed` audit, **or** `Validated<T>`); the two handler tests.
  **Leave the ~21 Bucket-B1 manual-validate handlers untouched.** The plan regenerates the exact bypass list via the
  per-fn scan.
- `ci/verify_mutating_requests_validated.sh` + `_allowlist.txt` — new gate; wire into `.github/workflows/ci.yml`.

### Documentation deliverables

- `docs/development/coding-standards.md` — request-type-validation section: rule is now **CI-enforced** (name the
  gate); document that a mutating body must **invoke validation** via one of the two accepted patterns (`Validated<T>`,
  or manual `body.validate()` + `ValidationFailed` audit); document the Update/PATCH `Some("")` idiom and the
  **`*Request` naming requirement** the gate depends on.
- `AGENTS.md` quick-start Rust block **and** `docs/development/quality-gates.md` (canonical) — add the gate command,
  same commit.
- **OpenAPI:** wiring validation adds a `400` response to the bypass paths. Add `(status = 400, …)` to any bypass
  handler's `#[utoipa::path(responses(...))]` that lacks it (grounded: `update_provider`, `update_host`,
  `update_plugin_config`, `update_user_active` lack it; plan audits all ~17), then run `./scripts/regen-api.sh` and
  commit `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`. Body **shapes** unchanged, so
  `uptrakit-openapi-client` needs no signature change.
- **No ADR** (internal validation mechanics). **No wire/dependency change.**

## Alternatives considered

- **Convert the ~21 already-validated manual-validate handlers to `Validated<T>` too (a uniform "no `Json<*Request>`"
  sweep + gate)** — **rejected: the B1 handlers are heterogeneous and `Validated<T>` cannot preserve their behavior.**
  Three distinct properties would silently regress: (1) most emit domain-specific `AuditOutcome::ValidationFailed` audit
  events that the generic extractor cannot reproduce (no entity/action context) — a security-audit regression against
  the repo's audit invariant; (2) several deliberately return **422** on semantic-validation failure
  (`settings_access.rs:85`, `users.rs:679/904/1099`), not the extractor's generic **400** — a wire-contract change; (3)
  some run **authorization before validation** (`users.rs:672`) so an unauthorized caller never reaches validation —
  `Validated<T>` (a `FromRequest` extractor) runs *before* the handler body, inverting that order and leaking
  input-validity signal to unauthorized callers. The manual-validate pattern is idiomatic here, not an anti-pattern; the
  gate must accept it. This is why the gate enforces "validation is invoked," not "the extractor is `Validated`."
- **Fix only `UpdateOidcProviderRequest` / only the 9, skip the gate** — rejected: leaves the class open (the audit
  flags the missing enforcement as the root cause).
- **Gate on "type impls `Validate`" instead of "handler invokes validation"** — rejected: misses Group B (types that
  *have* a `Validate` impl the handler never calls). The invariant is the *call*, not the *impl*.
- **Gate with a typed-only allowlist instead of per-type `Validate`** — rejected for Group A: per-entry judgment that
  rots; the 3 typed-only `Ok(())` impls are the cheap price of the missing-rule fix. (An allowlist for genuinely
  non-mutating `*Request` bodies is a separate, repo-idiomatic escape hatch, seeded empty.)
- **Validate `plugin_type` capability inside `Validate`** — rejected: `PluginCatalog` is not in `web-api-types`; the
  check stays handler-side (where it already is).
- **Remove the `oidc_providers.rs:929` defensive guard** — rejected: protects reads of legacy rows; keep both.

## Out of scope

The ~21 already-validated Bucket-B1 manual-validate handlers (correct as-is). Other unspecced Medium+ findings
(short-term-backlog tier) — separate specs. No change to `Create`-type validation, the `PluginCatalog` capability
check, merge-time plugin-config validation, or PATCH clear-semantics (`null` vs `Some("")`) beyond `Some("")` failing
for required fields. No new request fields, no request-body shape change. `Query`/`Path` params and raw
`Bytes`/`serde_json::Value` bodies are out of scope — the gate targets typed `Json`/`Form` `*Request` bodies only.
