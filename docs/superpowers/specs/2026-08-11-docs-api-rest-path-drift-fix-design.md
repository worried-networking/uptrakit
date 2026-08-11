# docs/api REST Path Drift Fix — Design

- **Date:** 2026-08-11
- **Status:** Draft (pending review)
- **Scope class:** Pure documentation change — no code, no OpenAPI, no frontend edits.
- **Origin:** Deferred list of the M1.9 docs audit
  ([2026-08-09-m19-docs-adr-closure-design.md](2026-08-09-m19-docs-adr-closure-design.md)); tracked in
  project memory as `project_docs_rest_path_drift`.

## Problem

The M1.9 docs-closure audit surfaced six pre-existing defects in `docs/api` — wrong or fictional REST
endpoint paths, a stale key-files inventory, and a reference to the removed `owner` role — and left
them unfixed as out of scope for that branch. Spec-time verification (2026-08-11) against
`crates/ui/web-api/openapi.json` and the code confirmed all six, and found the same defect classes in
neighboring pages the memory note does not list. Each defect now contradicts a sibling line the M1.9
branch corrected.

**Ground truth for every path claim:** `crates/ui/web-api/openapi.json`. For file-inventory claims:
`[ -e <path> ]` from the repo root. For role claims: `crates/shared/types/src/role_bundle.rs`.

## Corrections to the memory note

Two claims in `project_docs_rest_path_drift` (and the task prompt) are wrong; verified at spec time:

1. **`owner` is a live role _bundle_.** `RoleBundle::Owner` exists
   (`crates/shared/types/src/role_bundle.rs:43`, name string `"owner"` at `:63`). What migration
   `m20260310_000002_granular_permissions.rs` removed is the `owner` **roles-table row**. Therefore
   `docs/api/user-management.md:101` and `docs/api/access-management.md:153` (bundle lists including
   `owner`) are **correct and must not be edited**. The sole defect is the _role_ phrasing at
   `docs/api/settings-runtime.md:93`.
2. **The stale-file defect is larger than stated.** An `[ -e ]` test over every `` `crates/...` ``
   citation in `docs/api/settings-runtime.md` (2026-08-11) found **11 missing paths**, not the 4 the
   memory note lists: 9 in the "Key files" table plus `crates/core/controller/src/reconcile.rs`
   (`:42,174`) and `crates/core/controller/src/crl_manager.rs` cited elsewhere in the page — both
   moved to `crates/core/controller-runtime/src/`. Inventory below.

## Verified defect inventory

Line numbers are as of commit `776c1f6f4`; the implementation plan re-derives them at execution time.

### D1 — software-items batch actions row

`docs/api/http-web-api.md:624`: row for `POST /api/v1/software-items/batch` lists supported actions
as `delete` only. Code accepts `approve` and `delete`
(`crates/ui/web-api/src/routes/software_items/batch.rs:61,80,114`), both gated by the single
`CanDeleteSoftware` extractor (`batch.rs:46`; OpenAPI security `software:delete`, `batch.rs:40`).
`docs/api/batch-actions.md:52` already states `approve, delete` — the fix aligns `http-web-api.md`
with it. Action column stays `software:delete` (correct).

### D2 — NATS settings path

`docs/api/settings-runtime.md:23,399,411` cite `/settings/nats` / `GET|PUT /api/v1/settings/nats`.
Real path: `GET|PUT /api/v1/global-settings/nats` (openapi.json; handler
`crates/ui/web-api/src/routes/settings_nats.rs`). The M1.9 branch corrected the same claim in
`http-web-api.md:110` — these three lines now contradict it.

### D3 — fictional settings endpoints

`docs/api/http-web-api.md:104,105,107,1223`:

- `/api/v1/settings/network` → real path is `GET|PUT /api/v1/global-settings/network`
  (`routes/settings_network.rs:145,186`).
- `/api/v1/settings/mqtt`, `/api/v1/settings/mqtt/{id}` → **no REST surface exists.** `openapi.json`
  has no MQTT path; `crates/ui/web-api/src/routes/` has no MQTT module. MQTT client configuration
  moved to the MQTT service itself (`crates/core/mqtt-runtime`), enrolled and configured over the
  service WebSocket like any other service. Fix: delete the endpoint bullet at `:105` and the
  `GET /api/v1/settings/mqtt` row in the "Endpoints NOT paginated" table at `:1223`; where a pointer
  is useful, link to `services-operations.md` / `../end-user/home-assistant-mqtt.md` (relative to
  the editing file) instead of inventing a path.
