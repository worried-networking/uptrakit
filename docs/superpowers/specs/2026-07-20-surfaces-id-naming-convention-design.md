# Surfaces ID Naming Convention + Rename Sweep

**Date:** 2026-07-20
**Status:** Approved (user interview 2026-07-20; supersedes the rename table in Deferred §5 of
[`2026-07-16-surfaces-dataload-get-typing-design.md`](2026-07-16-surfaces-dataload-get-typing-design.md) — that
table predates the HTTP method model and kept CRUD-verb IDs plus dual-dispatch aliases, both superseded here)
**Scope:** `crates/plugins/infrastructure/proxmox`, `crates/plugins/releases/docker`,
`crates/plugins/notifications/{core,email,telegram,webhook}`, `crates/core/agent-ssh-runtime`,
`crates/core/mqtt-runtime`, `crates/plugins/infrastructure/registry` (guard test home), `crates/ui/cli` (help
text), `frontend/tests/e2e/`, docs.
**Sequencing:** Depends on
[`2026-07-20-surfaces-rest-method-model-design.md`](2026-07-20-surfaces-rest-method-model-design.md) (method
model, `(id, method)` registry key, item path segment) landing first. Intent coupling: the pending agent-side
surface-task-timeout spec also edits `agent-ssh-runtime/src/surface_runtime.rs`.

## Problem

Dynamic surface contract identifiers drifted along three axes because the shared charset validator
(`validate_surface_identifier`, `crates/shared/surfaces/src/ids.rs` — first char `[a-z]`, rest `[a-z0-9._-]`)
permits every style at once:

1. **kebab-case vs snake_case:** notifications use `configure_smtp`, `save_global_smtp`, surface IDs
   `notifications.email.global_smtp`; every other provider uses kebab-case.
2. **CRUD verbs in IDs:** `list`, `get-info`, `get_smtp`, `preload-*`, `load-*`, `create`/`edit`/`delete`,
   `mqtt.create-client`, `remove-host` — the HTTP method (companion spec) now carries these semantics.
3. **Namespace prefixes:** MQTT prefixes every interaction (`mqtt.list-clients`) and data source
   (`mqtt.clients.primary`); proxmox prefixes data sources (`proxmox.hosts.mappings`); everyone else uses bare
   IDs scoped by the surface.

The existing rule in `docs/development/surfaces.md` (data-retrieval = noun phrases, mutations = verbs) is
document-only and widely violated — proof that the convention needs an executable guard.

## Decisions (settled — do not reopen)

