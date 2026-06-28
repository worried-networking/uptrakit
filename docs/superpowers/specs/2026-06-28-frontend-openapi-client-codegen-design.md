# Frontend OpenAPI Client Codegen — Design Spec

- **Date:** 2026-06-28
- **Status:** Draft (ready for plan — plan **must phase** per §13: Phase 1 foundation/spikes gates Phase 2 migration)
- **Author:** Andrey Yantsen (with Claude)
- **Scope target:** `frontend/src/lib/api.ts` + `frontend/src/lib/types.ts`, plus a new Rust
  spec-dump test and CI wiring.

## 1. Problem / Goal

`frontend/src/lib/api.ts` is a 1528-line hand-written REST client with a CodeScene
Code Health score of **5.57** (yellow / problematic technical debt). The smells are
mechanical and structural:

- **6 Complex Methods** (cc > 9), all query-string builders with long `if (x) params.set(...)`
  chains: `listAuditLogs` / `listSystemAuditLogs` (cc = 25, near-identical twins),
  `listUpdateHistory` (11), `getSoftwareItems` (10), `getSystemServices` (10), `getServices` (10).
- **1 Complex Conditional** — `extractErrorMessage` (duplicates `extractApiError`).
- **~13 Duplicated functions** — pagination param builders + unauthenticated `fetch` wrappers
  (`refreshAccessToken`, `mfaVerify`, `mfaSendEmail`, `oidcExchange`).
- **Excess arguments** — `getSoftwareItems` takes 7 positional args.
- **Module-level Primitive Obsession / String-Heavy Arguments** — single giant module, most
  params untyped strings.

`frontend/src/lib/types.ts` is a **1147-line hand-maintained** mirror of the Rust
`web-api-types` crate — a continuous drift risk with no compiler link to the backend.

The backend already produces an OpenAPI 3.1 spec via `utoipa` (144 `#[utoipa::path]`
annotations, served at runtime on `/api/openapi.json` + SwaggerUI at `/api/docs`).
There is **no checked-in `openapi.json`** today and the Rust `uptrakit-openapi-client`
crate is **hand-written**, not generated.

**Goal:** Replace the hand-written `api.ts` + `types.ts` with a **generated client + types**
produced from the backend OpenAPI spec, regenerated and staleness-checked on every PR. This
removes the CodeScene hotspot entirely (generated code is analysis-excluded), eliminates type
drift, and makes the spec the single source of truth for the frontend↔backend contract.

## 2. Locked Decisions

Decided during grilling (2026-06-28):

| #   | Decision      | Choice                                                                                                                                                                                                                                                                                                                                                  |
| --- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Scope         | **Full client codegen** — replace `api.ts` + `types.ts` with a generated client.                                                                                                                                                                                                                                                                        |
| D2  | Codegen tool  | **`@hey-api/openapi-ts`** (`^0.99.0`, devDependency only). The fetch client is **bundled into the generated output** (`bundle: true` default); the standalone `@hey-api/client-fetch` npm package is deprecated (bundled into `openapi-ts` since v0.73) and is **not** added as a runtime dep.                                                          |
| D3  | Spec source   | **Rust test serializes the complete served spec (the `/api/openapi.json` document, §4.1) → committed `crates/ui/web-api/openapi.json`**; CI fails on drift.                                                                                                                                                                                             |
| D4  | Artifact      | **Commit both** `openapi.json` and generated client; CI staleness gate via `git diff --exit-code`.                                                                                                                                                                                                                                                      |
| D5  | Spec location | **Co-located with its producer: `crates/ui/web-api/openapi.json`** — matches the repo precedent `crates/shared/wire/asyncapi.yaml` (protocol spec lives with the crate that emits it). Consumed by the frontend codegen now and the Rust client codegen in the follow-up spec (see §10). Clean `CARGO_MANIFEST_DIR`-relative resolution, no `../../..`. |
| D6  | Architecture  | **Thin client + interceptors**; call sites use the generated SDK directly (via the `$lib/api` barrel).                                                                                                                                                                                                                                                  |
| D7  | No ADR (yet)  | This spec is a transitional, reversible tooling change scoped to the frontend; the design spec + `AGENTS.md` record the rationale. The **ADR is authored in the Rust-client follow-up spec**, where spec-as-source-of-truth becomes a workspace-wide convention (TS + Rust both generated).                                                             |

