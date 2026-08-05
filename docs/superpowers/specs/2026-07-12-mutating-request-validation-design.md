# Mutating Request Types Bypass `Validate` — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/shared/web-api-types/src/*` (add `Validate` impls + unit tests),
`crates/ui/web-api/src/routes/*` (wire validation into the handlers that skip it entirely),
`xtask/src/request_validation_check/` (new `cargo xtask` gate + allowlist) + its wiring. No new deps, no wire change.

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

## Verified current reality (grounded 2026-07-12; re-verified 2026-08-05)

> **Re-grounding 2026-08-05:** all claims re-checked against the current tree. Line numbers below drifted by ≤15 lines
> (non-material; the plan regenerates the exact bypass list via the per-fn scan). One material change: the
> surface-interaction route family grew from 1 to 5 bypass handlers (see the `InvokeSurfaceInteractionRequest` row).

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

- **The actual bug = handlers that validate _not at all_** (`Json<*Request>` body, and the fn body contains **no**
  `.validate()` call). Grounded via a per-`fn` scan of `crates/ui/web-api/src/routes/`. Two groups:

  **Group A — the `*Request` type has no `Validate` impl at all** (absent from the `impl Validate for …` set in
  `web-api-types`). The 9 audit-named types plus 5 the audit missed:

  | Type                                    | Handler                                                                                                                                                                                                               | Fields → invariant                                                                                 |
  | --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
  | `UpdateOidcProviderRequest`             | `oidc_providers.rs:430`                                                                                                                                                                                               | `Option` name/slug/logo_url/issuer_url/client_id/…                                                 |
  | `UpdateHostRequest`                     | `hosts.rs:128`                                                                                                                                                                                                        | `friendly_name: Option<String>`                                                                    |
  | `UpdatePluginConfigRequest`             | `plugin_configs/crud.rs:474`                                                                                                                                                                                          | `name: Option<String>`, `config`, `enabled`                                                        |
  | `TriggerUpdateRequest`                  | `software_items/updates.rs:44`                                                                                                                                                                                        | `to_version: String`, `release_info`, `interactive`                                                |
  | `AssignHostsRequest`                    | `software_items/host_assignments.rs:56`                                                                                                                                                                               | `host_assignments: Vec<HostSoftwareAssignment>`                                                    |
  | `UpdateHostAssignmentRequest`           | `software_items/host_assignments.rs:433`                                                                                                                                                                              | `role`, `ordinal:i32`, 3 mutually-exclusive config sources, `package_identifier`, `execution_site` |
  | `UpdateUserActiveRequest`               | `users.rs:735`                                                                                                                                                                                                        | `is_active: bool`                                                                                  |
  | `MergeAgentRequest`                     | `services/merge.rs:50`                                                                                                                                                                                                | `source_id: Uuid`                                                                                  |
  | `CreateDiscoveryAllowlistEntryRequest`  | `discovery_allowlist.rs:96,450`                                                                                                                                                                                       | `plugin_type: PluginTypeId`                                                                        |
  | `UpdateAgentCertificateSettingsRequest` | `settings_agent_certs.rs:100`                                                                                                                                                                                         | swept-in — plan inventories                                                                        |
  | `MergeSoftwareItemsExecuteRequest`      | `software_items/merge.rs:63`                                                                                                                                                                                          | swept-in                                                                                           |
  | `InvokeSurfaceInteractionRequest`       | `surfaces.rs` — **5 handlers** (grew 1→5 post-grounding, commit `8b695e4a5`): `invoke_surface_interaction` (POST, `Json<T>`) + 4 method-mapped PUT/DELETE handlers binding `Option<Json<T>>` + `.unwrap_or_default()` | swept-in                                                                                           |
  | `OidcExchangeRequest`                   | `oidc_auth.rs:1351`                                                                                                                                                                                                   | swept-in                                                                                           |
  | `OidcCompleteRegistrationRequest`       | `oidc_auth.rs:1421`                                                                                                                                                                                                   | swept-in                                                                                           |

  **Group B — the type _has_ a `Validate` impl but the handler never calls it** (plain `Json`, no `Validated`, no
  manual `.validate()`): `DeviceAuthApproveRequest` (`device_auth.rs:72`), `UpdateSoftwareItemRequest`
  (`software_items/crud.rs:271`). Fix is wiring-only; no new impl.

- **No CI gate** enforces this. The repo has two enforcement idioms: text-level `ci/verify_*.sh` gates
  (`verify_handler_state_contract.sh` uses `perl -0777` to extract `async fn` **signatures** — not full bodies; sibling
  gates ship `*_allowlist.txt` companions) and **`syn`-based `cargo xtask` checks** (`audit_coverage_check`,
  `contribution_monotonicity_check`, `openapi_client_check`; `xtask` depends on `syn` with `full`/`visit`). This gate
  needs full fn _bodies_, so the xtask idiom is the right model — see Part 3.