| #   | Decision            | Resolution                                                                                                                                                                                                                                  |
| --- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C1  | Convention          | See §Convention below. Kebab-case; CRUD = plural noun + HTTP method; singletons = singular noun + GET/PUT; domain operations = verb phrase + POST.                                                                                          |
| C2  | Skew policy         | No transitional dual-registration aliases. Renames land atomically per binary (user decision 2026-07-20, overriding the absorbed spec's alias plan).                                                                                        |
| C3  | Enforcement         | Guard tests over first-party registrations only. `validate_surface_identifier` stays permissive — old service binaries must not be rejected at admission.                                                                                   |
| C4  | Slot IDs            | Out of scope (`settings.tabs`, `host_detail.tabs`, …) — shared contract with a larger blast radius; unchanged.                                                                                                                              |
| C5  | Permission literals | Notifications/MQTT raw permission strings switch to the `Permission` enum via `.to_string()`. Retyping descriptor fields `Option<String>` → `Option<Permission>` stays deferred (registered follow-up of the interaction-unification spec). |

## Convention (normative — this text lands in `docs/development/surfaces.md`)

- **Interaction IDs and data-source IDs:** kebab-case only — `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`. No underscores, no
  dots, no provider/surface prefixes (the surface ID already namespaces).
- **Surface IDs:** dot-separated kebab-case segments — each segment matches the interaction regex. First segment
  names the provider family (`proxmox`, `notifications.email`, `mqtt`, `ssh-agent`).
- **CRUD over a collection:** one plural noun registered under multiple HTTP methods — GET (list / item read via
  `/{item_id}`), POST (create), PUT `/{item_id}` (replace), DELETE `/{item_id}`. Never `list-`, `get-`,
  `create-`, `edit-`, `delete-`, `remove-`, `preload-`, `load-`, `save-` prefixes.
- **Singleton resources** (settings blobs with no collection): singular/uncounted noun under GET + PUT
  (`smtp`, `global-defaults`, `overrides`).
- **Domain operations** (not CRUD): imperative verb phrase under POST (`test-connection`, `discover`, `match`,
  `bootstrap`, `switch-tag`, `sync`). Bare verbs fine; no nouns the surface already implies.
- **Data sources** pair with their GET interaction: `DataSourceKind::ProviderQuery.operation_id` equals the
  paired GET interaction ID, and the data-source ID uses the same noun (`mappings` ↔ GET `mappings`).
- **Workflow step-submit IDs** follow the domain-operation rule (`sync-connect`, `bootstrap-execute` — already
  compliant).

## Rename table (exhaustive for first-party providers)

Registration sites: proxmox `crates/plugins/infrastructure/proxmox/src/plugin.rs` (+ dispatch in
`src/surfaces.rs`), docker `crates/plugins/releases/docker/src/plugin.rs`, notifications
`crates/plugins/notifications/{email,telegram,webhook}/src/plugin.rs` (+ shared
`notifications/core/src/list_channels.rs`), agent-ssh `crates/core/agent-ssh-runtime/src/surface_runtime.rs`,
MQTT `crates/core/mqtt-runtime/src/surface_runtime.rs`. The plan must re-grep every old ID workspace-wide
(including `#[cfg(test)]` code, `frontend/tests/e2e/` route-matcher regexes, CLI help text at
`crates/ui/cli/src/commands/surfaces.rs`, and all `*.md` repo-wide including top-level docs) — this table names
the contracts, not every reference site.

| Surface                                          | Current ID                                                                             | New: method + ID                                                                  |
| ------------------------------------------------ | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `ssh-agent.hosts`                                | `list-hosts`                                                                           | GET `hosts`                                                                       |
| `ssh-agent.hosts`                                | `remove-host`                                                                          | DELETE `hosts/{id}`                                                               |
| `ssh-agent.hosts`                                | `sync-host`                                                                            | POST `sync` (workflow)                                                            |
| `ssh-agent.hosts`                                | `bootstrap`, `sync-connect`, `sync-execute`, `bootstrap-connect`, `bootstrap-execute`  | unchanged (compliant)                                                             |
| `ssh-agent.hosts`                                | DS `data.primary` (op `list-hosts`)                                                    | DS `hosts` (op `hosts`)                                                           |
| `proxmox.hosts`                                  | `list`                                                                                 | GET `mappings`                                                                    |
| `proxmox.hosts`                                  | `discover`, `test-connection`, `approve-match`, `match`, `unmatch`, `unmatched-guests` | unchanged (POST verbs; `unmatched-guests` GET noun)                               |
| `proxmox.hosts`                                  | DS `proxmox.hosts.mappings` (op `list`)                                                | DS `mappings` (op `mappings`)                                                     |
| `proxmox.host-info`                              | `get-info`                                                                             | GET `info`                                                                        |
| `proxmox.host-info`                              | DS `proxmox.host-info.primary` (op `get-info`)                                         | DS `info` (op `info`)                                                             |
| `proxmox.settings.update-hooks`                  | `save-global-defaults` / `preload-global-defaults`                                     | PUT `global-defaults` / GET `global-defaults`                                     |
| `proxmox.settings.update-hooks`                  | `load-backup-target-options`                                                           | GET `backup-target-options`                                                       |
| `proxmox.settings.resource-scaling`              | `save-scaling-global-defaults` / `preload-scaling-global-defaults`                     | PUT `global-defaults` / GET `global-defaults`                                     |
| `proxmox.software-item.update-hooks`             | `save-item-overrides` / `preload-item-overrides`                                       | PUT `overrides` / GET `overrides`                                                 |
| `proxmox.software-item.update-hooks`             | `load-backup-target-options`                                                           | GET `backup-target-options`                                                       |
| `proxmox.software-item.resource-scaling`         | `save-scaling-item-overrides` / `preload-scaling-item-overrides`                       | PUT `overrides` / GET `overrides`                                                 |
| `docker.item-host-actions`                       | `get-current-tag`                                                                      | GET `current-tag` (update `pre_load_interaction_id` on `switch-tag`)              |
| `docker.item-host-actions`                       | `switch-tag`                                                                           | unchanged (POST verb)                                                             |
| `notifications.{email,telegram,webhook}`         | `list` / `create` / `edit` / `delete`                                                  | GET `channels` / POST `channels` / PUT `channels/{id}` / DELETE `channels/{id}`   |
| `notifications.{email,telegram,webhook}`         | `test`                                                                                 | POST `test` (unchanged; channel id stays a param)                                 |
| `notifications.{email,telegram,webhook}`         | DS `data.primary` (op `list`)                                                          | DS `channels` (op `channels`)                                                     |
| `notifications.email`                            | `configure_smtp` / `get_smtp`                                                          | PUT `smtp` / GET `smtp`                                                           |
| surface `notifications.email.global_smtp`        | —                                                                                      | surface renamed `notifications.email.global-smtp`                                 |
| `notifications.email.global-smtp`                | `save_global_smtp` / `get_global_smtp` / `test_global_smtp_email`                      | PUT `smtp` / GET `smtp` / POST `test`                                             |
| surface `notifications.telegram.global_settings` | —                                                                                      | surface renamed `notifications.telegram.global-settings`                          |
| `notifications.telegram.global-settings`         | `save_global_telegram` / `get_global_telegram`                                         | PUT `settings` / GET `settings`                                                   |
| `mqtt.clients`                                   | `mqtt.list-clients` / `mqtt.get-client`                                                | GET `clients` / GET `clients/{id}`                                                |
| `mqtt.clients`                                   | `mqtt.create-client` / `mqtt.edit-client` / `mqtt.delete-client`                       | POST `clients` / PUT `clients/{id}` / DELETE `clients/{id}`                       |
| `mqtt.clients`                                   | DS `mqtt.clients.primary` (op `mqtt.list-clients`)                                     | DS `clients` (op `clients`)                                                       |
| agent-side (`ssh-agent.hosts` injected)          | `bootstrap-proxmox-guest`, `discovered-guests`                                         | unchanged (compliant; also wire-referenced by agent binaries — see Compatibility) |

Resolved in-table ambiguity: notifications `test` stays a bare POST domain operation taking the channel id as a
param (it is not an item CRUD; the ID it tests rides `params` exactly as today).

## Compatibility (honest statement, C2)

No transitional aliases. Analysis of actual cross-binary references:

- Every renamed ID is **provider-local**: registered and dispatched by the same binary (plugins live in the
  controller; `ssh-agent.hosts` IDs are registered and matched inside `agent-ssh-runtime`; `mqtt.clients` IDs
  inside `mqtt-runtime`). The frontend and CLI are fully data-driven — no hardcoded interaction IDs outside one
  CLI help-text example and e2e mocks, both swept here.
- The only IDs hardcoded across a binary boundary (`match`, `unmatched-guests`, `bootstrap-proxmox-guest`,
  `discovered-guests` — invoked by agent-ssh binaries against controller-side proxmox interactions, or
  vice-versa) are **unchanged by this spec**.
- Old service binaries (pre-rename mqtt/agent-ssh) remain self-consistent against a new controller: they
  register their old IDs, the data-driven frontend invokes what is registered, and the permissive wire validator
  (C3) admits them. Their old snake-free IDs simply don't match the new convention until the binary updates —
  cosmetic, not functional.
- Policy if the plan-time grep finds a missed cross-binary reference: rename anyway; accepted breakage (user
  decision). Do **not** justify with a lockstep-release claim — agent binaries ship separately and skew is the
  steady state; state the breakage and the affected version combinations explicitly in the plan.
- **New service → OLD controller (from the companion spec, restated):** the collapsed multi-method registrations
  this spec introduces for mqtt/agent-ssh hard-fail registration on a pre-method-model controller (old id-only
  uniqueness check rejects the duplicate ID; `ActionRef` object form fails deserialization). Loud failure, not
  silent. Controller upgrades first; the release notes for the mqtt/agent-ssh binaries carrying these renames
  must state the minimum controller version.

Interaction IDs are not persisted (no DB columns, no MQTT topics — verified 2026-07-16 in the absorbed spec;
re-verify by grep at plan time). Audit rows contain historical IDs as data — fine.

## Enforcement (C3)

1. **Catalog guard test** (in `crates/plugins/infrastructure/registry`, where `all_descriptors()` lives):
   iterate every `PluginSurfaceRegistration` from the built catalog and assert, per surface: surface-ID regex;
   interaction-ID + data-source-ID regex (no `_`, no `.`); `ProviderQuery.operation_id` equals a registered GET
   interaction ID; DS ID equals that interaction ID. Feature honesty (mandatory): the test `#[cfg]`-requires the
   feature world that includes notifications + proxmox registrations, and **asserts the expected providers are
   present before asserting they are clean** (green-on-empty forbidden). Name the exact command that exercises
   it (expected: `cargo test --all-features`; the plan must verify which feature set populates the catalog and
   whether resolver-3 unification affects a scoped `-p` run — do not trust `cfg!` in a foreign crate).
2. **Service-runtime guard tests**: sibling unit tests in `agent-ssh-runtime` and `mqtt-runtime` over their own
   registration builders (same assertions, no feature-unification exposure).
3. **Doc-only rule for third parties:** external/service providers get the convention as normative guidance in
   `docs/development/surfaces.md`; the wire validator is unchanged.

The semantic half of the rule (noun vs verb) is not machine-checkable; the guard enforces charset/style and the
DS/operation pairing, the doc carries the semantics, and review enforces the rest.

## Testing

- Guard tests above (RED demonstration: perturb a **value** to a charset-valid-but-nonconforming ID, e.g. an
  underscore ID — never by deleting a registration, which trips dead-code deny before the assertion runs).
- Every provider's existing surface-contract tests re-pointed at the new IDs — the plan must enumerate every
  referencing test workspace-wide per renamed symbol/ID (runtime `.expect()` field reads included, not just
  compile breaks), including consumers in other crates (`surface-proxy` bootstrap tests, web-api
  provider-origin e2e tests).
- One invocation test per renamed CRUD family driving the new method + item segment through the real dispatch
  path (production caller shape: `target_provider_id: None`).
- e2e Playwright mocks updated and — if frontend routing is touched — `npm run test:e2e:parity` run locally.

Verification: canonical gates (`cargo test --all-features` with `frontend/build/` present;
`cargo clippy --all-targets --no-default-features --features db-sqlite`); repo-wide staleness grep for every old
ID over `--include='*.md'` (top-level docs included, plan/spec history excluded) plus source, asserting zero
hits outside history; presence grep asserting every new ID registers (survivor strings non-zero).

## Deliverables

**Code:** renames per the table (registration builders, dispatch match arms, interaction constants, DS
descriptors, `pre_load_interaction_id` references); notifications + MQTT permission literals → `Permission`
enum (C5); CLI help-text example; e2e mock matchers; guard tests (catalog + two service runtimes).

**Docs (non-optional):**

- `docs/development/surfaces.md` — replace the current naming paragraph with the §Convention text (normative).
- New ADR `docs/adr/0031-surface-identifier-naming.md` (verify next-free number at implementation time; 0030 is
  the companion spec's) — convention, C1–C5, no-alias skew rationale.
- `docs/api/surfaces.md` — example IDs updated to the new convention.
- `frontend/AGENTS.md` — owned by the companion method-model spec (stale `surfaces.ts` carve-out replaced there
  with the DataLoad query-wrapper escape hatch); this spec only verifies no renamed ID appears in it.
- Any doc page naming a renamed ID (found by the repo-wide grep above).
- `CONTEXT.md`: no change — no new domain vocabulary.

## Deferred (named follow-ups, out of scope)

1. Descriptor/gate permission typing `Option<String>` → `Option<Permission>` (registered deferral of the
   interaction-unification spec; C5 only fixes the literals).
2. Slot ID normalization (C4).
3. Deleting the notifications surface-CRUD duplicate path in favor of the existing
   `/api/v1/notifications/channels` REST family (needs either first-class settings UI or a built-in-API
   transport revival; blocked on UI-redesign rollout).
4. Tightening `validate_surface_identifier` for newly-registered contracts (needs a deprecation window for
   service providers).

## Open questions

None.