## 3. Architecture

### 3.1 Module layout (`frontend/src/lib/api/`)

`api/` already exists (`api/oauth.ts`, `api/settings.ts`). `api.ts` becomes the `api/` directory;
`$lib/api` resolves to `api/index.ts` so the **103 existing import sites keep their import path**.

```text
frontend/src/lib/api/
  generated/            # committed, analysis-excluded — @hey-api output
    types.gen.ts        # replaces the hand-written types.ts
    sdk.gen.ts          # one fn per operationId
    client/             # bundled fetch client (bundle: true; no npm runtime dep)
    client.gen.ts       # bundled-client entry / re-export
    index.ts            # hey-api barrel
  client.ts             # configures the hey-api client + ALL interceptors (cross-cutting logic)
  errors.ts             # ApiError, extractApiError, extractErrorMessage, truncateError
  raw.ts                # authenticatedFetch, apiGet, loginRaw (used by surfaces/oauth/login)
  surfaces.ts           # listSurfaces, listSurfaceProviders, getSurfaceRead, invokeSurfaceInteraction (NOT in spec)
  crypto.ts             # sealedBoxEncrypt, bytesToBase64, base64ToBytes (not API calls)
  batch.ts              # executeBatchChunked
  oauth.ts              # existing
  index.ts              # barrel: re-exports generated SDK + the hand-written modules above
```

The hand-written modules (`raw.ts`, `surfaces.ts`, `crypto.ts`, `batch.ts`, `errors.ts`) are
**extracted and refactored** out of the `api.ts` monolith into small single-responsibility
modules — retyped against the generated types where they overlap, routed through the configured
client (§3.2) instead of bespoke `fetch` calls, and deduplicated — **not lifted verbatim**. They
carry the remainder that the generated SDK cannot cover (see §3.4), and each must stand on its own
CodeScene footing (none-red), not act as a dumping ground for the smells removed from `api.ts`.

`frontend/src/lib/types.ts` is **deleted**; all `from './types'` / `$lib/types` imports
re-point to `$lib/api` (which re-exports `generated/types.gen.ts`). Hand-written-only types
that have no spec equivalent (if any survive the audit — see §8 R1) move into a small
`api/local-types.ts` re-exported from the barrel.

### 3.2 Cross-cutting logic → interceptors

`api/client.ts` configures **one** instance of the bundled hey-api fetch client (set
`throwOnError: true` in codegen config, §5.1) with
`baseUrl = import.meta.env.VITE_API_BASE || '/api/v1'`. Every behavior
currently inlined in `authenticatedFetch` / `request` / `requestVoid` moves to an interceptor:

| Current behavior (api.ts)                                        | New home                                                            |
| ---------------------------------------------------------------- | ------------------------------------------------------------------- |
| `Authorization: Bearer` header                                   | request interceptor (reads `getAccessToken()`)                      |
| Settings `If-Match` ETag auto-cache (`settingsScope`, PUT/PATCH) | request interceptor + response interceptor (captures `ETag` on GET) |
| Per-request timeout (`AbortSignal.timeout` + `AbortSignal.any`)  | request interceptor (merge caller signal)                           |
| 401 → dedup token refresh + retry-with-new-token                 | response interceptor (see §3.3)                                     |
| 403 `2fa_setup_required` → redirect `/profile#security`          | response interceptor                                                |
| Non-2xx → typed `ApiError(message, status, errorCode)`           | response/error interceptor in `errors.ts`                           |
| Session-expired banner (`setSessionExpired`)                     | response interceptor                                                |
| Subject-change ETag cache wipe (`onTokenChange`)                 | stays in `client.ts` module scope                                   |