- **`to_version` reaches a shell command** — `apt` `update.rs:68` builds `format!("{package_identifier}={to_version}")`
  (`validate_version(to_version)` at `:51`). So a length cap on `to_version` is grounded defense-in-depth, not
  speculative.

## Approach (chosen — validate the bypass handlers using the repo's own idiom + a per-fn CI gate; root-cause, YAGNI)

### Part 1 — add `Validate` to the Group A types (the missing rules)

Each Group A type gets an `impl Validate`. Update types are `Option`-wrapped (PATCH semantics): **apply each rule only
when the field is `Some`.**

`Some("")` **must fail for required, non-clearable fields.** Justification: `name`/`slug`/`issuer_url`/`client_id` map
to non-`Option` DB columns the update writes unconditionally (`Set(...)`) — there is no "clear to empty" semantics, so
`Some("")` is always malformed, never an intentional clear. (A genuinely _clearable_ optional string would need a typed
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
  Note: `validate_command_length` returns `Result<(), String>` — map into the trait's error with
  `.map_err(|message| ValidationError { field: "to_version".into(), message })` (or equivalent); do not invent a second
  error type.
- **`AssignHostsRequest`**: `host_assignments` non-empty **and** len ≤ 100 (mirror the `BatchActionRequest` cap).
  `HostSoftwareAssignment` itself carries no strings (`{ host_id: Uuid, plugins: Vec<HostPluginRoleAssignment> }`);
  the string fields (`package_identifier`, `execution_site`, nested `plugin_config`) live on the nested
  `HostPluginRoleAssignment` — give **that** type the `Validate` impl and iterate two levels deep
  (`host_assignments[].plugins[]`).
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

For each of the ~21 bypass handlers (count grew from ~17 at re-grounding: the 4 method-mapped surface-interaction
handlers added post-grounding), invoke validation using the pattern its **sibling** mutations already use:

- **Auditable-entity mutations whose Create/sibling handler emits `ValidationFailed`** (e.g. `update_provider` beside
  `create_provider`; `update_host`; `update_plugin_config`; `update_user_active` beside `update_user_roles`;
  `merge_agent`; host-assignment + merge handlers) → add a manual `body.validate()` that emits the entity's
  `ValidationFailed` audit event on failure, then 400 — **consistent with the sibling**, preserving audit coverage.
- **Handlers with no `ValidationFailed`-audit family** (plan determines per handler; likely `trigger_update`,
  `device_auth` approve, the OIDC exchange/register bodies, surface invoke) → switch to the `Validated<T>` extractor
  (simplest; generic 400 is acceptable where no sibling audits validation failure).
- **`Option<Json<T>>` bodies** (the 4 method-mapped surface-interaction handlers): `Validated<T>` does not fit — it
  errors on a missing body instead of defaulting. Keep the `Option<Json<T>>` extraction and validate manually on
  `Some`: `if let Some(body) = &body { body.validate()… }` (an absent body defaults and needs no validation).

The plan classifies each of the ~21 handlers by inspecting its sibling pattern and picks per handler. **Do not touch
the ~21 already-validated Bucket-B1 manual-validate handlers** — they are correct.

### Part 3 — the CI gate (root-cause regression guard)

Add a **`syn`-based `cargo xtask` check** — `cargo xtask request-validation-check`
(`xtask/src/request_validation_check/`), modeled on the existing xtask checks (`audit_coverage_check`,
`contribution_monotonicity_check`, `openapi_client_check`; `xtask` already depends on `syn` with
`features = ["full", "visit", "extra-traits"]`). A text-level perl/grep gate was the original design and was rejected
after contrarian review — see Alternatives.

**Invariant enforced:** every fn under `crates/ui/web-api/src/routes/` with a body parameter whose type is or contains
**`Json<T>` or `Form<T>`** (including `Option<Json<T>>`, namespace-qualified forms, and both binding styles —
**any `T`, name-independent**) must **invoke validation**: the fn body calls `.validate()` **on the binding bound from
that body parameter** (receiver-checked, not any-`.validate()`-anywhere), **or** the body is extracted via
`Validated<T>` instead, **or** `T` is allowlisted. Fail otherwise. `Form` is included because OAuth token/device
handlers (`token.rs:57,238`, `device_authorization.rs:40`) take `Form<*Request>` bodies — a `Json`-only gate would
silently miss them (contrarian MAJOR). Keying on the **extractor** rather than a `*Request` naming convention means
non-`Request`-named bodies (e.g. `Json<ConsentDecision>`) cannot silently evade the gate — an unchecked naming
convention would reinstate the spec's own root cause (contrarian CRITICAL). `syn` visitation skips `#[cfg(test)]`
modules and gives real fn bodies — no brace-balancing or span-bleed false negatives a text scanner would carry.

Ship an allowlist companion `ci/request_validation_allowlist.txt` (repo `_allowlist.txt` idiom; read by the xtask
check). **Entries are keyed on the handler, not the bare type** — `file::fn` (+ body type), because both seeded
justifications are properties of a _handler_ (a type-keyed entry would silently exempt every future handler taking the
same body — contrarian pass 2). One per line with a justification comment. **Seeded with two entries:**
`software_items/merge.rs::preview_software_item_merge` / `MergeSoftwareItemsPreviewRequest` (read-only dry-run, no
state change) and
`oauth/consent.rs::approve_consent` / `ConsentDecision` (an **empty struct**; the handler binds `Json(_body)` and
discards it, so there is nothing to validate. Note: the spec previously claimed this handler "already validates" —
wrong; corrected at re-review. The plan may add a cheap tripwire test asserting `ConsentDecision` stays field-free, so
the exemption's premise fails loudly if fields are ever added).

**Future-extractor tripwire:** the same `syn` pass also flags any `impl FromRequest` under `crates/ui/web-api/src/`
outside a known set (today exactly one: `Validated<T>`, `extract.rs:304`). Otherwise a future custom body extractor
(`Signed<T>`, size-limiting wrapper, …) would carry user input invisibly to a `Json`/`Form`-keyed gate — the same
unchecked-convention hole the `*Request`-naming rejection closed, one level down. ~10 lines in the visitor already
being written; a new body extractor must then be explicitly reasoned about (added to the known set or gated).

**Scope of the guarantee (stated honestly):** the check covers typed `Json`/`Form` bodies. It does **not** cover
raw-`Bytes`/`serde_json::Value` bodies (no type to key on), and it enforces that validation is **invoked** — not that
the result is propagated nor that a `ValidationFailed` audit event is emitted; those remain review-time concerns
(extending `audit-catalog.toml` coverage to validation events is out of scope). Propagation is partly
compiler-enforced already: `Result` is `#[must_use]` and the workspace denies warnings, so a bare `body.validate();`
fails the build — only an explicit `let _ =` silences it, and that is visible in review. The
OAuth DCR (`Json`) and token/device (`Form`) handlers keep their **manual** `.validate()` calls — the check _accepts_
them (it enforces "validation invoked", not a specific extractor), which is required: those handlers return RFC-shaped
`oauth_400` error bodies (not the generic `Validated<T>` 400) and order rate-limiting before validation, both of which
`Validated<T>` would break.

Wire the check into `.github/workflows/ci.yml`, `AGENTS.md` quick-start (Rust block), and
`docs/development/quality-gates.md` (canonical) in the **same commit** (quality-gate-authoring invariant). No husky
pre-commit wiring (compiling `xtask` is too heavy for a commit hook; CI + on-demand, like the sibling xtask checks).

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
- **No audit regression.** Bucket B1 is untouched; the newly-wired bypass handlers _gain_ `ValidationFailed` audit
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
- **Gate self-check:** `cargo xtask request-validation-check` exits 0 after the fix; the xtask module gets unit tests
  parsing fixture snippets (a `Json<X>` handler with no `.validate()` → violation reported; a `query.validate()` on a
  _different_ binding → still a violation, proving the receiver check; an allowlisted type → pass) to prove it bites.
- No `start_paused` (no `tokio::time`). **Do not test** the `Validated` extractor machinery or serde deserialization
  (framework behavior).

## Deliverables

- `crates/shared/web-api-types/src/*` — 14 `impl Validate` (9 rich + 5 minimal) + `HostPluginRoleAssignment` (the
  nested type carrying the string fields); unit tests for the rich/moderate ones.
- `crates/ui/web-api/src/routes/*` — wire validation into the ~21 bypass handlers (Groups A+B), each matching its
  sibling idiom (manual `body.validate()` + `ValidationFailed` audit, **or** `Validated<T>`); the two handler tests.
  **Leave the ~21 Bucket-B1 manual-validate handlers untouched.** The plan regenerates the exact bypass list via the
  per-fn scan.
- `xtask/src/request_validation_check/` (new `cargo xtask request-validation-check` subcommand) +
  `ci/request_validation_allowlist.txt` — new gate; wire into `.github/workflows/ci.yml`.

### Documentation deliverables

- `docs/development/coding-standards.md` — request-type-validation section: rule is now **CI-enforced** (name
  `cargo xtask request-validation-check`); document that a mutating body must **invoke validation** via one of the two
  accepted patterns (`Validated<T>`, or manual `body.validate()` + `ValidationFailed` audit); document the Update/PATCH
  `Some("")` idiom. (`*Request` naming stays a soft convention — the gate keys on the extractor and does not depend on
  it.) Draw the line between the two escape hatches so they don't overlap: the **allowlist** is only for handlers that
  perform **no state change**; every mutating body type gets an `impl Validate` — `Ok(())` if there is genuinely
  nothing to check.
- `AGENTS.md` quick-start Rust block **and** `docs/development/quality-gates.md` (canonical) — add the gate command,
  same commit.
- **OpenAPI:** wiring validation adds a `400` response to the bypass paths. Add `(status = 400, …)` to any bypass
  handler's `#[utoipa::path(responses(...))]` that lacks it (grounded: `update_provider`, `update_host`,
  `update_plugin_config`, `update_user_active` lack it; plan audits all ~21), then run `./scripts/regen-api.sh` and
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
  `Validated<T>` (a `FromRequest` extractor) runs _before_ the handler body, inverting that order and leaking
  input-validity signal to unauthorized callers. The manual-validate pattern is idiomatic here, not an anti-pattern; the
  gate must accept it. This is why the gate enforces "validation is invoked," not "the extractor is `Validated`."
- **Fix only `UpdateOidcProviderRequest` / only the 9, skip the gate** — rejected: leaves the class open (the audit
  flags the missing enforcement as the root cause).
- **Gate on "type impls `Validate`" instead of "handler invokes validation"** — rejected: misses Group B (types that
  _have_ a `Validate` impl the handler never calls). The invariant is the _call_, not the _impl_.
- **Gate with a typed-only allowlist instead of per-type `Validate`** — rejected for Group A: per-entry judgment that
  rots; the 3 typed-only `Ok(())` impls are the cheap price of the missing-rule fix. (An allowlist for bodies genuinely
  needing no validation is a separate, repo-idiomatic escape hatch, seeded with 2 entries — see Part 3.) Residual risk,
  stated: an `Ok(())` impl also rots — a new `String` field on such a type passes the gate with zero rules. Mitigation:
  the impl + its `// No format/length invariants…` comment sit adjacent to the struct, so a field-adding diff shows
  them; neither an allowlist nor an impl can force rules onto future fields, and the impl at least keeps the decision
  in the same file.
- **Text-level perl/grep gate (`ci/verify_mutating_requests_validated.sh`, modeled on
  `verify_handler_state_contract.sh`)** — the original Part 3 design; **rejected at contrarian review.** The sibling
  gate extracts only fn _signatures_ (first brace); this gate needs full _bodies_, which a text scanner gets wrong in
  ways that produce **false negatives**: naive fn-start→fn-start spans let the last handler in a file absorb the
  `#[cfg(test)]` module (a `.validate()` in tests grants a false pass), and a substring check is receiver-agnostic
  (`query.validate()` satisfies it for an unvalidated body). A gate whose failure mode is false-negative is worse than
  no gate. The repo's closer precedent for body-level analysis is `xtask` + `syn` (already used by
  `audit_coverage_check` et al.) — adopted instead.