- `/api/v1/settings/service-certificates` → real path is
  `GET|PUT /api/v1/settings/agent-certificates` (`routes/settings_agent_certs.rs:63,83`).

### D4 — fictional agent trigger endpoints

`docs/api/services-operations.md:27,28` cite `/api/v1/agents/{agent_id}/version-check` and
`/api/v1/agents/{agent_id}/execute-update` — the exact fictional paths M1.9 removed from
`http-web-api.md`. Real triggers (all `POST`, verified against openapi.json):

- `POST /api/v1/software-items/{id}/check-versions` — version check across all linked hosts.
- `POST /api/v1/software-items/{id}/hosts/{host_id}/check-versions` — single host.
- `POST /api/v1/software-items/{id}/hosts/{host_id}/update` — execute one update.
- `POST /api/v1/hosts/{host_id}/batch-update` — batch update for a host.

The surrounding prose ("instructs the controller to send `check_versions` over WebSocket") stays —
only the paths are wrong. The implementation verifies the reworded bullets against the handler docs
in `crates/ui/web-api/src/routes/software_items/version_check.rs` and `hosts.rs`.

### D5 — stale file citations in settings-runtime.md

An `[ -e ]` test over every `` `crates/...` `` citation in the page found 11 missing paths. In the
"Key files" table (`:549-564`), 9 rows:

| Cited (missing)                                     | Disposition                                                          |
| --------------------------------------------------- | -------------------------------------------------------------------- |
| `crates/shared/web-api-types/src/mqtt_transport.rs` | Delete row — `MqttTransport` now lives in `crates/core/mqtt-runtime` |
| `crates/shared/web-api-types/src/mqtt_url.rs`       | Delete row — same relocation                                         |
| `crates/shared/web-api-types/src/settings_mqtt.rs`  | Delete row — REST MQTT settings types removed                        |
| `crates/shared/db/src/entity/mqtt_client.rs`        | Delete row — entity removed                                          |
| `crates/shared/db/src/entity/mqtt_lease.rs`         | Delete row — entity removed                                          |
| `crates/ui/web-api/src/mqtt_client_store.rs`        | Delete row — store removed                                           |
| `crates/ui/web-api/src/mqtt_lease_coordinator.rs`   | Delete row — coordinator removed                                     |
| `crates/ui/web-api/src/routes/settings_mqtt.rs`     | Delete row — route removed                                           |
| `crates/ui/web-api/src/routes/services.rs`          | Repoint to `crates/ui/web-api/src/routes/services/` (module dir)     |