The 6 Complex-Method query builders **disappear** — hey-api builds query strings from typed
`query` objects. The duplicated pagination builders and unauthenticated `fetch` wrappers
disappear with them.

### 3.3 Token-refresh-retry (highest-risk piece)

The current dedup-refresh-then-retry must be reproduced on top of hey-api. Preferred: a response
interceptor that, on `401` with a live token, awaits a **shared** refresh promise (dedup), then
re-issues the original request once with the new token and returns the retried `Response`. If
hey-api's interceptor surface cannot cleanly re-issue a request, fall back to a thin wrapper
around `client.request` that owns the retry loop, with the SDK pointed at that client. This is
flagged as a spike in the plan (§8 R3) and must be covered by the migrated tests (§7).

**Body-consumption caveat:** in a response interceptor the `Request` argument has an
already-consumed `body` ReadableStream — re-issuing via that `Request` silently sends an empty
body. The retry must reconstruct the body from the call **options** (method/body/headers), not
from the `Request`. The `client.request` wrapper fallback owns the body as a plain value and
sidesteps this entirely; **default to the wrapper** if the spike shows the interceptor path is
fragile.

### 3.4 Non-spec remainder (refactored, hand-written)

These are **not** in the utoipa spec (or are not REST calls), so the generated SDK cannot cover
them. They are kept hand-written but **refactored** per §3.1 (extracted into focused modules,
retyped against the generated types, routed through the configured client) — not copied as-is:

- **Surfaces** (`surfaces.ts`) — confirmed **0** `#[utoipa::path]` annotations. Dynamic UI
  extension surface; intentionally out of the typed contract. Refactored to call the configured
  client (or `raw.ts`) rather than re-implement request plumbing; the Surface component call sites
  keep their import path.
- **`raw.ts`** — `authenticatedFetch`, `apiGet`, `loginRaw`, used directly by surface components,
  `api/oauth.ts`, and `routes/login/+page.svelte` (the 202 MFA-challenge raw response). Retained
  as the explicit raw-response escape hatch, but reduced to thin wrappers over the configured
  client so the auth/timeout/error behavior is shared with the generated path, not duplicated.
- **`crypto.ts`** — `sealedBoxEncrypt` + base64 helpers (Web Crypto, not HTTP). Moved out of the
  API module's concern into a focused crypto module.
- **`batch.ts`** — `executeBatchChunked`. Extracted as a standalone helper.
- **`sse.ts`** — already standalone (`$lib/sse.ts`, own fetch+auth). **Untouched** (hey-api does
  not generate SSE).
- **`api/settings.ts`** (existing) — exposes two **spec-covered** endpoints
  (`get_access_settings`, `update_access_settings`) under the direct import path
  `$lib/api/settings`, imported by `AccessSettings.svelte`, `AccessSettings.test.ts`,
  `surface-tabs.test.ts`. After codegen these become generated SDK entries: **delete
  `settings.ts`** and migrate those 3 import sites to `$lib/api`. (Distinct from the hand-written
  remainder above — this file is _replaced_ by generation, not kept.)

> Phrasing guard: this spec does **not** claim these endpoints can never be generated. Surfaces
> annotation is a separate deferred item (§10); the rest are non-HTTP or raw-response cases that
> legitimately sit outside the typed SDK.

## 4. Rust spec dump (D3)

### 4.1 Expose the **complete served spec**, not just `ApiDoc::openapi()`

**Critical:** the spec served at `/api/openapi.json` is **not** `ApiDoc::openapi()`. In
`router.rs` the final spec (`api`) is produced by
`OpenApiRouter::with_openapi(openapi).routes(routes!(...))…split_for_parts()` (≈L940–L972), where:

