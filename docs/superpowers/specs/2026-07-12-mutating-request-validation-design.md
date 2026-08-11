# Mutating Request Types Bypass `Validate` — Design

**Date:** 2026-07-12
**Revised:** 2026-08-05 — owner decision: staged type-state extractor (`Unvalidated<T>`) replaces per-handler manual
wiring + the `syn` xtask gate. For extractor-borne bodies, bypass becomes a **compile error**, not a CI finding
(hand-rolled `Request`/`Bytes` reads are gate-enumerated, not compile-blocked — see Part 4); see Alternatives for the
superseded designs.
**Status:** Design (pending plan)
**Scope:** `crates/ui/web-api/src/extract.rs` (new `Unvalidated<T>`/`UnvalidatedForm<T>` extractors),
`crates/shared/web-api-types/src/*` (add `Validate` impls + unit tests),
`crates/ui/web-api/src/routes/*` (convert the bypass handlers), `ci/verify_no_raw_body_extractors.sh` (+ frozen
allowlist) + its wiring, one ADR. No new deps, no wire change.

> **Resolved by:** the formerly-parked owner decisions and minor deferrals in this design were resolved in
> [2026-08-06-mutating-request-validation-hardening-design.md](2026-08-06-mutating-request-validation-hardening-design.md).

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
     event, then returns 400** — the **dominant** pattern for auditable-entity mutations. Grounded at ~26 sites
     (`host_tags`, `notifications`, `services/*`, `system_services`, `users` roles/profile, `scheduler`,
     `settings_access`, `enrollment_tokens`, `system_enrollment_tokens`, `access_presets`, batch handlers, …), each
     emitting the entity's own action (`HOST_TAG_UPDATE`, `NOTIFICATION_CHANNEL_UPDATE`, `SERVICE_UPDATE`,
     `USER_UPDATE`, `SCHEDULED_TASK_UPDATE`, …) with `outcome(ValidationFailed)`.

  **The generic `Validated<T>` extractor cannot reproduce pattern 2** — it has no entity/action context and runs before
  the handler body. So pattern 2 is **not** a deletable anti-pattern; deleting those manual blocks in favor of
  `Validated<T>` would **silently drop `ValidationFailed` audit coverage** (a security-audit regression against the
  repo's audit invariant). **These ~26 handlers are already correct and are OUT OF SCOPE.**

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
  gates ship `*_allowlist.txt` companions) and **`syn`-based `cargo xtask` checks** (`audit_coverage_check` et al.).
  Under the type-state design the body-level semantics live in the compiler, so the remaining gate is
  **signature-level** — squarely the `ci/verify_*.sh` idiom's competence; see Part 4.
- **`to_version` reaches a shell command** — `apt` `update.rs:68` builds `format!("{package_identifier}={to_version}")`
  (`validate_version(to_version)` at `:51`). So a length cap on `to_version` is grounded defense-in-depth, not
  speculative.
- **`#[utoipa::path]` declares `request_body = <Type>` explicitly in the attribute** (grounded:
  `oidc_providers.rs:98,411`) — utoipa never introspects the extractor, so an extractor swap has **zero OpenAPI
  impact** and a wrapper extractor needs no `ToSchema`. `Validated<T>` is used at ~30 sites in `routes/` today
  (pattern-1 handlers); those sites are already structurally safe and are untouched here.

## Approach (chosen — staged type-state extractor: extractor-borne unvalidated bodies unreachable at compile time)

One cause underlies both bug groups: nothing stops a handler from reading body fields before validating. The fix makes
that structurally impossible for every converted handler, staged so the ~26 correct Bucket-B1 handlers are not touched
now:

1. **Part 1** — introduce `Unvalidated<T>`/`UnvalidatedForm<T>`: extractors that deserialize but keep `T` private;
   the only way to the fields is a consuming `require_valid()`.
2. **Part 2** — add the missing `Validate` impls (unchanged from the original design; needed under any approach).
3. **Part 3** — convert the ~23 bypass handlers (Groups A+B + the two no-invariant sites) to the new extractors.
4. **Part 4** — a signature-level grep gate bans raw `Json<`/`Form<` body extractors in `routes/`, with a **frozen,
   shrink-only allowlist** of the ~26 legacy B1 sites.
