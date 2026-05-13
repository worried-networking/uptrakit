# Graceful Reload — Plan 4: Frontend + Documentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the user-facing surfaces — Dashboard "Instance Configuration" tab consuming the new endpoints,
audit-log views for the four new event types, regenerated frontend OpenAPI types — plus every required documentation
deliverable from the spec: ADR 0008, `CONTEXT.md` glossary, `ARCHITECTURE.md` section, three updated
`docs/development/*.md` files, the new operator runbook, README, CLI help, CHANGELOG, plugin-guidelines.

**Architecture:** All work in this plan is additive on top of Plans 1–3. Frontend reads the existing
OpenAPI-generated client (regenerated after Plan 3's API surface changes) and renders the new tab through the
existing Surface framework. Docs are mechanical — fill the spec's enumerated deliverables.

**Tech Stack:** SvelteKit 2 + TypeScript 5 (strict), Vitest + Playwright (existing test framework), Prettier +
ESLint per `frontend/eslint.config.js`, `markdownlint`, `npx prettier --write` for markdown.

**Spec:** `docs/superpowers/specs/2026-05-12-graceful-reload-design.md` (sections §15.5, §18, §19, §23 + §20
operator runbook entry).

**Status:** Draft → Ready for review.

---

## Prerequisites

- Plan 3 merged (endpoints, permissions, audit events exist; OpenAPI source-of-truth updated).
- Backend running locally so `npm run check` + Playwright can hit it.

## Snapshot binding

- "TypeScript tsconfig: strict: true, checkJs: true, forceConsistentCasingInFileNames: true" — frontend code passes
  these checks.
- "ESLint: `@typescript-eslint/no-unused-vars` with `argsIgnorePattern: '^_'`, `varsIgnorePattern: '^_'`" — observed
  in new code.
- "Prettier: tab width: tabs (useTabs: true), trailing commas: none, printWidth: 120" — applied via
  `npm run format`.
- "UI parity: snapshot regeneration required on macOS + Chromium only" — Playwright snapshots only refreshed there.
- "UI design language: `frontend/src/theme/tokens.ts` is single source of truth for semantic tokens" — new tab uses
  semantic tokens, no raw hex.
- "frontend e2e tests: `npm run test:e2e` (Playwright suite) mandatory for visual/DOM-contract changes" — run before
  merge.
- "markdown line_length: 150 chars; code_blocks/tables exempt" — every new markdown file.
- "markdown MD024 siblings_only: true" — okay to reuse headings across top-level sections.
- "update documentation under docs/ whenever behavior, config, or UI changes" — entire purpose of this plan.
- Conventional Commits: `feat(frontend)`, `docs(adr)`, `docs(context)`, `docs(architecture)`, `docs(development)`,
  `docs(runbook)`, `docs(readme)`, `chore(changelog)`.

---

## File Structure

**New frontend files:**

- `frontend/src/routes/(authed)/settings/instance-config/+page.svelte` — the new tab.
- `frontend/src/routes/(authed)/settings/instance-config/+page.ts` — load() fetcher.
- `frontend/src/lib/components/instance-config/FileStatus.svelte` — file digest + pending change card.
- `frontend/src/lib/components/instance-config/LastReload.svelte` — outcome card.
- `frontend/src/lib/components/instance-config/SectionCard.svelte` — per-section read-only renderer.
- `frontend/src/lib/components/instance-config/RecentEvents.svelte` — table of last 20 reload events.
- `frontend/src/lib/components/audit/ConfigReloadEventRow.svelte` — used by the existing audit log view.
- `frontend/tests/instance-config.spec.ts` — Playwright e2e.
- `frontend/src/lib/components/instance-config/*.spec.ts` — Vitest unit tests.

**Modified frontend files:**

- `frontend/src/lib/openapi/*` — regenerated.
- `frontend/src/lib/components/audit/AuditEventRow.svelte` — dispatch to the new row for ConfigReload variants.
- `frontend/src/lib/nav/settings.ts` — register the new "Instance Configuration" surface (gated by
  `ViewInstanceConfigState`).
- `frontend/src/lib/permissions.ts` — type-mirror the two new permission strings.

**New docs:**

- `docs/adr/0008-graceful-reload-architecture.md`
- `docs/end-user/operator-runbook-reload.md`

**Modified docs:**

- `CONTEXT.md` — glossary additions
- `ARCHITECTURE.md` — new "Configuration & Graceful Reload" section
- `docs/development/coding-standards.md` — Reloadable conventions + section assignment table
- `docs/development/plugin-guidelines.md` — cheap-constructor rule
- `docs/development/testing.md` — watchdog-revert + file-watch patterns
- `docs/development/quality-gates.md` — Docker reload-integration line
- `README.md` — Configuration section
- `CHANGELOG.md` (or release notes file equivalent) — breaking changes block

---

## Task 1: Regenerate `frontend/src/lib/openapi/*`

**Files:**

- Modify (regenerated): every file under `frontend/src/lib/openapi/`

The repo's existing pattern is a script that generates the OpenAPI types from the Rust crate's spec. Match whatever
the repo already does (`npm run openapi:generate`, or similar — confirm in `package.json`).

- [ ] **Step 1:** Confirm the regeneration command in `frontend/package.json` (likely `npm run openapi:generate`)
- [ ] **Step 2:** Run it
- [ ] **Step 3:** Confirm `frontend/src/lib/openapi/` now includes the new endpoint constants + permission strings +
      audit-event variants
- [ ] **Step 4:** `cd frontend && npm run check`
- [ ] **Step 5:** Commit — `feat(frontend): regenerate OpenAPI types for config-state + clear-degraded`

---

## Task 2: Permission type-mirror

**Files:**

- Modify: `frontend/src/lib/permissions.ts`

```typescript
export const PERMISSIONS = {
  // ...existing
  ViewInstanceConfigState: "view_instance_config_state",
  ManageInstanceConfigState: "manage_instance_config_state",
} as const;
```

- [ ] **Step 1: Add the two constants**
- [ ] **Step 2: Update the `Permission` type union** (the existing pattern in the file)
- [ ] **Step 3:** `cd frontend && npm run check && npm run lint`
- [ ] **Step 4:** Commit — `feat(frontend): mirror ViewInstanceConfigState + ManageInstanceConfigState perms`

---

## Task 3: Instance Configuration tab — Svelte page + load()

**Files:**

- Create: `frontend/src/routes/(authed)/settings/instance-config/+page.svelte`
- Create: `frontend/src/routes/(authed)/settings/instance-config/+page.ts`

`+page.ts`:

```typescript
import type { PageLoad } from "./$types";
import { getConfigState } from "$lib/api/instance-config";

export const load: PageLoad = async ({ fetch, parent }) => {
  await parent();
  const state = await getConfigState(fetch);
  return { state };
};
```

`+page.svelte`:

```svelte
<script lang="ts">
  import type { PageData } from './$types';
  import FileStatus from '$lib/components/instance-config/FileStatus.svelte';
  import LastReload from '$lib/components/instance-config/LastReload.svelte';
  import SectionCard from '$lib/components/instance-config/SectionCard.svelte';
  import RecentEvents from '$lib/components/instance-config/RecentEvents.svelte';
  import { getConfigState } from '$lib/api/instance-config';
  import { invalidateAll } from '$app/navigation';

  export let data: PageData;

  // Manual refresh — spec §15.5 does not require live updates. A 5-second poll would burn an
  // authenticated request every tab-open; instead, the operator clicks Refresh when they want
  // to re-check state. SvelteKit's `invalidateAll()` re-runs the page's load() function so
  // `data.state` updates without a hard navigation.
  let refreshing = false;
  async function refresh() {
    refreshing = true;
    try { await invalidateAll(); }
    finally { refreshing = false; }
  }
</script>

<h1>Instance Configuration</h1>
<button on:click={refresh} disabled={refreshing}>
  {refreshing ? 'Refreshing…' : 'Refresh'}
</button>
<FileStatus file={data.state.file} />
<LastReload reload={data.state.last_reload} degraded={data.state.degraded} />
<h2>Sections</h2>
{#each Object.entries(data.state.sections) as [name, payload]}
  <SectionCard {name} {payload} />
{/each}
<h2>Recent reload events</h2>
<RecentEvents events={data.state.recent_events} />
```

`PageData` is generated by SvelteKit from `+page.ts`'s `load` return type, so `data.state` is fully typed in
`strict: true` mode (no `any` leak through destructuring). Accessing `data.state.…` directly (rather than
destructuring into a local `let state`) keeps SvelteKit's reactive tracking working after `invalidateAll()` re-runs
the loader.

- [ ] **Step 1: Implement loader + page**
- [ ] **Step 2: Add `$lib/api/instance-config.ts`** wrapping the generated OpenAPI client
- [ ] **Step 3:** Commit — `feat(frontend): Instance Configuration page scaffold`

---

## Task 4: `FileStatus`, `LastReload`, `SectionCard`, `RecentEvents` components

**Files:**

- Create: each component as listed in File Structure

Each component is a small Svelte file rendering one piece of the response. Match the existing design language —
semantic tokens from `frontend/src/theme/tokens.ts`, no raw hex. Secret fields render as the literal
`<redacted>` (they arrive that way from the backend, but assert in tests).

`LastReload.svelte` reads the `coordinator_state` + `degraded` payload and renders a banner when state is
`degraded`. The banner explains how to clear via `POST /api/v1/instance/config-reload/clear-degraded` and includes
a button (gated on `ManageInstanceConfigState`) that POSTs to that endpoint.

`RecentEvents.svelte` renders four event-type-specific rows (mirror of `ConfigReloadEventRow.svelte` from Task 6
below).

- [ ] **Step 1: Implement each component**
- [ ] **Step 2: Vitest unit test per component** — assert redaction, severity rendering, button gating
- [ ] **Step 3:** `cd frontend && npm run test`
- [ ] **Step 4:** Commit — `feat(frontend): instance-config component set`

---

## Task 5: Register the surface in the settings nav

**Files:**

- Modify: `frontend/src/lib/nav/settings.ts` (or equivalent in the existing Surface model)

```typescript
{
  id: 'instance-config',
  label: 'Instance Configuration',
  href: '/settings/instance-config',
  requiredPermission: 'view_instance_config_state'
}
```

- [ ] **Step 1: Add nav entry**
- [ ] **Step 2:** `cd frontend && npm run check && npm run test`
- [ ] **Step 3:** Commit — `feat(frontend): register Instance Configuration in settings nav`

---

## Task 6: Audit-log view rendering for new event variants

**Files:**

- Create: `frontend/src/lib/components/audit/ConfigReloadEventRow.svelte`
- Modify: `frontend/src/lib/components/audit/AuditEventRow.svelte`

`ConfigReloadEventRow.svelte`:

- For `ConfigReloadRequested`: shows trigger source + sections.
- For `ConfigReloadApplied`: shows duration + per-subsystem timing (collapsible).
- For `ConfigReloadFailed`: shows phase + subsystem + error. Severity badge `Error`.
- For `ConfigReloadReverted`: shows subsystem + reason. Severity badge `Warning`.

Inside the dispatcher (`AuditEventRow.svelte`):

```svelte
{#if event.type === 'ConfigReloadRequested' || event.type === 'ConfigReloadApplied'
       || event.type === 'ConfigReloadFailed' || event.type === 'ConfigReloadReverted'}
  <ConfigReloadEventRow {event} />
{:else}
  <!-- existing dispatch -->
{/if}
```

- [ ] **Step 1: Implement the dispatcher branch + row component**
- [ ] **Step 2: Vitest unit test for each event variant**
- [ ] **Step 3:** Commit — `feat(frontend): audit-log rows for ConfigReload events`

---

## Task 7: No "Reload Now" button — verification

Spec §15.5 explicitly forbids a Reload Now button. Verify by:

- [ ] **Step 1:** `rg -n 'Reload Now|reload-now' frontend/src/` — must return zero matches
- [ ] **Step 2:** Document the rationale in a code comment on the InstanceConfig page so future contributors
      understand
- [ ] **Step 3:** No commit needed unless code changed.

---

## Task 8: Playwright e2e for Instance Configuration tab

**Files:**

- Create: `frontend/tests/instance-config.spec.ts`

```typescript
import { test, expect } from "@playwright/test";
import { loginAs } from "./helpers";

test("instance config tab requires permission", async ({ page }) => {
  await loginAs(page, "operator-without-view-perm");
  await page.goto("/settings/instance-config");
  await expect(page.getByText(/forbidden/i)).toBeVisible();
});

test("instance config tab renders sections + recent events", async ({
  page,
}) => {
  await loginAs(page, "operator-with-view-perm");
  await page.goto("/settings/instance-config");
  await expect(page.getByText("Instance Configuration")).toBeVisible();
  await expect(page.getByText(/file path/i)).toBeVisible();
});

test("degraded banner shows clear-button gated by manage perm", async ({
  page,
  request,
}) => {
  // Force coordinator into Degraded via the existing test-only debug endpoint, or by
  // injecting a failing Reloadable when launching the test backend.
  await loginAs(page, "operator-with-manage-perm");
  await page.goto("/settings/instance-config");
  await expect(
    page.getByRole("button", { name: /clear degraded/i }),
  ).toBeVisible();
});
```

- [ ] **Step 1:** Implement e2e
- [ ] **Step 2:** Generate macOS + Chromium snapshots only (per snapshot rule)
- [ ] **Step 3:** `cd frontend && npm run test:e2e`
- [ ] **Step 4:** Commit — `test(frontend): Playwright e2e for instance config tab`

---

## Task 9: ADR `0008-graceful-reload-architecture.md`

**Files:**

- Create: `docs/adr/0008-graceful-reload-architecture.md`

Mirror the structure of `docs/adr/0006-instance-scoped-plugins.md`. Content per spec §19:

- Status: Accepted
- Date: 2026-05-12
- Context: prior fragmented config surface (CLI + env + DB), partial reactivity via `CaSnapshotReceiver`.
- Decision: single TOML + per-section `tokio::sync::watch<Arc<…>>` + `Reloadable` trait + two-phase
  validate/apply + atomic revert-all + reexec for irreversibly-bound keys.
- Alternatives considered:
  - Best-effort per-subsystem reload (rejected — partial state).
  - In-process DB pool URL swap (rejected — sqlx has no `resize()`; ABA hazards).
  - RPC-based reload control (rejected — bootstrap paradox).
  - Splitting cluster config to NATS (rejected — consensus problem).
- Consequences: hard CLI break; new deps (`notify-debouncer-full`, `listenfd`, `sd-notify`, `toml`); new
  permission; new endpoint; ongoing discipline on the irreversibly-bound key set (set membership =
  ADR amendments).
- Removed: `--reuseport`/`SIGUSR1`, `spawn_settings_reload` 30 s poll.

- [ ] **Step 1: Write ADR**
- [ ] **Step 2:** `npx prettier --write docs/adr/0008-graceful-reload-architecture.md`
- [ ] **Step 3:** `npx markdownlint docs/adr/0008-graceful-reload-architecture.md`
- [ ] **Step 4:** Commit — `docs(adr): 0008-graceful-reload-architecture`

---

## Task 10: `CONTEXT.md` glossary additions

**Files:**

- Modify: `CONTEXT.md`

Append the seven terms from spec §23 verbatim (paste from the spec): Reload Coordinator, Reloadable, Config Section,
Reexec, Irreversibly-bound key, Watchdog window, `ConfigReconciler`.

- [ ] **Step 1: Insert the seven glossary entries** in alphabetical position within `## Language`
- [ ] **Step 2:** `npx prettier --write CONTEXT.md`
- [ ] **Step 3:** `npx markdownlint CONTEXT.md`
- [ ] **Step 4:** Commit — `docs(context): glossary terms for graceful reload`

---

## Task 11: `ARCHITECTURE.md` new section

**Files:**

- Modify: `ARCHITECTURE.md`

Add a top-level section `## Configuration & Graceful Reload` covering:

- TOML file location + structure (link to spec for full schema).
- Per-section `tokio::sync::watch<Arc<…>>` propagation pattern.
- Reload triggers (SIGHUP, file-watch, `settings_version` bump via `ConfigReconciler`).
- Coordinator state machine (Idle → Reloading → Idle or Degraded).
- Reexec criteria + irreversibly-bound key set.
- Cross-reference to ADR 0008 and the spec.

- [ ] **Step 1: Write the section**
- [ ] **Step 2:** `npx prettier --write ARCHITECTURE.md && npx markdownlint ARCHITECTURE.md`
- [ ] **Step 3:** Commit — `docs(architecture): configuration & graceful reload section`

---

## Task 12: `docs/development/coding-standards.md` updates

**Files:**

- Modify: `docs/development/coding-standards.md`

Add new subsections:

- **Reloadable trait** — every long-lived subsystem implements `Reloadable` + a thin `ReloadableErased` via
  `#[async_trait]`. Validate is pure; apply snapshots pre-state internally; revert restores from snapshot.
- **Per-section watch pattern** — config flows through `tokio::sync::watch<Arc<SectionConfig>>`; consumers hold
  `Receiver`s injected at construction.
- **No static-init config** — forbid `lazy_static!`/`OnceCell` of configuration; all config goes through the watch
  channels.
- **Plugin constructor cheap rule** — plugin `from_config()` must be O(small); expensive resources (HTTP clients,
  SMTP) live in shared `Arc`/`OnceCell` outside the plugin struct.
- **File vs DB section assignment table** — exhaustive list of every config key and which source owns it.

- [ ] **Step 1: Add the subsections**
- [ ] **Step 2: Build the assignment table** from the SettingKey + TOML schema
- [ ] **Step 3:** Prettier + markdownlint
- [ ] **Step 4:** Commit — `docs(coding-standards): Reloadable conventions + section assignment table`

---

## Task 12a: AppState call-site migration (Plan 2 Task 13 carry-over)

**Context:** Plan 2 Task 13 was deferred. AppState already carries watch receivers for
config sections (added in Plan 1) but the legacy direct-owned fields and their downstream
call sites were never migrated.

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs`
- Modify: every call site that reads `state.audit_log_filter` directly

### Part A — `audit_log_filter` call-site migration

`AuditDispatcherReloadable` already publishes updated `AuditConfig` snapshots through
`app_state.audit_log_filter_rx: watch::Receiver<Arc<AuditConfig>>`. The legacy
`pub audit_log_filter: AuditFilter` field is stale and bypasses the reload path.

- [ ] **Step 1:** `rg -n 'audit_log_filter' crates/` — list every consumer.
- [ ] **Step 2:** At each consumer, replace `state.audit_log_filter` with
      `state.audit_log_filter_rx.borrow().clone()` (returns `Arc<AuditConfig>`; unwrap the
      inner `AuditFilter` as needed).
- [ ] **Step 3:** Remove the `pub audit_log_filter` field and its builder method from
      `app_state.rs`. The initial value must flow through `audit_log_filter_rx` instead.
- [ ] **Step 4:** Confirm `audit_log_dispatcher` and `audit_emitter` do **not** need
      watch-receiver conversion — the reloadable updates config in-place on the shared Arc;
      the instances themselves stay stable.
- [ ] **Step 5:** `cargo test -p uptrakit-web-api -p uptrakit-controller-runtime`

### Part B — DB pool spawn-site migration

`DbPoolReloadable` publishes fresh `Arc<DbConnHandle>` handles but AppState still hands
a cloned `DatabaseConnection` directly to every `tokio::spawn` site. Long-lived tasks
that capture a bare `DatabaseConnection` via `move` pin the old pool forever.

Approximately 7 sites in `controller-runtime/src/{lib,tasks}.rs` are affected (CRL
manager, audit enricher, denylist cleanup, zeroconf advertiser, embedded-service bridges,
ca_reload, pki_http). Pattern to apply (documented in Plan 4 Task 12):

1. **Re-read pattern** (preferred for loop bodies): hold
   `watch::Receiver<Arc<DbConnHandle>>` in captured set; call
   `let db = state.db_rx.borrow().clone().conn().clone()` at top of every iteration.
2. **Watch-driven re-spawn pattern** (for tasks holding connection state across
   iterations): `select!` on `db_rx.changed()` and the worker handle; abort + respawn on
   pool change.

- [ ] **Step 1:** Add `pub db_rx: watch::Receiver<Arc<DbConnHandle>>` to `AppState` and
      wire it from `DbPoolReloadable::receiver()` in `boot_config`/`lib.rs`.
- [ ] **Step 2:** `rg -n 'tokio::spawn.*\bdb\b|tokio::spawn.*db_conn' crates/core/` —
      enumerate all affected sites.
- [ ] **Step 3:** Convert each site to pattern (1) or (2). One commit per site or
      logical group.
- [ ] **Step 4:** Integration test — boot against in-memory SQLite, trigger
      `db.pool_size` reload, assert `Arc::strong_count` on the prior `DbConnHandle` drops to
      zero within 5 s.
- [ ] **Step 5:** Full quality gate suite.
- [ ] **Step 6:** Commit — `feat(controller-runtime): AppState watch-receiver migration`

---

## Task 13: `docs/development/plugin-guidelines.md` updates

**Files:**

- Modify: `docs/development/plugin-guidelines.md`

Append a section:

> ## Plugin constructor budget
>
> Plugin constructors are called every time the plugin's config changes (drop-and-recreate reload model).
> Constructors must therefore be O(small). Expensive resources — `reqwest::Client` with connection pool, SMTP
> sessions, JIT-compiled regexes — live in module-level `OnceCell` or `Arc<…>` _outside_ the plugin struct, so the
> plugin instance itself can be cheaply replaced. See `crates/plugins/notifications/email/src/plugin.rs` for a
> reference implementation.

- [ ] **Step 1: Append the section**
- [ ] **Step 2: Prettier + markdownlint**
- [ ] **Step 3:** Commit — `docs(plugin-guidelines): cheap constructor + shared resources rule`

---

## Task 14: `docs/development/testing.md` updates

**Files:**

- Modify: `docs/development/testing.md`

Add:

- **Watchdog-revert tests** — pattern: build a coordinator with mock Reloadables, inject a failing
  `health_check`, assert atomic revert-all + audit row.
- **File-watch tempdir tests** — pattern: `tempfile::NamedTempFile` in a temp directory, attach a debouncer, write
  via atomic-rename, assert single `ReloadRequest` after 500 ms debounce.

- [ ] **Step 1: Add the two patterns**
- [ ] **Step 2: Prettier + markdownlint**
- [ ] **Step 3:** Commit — `docs(testing): watchdog-revert + file-watch tempdir patterns`

---

## Task 15: `docs/development/quality-gates.md` note

**Files:**

- Modify: `docs/development/quality-gates.md`

Append one line to the existing conditional tests block:

> - Reload-mechanism changes: mandatory `docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
&& cargo test -p uptrakit-integration-tests reexec -- --ignored`

- [ ] **Step 1: Add the line**
- [ ] **Step 2: Prettier + markdownlint**
- [ ] **Step 3:** Commit — `docs(quality-gates): Docker reload-integration test note`

---

## Task 16: Operator runbook `docs/end-user/operator-runbook-reload.md`

**Files:**

- Create: `docs/end-user/operator-runbook-reload.md`

Sections:

1. **Triggering reload** — SIGHUP, file edit + file-watch, DB mutation via Dashboard.
2. **Reading reload state** — `GET /api/v1/instance/config-state` + the Instance Configuration tab.
3. **Failure matrix** — copy spec §16 verbatim.
4. **Reexec semantics** — what changes (irreversibly-bound keys list), what happens (TCP-level reset on accepted
   connections is acceptable cost per spec §11.3 / §20).
5. **Recovery from Degraded state** — POST `clear-degraded` flow (gated by `ManageInstanceConfigState`).
6. **Recovery from a stuck reexec crash loop** — `--check-config` + systemd inspection + revert TOML.

- [ ] **Step 1: Write the runbook**
- [ ] **Step 2:** Prettier + markdownlint
- [ ] **Step 3:** Commit — `docs(runbook): operator runbook for graceful reload`

---

## Task 17: `README.md` Configuration section

**Files:**

- Modify: `README.md`

Add a `## Configuration` section explaining:

- Path: `/etc/uptrakit/controller.toml` (override `--config <path>` or `UPTRAKIT_CONFIG`).
- Surviving CLI flags: `--config`, `--master-key-from`, `--migrate-and-exit`, `--check-config`, `--version`, `--help`.
- Pointer to the operator runbook + spec.

- [ ] **Step 1: Add the section**
- [ ] **Step 2:** Prettier + markdownlint
- [ ] **Step 3:** Commit — `docs(readme): Configuration section`

---

## Task 18: CLI help text polish

**Files:**

- Modify: `crates/core/controller/src/main.rs` + `cli.rs`
- Modify: `crates/core/controller-standalone/src/main.rs`

Confirm each surviving flag has a one-sentence `#[arg(help = "...")]` description that matches the runbook.

- [ ] **Step 1: Update doc comments + arg helps**
- [ ] **Step 2:** Manual smoke: `cargo run --bin uptrakit-controller -- --help`
- [ ] **Step 3:** Commit — `docs(controller): polish CLI help text`

---

## Task 19: CHANGELOG entry

**Files:**

- Modify: `CHANGELOG.md` (or the repo's equivalent release-notes file — confirm location first)

Breaking changes block:

```markdown
## Unreleased

### Breaking changes — graceful reload

- CLI surface shrunk to `--config`, `--master-key-from`, `--migrate-and-exit`, `--check-config`, `--version`,
  `--help`. All other flags are removed without alias. Operators must produce a TOML file (`controller.toml`)
  before upgrading; see the runbook.
- `--reuseport` / `--takeover-from` / `SIGUSR1` graceful-restart path is removed. Reexec-style reload via SIGHUP +
  TOML edit replaces routine restarts; external load-balancing across two controllers covers accepted-connection
  preservation if required.
- `spawn_settings_reload` 30 s poll task is replaced by the 2 s `ConfigReconciler` task.
- The following `SettingKey` rows are dropped from `global_settings` at boot via migration
  `m20260512_000001_drop_file_keys`: HTTPS / PKI listen addrs, trusted proxies, real-IP / forwarded headers,
  zeroconf, NATS URL, global audit-log filter, global audit-log retention. Per-tenant audit-log rows are
  untouched.
- Reexec via `exec()` preserves listening sockets but resets accepted TCP connections — clients reconnect via
  their existing retry loops.
- All settings mutation endpoints now require `If-Match` (428 on missing, 409 on stale).

### Added

- `GET /api/v1/instance/config-state` (requires `ViewInstanceConfigState`).
- `POST /api/v1/instance/config-reload/clear-degraded` (requires `ManageInstanceConfigState`).
- "Instance Configuration" tab under Settings.
```

- [ ] **Step 1: Append the block**
- [ ] **Step 2:** Prettier + markdownlint
- [ ] **Step 3:** Commit — `chore(changelog): graceful reload release notes`

---

## Task 20: Final quality gates + release PR

- [ ] **Step 1:** Run the full Rust gate suite

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
cargo test --no-default-features --features db-sqlite
cargo test --all-features
```

- [ ] **Step 2:** Run the Docker reexec test

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
```

- [ ] **Step 3:** Run the frontend gates

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run test:e2e
npm run build
```

- [ ] **Step 4:** Run markdown gates

```bash
npx prettier --write docs/ README.md CONTEXT.md ARCHITECTURE.md CHANGELOG.md
npx markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 5:** Open the release PR titled
      `feat!: complete graceful reload (TOML + reload coordinator + reexec)`. Body must enumerate the four merged
      plan PRs and link the spec + ADR.

## Self-review

- Spec §15.5 Dashboard tab — Tasks 3, 4, 5 ✓
- Spec §18 doc deliverables — Tasks 9–17 ✓
- Spec §19 ADR — Task 9 ✓
- Spec §20 migration / breaking-change documentation — Task 19 ✓
- Spec §23 glossary — Task 10 ✓
- Spec §15 "No Reload Now button" — Task 7 verifies absence ✓
- Frontend snapshot rules: TypeScript strict, Prettier useTabs, ESLint unused-vars allowlist, Vitest + Playwright,
  semantic tokens via `frontend/src/theme/tokens.ts` ✓
- Markdown lint conformance: prettier-driven, line_length 150, MD024 siblings_only ✓
- Conventional Commits in every step ✓
- No manual edit of `.superpowers/standards-snapshot.md` (regenerated artifact) ✓