- the **seed** `openapi` = `ApiDoc::openapi()` + feature-gated sub-doc merges (`ZeroconfApiDoc`,
  `OAuthSettingsApiDoc`, and under cfg `NatsApiDoc`, `ResetDataApiDoc`, `OidcApiDoc`), **and**
- `split_for_parts()` collects every `routes!(...)`-registered handler's `#[utoipa::path]` into the
  document. Many endpoints (e.g. `plugin_type_settings::*`, `software_items::delete_plugin_assignment`)
  are **only** in the `routes!()` chain, not in `ApiDoc::paths(...)`.

A builder that returns `ApiDoc::openapi()` + merges alone would emit an **incomplete** spec (those
`routes!()`-only paths missing → missing generated SDK functions). The dump must reproduce the
**same `api` value** the runtime serves.

Complication: the `routes!()` chain is interleaved with stateful middleware
(`from_fn_with_state`, `.merge(auth_routes)`), so a state-free pure builder is not free. Resolve
one of two ways (decide in the plan):

1. **Preferred — extract the OpenApiRouter assembly from middleware.** Refactor `router.rs` so the
   `OpenApiRouter` path-registration (seed + every `routes!()`) lives in a function that yields the
   `OpenApi` via `OpenApiRouter::into_openapi()` / `split_for_parts()`, with stateful `.layer()` /
   `.route_layer()` applied **after** in `build_router`. The test calls that function; the runtime
   reuses the identical assembly. This is **DB-free** (`split_for_parts()` derives the doc from
   macro annotations, needs no state instance).
2. **Fallback — build the full router in the test** with a stub `AppState` and capture the `api`
   half of `split_for_parts()` (have `build_router` return or expose it). Avoids the refactor but
   pulls `AppState` (and possibly a DB handle) into the test — prefer option 1.

Do **not** maintain a parallel spec-only route list — it would drift from the served router (the
exact failure this whole spec is removing). The test asserts the dumped doc equals the **same
`OpenApi` value** the runtime serves (compare the value / its deterministic pretty-serialization —
**not** "byte-identical to the served bytes": `/api/openapi.json` is served compact via
`axum::Json`, the committed file is pretty).

**Coverage gate (served-but-not-in-spec drift).** Sharing the assembly removes spec↔serving drift
in one direction only. Endpoints registered via raw `.route()` **after** `split_for_parts()`
(today: healthz, pki, ocsp, ws, and the email-change/MFA raw routes) serve fine yet never enter the
spec → silently absent from the generated SDK, and the staleness gate cannot see it (both sides
derive from the same assembly). Add (a) a one-line **placement rule** in `frontend/AGENTS.md` +
router comment ("spec-eligible REST handlers must be registered via `routes!()` before the split"),
and (b) a cheap **test/grep gate** asserting no spec-eligible handler is added post-split. Claim
"construction is shared; placement rule + coverage gate enforce coverage" — not "cannot diverge."

### 4.2 Staleness test

`crates/ui/web-api/tests/openapi_spec.rs`:

- Serialize the **complete served spec** (§4.1) to pretty JSON.
- Write to **`crates/ui/web-api/openapi.json`**, resolved as `CARGO_MANIFEST_DIR + "/openapi.json"`
  — the artifact sits with its producing crate (mirrors `crates/shared/wire/asyncapi.yaml`), and
  the path needs no `../../..` traversal. Both consumers reference it explicitly: the frontend
  codegen via a relative `input` (§5.1) and the Rust client crate (§10 follow-up) from within the
  same workspace.
- `UPDATE_OPENAPI=1 cargo test … openapi_spec` → **writes** the file.
- Default run → **asserts** the on-disk file matches (fails with a regen hint). `expect()` is
  allowed in test code per coding-standards.
- Must run under `--all-features` so feature-gated paths (NATS, reset-data, OIDC, zeroconf) are
  present; document that the generated client is a **superset** — endpoints absent at runtime in
  a slimmer build return 404, handled normally.

