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

**Ground truth for every path claim:** the **union** of (a) the parsed `paths` keys of
`crates/ui/web-api/openapi.json` (via a JSON parser — **never a grep against the file**: its schema
_descriptions_ carry the wrong literal `/api/v1/settings/nats` twice via the deferred
`settings_nats.rs` rustdoc, so a grep self-certifies the exact drift D2 deletes) and (b) the
`"/api/v1/..."` route literals registered in `crates/ui/web-api/src/router.rs` — the router carries
real non-OpenAPI routes (`/api/v1/events/stream` SSE, `/api/v1/pki/{ca.crt,ca.crl,ocsp,ocsp/{encoded}}`,
`/api/v1/notifications/callback/{channel_type}/{channel_id}`,
`/api/v1/update-history/{id}/interactive` behind `feature = "interactive"`) that a
openapi.json-only universe would flag as fictional. For file-inventory claims: `[ -e <path> ]` from
the repo root. For role claims: `crates/shared/types/src/role_bundle.rs`.

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
`http-web-api.md:109` — these three lines now contradict it. The same table's Network row
(`settings-runtime.md:17`) has the identical defect: path cell `/settings/network` →
`/global-settings/network` (its `network.*` key column is correct —
`crates/ui/web-api-auth/src/setting_key.rs:17-20`).

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
"Key files" table (`:549-569`), 9 rows:

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

(9 rows: 8 deletions + 1 repoint.) Outside the table, more citations repoint from
`crates/core/controller/src/` to `crates/core/controller-runtime/src/`: `reconcile.rs`
(`settings-runtime.md:42,174`), `crl_manager.rs` wherever `settings-runtime.md` cites it, and
`crl_manager.rs` in `docs/hackme/15-certificate-revocation-bypass.md:112` — the hackme pages are
living docs ("tracks both existing mitigations and implemented fixes", `docs/hackme/README.md`),
not historical records, so the sweep correctly forces this one. The implementation re-runs the
`[ -e ]` extraction over the whole of `settings-runtime.md` as its gate, not just these rows. If deleting MQTT rows leaves the
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

### D8 — additional drift found by the review's discovery probe (same classes)

A path-extraction probe over the docs tree against the corrected ground-truth union (review round,
2026-08-11) found four more wrong-path families the literal sweep alone could not discover — proof
that the D-inventory must be _derived_ by a discovery pass, not enumerated by hand:

- `POST /api/v1/settings/rotate-ca` → real `POST /api/v1/global-settings/ca/rotate` (openapi.json).
  Five hits: `docs/security/pki-certificates.md:23,147`, `docs/security/reverse-proxy-security.md:67`,
  `docs/end-user/deployment/reverse-proxy.md:110`, `docs/development/openapi-client.md:602` (the mock
  row directly adjacent to the two the D7 network fix edits).
- `POST /api/v1/auth/token` (`docs/api/http-web-api.md:82`) → real `POST /api/v1/oauth/token`
  (`router.rs:519`; openapi.json). Sweep literal must be the full `-F '/api/v1/auth/token'` — the
  real path ends in `auth/token`, so a bare substring pattern would false-positive on it.
- `/api/v1/software-ignores` → real `/api/v1/autodiscovery/ignores{,/{id},/batch}` (openapi.json).
  Hits: `docs/api/http-web-api.md:169,1217` and three **headings** in
  `docs/api/autodiscovery.md:92,141,189` — heading renames move anchors; verified no inbound
  `autodiscovery.md#...software-ignores...` anchors exist in the repo, and the implementation
  re-runs that inbound-anchor check after renaming.
- `/api/v1/services/enrollment-token` (`docs/api/auth-flows.md:105`) → real
  `/api/v1/enrollment-tokens` family (openapi.json).

## Approach

**Recommended (chosen): repo-wide `.md` drift fix, discovery-pass-first.** Fix every verified
wrong-path / stale-file / dead-role reference in Markdown across the repo.

**Order of operations (inverted from a literal-sweep-first draft after review):**