5. **Stage 2 (follow-up, out of this spec):** convert the B1 handlers opportunistically, shrinking the allowlist to
   zero; optionally retire `Validated<T>` afterwards.

### Part 1 — the `Unvalidated<T>` / `UnvalidatedForm<T>` type-state extractors

Add to `crates/ui/web-api/src/extract.rs` (beside `Validated<T>`):

```rust
/// A deserialized-but-not-yet-validated request body. The inner value is
/// private; the only way to reach the fields is `require_valid()`.
pub struct Unvalidated<T>(T); // field NOT pub — routes/ modules cannot destructure it

impl<T: Validate> Unvalidated<T> {
    pub fn require_valid(self) -> Result<T, ValidationError> {
        self.0.validate()?;
        Ok(self.0)
    }
}
```

- **`FromRequest` delegates to `axum::Json<T>` internally** (and `UnvalidatedForm<T>` to `Form<T>`), then wraps — so
  malformed-body rejections stay **byte-identical** to today's `Json`/`Form` behavior. No custom deserialization.
  **Carry the `T: Validate` bound on the `FromRequest` impl too** — then a body type without a `Validate` impl cannot
  even be extracted, and Part 2's completeness is compile-forced at the signature, not just at the `require_valid()`
  call site.
- **No `Deref`, no field access, no other accessor.** The private field plus module privacy make `body.issuer_url`
  without `require_valid()` a **compile error** from any `routes/` module. Ignoring the returned `Result` yields no
  `T` to misuse — there is nothing to bypass.
- **Handler keeps full control** of the three properties that made `Validated<T>` unusable for B1: _when_ validation
  runs (after authz), _which status_ maps a failure (400 / 422 / RFC-shaped `oauth_400`), and _which
  `ValidationFailed` audit event_ is emitted. `Unvalidated<T>` is pattern-2-compatible by construction.
- **`Option<Unvalidated<T>>`** (the 4 method-mapped surface-interaction handlers): on axum 0.8, `Option<T>` extraction
  requires a **separate `OptionalFromRequest` impl** — a distinct impl block, not a byproduct of `FromRequest`
  (grounded: `axum::Json` implements both; the blanket `impl FromRequest for Option<T>` in `axum-core` requires
  `T: OptionalFromRequest`). Ship
  `impl<S: Send + Sync, T: Validate + DeserializeOwned> OptionalFromRequest<S> for Unvalidated<T>` delegating to
  `Json<T>`'s optional path (the `S: Send + Sync` bound is required — `Json<T>`'s own `OptionalFromRequest` impl
  demands it, and the `&S` held across the internal `.await` needs `S: Sync`; `T: Send` is **not** needed, matching
  upstream), so absent bodies still default:
  `let body = match body { Some(b) => b.require_valid()…?, None => T::default() };`
- **Naming:** `Body<T>` was rejected — it collides with `axum::body::Body`. `Unvalidated<T>` is honest (the extractor
  yields a not-yet-validated body) and pairs with the existing `Validated<T>`.

### Part 2 — add `Validate` to the Group A types (the missing rules)

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
  per-field design is reserved for the 9 audit-named types.
- **2 no-invariant conversion types — `MergeSoftwareItemsPreviewRequest` (read-only dry-run) and `ConsentDecision`
  (empty struct; handler discards the body)**: documented `Ok(())` impls. Under the type-state design these need no
  allowlist entry — the `T: Validate` bound requires an impl, and a trivially-passing `require_valid()` is cheaper and
  more uniform than a carve-out. (The previous design allowlisted both; superseded.)

**Keep the defensive `oidc_providers.rs:929` `is_empty()` guard** — it protects reads of legacy rows written before
validation existed; orthogonal to input validation.

### Part 3 — convert the bypass handlers to `Unvalidated<T>` (Groups A + B + the 2 no-invariant sites)