### 4.3 operationId hygiene

hey-api derives SDK function names from `operationId` (utoipa defaults it to the handler fn name,
e.g. `list_services` → `listServices`). The audit (§8 R1) records the name delta vs current
(`getServices`, etc.). Optionally set explicit `operation_id = "..."` in `#[utoipa::path]` to
control generated names where the default is ugly; otherwise call sites adopt the generated name.

## 5. Codegen + tooling (D2, D4)

### 5.1 Config — `frontend/openapi-ts.config.ts`

```ts
import { defineConfig } from "@hey-api/openapi-ts";

export default defineConfig({
  input: "../crates/ui/web-api/openapi.json", // co-located with producer (D5)
  output: { path: "./src/lib/api/generated", postProcess: ["prettier"] }, // `format` is deprecated in 0.99.x
  plugins: [
    "@hey-api/typescript",
    "@hey-api/sdk",
    { name: "@hey-api/client-fetch", throwOnError: true }, // throwOnError lives on the client plugin, not sdk
  ],
});
```

Notes (verified against `@hey-api/openapi-ts@0.99.x`):

- `throwOnError` is a property of the **`@hey-api/client-fetch`** plugin, not `@hey-api/sdk`;
  placing it on the SDK plugin is silently ignored (client defaults to `throwOnError: false`).
- `output.format` is `@deprecated` in 0.99.x in favor of `output.postProcess` (ordered array).
- The client plugin is **bundled** (`bundle: true` default) → its source lands under
  `generated/client/`; no `@hey-api/client-fetch` runtime dependency is installed.

### 5.2 npm scripts (`frontend/package.json`)

- `"gen:api": "openapi-ts"` — regenerate from the committed `openapi.json`.
- Do **not** add to `prebuild`/`postinstall` (artifact is committed; D4). CI is the gate.

### 5.3 Dependencies (latest stable, verified against npm 2026-06-28)

| Package               | Version   | Type          | Notes                          |
| --------------------- | --------- | ------------- | ------------------------------ |
| `@hey-api/openapi-ts` | `^0.99.0` | devDependency | latest stable (npm 2026-06-28) |

No runtime dependency is added: the fetch client is bundled into the committed generated output
(`bundle: true`). The standalone `@hey-api/client-fetch` package is deprecated (folded into
`openapi-ts` since v0.73) and must **not** be installed.

### 5.4 Analysis / lint exclusions

- **ESLint** flat config (`frontend/eslint.config.js`): add `src/lib/api/generated/` to `ignores`.
- **Prettier**: add `src/lib/api/generated/` to `frontend/.prettierignore` (hey-api formats its
  own output; avoids `format:check` churn).
- **CodeScene**: exclude `**/api/generated/**` from analysis (CodeScene project path filter /
  `.codescene/` config — exact mechanism confirmed during implementation via the CodeScene
  project settings). Generated code is excluded because it is machine-authored and not
  hand-maintained — standard practice, not the success metric.

**Success criterion (explicit, to avoid score-gaming):** the goal is **not** an improved aggregate
score achieved by excluding the hotspot. The goal is that the _hand-maintained_ surface shrinks and
stays healthy: **each remaining hand-written module** (`client.ts`, `errors.ts`, `raw.ts`,
`surfaces.ts`, `crypto.ts`, `batch.ts`) measures **non-red on its own**, and no module reabsorbs
the complexity removed from `api.ts` (see R3 fallback gate). Excluding `generated/` is incidental
to that, not the win.

## 6. CI wiring

Add to `.github/workflows/ci.yml` (and mirror in the pre-push gate where practical):

1. `cargo test -p uptrakit-web-api --all-features openapi_spec` — fails if
   `crates/ui/web-api/openapi.json` is stale.
2. `cd frontend && npm run gen:api && cd .. && git diff --exit-code crates/ui/web-api/openapi.json frontend/src/lib/api/generated`
   — fails if the committed generated client is stale.