1. **Discovery pass** — extract every `` `/api/v1/...` `` literal from all non-excluded `.md` files
   and check each against the ground-truth union (parsed openapi.json `paths` keys ∪ `router.rs`
   route literals). **Within `docs/api/**` additionally** extract backticked prefix-less path cells
   (`` `/settings/...` `` etc.) and prepend `/api/v1` before matching — the D2/D3/D7 table-cell
   defects (`settings-runtime.md:17,20,23`) carry no `/api/v1/` prefix, so a prefix-anchored
   extraction is blind to exactly the founding defect form; outside `docs/api` the prefix-less form
   is pure filesystem-path noise (`/etc/machine-id`, `/usr/bin/npm` — review-verified). Cost: one
   allowlist entry (`/readyz`). Hand-triage every miss into _fix_ (with verified replacement) or
   _allowlist-with-reason_ (e.g. deliberately documented removed endpoints such as
   `GET /api/v1/permissions` described as removed). This pass — not a hand-list — freezes the final
   defect inventory and deliverable set; D1–D8 are its verified minimum, and a review-round run of
   both extraction forms over `docs/` found **no fifth family** — the inventory is a measured bound,
   not a hope.
2. **Fix pass** — apply the triaged corrections.
3. **Regression gates** — the wrong-literal sweep (gate 1) and the re-run extraction (gate 2) prove
   the triaged set landed; the literal sweep is a regression check for known drift, never the
   scoping tool (a literal sweep cannot discover unknown drift — that failure mode produced D8).

**Artifact persistence (condition of the CI-gate deferral):** the implementation plan must embed
the discovery script **verbatim** and the full triage table (every miss → fix / allowlist + reason)
in the plan document itself. A plan file is a doc, so this stays inside the pure-docs constraint —
and it is what makes the deferred `ci/verify_doc_api_paths.py` follow-up genuinely
"copy into `ci/` + wiring + sandbox test" instead of a full re-derivation. Without persistence the
deferral's cheap-follow-up premise is false.

Exclusions, stated as a rule rather than a bare list: **dated point-in-time records are never
retro-edited** — `docs/superpowers/**` (specs/plans), `**/CODEREVIEW.md` (dated reviews),
`docs/adr/**` (decision records; e.g. ADR-0039's `GET /api/v1/access-presets` describes the
then-current state and stays), `.claude/worktrees/**` (disposable copies), `.superpowers/**`
(generated tracker/snapshot files that legitimately quote the wrong literals as defect
descriptions). Docs describing **current** behavior (`docs/api`, `docs/development`,
`docs/end-user`, `docs/security`, `docs/hackme` — the hackme pages track implemented mitigations
per `docs/hackme/README.md`) are always in scope. (The only out-of-`docs/` hits of the spec-time
sweep were `controller/src/crl_manager.rs` citations in the root and `controller-runtime`
`CODEREVIEW.md` files — excluded by the rule.)

Rationale: the task's goal is killing path drift; a docs/api-only fix leaves seven deployment guides
directing users to 404 paths for the same defect. The discovery pass (not a hand-list) defines the
final deliverable set — hand-listing is how the M1.9 deferral under-counted D5, and how this spec's
own pre-review draft missed all of D8.

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