- **`Body<T>` type-state extractor (deserialize-only wrapper; handler must call a consuming `require_valid()` to reach
  the fields — bypass becomes a compile error)** — genuinely stronger (compile-time, no gate needed) but **rejected as
  disproportionate for this fix**: it touches every `Json`/`Form` body site in `routes/` (~50+, including the ~21
  correct Bucket-B1 handlers this spec deliberately leaves untouched), needs a new extractor with its own
  utoipa/OpenAPI schema integration, and converts a targeted bug-fix into a cross-cutting migration. Noted as a
  possible future hardening that would supersede the xtask check; not this change.
- **Validate `plugin_type` capability inside `Validate`** — rejected: `PluginCatalog` is not in `web-api-types`; the
  check stays handler-side (where it already is).
- **Remove the `oidc_providers.rs:929` defensive guard** — rejected: protects reads of legacy rows; keep both.

## Out of scope

The ~21 already-validated Bucket-B1 manual-validate handlers (correct as-is). Other unspecced Medium+ findings
(short-term-backlog tier) — separate specs. No change to `Create`-type validation, the `PluginCatalog` capability
check, merge-time plugin-config validation, or PATCH clear-semantics (`null` vs `Some("")`) beyond `Some("")` failing
for required fields. No new request fields, no request-body shape change. `Query`/`Path` params and raw
`Bytes`/`serde_json::Value` bodies are out of scope — the gate targets typed `Json`/`Form` bodies only.