3. Existing frontend gates (`lint`, `format:check`, `check`, `test`, `build`) run against the
   committed generated output.

**Contributor friction mitigation.** A backend-only change now also requires regenerating the spec

- client, which means running Node. To keep that to one step: provide a **single combined regen
  command** (e.g. a `just gen-api` / `make gen-api` target that runs the Rust dump test with
  `UPDATE_OPENAPI=1` **then** `npm run gen:api`), and have both staleness gates above print _exactly
  that command_ in their failure message. Mark the committed generated output `linguist-generated` in
  `.gitattributes` (`frontend/src/lib/api/generated/** linguist-generated=true`) so PR review
  collapses it and focuses on the real reviewable artifact: the `openapi.json` diff.

## 7. Tests

- **Rust:** new `openapi_spec` staleness test (§4.2).
- **Frontend (rewrite `api.test.ts`, 483 lines):** retarget tests from `api.ts` internals to
  `api/client.ts` interceptors — **token refresh dedup + retry**, **ETag If-Match cache**,
  **2FA-403 redirect**, **ApiError mapping (status + error_code)**, **timeout mapping**, plus
  unit tests for `crypto.ts` (sealed-box round-trip vs Rust vector), `batch.ts`
  (`executeBatchChunked`), and `surfaces.ts`. Per the testing standard, the **generated SDK is
  not unit-tested** ("do not test upstream/generated behavior").
- **E2E (Playwright):** existing auth / settings / software / audit flows must pass unchanged
  (behavioral regression gate). No new E2E required; run the existing buckets. **Note:** E2E is a
  coarse backstop, **not** the primary net for silent value-flips (R5/R7) — those are gated by the
  golden value-equality assertion, not by E2E coverage.

## 8. Risks & mitigations

- **R1 — Spec coverage gaps.** 144 annotated paths vs ~80 `api.ts` functions; surfaces confirmed
  missing. **Mitigation:** a one-time audit mapping every current `api.ts` function → spec
  operationId. Each gap is resolved by (a) annotating the handler in Rust (preferred — completes
  the contract) or (b) a hand-written shim. Surfaces stay hand-written.
  - **Raw-`.route()` sub-class (needs more than annotation):** `initiate_email_change`
    (POST `/users/{id}/email`), `cancel_email_change` (DELETE `/users/{id}/email`),
    `change_password` (PUT `/users/{id}/password`), `confirm_email_change`
    (GET `/auth/email-change/confirm`) are registered via raw `.route()` in `router.rs` (≈L745,
    L750, L945) with **no `#[utoipa::path]`**. Bringing them into the spec requires both a
    `#[utoipa::path]` annotation **and** adding them to `ApiDoc`'s `paths(...)` list (migrate off
    raw `.route()` registration). If not done, all four become permanent hand-written shims. The
    audit must call out this sub-class explicitly. **Also in this sub-class:** the OAuth management
    raw routes (`oauth/clients_api`, `oauth/consents_api`) — already isolated in the hand-written
    `oauth.ts`, so they may simply stay hand-written; the audit enumerates the full set.
- **R2 — Feature-flag superset.** Spec dumped with `--all-features`; slimmer runtime builds 404
  on absent endpoints. Accepted; client is a typed superset.

  > **Blocking pre-migration spikes (gate the whole 103-site migration).** S-A (R3+R6 combined),
  > R5-value-equality, and R7 below must be **proven before any call site is touched**. The loud risk
  > (R3) has tests + a fallback; the dangerous ones (R5/R6/R7) fail **silently** at every call site
  > and compile clean. Do not start the migration until they pass. See §13 for how they gate phasing.