1. **Wrong-literal sweep = 0 hits** over `-g '*.md' -g '!docs/superpowers/**' -g '!docs/adr/**' -g
'!.claude/worktrees/**' -g '!**/CODEREVIEW.md' -g '!.superpowers/**' -g '!target/**' -g
'!frontend/node_modules/**' -g '!frontend/build/**' -g '!frontend/.svelte-kit/**'` for each of
   (exclusions beyond the historical ones: `.superpowers/` holds generated tracker/snapshot files
   that legitimately quote the wrong literals; `target/` and the frontend vendor/build trees are
   `--no-ignore` collateral — vendored `.md` noise plus reproducible rg exit-2 IO races when a
   concurrent build is writing `target/`). Pass/fail reading — applies to the wrong-literal
   patterns only, NOT to the two companion count gates below (those pass/fail by printed value,
   not exit code: a companion's rg exiting 1 means zero matches, which FAILS its threshold): rg
   exits **1** on zero matches — for the wrong-literal patterns, exit 1 with no output is PASS
   and exit 0 (any hit) is FAIL; a gate script must not run these under `set -e` semantics that
   treat exit 1 as tooling failure, and rg exit ≥ 2 is a tooling error, never a pass:
   - `--pcre2 '(?<!global-)settings/nats'` — lookbehind because the _correct_ path contains
     `settings/nats` as a substring. Companion inverse gate (a lookbehind can blind the sweep;
     the awk summation makes zero matches print `0` instead of empty — bare `rg -c` prints
     nothing on a zero-match file, breaking numeric comparison):
     `rg -c -F 'global-settings/nats' docs/api/settings-runtime.md | awk -F: '{s+=$NF} END {print s+0}'` ≥ 3.
   - `--pcre2 '(?<!global-)settings/network'` — same shape; companion (rg `-c` on a directory
     prints per-file `path:count` lines, so the comparison needs explicit summation):
     `rg -c -F 'global-settings/network' docs/end-user/deployment/ | awk -F: '{s+=$2} END {print s+0}'`
     equals the number of replacements made there (10 at spec time; the implementation plan
     re-derives and pins the count from its own pre-fix dry-run).
   - `-F 'settings/mqtt'`, `-F 'settings/service-certificates'`, `-F 'service_certificates.*'`,
     `-F 'agents/{agent_id}/version-check'`, `-F 'agents/{agent_id}/execute-update'`,
     `-F 'settings_mqtt'`, `-F 'mqtt_client_store'`, `-F 'mqtt_lease_coordinator'`,
     `-F 'entity/mqtt_lease'`, `-F 'entity/mqtt_client'`, `-F 'controller/src/reconcile.rs'`,
     `-F 'controller/src/crl_manager.rs'`, and ``-F 'promoted to the `owner` role'``.
   - D8 literals: `-F 'settings/rotate-ca'`, `-F '/api/v1/auth/token'` (full form — the real
     `/api/v1/oauth/token` ends in `auth/token`, so a bare substring would false-positive),
     `-F '/api/v1/software-ignores'` (path-anchored — the bare name false-positives on the live
     CLI subcommand `uptrakit software-ignores` in `docs/end-user/{cli-usage,plugin-configs}.md`
     and `TODO.md`, which is correct usage), `-F 'services/enrollment-token'`.
     All four dry-run verified pre-fix: 5, 1, 5 (2 in `http-web-api.md` + 3 in `autodiscovery.md`),
     1 hits respectively; the anchored software-ignores pattern hits exactly the defect set.
     Every pattern above was dry-run at spec time and produced a non-zero pre-fix hit count on the
     current corpus (or, for the two `controller/src/` literals, was derived from the `[ -e ]`
     extraction), so the sweep is live, not vacuously green. The implementation plan re-runs the
     pre-fix dry-run and records each pattern's expected-before count next to its required-after
     count of zero.
2. **Path-extraction audit (discovery pass re-run)**: extract every `` `/api/v1/...` `` literal from
   **all non-excluded `.md` files** (same exclusion set as gate 1), plus the prefix-less backticked
   path cells within `docs/api/**` per Approach step 1, and assert each (after normalizing
   `{param}` names) appears in the ground-truth union — parsed openapi.json `paths` keys (JSON
   parser, never grep; see Ground truth) ∪ `router.rs` route literals — or in the reviewed
   allowlist (each entry with a reason: WS upgrade path `/api/v1/ws/service`, `/readyz`, endpoints
   a doc deliberately describes as removed, etc.). The implementation plan embeds this script and
   the triage table verbatim (see Approach — artifact persistence); zero unexplained misses is the
   gate. The same script runs twice: pre-fix as the discovery pass that freezes the inventory,
   post-fix as this gate.
3. **Key-files table**: every path in the `settings-runtime.md` "Key files" table passes `[ -e ]`.
4. **Lint/format**: `markdownlint --config .markdownlint.json` on every touched file;
   `npx prettier --write` on touched `.md` files (project feedback convention).
5. **Website tree caution**: touched files under `docs/end-user/` and `docs/security/` are
   website-symlinked; edits there are inline path-string swaps only. `docs/api/autodiscovery.md`
   (not website-symlinked) gets three heading renames (D8) — anchors move, so the implementation
   re-runs the inbound-anchor check (`rg 'autodiscovery\.md#'` over the docs tree) after renaming;
   spec-time check found no inbound links to the renamed headings. If any heading edit lands in a
   website-symlinked tree after all, the plan must add the CI-pinned `zola check` (0.22.1) gate
   before trusting or bypassing a check failure.

## Deliverables

Doc deliverables ARE the change (pure-docs task). Enumerated set as of spec time — the
implementation's own sweep re-derives it and may only grow it:

- `docs/api/http-web-api.md` (D1, D3, D8)
- `docs/api/settings-runtime.md` (D2, D5, D6, D7)
- `docs/api/services-operations.md` (D4)
- `docs/api/autodiscovery.md` (D8 — three heading renames)
- `docs/api/auth-flows.md` (D8)
- `docs/development/openapi-client.md` (D7, D8)
- `docs/security/pki-certificates.md`, `docs/security/reverse-proxy-security.md` (D8)
- `docs/hackme/15-certificate-revocation-bypass.md` (D5 — `crl_manager.rs` repoint; living doc,
  not a historical record)
- `docs/end-user/deployment/reverse-proxy.md`, `traefik.md`, `nginx-proxy-manager.md`, `caddy.md`,
  `nginx.md`, `envoy.md`, `haproxy.md` (D7; `reverse-proxy.md` also D8; plus any files the
  discovery pass adds)
- Post-merge memory update: mark `project_docs_rest_path_drift` resolved and correct its two wrong
  claims (owner-bundle nuance; 11-not-4 stale file citations).

No ADR: no architectural decision — documentation is being reconciled with decisions already made
and shipped. No README/CONTEXT/ARCHITECTURE impact: a spec-time whole-repo `.md` sweep outside
`docs/` (root files included, `--no-ignore --hidden`) found zero hits for every wrong literal except
the historical `CODEREVIEW.md` citations and the generated `.superpowers/` tracker/snapshot files —
both excluded above. The one sweep-forced file outside `docs/api`/`docs/development`/`docs/end-user`
is `docs/hackme/15-certificate-revocation-bypass.md:112`, enumerated in Deliverables (see D5).

## Out of scope / deferred

- Stale rustdoc in code files (constraint: this task is docs-only). Candidate follow-up task:
  - `crates/shared/web-api-types/src/settings_nats.rs:3,4,18,30` citing `/api/v1/settings/nats` —
    feeds openapi.json descriptions, so the fix needs `./scripts/regen-api.sh` + committing
    `openapi.json` and the generated frontend client. Until it lands, the **rendered** OpenAPI
    docs (the artifact API consumers actually read) still show the wrong path in two schema
    descriptions — this defer is user-visible, not merely a code comment.
  - `crates/shared/openapi-client/src/mock.rs:687,692` citing `/api/v1/settings/network` — internal
    rustdoc only (no OpenAPI flow), but still a code-file edit.
- **Durable CI gate** (`ci/verify_doc_api_paths.py` + reviewed allowlist + sandbox test, per the
  repo's existing `ci/verify_*` + allowlist pattern): the discovery-pass script this spec mandates
  is that gate minus the CI wiring, and the plan persists the script + triage table verbatim (see
  Approach — artifact persistence), so the follow-up is copy into `ci/` + wiring + sandbox test.
  Landing it durably is deliberately deferred — a code/CI deliverable outside this task's "pure
  docs" constraint, same follow-up class as M1.9's deferred permission greps. Without it, the next
  route rename re-seeds this drift.
- Stale REST paths inside `docs/adr/**` (e.g. ADR-0039's `GET /api/v1/access-presets`) — ADRs are
  dated decision records, never retro-edited; excluded by the boundary rule stated in Approach.
- `docs/hackme/*` permission-era prose (`"arbitrary permissions"`, forged-JWT `owner` role
  narratives) and `docs/development/testing.md:360` ("the default `owner` role") — stale _wording_,
  not path drift; belongs to a permission-prose sweep, not this fix. The final sweep patterns here
  deliberately do not match them.
- `docs/superpowers/**` historical specs/plans — never retro-edited.
- The six docs/api REST defects' cousins in `openapi-client` _code_ — none found (the crate has no
  MQTT module; its docs were the stale half).

## Standards conformance

- Snapshot rule "Document every change in code or docs" — this task is that documentation.
- markdownlint gate + no `.markdownlintignore` additions.
- No wire/OpenAPI/asyncapi regeneration required or permitted (nothing under `crates/` changes).
- Ledger rows applied: 5 (lookbehind + inverse gate), 9 (every universal claim above carries a
  file:line or command citation), 10 (whole-repo sweep incl. hidden dirs; literal path strings, not
  concepts), 12 (deliverables sweep-derived, not hand-listed), 50 (`--no-ignore --hidden` pinned in
  the gate commands).
