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
        ├── api.ts                   # Typed API client — all HTTP calls go here
        ├── api.test.ts              # Unit tests for api.ts
        ├── auth.svelte.ts           # Auth state store (current user, session)
        ├── auth.test.ts             # Unit tests for auth.svelte.ts
        ├── extensions.svelte.ts     # UI extensions store (manifest registry)
        ├── extensions.test.ts       # Unit tests for extensions.svelte.ts
        ├── interactive.ts           # Interactive update session utilities
        ├── notifications.svelte.ts  # Toast notification store
        ├── sse.ts                   # Low-level SSE connection utilities
        ├── theme.svelte.ts          # Dark/light theme store
        ├── token-store.svelte.ts    # API token management store
        ├── types.ts                 # Shared TypeScript types mirroring web-api-types
        ├── utils.ts                 # General utility functions
        ├── utils.test.ts            # Unit tests for utils.ts
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
        │   └── extensions/                     # Schema-driven extension UI components
        └── stores/
            ├── events.svelte.ts    # Centralized admin event SSE store (lazy, debounced)
            └── network.svelte.ts   # Network connectivity state store
```

Route directories under `src/routes/`:

```text
src/routes/
├── audit-logs/       # /audit-logs — tenant and system audit log viewer
├── device/           # /device — device authorization flow
├── extensions/       # /extensions — service UI extension pages
├── history/          # /history — update history with filters and SSE output
├── host-tags/        # /host-tags — host tag management
├── hosts/            # /hosts — host list, detail, discovery
├── login/            # /login — password and OIDC login
├── profile/          # /profile — account info and API token management
├── register/         # /register — first-user registration
├── services/         # /services — tenant service management
├── settings/         # /settings — global settings (auth, certs, MQTT, OIDC, NATS)
├── software/         # /software — software item list and pending discovery
└── system-services/  # /system-services — system-tier service management
```

## Rules for AI agents

1. **All API calls go through `src/lib/api.ts`.** Do not call `fetch` directly for API endpoints;
   the client mirrors the `uptrakit-openapi-client` Rust crate's endpoint set.
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
7. **Extension components must match built-in page conventions.** Tables use
   `<div class="table-wrap"><table class="table">`, forms use Skeleton's `.label` class per field,
   empty states use a two-line pattern inside `<td colspan>`, and page headings use
   `<h1 class="h1 mb-6">` — the same as all built-in pages.
8. **Batch selection uses `SvelteSet<string>`.** The `svelte/prefer-svelte-reactivity` ESLint rule
   requires `SvelteSet` (not `Set`) for reactive multi-select state in Svelte 5 components.