- **S-A (combines R3 + R6) — refresh-retry AND ApiError identity, one spike.** These are coupled,
  not independent: with `throwOnError: true` a 401 surfaces to the retry layer as a **thrown**
  `ApiError`, not a `Response`, so retry logic must catch-and-retry on a thrown error and the
  body-reconstruction-from-options (§3.3) sits below the throw boundary. **Single acceptance test:**
  a real SDK call gets a 401 → the interceptor (or `client.request` wrapper fallback) dedups
  concurrent refreshes, retries once with the **reconstructed body** + new token, succeeds; and a
  subsequent non-401 error still arrives at the call site as `e instanceof ApiError` carrying
  `status` + `errorCode`. **Fallback acceptance gate:** if the `client.request` wrapper owns the
  retry, it must measure **non-red** on its own and must **not** reabsorb ETag If-Match, 2FA-403
  redirect, session banner, or ApiError mapping — those stay in **separate** interceptors. A
  wrapper that re-grows into `authenticatedFetch` (the original hotspot, relocated) means the spike
  **failed**, not "degraded gracefully."
- **R4 — operationId → name churn** across 103 sites. Migrate per-domain; barrel re-exports keep
  the import path stable. Optionally pin names via `operation_id`.
- **R5 — Enum representation + value equality (spike, golden test — not E2E).** `Permission`
  (string enum, widely compared) and other enums must generate as usable string unions/enums
  **whose string values equal what the app compares against today**. Because `types.ts` is a
  _drifted_ mirror (R7), a generated enum may "correct" a spelling the app depends on, silently
  flipping every comparison while types still align. **Gate:** a **golden value-equality
  assertion** — extract the enum/field string literals the app currently compares against (from
  `types.ts` + usage sites) and assert byte-equality against the generated unions. E2E (§7) is only
  a coarse backstop here, **not** the primary net (a flipped value behind a rarely-E2E'd permission
  ships green).
- **R6 — folded into S-A** (ApiError identity is coupled to refresh-retry under `throwOnError`).
- **R7 — types.ts drift reconciliation (prerequisite, not just a risk).** `types.ts` is hand-
  maintained and may have **drifted** from the backend — i.e. the working app may depend on values
  the spec spells differently. Before generating, **diff the current `types.ts` against a freshly
  dumped `openapi.json` and triage every field/enum wire-value delta**, deciding per-delta which
  side is authoritative. Without this, a migration sold as "no behavior change" ships untriaged
  behavior changes. This is a deliverable of the audit, distinct from R1's function→operationId map,
  and its outcome can invalidate the "no behavior change" premise (see §13 decision gate).

## 9. Documentation deliverables (non-optional)

- **No ADR in this spec** (D7). Rationale lives in this design spec + `AGENTS.md`. The ADR is
  authored in the Rust-client follow-up spec, where spec-as-source-of-truth becomes a
  workspace-wide convention (TS + Rust both generated). Revisit if the audit (§8 R1) forces a
  durable architectural choice (e.g. dropping hand-written shims by annotating surfaces).
- **`frontend/AGENTS.md`** — rewrite the lines stating "`types.ts` mirrors web-api-types" and
  "`api.ts` mirrors the `uptrakit-openapi-client` crate"; document the `gen:api` workflow,
  `openapi.json` source, interceptor location, and where to add non-spec endpoints. **Scope the
  claim honestly:** `openapi.json` is the source of truth **for the frontend client**; the Rust
  `uptrakit-openapi-client` is **not yet gated against it** (follow-up, §10) — do not imply
  workspace-wide source-of-truth until that lands.
- **`AGENTS.md` (root) / `CONTRIBUTING.md`** — add the codegen + staleness step (the single
  combined regen command, §6) to the dev/PR workflow.
- **`docs/development/quality-gates.md`** — add the two staleness gates (§6) to the CI/pre-push list.
- **`.gitattributes`** — mark `frontend/src/lib/api/generated/** linguist-generated=true` (§6).
- **`README.md`** — update frontend dev instructions if they describe the API client.
- **(Optional) `docs/development/frontend-api-client.md`** — short "how to regenerate" runbook.