(9 rows: 8 deletions + 1 repoint.) Outside the table, two more citations repoint from
`crates/core/controller/src/` to `crates/core/controller-runtime/src/`: `reconcile.rs`
(`settings-runtime.md:42,174`) and `crl_manager.rs` (wherever the page cites it — the
implementation's own extraction finds the exact lines). The implementation re-runs the `[ -e ]`
extraction over the whole page as its gate, not just these rows. If deleting MQTT rows leaves the
surrounding section describing an MQTT REST config flow, prune that prose to match (the section's
WS/service-identity rows stay).

### D6 — removed `owner` role phrasing

`docs/api/settings-runtime.md:93`: "automatically promoted to the `owner` role". No such role exists
(removed in `m20260310_000002_granular_permissions.rs`); the first user is assigned all built-in
roles (`assign_owner_roles`, `crates/ui/web-api/src/routes/auth.rs:3052`). Fix mirrors the corrected
phrasing precedent at `docs/end-user/user-management.md:16`: "automatically receives all built-in
roles (the `owner` role bundle)".

### D7 — same-class drift outside the audited six (found by spec-time sweep)

Repo-wide `.md` sweep (`rg --no-ignore --hidden`, excluding `docs/superpowers/`) for each wrong-path
literal found:

- `docs/api/settings-runtime.md:20` — Service Certificates row: path `/settings/service-certificates`
  → `/settings/agent-certificates`; setting keys `service_certificates.*` → `agent_certificate.*`
  (`crates/ui/web-api-auth/src/setting_key.rs:143-144`). Row label follows the endpoint rename.
- `docs/development/openapi-client.md:65,263-271,609-615` — documents a `settings_mqtt.rs` client
  module with seven methods and seven mock-handler rows; `crates/shared/openapi-client/src/` has no
  such module (zero `mqtt` hits). Fix: delete the "Settings MQTT" section, the file-tree line, and
  the mock rows. No inbound `#settings-mqtt` anchors exist (verified by sweep).
- `docs/development/openapi-client.md:600,601` — mock-handler rows cite `/api/v1/settings/network`;
  the mock code itself registers the correct constant
  (`crates/shared/openapi-client/src/paths.rs:296` — `/api/v1/global-settings/network`). Fix the two
  doc rows; the stale rustdoc at `mock.rs:687,692` is deferred (code file).
- Seven end-user deployment guides cite `/api/v1/settings/network` (10 hits:
  `reverse-proxy.md` ×4, `caddy.md`, `envoy.md`, `haproxy.md`, `nginx-proxy-manager.md`, `nginx.md`,
  `traefik.md` ×1 each; full-repo `.md` total is 13 counting `http-web-api.md:104` (D3) and the two
  openapi-client.md rows above) → `/api/v1/global-settings/network`. UI breadcrumb references
  ("Settings > Network") are checked against the current frontend nav labels; if the label moved
  with the endpoint, update it, otherwise leave breadcrumbs untouched.

## Approach

**Recommended (chosen): repo-wide `.md` drift fix, sweep-driven.** Fix every verified wrong-path /
stale-file / dead-role reference in Markdown across the repo, with two exclusions:

- `docs/superpowers/**` — specs and plans are point-in-time historical records, never retro-edited.
- `CODEREVIEW.md` files (root and per-crate) — dated point-in-time review artifacts, same historical
  class (the only out-of-`docs/` hits of the sweep: `controller/src/crl_manager.rs` citations in
  `CODEREVIEW.md` and `crates/core/controller-runtime/CODEREVIEW.md`).
- `.claude/worktrees/**` — disposable copies.

Rationale: the task's goal is killing path drift; a docs/api-only fix leaves six deployment guides
directing users to 404 paths for the same defect. The sweep (not a hand-list) defines the final
deliverable set — hand-listing is how the M1.9 deferral under-counted D5 in the first place.

**Alternatives considered:**

- _Strict docs/api-only_ — matches the audit's literal deferred list; rejected: leaves live drift of
  the identical class in `docs/end-user/deployment/` and `docs/development/openapi-client.md`.
- _Include rustdoc fixes + regen-api_ — `crates/shared/web-api-types/src/settings_nats.rs:3,4,18,30`
  carries the same wrong path in doc comments; rejected here because those comments flow into
  `openapi.json` schema descriptions (verified: the schema description string
  ``"Response body for `GET /api/v1/settings/nats`"`` appears in openapi.json), so fixing them
  requires `./scripts/regen-api.sh` + committing
  `openapi.json` and the generated frontend client — violating this task's "no code/OpenAPI/frontend
  change" constraint. Deferred; see Out of scope.

## Verification gates (done-when)

All commands run from the repo root. Sweep flags per the drift-sweep lessons: `--no-ignore --hidden
-g '!.git/**'` so hidden dirs and locally-ignored files are covered.

1. **Wrong-literal sweep = 0 hits** over `-g '*.md' -g '!docs/superpowers/**' -g
'!.claude/worktrees/**' -g '!**/CODEREVIEW.md'` for each of:
   - `--pcre2 '(?<!global-)settings/nats'` — lookbehind because the _correct_ path contains
     `settings/nats` as a substring. Companion inverse gate (a lookbehind can blind the sweep):
     `rg -c -F 'global-settings/nats' docs/api/settings-runtime.md` ≥ 3.
   - `--pcre2 '(?<!global-)settings/network'` — same shape; companion:
     `rg -c -F 'global-settings/network'` over `docs/end-user/deployment/` equals the number of
     replacements made there (10 at spec time; the implementation plan re-derives and pins the
     count from its own pre-fix dry-run).
   - `-F 'settings/mqtt'`, `-F 'settings/service-certificates'`, `-F 'service_certificates.*'`,
     `-F 'agents/{agent_id}/version-check'`, `-F 'agents/{agent_id}/execute-update'`,
     `-F 'settings_mqtt'`, `-F 'mqtt_client_store'`, `-F 'mqtt_lease_coordinator'`,
     `-F 'entity/mqtt_lease'`, `-F 'entity/mqtt_client'`, `-F 'controller/src/reconcile.rs'`,
     `-F 'controller/src/crl_manager.rs'`, and ``-F 'promoted to the `owner` role'``.
     Every pattern above was dry-run at spec time and produced a non-zero pre-fix hit count on the
     current corpus (or, for the two `controller/src/` literals, was derived from the `[ -e ]`
     extraction), so the sweep is live, not vacuously green. The implementation plan re-runs the
     pre-fix dry-run and records each pattern's expected-before count next to its required-after
     count of zero.
2. **Path-extraction audit of edited docs/api pages**: extract every `` `/api/v1/...` `` literal from
   `http-web-api.md`, `settings-runtime.md`, `services-operations.md`, `batch-actions.md` and assert
   each (after normalizing `{param}` names to the openapi.json spelling) appears in
   `crates/ui/web-api/openapi.json` — or is explicitly documented as non-REST (WS upgrade path
   `/api/v1/ws/service`). The implementation plan turns this into a concrete one-shot script with a
   reviewed false-positive allowlist; zero unexplained misses is the gate.
3. **Key-files table**: every path in the `settings-runtime.md` "Key files" table passes `[ -e ]`.
4. **Lint/format**: `markdownlint --config .markdownlint.json` on every touched file;
   `npx prettier --write` on touched `.md` files (project feedback convention).
5. **Website tree caution**: touched files under `docs/end-user/` are website-symlinked. Edits here
   are inline path-string swaps only — no heading additions/deletions/renames, so no anchors move. If
   any heading edit becomes necessary, the plan must add the CI-pinned `zola check` (0.22.1) gate
   before trusting or bypassing a check failure.

## Deliverables

Doc deliverables ARE the change (pure-docs task). Enumerated set as of spec time — the
implementation's own sweep re-derives it and may only grow it:

- `docs/api/http-web-api.md` (D1, D3)
- `docs/api/settings-runtime.md` (D2, D5, D6, D7)
- `docs/api/services-operations.md` (D4)
- `docs/development/openapi-client.md` (D7)
- `docs/end-user/deployment/reverse-proxy.md`, `traefik.md`, `nginx-proxy-manager.md`, `caddy.md`,
  `nginx.md`, `envoy.md`, `haproxy.md` (D7; plus any files the implementation sweep adds)
- Post-merge memory update: mark `project_docs_rest_path_drift` resolved and correct its two wrong
  claims (owner-bundle nuance; 11-not-4 stale file citations).

No ADR: no architectural decision — documentation is being reconciled with decisions already made
and shipped. No README/CONTEXT/ARCHITECTURE impact: a spec-time whole-repo `.md` sweep outside
`docs/` (root files included, `--no-ignore --hidden`) found zero hits for every wrong literal except
the historical `CODEREVIEW.md` citations excluded above.

## Out of scope / deferred

- Stale rustdoc in code files (constraint: this task is docs-only). Candidate follow-up task:
  - `crates/shared/web-api-types/src/settings_nats.rs:3,4,18,30` citing `/api/v1/settings/nats` —
    feeds openapi.json descriptions, so the fix needs `./scripts/regen-api.sh` + committing
    `openapi.json` and the generated frontend client.
  - `crates/shared/openapi-client/src/mock.rs:687,692` citing `/api/v1/settings/network` — internal
    rustdoc only (no OpenAPI flow), but still a code-file edit.
- `docs/hackme/*` permission-era prose (`"arbitrary permissions"`, forged-JWT `owner` role
  narratives) and `docs/development/testing.md:360` ("the default `owner` role") — stale _wording_,
  not path drift; belongs to a permission-prose sweep, not this fix. The final sweep patterns here
  deliberately do not match them.
- `docs/superpowers/**` historical specs/plans — never retro-edited.
- The six docs/api REST defects' cousins in `openapi-client` _code_ — none found (the crate has no
  MQTT module; its docs were the stale half).

## Standards conformance

- Snapshot rule "Document every change in code or docs" — this task is that documentation.
- Snapshot rule "no hardcoded counts" (AGENTS.md scope) — the edited pages keep counts out of prose;
  fix text points at sources instead.
- markdownlint gate + no `.markdownlintignore` additions.
- No wire/OpenAPI/asyncapi regeneration required or permitted (nothing under `crates/` changes).
- Ledger rows applied: 5 (lookbehind + inverse gate), 9 (every universal claim above carries a
  file:line or command citation), 10 (whole-repo sweep incl. hidden dirs; literal path strings, not
  concepts), 12 (deliverables sweep-derived, not hand-listed), 50 (`--no-ignore --hidden` pinned in
  the gate commands).