Each of the ~23 bypass handlers (the ~21 Groups A+B sites — count grew from ~17 at re-grounding via the 4
method-mapped surface-interaction handlers — plus `preview_software_item_merge` and `approve_consent`) swaps its raw
extractor for `Unvalidated<T>` (or `UnvalidatedForm<T>` / `Option<Unvalidated<T>>`) and calls `require_valid()`. The
**error mapping** follows the handler's sibling family — same classification the previous design used, applied to the
`Err` arm instead of a manual call site:

- **Auditable-entity mutations whose Create/sibling handler emits `ValidationFailed`** (e.g. `update_provider` beside
  `create_provider`; `update_host`; `update_plugin_config`; `update_user_active`; `merge_agent`; host-assignment +
  merge handlers) → on `Err`, emit the entity's `ValidationFailed` audit event, then 400 — **consistent with the
  sibling**, preserving audit coverage. Validation runs where the sibling runs it (after authz where the family does).
- **Handlers with no `ValidationFailed`-audit family** (`trigger_update`, `device_auth` approve, the OIDC
  exchange/register bodies, surface invoke, preview/consent) → on `Err`, return
  `error_response(StatusCode::BAD_REQUEST, e.to_string())` — the same response shape `Validated<T>` produces, so the
  wire contract matches the pattern-1 family.

The plan classifies each handler by inspecting its sibling and picks the mapping per handler. **Do not touch the ~26
already-validated Bucket-B1 manual-validate handlers** — they are correct and are Stage 2. `Validated<T>` sites (~30)
are also untouched — already structurally safe.

### Part 4 — the CI gate (frozen legacy allowlist; ratchet to zero)

With the compile-time guarantee carrying the semantic load, the gate's primary job shrinks to: **no new raw
`Json<T>` / `Form<T>` body extractors in `routes/`** — a **signature-level** check, exactly the sibling idiom's
competence — plus one narrow **body-span** check scoped to the allowlisted fns (see Residual check below). Add
`ci/verify_no_raw_body_extractors.sh`, modeled on `ci/verify_handler_state_contract.sh` (perl `-0777` extracts each
fn **signature** span to the first `{`): strip the return type (`s/->.*//s` — rustfmt-formatted
signatures make this safe; excludes `-> Json<XResponse>` response positions — grounded: `device_auth.rs:198` returns
`Result<axum::Json<…>, ApiError>` and would false-positive without the strip), then flag any `Json<` / `Form<` /
`Option<Json<` remaining in the params region unless the site is allowlisted. Derive the pattern empirically against
the known-true inventory and RED-test it (gate-authoring ledger rules). Add a `command -v perl` guard — the guard
idiom comes from the `rg`-based siblings (`verify_no_inline_query_params.sh` et al.); the perl-based model script has
none, so this is a small improvement, not copied precedent.

**Frozen allowlist:** `ci/verify_no_raw_body_extractors_allowlist.txt`, in the sibling gates' **`path|text-regex` row
format** (e.g. `crates/ui/web-api/src/routes/host_tags.rs|fn update_host_tag\b` — the regex anchors the fn name, so
entries stay handler-keyed; a type-keyed entry would exempt future handlers; reuse the siblings' malformed-row and
stale-entry validation instead of inventing a new key format), seeded with the **~26** legacy Bucket-B1
manual-validate sites (the plan regenerates the exact list via the same per-fn scan that produces the ~23 conversion
targets — do **not** trust this count; an undercounted seed breaks the gate on day one against correct handlers),
header comment `frozen 2026-08-05 — shrink-only; additions prohibited`. The gate enforces the **ratchet**: it fails
on any flagged site not in the allowlist, on any stale entry (allowlisted site no longer flagged — must be deleted),
and if the entry count exceeds the script's `MAX` baseline constant (decremented as Stage 2 converts sites). The list
can only shrink.

**Future-extractor tripwire:** the gate also greps `impl … FromRequest<` under `crates/ui/web-api/src/` — matching
`FromRequest<` and **excluding `FromRequestParts<`** (the repo has ~9 legitimate parts-only extractors — `TenantDb`,
`IfMatch`, the `action_extractor!`/`permission_extractor!` macro output, … — that carry no body and must not trip the
gate) — and fails on any body-extractor impl outside the known set (`Validated`, `Unvalidated`, `UnvalidatedForm`).
A future custom body extractor (`Signed<T>`, size-limiting wrapper, …) must be explicitly reasoned about rather than
silently widening the gap.