No user-facing behavior changes (same endpoints, same auth), so no end-user doc impact beyond the
above developer docs.

## 10. Out of scope (this spec) / Follow-ups

- **Rust `uptrakit-openapi-client` codegen — planned follow-up spec, not abandoned.** This spec
  deliberately lands `crates/ui/web-api/openapi.json` (D5) as the shared source so the Rust client
  can be regenerated from the same spec next. It stays hand-written **only until** that follow-up;
  no permanent split is intended. **The ADR (spec-as-source-of-truth, workspace-wide) is authored
  in that follow-up**, covering TS + Rust together (D7).
- Annotating the surfaces endpoints into the OpenAPI spec — deferred; surfaces stay hand-written
  **for now** (could be brought into the typed contract later, see §3.4 guard).
- SSE / event-stream codegen (`sse.ts`, `events_stream`, `update_output_stream` stay hand-written;
  hey-api does not generate streaming clients).
- TanStack-Query or other hey-api plugins.
- Publishing the spec or an external API-docs site.

## 11. Quality gates (run before completion)

Rust (spec-dump refactor — the extracted spec builder/§4.1 router change must compile under both
feature sets): `cargo fmt --all`, `cargo check --no-default-features --features db-sqlite`,
`cargo check --all-features`, `cargo clippy --all-targets --no-default-features --features db-sqlite`,
`cargo clippy --all-targets --all-features`, `cargo test -p uptrakit-web-api --all-features`,
`cargo deny check` (per the pre-push standard; no-op unless Cargo manifests change).
Frontend: `npm run gen:api` then `lint`, `format:check`, `check`, `test`, `build`.
Docs: `markdownlint` + `prettier` on changed markdown. New `openapi.json` / generated-client
staleness gates (§6).

## 12. Open questions

- **OQ1:** Exact CodeScene exclusion mechanism for `api/generated/` (project path filter vs
  `.codescene/` config) — confirm against the live CodeScene project during implementation.
- **OQ2:** Whether to invest in explicit `operation_id` on handlers to keep generated names close
  to today's (reduces R4 churn) — decide after the §8 R1 audit quantifies the delta.

## 13. Sequencing (plan must phase this)

This spec is **two units of work**; the plan splits them, and Phase 2 is contingent on Phase 1's
findings — do not implement as one push.

- **Phase 1 — foundation (independently mergeable, valuable alone):**
  1. `router.rs` §4.1 refactor (extract OpenApiRouter assembly) + `openapi_spec` staleness test +
     post-split coverage gate (§4.1).
  2. Codegen wiring (§5), committed `openapi.json` + generated client, CI gates (§6) — but **no
     call-site migration yet**.
  3. The §8 **audit** (R1 function→operationId map incl. raw-`.route()` sub-class; **R7** drift
     reconciliation) and the blocking spikes **S-A** (refresh-retry + ApiError identity) and **R5**
     (golden enum value-equality).
     This phase compiles, the generated client exists and is gated, and the audit/spikes produce a
     go/no-go with quantified risk — all without touching the 103 sites.

- **Decision gate (between phases):** if the **R7 audit finds large or value-flipping drift**, the
  "no behavior change" promise is already void and each delta needs explicit triage; **and** if a
  spike (S-A / R5) fails, re-scope before migrating. At this gate, re-confirm **D1**: full codegen's
  payoff over the rejected targeted refactor is drift-elimination + the Rust-client follow-on — if
  R7 shows the contract isn't something the app safely depends on, re-cost full-codegen vs the
  cheaper refactor **here**, with data, rather than pushing through on inertia. (Not reopening D1
  now; this is a checkpoint, not a reversal.)

- **Phase 2 — migration (gated on Phase 1):** migrate the 103 call sites per-domain to the generated
  SDK + interceptors, delete `api.ts`/`types.ts`/`settings.ts`, rewrite `api.test.ts`, finish docs.
