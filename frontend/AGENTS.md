# AGENTS -- AI Agent Guide for Uptrakit Frontend

This file scopes AI agent context to the SvelteKit frontend in `frontend/`. For project-wide rules,
architecture invariants, Rust crate layout, and quality gates, **read the root
[`AGENTS.md`](../AGENTS.md) first**. The root file takes precedence on any topic not covered here.

## Technology stack

- **Framework:** SvelteKit `^2.55.0` (static adapter, see `svelte.config.js`)
- **UI library:** Skeleton UI `^4.13.0` + Tailwind CSS `^4.2.1`
- **Language:** TypeScript `^5.9.3`
- **Unit tests:** Vitest `^4.1.0` (`vitest.config.ts`)
- **E2E tests:** Playwright `^1.58.2` (`playwright.config.ts`)
- **Linting:** ESLint `^10.0.3` (`eslint.config.js`) + Prettier `^3.8.1`

## Commands

| Command                 | Purpose                                                               |
| ----------------------- | --------------------------------------------------------------------- |
| `npm run dev`           | Local dev server with HMR (proxies `/api` → `https://localhost:8443`) |
| `npm run build`         | SvelteKit production build to `build/`                                |
| `npm run preview`       | Preview the production build locally                                  |
| `npm run check`         | Svelte/TypeScript type check via `svelte-check`                       |
| `npm run check:watch`   | Same, in watch mode                                                   |
| `npm run test`          | Vitest unit and component tests                                       |
| `npm run test:coverage` | Vitest with V8 coverage                                               |
| `npm run test:e2e`      | Playwright end-to-end tests                                           |
| `npm run lint`          | ESLint                                                                |
| `npm run format`        | Prettier auto-fix                                                     |
| `npm run format:check`  | Prettier read-only check (used in CI)                                 |

> Run `lint`, `format:check`, `check`, and `build` before committing frontend changes. These four
> are enforced by the root quality gates.

## File structure

```text
frontend/
├── package.json            # npm scripts and dependency manifest
├── svelte.config.js        # SvelteKit config (static adapter)
├── tsconfig.json           # TypeScript config
├── vite.config.ts          # Vite config (Tailwind CSS, dev proxy to :8443)
├── vitest.config.ts        # Vitest unit test config
├── playwright.config.ts    # Playwright E2E test config
├── static/                 # Static assets served as-is
└── src/
    └── lib/
        ├── api/                     # API client (see "Regenerating the API client" below)
        │   ├── index.ts             # `$lib/api` barrel — re-exports the generated SDK + the helpers below
        │   ├── generated/           # @hey-api/openapi-ts output from openapi.json (committed; do not hand-edit)
        │   ├── client.ts            # configured hey-api client + interceptors (auth, 401 refresh-retry, ETag, 2FA, ApiError)
        │   ├── errors.ts            # ApiError + Response→message/ApiError mappers
        │   ├── raw.ts               # raw-Response escape hatch (authenticatedFetch/apiGet/loginRaw) over the configured client
        │   ├── surfaces.ts          # non-spec surface endpoints (0 utoipa paths) over the configured client
        │   ├── crypto.ts            # sealed-box encryption (Web Crypto)
        │   ├── batch.ts             # executeBatchChunked (client-side 100-id chunking)
        │   ├── oauth.ts             # OAuth client/consent shim (paths outside /api/v1)
        │   └── local-types.ts       # frontend-only types with no generated equivalent
        ├── auth.svelte.ts           # Auth state store (current user, session)
        ├── auth.test.ts             # Unit tests for auth.svelte.ts
        ├── interactive.ts           # Interactive update session utilities
        ├── notifications.svelte.ts  # Toast notification store
        ├── sse.ts                   # Low-level SSE connection utilities
        ├── theme.svelte.ts          # Dark/light theme store
        ├── token-store.svelte.ts    # API token management store
        ├── utils.ts                 # General utility functions
        ├── utils.test.ts            # Unit tests for utils.ts
        ├── surfaces/                # Shared surface runtime store, read models, interactions
        ├── components/
        │   ├── AddSoftwareModal.svelte        # Modal to add a software item
        │   ├── AssignToHostModal.svelte        # Modal to assign software to a host
        │   ├── BatchActionBar.svelte           # Fixed-position bar for bulk actions
        │   ├── BatchResultDialog.svelte        # Partial-success results dialog
        │   ├── CheckboxList.svelte             # Reusable multi-select checkbox list
        │   ├── ConfirmDialog.svelte            # Destructive action confirmation dialog
        │   ├── ContextMenu.svelte              # Viewport-aware, keyboard-navigable context menu
        │   ├── EditHostAssignmentModal.svelte  # Modal to edit host plugin assignments
        │   ├── Modal.svelte                    # Base modal wrapper
        │   ├── ModalBackdrop.svelte            # Focus-trapped modal backdrop
        │   ├── Pagination.svelte               # Page number buttons with ellipsis and total count
        │   ├── TagBadge.svelte                 # Host tag badge chip
        │   ├── TerminalOutput.svelte           # xterm.js wrapper (dark/light theme)
        │   ├── ToastNotifications.svelte       # Toast notification display container
        │   └── surfaces/                       # Shared surface UI components
        └── stores/
            ├── events.svelte.ts    # Centralized admin event SSE store (lazy, debounced)
            └── network.svelte.ts   # Network connectivity state store
```

Route directories under `src/routes/`:

```text
src/routes/
├── audit-logs/       # /audit-logs — tenant and system audit log viewer
├── device/           # /device — device authorization flow
├── history/          # /history — update history with filters and SSE output
├── host-tags/        # /host-tags — host tag management
├── hosts/            # /hosts — host list, detail, discovery
├── login/            # /login — password and OIDC login
├── profile/          # /profile — account info and API token management
├── register/         # /register — first-user registration
├── services/         # /services — tenant service management
├── settings/         # /settings — global settings (auth, certs, MQTT, OIDC, NATS)
├── surfaces/         # /surfaces — shared surface pages
├── software/         # /software — software item list and pending discovery
└── system-services/  # /system-services — system-tier service management
```

## Rules for AI agents

1. **API calls use the generated SDK via `$lib/api`.** Do not call `fetch` directly for API
   endpoints. Import the generated operation (e.g. `listServices`) from `$lib/api` and call it with
   option-object args (`{ path, query, body }`, snake_case keys), destructuring `{ data }`; the
   configured client adds bearer auth, deduped 401 refresh-retry, ETag `If-Match`, the 2FA redirect,
   and `ApiError` mapping. Generated types come from `$lib/api` too (no hand-maintained mirror).
   Non-spec endpoints (surfaces) and escape hatches live in the hand-written `api/*` modules listed
   in the tree above. The source of truth for the frontend client is the committed spec
   `crates/ui/web-api/openapi.json`; after any backend route change run `./scripts/regen-api.sh` and
   commit both `openapi.json` and `src/lib/api/generated/`.
   - **Scope (honest):** today only the **frontend** client is generated from / gated against the
     spec. The Rust `uptrakit-openapi-client` crate is **not yet** generated from this spec — that
     is a planned follow-up — so the spec is not yet a workspace-wide source of truth.
   - **OpenAPI placement rule:** for an endpoint to appear in `openapi.json` (and thus the generated
     client), the backend handler MUST be registered via `.routes(routes!(...))` **before**
     `split_for_parts()` in `crates/ui/web-api/src/router.rs` — handlers added via raw `.route()`
     after the split are silently absent from the spec and the generated client. See `router.rs`
     (placement comment) +
     `integration_tests/openapi_spec.rs::openapi_spec_eligible_endpoints_present`. List endpoints
     should expose their query filters via `params(<IntoParamsStruct>)` (not a hand-maintained
     `params(...)` list) so the spec can't silently drop a filter the handler accepts. Canonical rule +
     rationale: `docs/development/coding-standards.md` ("OpenAPI parameter & schema authoring") +
     `docs/adr/0025-drift-proof-openapi-params.md` (enforced by `ci/verify_no_inline_query_params.sh`).
2. **Use existing shared components first.** Before creating a new UI component, check
   `src/lib/components/` for: `AddSoftwareModal`, `AssignToHostModal`, `BatchActionBar`,
   `BatchResultDialog`, `CheckboxList`, `ConfirmDialog`, `ContextMenu`, `EditHostAssignmentModal`,
   `Modal`, `ModalBackdrop`, `Pagination`, `TagBadge`, `TerminalOutput`, `ToastNotifications`.
3. **Preserve the static adapter.** The frontend compiles to static files embedded into
   `uptrakit-controller` via `rust-embed` (`embed-frontend` Cargo feature). Do not add SvelteKit
   server routes, API routes, or non-static adapters.
4. **TypeScript is required.** All new files must be `.ts` or `.svelte` with
   `<script lang="ts">`. Avoid `any` casts.
5. **SSE subscriptions use the centralized store.** For admin events (software state changes,
   service enrollment, scheduler events, etc.) use `subscribeToEvent()` from
   `src/lib/stores/events.svelte.ts` — it provides lazy connection, deduplication, and debouncing.
   For update output streams, use `connectOutputStream()` from `src/lib/sse.ts` directly.
6. **Run all four frontend quality gates after changes:** `npm run lint`,
   `npm run format:check`, `npm run check`, `npm run build`. These map directly to the root quality
   gates for frontend-only changes.
7. **Shared surface components must match built-in page conventions.** Tables use
   `<div class="table-wrap"><table class="table">`, forms use Skeleton's `.label` class per field,
   empty states use a two-line pattern inside `<td colspan>`, and page headings use
   `<h1 class="h1 mb-6">` — the same as all built-in pages.
8. **Batch selection uses `SvelteSet<string>`.** The `svelte/prefer-svelte-reactivity` ESLint rule
   requires `SvelteSet` (not `Set`) for reactive multi-select state in Svelte 5 components.

## Regenerating the API client

The generated TypeScript client at `src/lib/api/generated/` is committed and must stay in sync with
`crates/ui/web-api/openapi.json`. After any backend route or REST-contract change, regenerate both in one step:

```sh
./scripts/regen-api.sh
```

This runs `UPDATE_OPENAPI=1 cargo test -p uptrakit-web-api --all-features openapi_` to dump `openapi.json`,
then `npm run gen:api` to regenerate the TypeScript client. Commit both `crates/ui/web-api/openapi.json`
and `frontend/src/lib/api/generated/`. CI gates on staleness of both paths — a stale generated client or
spec will fail the build.

The generated directory is excluded from ESLint (`ignores` in `eslint.config.js`) and Prettier
(`.prettierignore`). It is also excluded from CodeScene via the project's path-filter setting
(pattern `**/api/generated/**`), configured in the CodeScene dashboard rather than a repo file.