**Third-door tripwire (hand-rolled body reads):** a handler can bypass extractors entirely — take
`axum::extract::Request` (or `Bytes`), call `body::to_bytes`, and `serde_json::from_slice` by hand. **Live instances
(contrarian, grounded):** `auth.rs` `logout` and `refresh` deserialize `LogoutRequest`/`RefreshRequest` this way, and
neither type has a `Validate` impl — invisible to the compiler bound, the extractor grep, and the `FromRequest`
tripwire alike. Add one more alternation to the same signature scan: flag `Request` / `Bytes` body params in
`routes/` — **anchored to param type position** (`:\s*(axum::extract::)?Request\s*[,)]`, likewise `Bytes`), never a
bare token: `Request` is a substring of every `*Request` body type (71 in `web-api-types`), so an unanchored pattern
would fire on the very `Unvalidated<UpdateHostRequest>` params Part 3 just produced. Allowlist the known-legitimate
sites (the auth pair and `oidc_auth.rs::oidc_link` — natural `Option<Unvalidated<T>>` shapes to convert in Stage 2;
`ocsp.rs` DER payload; the `notifications.rs` raw-body site — `oidc_link` was found at plan review, missing from the
earlier enumeration; the gate's stale-entry check keeps the list honest). Exempt axum middleware signatures
(`next: Next` param — body passes through untouched; `oauth/mod.rs::optional_oauth_auth` would otherwise
false-positive). The third door becomes **enumerated**, not invisible.

**Residual check for frozen entries:** an allowlisted fn is exempt from the raw-extractor flag — so a later refactor
that drops its manual `.validate()` call would go unseen by both compiler and gate. For **allowlisted fns only**, the
gate additionally requires a `.validate(` call within that fn's **body span** — deliberately more than the
signature-level scan the rest of the gate uses. To avoid the test-module-absorption false negative that killed the
body-scanning alternative (an allowlisted fn last in its file absorbs the `#[cfg(test)]` module, and a stray
`.validate()` in tests grants a false pass — 25 route files carry such modules), **truncate each file at its first
`#[cfg(test)]` before any scanning** (one line of perl, independently correct for the whole gate). (Body-level text
scanning was rejected as the _general_ mechanism; against ~26 known, enumerated, shrinking handlers its
false-negative risk is bounded and the protection is real — the rejection reasoning does not transfer to this scope.)

**Scope of the guarantee (stated honestly):** for **converted** handlers the guarantee is the compiler's — fields are
unreachable without `require_valid()`; the gate adds nothing there. The gate covers the residual: new raw-extractor
sites (until an author converts them the compiler has no say), the frozen legacy list, and — via the third-door
tripwire — hand-rolled `Request`/`Bytes` body reads (**enumerated and allowlisted, not compile-blocked**; validating
their content stays the handler's job). It does not verify _what the handler does_ with
a `require_valid()` failure — the `ValidationFailed`-audit half of pattern 2 remains a review-time concern (extending
`audit-catalog.toml` coverage to validation events is out of scope). The OAuth DCR (`clients_api.rs`, `register.rs`)
and token/device (`Form`) handlers already manual-validate — they are **Bucket B1**: frozen-allowlisted in Stage 1,
untouched. Their Stage 2 conversion must preserve the RFC-shaped `oauth_400` mapping and
rate-limit-before-validate ordering — `require_valid()` supports both (called wherever the handler chooses).

Wire the gate into `.github/workflows/ci.yml`, `AGENTS.md` quick-start (Rust block), and
`docs/development/quality-gates.md` (canonical) in the **same commit** (quality-gate-authoring invariant). Husky
pre-commit inclusion optional (grep-only, fast — unlike the superseded xtask design).

### Staging

- **Stage 1 (this spec):** Parts 1–4. Live hole closed with compile-time guarantees; B1 and `Validated<T>` sites
  untouched → zero regression risk on working, audited handlers.
- **Stage 2 (follow-up, separate plan):** convert the ~26 B1 handlers — each a ~5-line restructure of its existing
  validate+audit block onto `require_valid()`'s `Err` arm, preserving that handler's exact authz/validate ordering,
  status code (422 sites stay 422), and audit emission. Also converts the `auth.rs` `logout`/`refresh` hand-rolled
  body reads to `Option<Unvalidated<T>>`. Decrement the gate's `MAX` per conversion; allowlist reaches zero.
  **Commitment, not intention** (contrarian: a ratchet with no clock never reaches zero): draft the Stage 2 plan
  promptly after Stage 1 lands — the per-handler work is small and enumerated; until then the frozen entries carry the
  residual `.validate(`-presence check (Part 4).
- **Stage 3 (optional):** migrate the ~30 `Validated<T>` sites to `Unvalidated<T>` + `require_valid()` and delete
  `Validated<T>`, leaving one body-extraction mechanism. Decide in the Stage 2/3 plan; nothing here depends on it.

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
- **No audit regression.** Bucket B1 is untouched; the converted bypass handlers _gain_ `ValidationFailed` audit
  coverage where their sibling family audits (no-audit-family handlers gain a generic 400 with no audit, matching
  their siblings).
- **No behavior change from the extractor swap itself.** `Unvalidated<T>`/`UnvalidatedForm<T>` delegate
  deserialization to `axum::Json`/`Form`, so malformed-body rejections are unchanged; `preview_software_item_merge`
  and `approve_consent` gain trivially-passing `require_valid()` calls (`Ok(())` impls) — no observable change.

## Tests

- **Unit (`web-api-types`, the 9 rich/moderate types):** valid input passes; each violation returns
  `Err(ValidationError)` — `Some("")` name; slug `"UPPER"`/`"-x"`/65-char/`"a/b"`; `issuer_url` `Some("ftp://x")`,
  `Some("")`; `client_id` `Some("")`; `to_version` `""` and over-length; `host_assignments` empty and 101-length;
  mutual-exclusivity (zero set, two set). Assert `field` where it disambiguates. **Omitted (`None`) fields pass** (PATCH
  keep-semantics).
- **Typed-only + swept-in + no-invariant `Ok(())`:** one `validate()`→`Ok(())` smoke test each.
- **Extractor unit (`web-api`):** `Unvalidated::<T>::require_valid()` returns `Ok(inner)` for a valid fixture type and
  `Err(ValidationError)` for an invalid one — our logic only; deserialization/rejection behavior is axum's, untested.
  (No compile-fail test — the privacy guarantee is structural, and a `trybuild` dev-dep would violate "no new deps".)
- **Handler (TestApp harness):** `PATCH /api/v1/oidc-providers/{id}` with `issuer_url:"ftp://x"` → 400 **and** assert a
  `ValidationFailed` audit event is emitted (proves the `Err`-arm audit mapping, not just a bare 400); one
  `UpdateHostAssignmentRequest` two-sources-set → 400 (proves the 404→400 shift); one `Option<Unvalidated<T>>` surface
  handler with no body → success (proves absent-body defaulting survived the swap).
- **Gate self-check:** `bash ci/verify_no_raw_body_extractors.sh` exits 0 after the conversion; RED-test it
  (temporarily add a `Json<X>` param to a routes fn → non-zero; a stale allowlist entry → non-zero; a fn returning
  `-> Json<XResponse>` with no body param → still green, proving the return-type strip; an unallowlisted
  `Request`/`Bytes` param → non-zero, proving the third-door tripwire; a converted `Unvalidated<XRequest>` handler →
  still green, proving the third-door pattern's param anchoring; removing `.validate()` from an allowlisted fn →
  non-zero, proving the residual check); empty-input guard per the sibling gates.
- No `start_paused` (no `tokio::time`). **Do not test** serde deserialization or axum extraction mechanics (framework
  behavior).

## Deliverables

- `crates/ui/web-api/src/extract.rs` — `Unvalidated<T>` + `UnvalidatedForm<T>` (+ optional-extraction support for
  `Option<Unvalidated<T>>`); extractor unit tests.
- `crates/shared/web-api-types/src/*` — 17 `impl Validate` (9 rich + 5 minimal + 2 no-invariant +
  `HostPluginRoleAssignment`, the nested type carrying the string fields); unit tests for the rich/moderate ones.
- `crates/ui/web-api/src/routes/*` — convert the ~23 bypass handlers to the new extractors with per-family `Err`
  mapping; the handler tests. **Leave the ~26 Bucket-B1 manual-validate handlers and the ~30 `Validated<T>` sites
  untouched.** The plan regenerates the exact bypass list via the per-fn scan.
- `ci/verify_no_raw_body_extractors.sh` + `ci/verify_no_raw_body_extractors_allowlist.txt` (frozen, ~26 B1 entries,
  ratchet `MAX`) — new gate; wire into `.github/workflows/ci.yml`.

### Documentation deliverables

- `docs/development/coding-standards.md` — request-type-validation section rewritten: request bodies are extracted via
  **`Unvalidated<T>` + `require_valid()`** (handler-controlled status/audit) or **`Validated<T>`** (pre-handler,
  generic 400) — both structurally safe, **not co-equal**: `Unvalidated<T>` is the **default for auditable-entity
  mutations** (only it can emit the family's `ValidationFailed` audit event, return 422, or validate after authz);
  `Validated<T>` only where the entity family does not audit validation failures (contrarian: presenting them as
  co-equal re-opens the audit-coverage asymmetry the design exists to preserve). Raw `Json<T>`/`Form<T>` body
  extractors are **banned and CI-enforced** (name the gate); document the Update/PATCH `Some("")` idiom and that every
  body type needs an `impl Validate` (`Ok(())` if genuinely nothing to check — the allowlist is only the frozen legacy
  list, never for new handlers). `*Request` naming stays a soft convention.
- **One ADR** (`adrs new`, never hand-numbered): the type-state body-extraction contract + the staged migration
  (frozen-allowlist ratchet, Stage 2/3). This bans a raw-extractor class repo-wide and introduces a new structural
  invariant — an architectural decision, not internal mechanics (flipped from the previous design's "No ADR"). **State
  the coverage precisely**: compile-blocked for extractor-borne bodies; gate-enumerated (not compile-blocked) for
  hand-rolled `Request`/`Bytes` reads — an ADR that overstates its own coverage is the artifact people quote later.
- `AGENTS.md` — quick-start Rust block gains the gate command; evaluate a one-line MUST-FOLLOW rule for the extractor
  contract (points at coding-standards as canonical), same commit as `docs/development/quality-gates.md` (canonical
  gate list).
- **OpenAPI:** validation adds a `400` response to the converted paths. Add `(status = 400, …)` to any converted
  handler's `#[utoipa::path(responses(...))]` that lacks it (grounded: `update_provider`, `update_host`,
  `update_plugin_config`, `update_user_active` lack it; plan audits all ~23), then run `./scripts/regen-api.sh` and
  commit `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`. `request_body = <Type>` attrs are
  untouched by the extractor swap (grounded above), so body **shapes** are unchanged and `uptrakit-openapi-client`
  needs no signature change.
- **No wire/dependency change.** **No `db_access_policy.toml` change** — the extractor swap alters only the body
  extractor; `ci/verify_db_access_policy.py` classifies handlers by their `State<…>` sub-state extractors, which are
  untouched (stated explicitly so the plan doesn't re-derive it).

## Alternatives considered

- **Convert everything (B1 included) to `Validated<T>` (a uniform "no `Json<*Request>`" sweep)** — **rejected: the B1
  handlers are heterogeneous and `Validated<T>` cannot preserve their behavior.** Three distinct properties would
  silently regress: (1) most emit domain-specific `AuditOutcome::ValidationFailed` audit events that the generic
  extractor cannot reproduce (no entity/action context) — a security-audit regression against the repo's audit
  invariant; (2) several deliberately return **422** on semantic-validation failure (`settings_access.rs:85`,
  `users.rs:679/904/1099`), not the extractor's generic **400** — a wire-contract change; (3) some run **authorization
  before validation** (`users.rs:672`) so an unauthorized caller never reaches validation — `Validated<T>` (a
  `FromRequest` extractor) runs _before_ the handler body, inverting that order and leaking input-validity signal to
  unauthorized callers. These three properties are exactly what `Unvalidated<T>`'s handler-controlled `require_valid()`
  preserves — the adopted design is this alternative with the pre-handler validation removed.
- **Per-handler manual wiring + `syn`-based `cargo xtask request-validation-check`** — the design chosen at the
  2026-08-05 re-review; **superseded the same day by owner decision** in favor of the type-state extractor. Reasons:
  (1) the xtask check (~300–500 lines: visitor, receiver-checked `.validate()`, `cfg(test)` handling, handler-keyed
  allowlist, `FromRequest` tripwire, fixture tests) is meta-tooling enforcing a CI-time _proxy_ for the invariant —
  satisfiable vacuously and silent-when-blind, as two contrarian passes showed — where the type system enforces the
  invariant itself and reduces the gate to a signature-level grep; (2) its projected `utoipa` integration cost was
  disproven (`request_body` is explicit in the attr — extractor swaps have zero OpenAPI impact); (3) total effort is
  comparable or lower, spent on product code instead of tooling. Staging removed the original objection (touching the
  ~26 correct B1 handlers): they stay untouched behind the frozen allowlist.
- **Fix only `UpdateOidcProviderRequest` / only the 9, skip enforcement** — rejected: leaves the class open (the audit
  flags the missing enforcement as the root cause).
- **Text-level perl/grep gate over fn _bodies_ (`ci/verify_mutating_requests_validated.sh`)** — the original 2026-07-12
  design; **rejected at contrarian review.** Body-level text scanning produces **false negatives**: naive
  fn-start→fn-start spans let the last handler in a file absorb the `#[cfg(test)]` module, and a substring check is
  receiver-agnostic (`query.validate()` satisfies it for an unvalidated body). A gate whose failure mode is
  false-negative is worse than no gate. (The adopted Part 4 gate is also perl/grep, but **signature-level** — the
  sibling idiom's actual competence — because the type system now carries the body-level semantics.)
- **Unvalidated-escape accessor on the extractor (`into_inner_unchecked()` or a `Deref`)** — rejected: any non-consuming
  or non-validating exit reopens the bypass the type exists to close. `require_valid()` is the only exit.
- **Big-bang conversion (B1 + `Validated<T>` sites in this change)** — rejected: touching ~26 working, audited,
  security-sensitive handlers plus ~30 `Validated<T>` sites in the same change as the bug fix maximizes regression risk
  for zero additional guarantee on the sites that are already safe. Staging gets the compile-time guarantee where the
  bug lives now; the rest ratchets.
- **`Ok(())` impls for no-invariant types (vs allowlisting them)** — kept from the original design, now
  compile-motivated: the `T: Validate` bound requires an impl anyway. Residual risk, stated: an `Ok(())` impl rots — a
  new `String` field on such a type validates nothing. Mitigation: the impl + its `// No format/length invariants…`
  comment sit adjacent to the struct, so a field-adding diff shows them; nothing can force rules onto future fields.
- **Naming the extractor `Body<T>`** — rejected: collides with `axum::body::Body`, which web-api code imports.
- **Validate `plugin_type` capability inside `Validate`** — rejected: `PluginCatalog` is not in `web-api-types`; the
  check stays handler-side (where it already is).
- **Remove the `oidc_providers.rs:929` defensive guard** — rejected: protects reads of legacy rows; keep both.

## Out of scope

**Stage 2** (converting the ~26 Bucket-B1 manual-validate handlers — correct as-is, frozen-allowlisted) and **Stage 3**
(migrating/retiring `Validated<T>`, ~30 sites) — follow-up plans; see Staging. Other unspecced Medium+ findings
(short-term-backlog tier) — separate specs. No change to `Create`-type validation, the `PluginCatalog` capability
check, merge-time plugin-config validation, or PATCH clear-semantics (`null` vs `Some("")`) beyond `Some("")` failing
for required fields. No new request fields, no request-body shape change. `Query`/`Path` params are out of scope. The
extractor contract targets typed `Json`/`Form` bodies; hand-rolled `Request`/`Bytes` body reads get no compile-time
guarantee — only the Part 4 third-door tripwire (enumerated + allowlisted; conversion of the `auth.rs` pair is
Stage 2).
