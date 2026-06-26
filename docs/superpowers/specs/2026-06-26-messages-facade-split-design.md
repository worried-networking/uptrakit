# Spec: `messages.rs` → `messages/` facade split

- **Date:** 2026-06-26
- **Status:** Approved, ready for planning
- **Target file:** `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- **Measured on:** `main` @ `c5528b91e` (CodeScene); live structure verified @ current `main` (`3273a89a2`)
- **Template:** sibling `crates/ui/web-api/src/routes/service_ws/handler/updates/` (the proven 3.3/RED → none-red split, merged in commits `c6e1c57f6`..`3273a89a2`)

## Problem

`messages.rs` is 4469 lines, CodeScene code health **2.81 / RED** — the worst (and only) red file in the
`service_ws/handler/` directory. The dominant drags are **file-aggregate, severity-3** smells that dedup alone
cannot move:

- **Low Cohesion (LCOM4)** + **100 functions** → Brain Class warning.

These are only fixable by **splitting** the file. Secondary smells (all relieved by the split, some by gated
extraction):

- Bumpy Road: `handle_renew_certificate` (208 LoC, cc≈14, mirrored system/tenant paths),
  `build_enriched_display_overrides` (161 LoC, cc≈20), `handle_discovery_results`, `enrich_discovered_items`.
- Complex Method: `handle_version_check_results` (167 LoC, cc≈12).
- Complex Conditional: `VersionCheckAuditSummary::outcome`, `link_reported_hosts`.
- Large Methods + Excess-Args: `emit_report_plugin_config_audit` (7 args),
  `process_discovery_page_for_host` (7 args).

## Goal & success metric

- **Optimized metric:** per-file CodeScene `code_health_score`.
- **Enforceable floor:** no file remains red (every resulting file ≥ 4.0).
- **Aspirational:** green (≥ 9.0). A **yellow** production submodule is acceptable.
- **Hard constraint — behavior-preserving:** no change to message semantics, audit outcomes/JSON, DB queries,
  SSE/MQTT payloads. This is a mechanical decomposition, not a behavior change.

## Live-code verification (corrections vs. original brief)

Verified against current `main`:

| Claim in brief                            | Verified reality                                                                                  |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Test module ~lines 2066–4468              | **1982–4469** (`#[cfg(all(test, feature = "db-sqlite"))]`, ~2488 lines)                           |
| `handle_renew_certificate` cc=14, 4 bumps | Confirmed bumpy; **208 LoC**; system path mirrors tenant path (prime harmless-dedup target)       |
| `build_enriched_display_overrides` cc=20  | Confirmed; **161 LoC**; nested plugin-group dispatch + repeated error-collapse                    |
| `link_reported_hosts` 7-arg               | **Not** 7 scalar args — single `ReportHostsPayload`; cohesive single-path; 82 LoC                 |
| `process_discovery_page_for_host` 7 args  | Confirmed 7 params; cohesive single path; 72 LoC                                                  |
| `emit_report_plugin_config_audit` 7 args  | Confirmed; orthogonal audit-builder params; 51 LoC                                                |
| module declaration                        | `pub(super) mod messages;` in `handler/mod.rs` (unchanged by this work — resolves to the new dir) |
| AGENTS.md                                 | Single GFM-aligned row for `messages.rs` in the handler-module table                              |

**External consumers** of `messages` items (determine visibility):

- `handle_ping` — referenced by `session_authenticated.rs`
- `handle_renew_certificate`, `handle_report_hosts`, `handle_version_check_results`,
  `handle_discovery_results`, `handle_report_plugin_config` — referenced by `message_processor.rs`

All other items are intra-`messages` helpers with no external references.

**Sibling-module imports** `messages` currently pulls (must be preserved through the facade):

```text
super::discovery::trigger_discovery_for_agent_host
super::message_processor::LoopAction
super::renewal::{sign_renewal_csr, sign_renewal_csr_system}
super::shared_types::{ProcessorResponse, load_linked_host_ids}
super::audit_service::{ingest_service_audit_event, emit_service_certificate_renew_audit_event}
super::updates::{resolve_software_item_name, resolve_host_name, emit_batch_progress_event,
                 handle_batch_completion, emit_batch_progress_from_db}
```

> ⚠ Naming collision note: `messages` already imports `super::discovery::trigger_discovery_for_agent_host`
> (a **handler sibling** module). The new `messages/discovery.rs` submodule is a different path. Inside
> submodules, the handler-sibling `discovery` is reached via `super::super::discovery::...`; the in-`messages`
> `discovery.rs` is reached via `super::discovery::...` from the facade only. Keep these straight — see
> Import Convention below.

## Chosen approach

Mirror the `updates/` template exactly: convert the file into a `messages/mod.rs` facade plus per-message-family
production submodules and a moved test module. (User decision: per-family cut; split + test-move + harmless
dedups with re-score-gated extraction — recorded in the grilling session.)

### Step 1 — Facade conversion

- `git mv messages.rs messages/mod.rs` (use `git mv` so history follows).
- `handler/mod.rs` keeps `pub(super) mod messages;` unchanged (now resolves to the directory).
- `mod.rs` becomes a thin facade containing, in `updates/mod.rs` order:
  1. Module doc-comment mapping each submodule to its responsibility.
  2. Sibling re-imports the submodules need by NAME, e.g.
     `use super::shared_types::{ProcessorResponse, load_linked_host_ids};` and the other five sibling imports
     listed above. Submodules then write `use super::{ProcessorResponse, ...}`.
  3. `mod <name>;` declarations for each submodule.
  4. Visibility-correct re-exports (see Visibility below).
  5. `#[cfg(all(test, feature = "db-sqlite"))] mod tests;`

### Step 2 — Move the test module FIRST (biggest cheap win)

- Move the inline test module (lines **1982–4469**) into `messages/tests.rs`.
- Header: `use super::*;` plus any explicit imports the test module already used (preserve verbatim).
- **Restore private-helper visibility for the moved tests.** Today the test block reaches production helpers
  (`apply_version_update_to_db`, `enrich_discovered_items`, `link_reported_hosts`,
  `finalize_version_check_results`, `resolve_matching_host_software_items`, and any other private fn it calls —
  ~17 call sites) via same-file scope. After the move, `use super::*` resolves to the facade, which does NOT
  re-export those private helpers. For each private production helper called by the test module: promote it to
  `pub(super)` in its destination submodule, and add an explicit `use super::<submodule>::<fn>;` import in
  `tests.rs`. Precedent: `updates/tests.rs` does exactly this (`use super::result::select_best_output;`,
  `use super::started::{UpdateStartedInfo, broadcast_update_started_events};`). An implementer who skips this
  hits compile errors, not silent breakage.
- This alone removes the ~2488-line test block from the production-scored file.

### Step 3 — Per-family production submodules

| Submodule                | Items (top-level)                                                                                                                                                                                                                                                                                          |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `certificate.rs`         | `handle_renew_certificate`, `emit_service_certificate_renew_non_success_audit_event`                                                                                                                                                                                                                       |
| `hosts.rs`               | `handle_report_hosts`, `link_reported_hosts`, `notify_reported_hosts_online`, `resolve_host_hostname`, `ReportHostsSummary` (struct + impl)                                                                                                                                                                |
| `version_check.rs`       | `handle_version_check_results`, `resolve_matching_host_software_items`, `apply_version_update_to_db`, `dispatch_version_update_notification`, `finalize_version_check_results`, `build_enriched_display_overrides`, `VersionCheckAuditSummary` (struct + impl), any `DisplayOverride`/`Group` helper types |
| `discovery.rs`           | `handle_discovery_results`, `find_linked_host_by_machine_id`, `process_discovery_page_for_host`, `enrich_discovered_items`                                                                                                                                                                                 |
| `plugin_config.rs`       | `handle_report_plugin_config`, `emit_report_plugin_config_audit`, `report_plugin_config_target_id`, `PluginConfigReportAuditCtx`                                                                                                                                                                           |
| `restart_progression.rs` | `trigger_host_progression_after_awaiting_restart`                                                                                                                                                                                                                                                          |
| `shared.rs`              | `emit_service_inventory_audit`, `handle_ping`                                                                                                                                                                                                                                                              |
| `tests.rs`               | moved test module (Step 2)                                                                                                                                                                                                                                                                                 |

> ⚠ Source-order trap: `enrich_discovered_items` sits at source line **1879**, AFTER
> `handle_report_plugin_config` (1707–1878). It belongs to **`discovery.rs`**, not `plugin_config.rs`. A
> sequential family-by-family extraction encounters it while working on plugin_config — do not leave it there.

Aim: each fn < 70 LoC where reasonable, cc < 9 where reasonable, each file well under brain-class size.
The large handlers (`handle_version_check_results`, `handle_discovery_results`,
`handle_report_plugin_config`) remain intact dispatchers within their family file — splitting them is **not**
required to clear red and risks the updates.rs regression (see Step 5).

### Step 4 — Harmless de-dups

- **All ~26 CodeScene-flagged duplicated test helpers/cases** — collapse the dup pairs while moving to
  `tests.rs` (shared setup helpers, repeated fixture builders, near-identical case bodies).
- **`handle_renew_certificate` mirrored system/tenant block** — the system path mirrors the tenant path across
  sign → revoke → bump_version → notify → audit. Extract ONE inner fn / closure carrying the shared sequence,
  parameterized by the system-vs-tenant discriminator. This is the explicitly-requested harmless dedup, not the
  risky decomposition of Step 5.
- Any other genuinely-identical production dup pairs surfaced during the split (collapse only when behavior is
  provably identical).

### Step 5 — Conditional, re-score-gated extraction

- Candidate: `build_enriched_display_overrides` (161 LoC, cc≈20) — extract the per-plugin-group enrichment
  (`enrich_single_plugin_group(plugin_type, group) -> ...`) to remove a bump and shrink the parent.
- **Gate (apply verbatim):** keep the extraction ONLY if a CodeScene re-score of `version_check.rs` shows the
  score improves (or at minimum does not regress) AND the extraction introduces **no** new excess-args helper
  and **no** new large method. If it redistributes complexity or spawns 5–7-arg helpers, **revert it** — this is
  exactly what regressed `updates.rs`'s `handle_update_result` decomposition and was reverted there.
- `handle_renew_certificate`'s bump is addressed by the Step-4 dedup (harmless), not by speculative splitting.
- Stop at yellow if green is not free.

### Step 6 — Bookkeeping (non-optional gates)

- **`crates/ui/web-api/db_access_policy.toml`** — replace the
  `[routes."service_ws/handler/messages.rs"]` section with one section per new file path:
  - Migrate each `async fn`'s entry to the section keyed by its new file, **value preserved**:
    - `certificate.rs`: `handle_renew_certificate = "no-db"`,
      `emit_service_certificate_renew_non_success_audit_event = "ignore"`
    - `hosts.rs`: `handle_report_hosts = "no-db"`, `link_reported_hosts = "ignore"`,
      `notify_reported_hosts_online = "ignore"`
    - `version_check.rs`: `handle_version_check_results = "no-db"`,
      `resolve_matching_host_software_items = "ignore"`, `apply_version_update_to_db = "ignore"`,
      `dispatch_version_update_notification = "ignore"`, `finalize_version_check_results = "ignore"`,
      `build_enriched_display_overrides = "ignore"`
    - `discovery.rs`: `handle_discovery_results = "no-db"`, `find_linked_host_by_machine_id = "ignore"`,
      `process_discovery_page_for_host = "ignore"`, `enrich_discovered_items = "ignore"`
    - `plugin_config.rs`: `handle_report_plugin_config = "no-db"`
    - `restart_progression.rs`: `trigger_host_progression_after_awaiting_restart = "ignore"`
    - `shared.rs`: `handle_ping = "no-db"`
  - **Sync fns are omitted** by the checker — do NOT add entries for `emit_service_inventory_audit`,
    `report_plugin_config_target_id`, `resolve_host_hostname`, `emit_report_plugin_config_audit`, or impl
    methods (they are not `async fn` at top level). Verify async-ness per fn before migrating; the canonical
    list above is derived from the existing `messages.rs` section (which tracks exactly these).
  - Add a `[routes."service_ws/handler/messages/tests.rs"]` section listing the test fns as `"ignore"`
    (convention: `handler/tests.rs` and `updates/tests.rs` already do this).
  - Verify with `python3 ci/verify_db_access_policy.py`.
- **`AGENTS.md`** — replace the single `messages.rs` row in the handler-module table with one row per new
  submodule. The table is MD060 aligned style: pad columns so markdownlint passes. Suggested purposes:
  - `messages/mod.rs` — Common message-handler facade (re-exports `ProcessorResponse` handlers)
  - `messages/certificate.rs` — Certificate-renewal message handler + renew audit
  - `messages/hosts.rs` — Host-report message handler + host linking/notify
  - `messages/version_check.rs` — Version-check results handler + enrichment/finalize
  - `messages/discovery.rs` — Discovery-results handler + page processing/enrichment
  - `messages/plugin_config.rs` — Plugin-config report handler + config audit
  - `messages/restart_progression.rs` — Post-restart host progression
  - `messages/shared.rs` — Ping handler + service-inventory audit helper
  - `messages/tests.rs` — Unit tests for the messages submodules
  - Update the AGENTS.md `updates/` precedent note already lists submodules — mirror that row style.

## Import & visibility conventions (apply verbatim — hard-won from updates.rs)

**Import convention:**

- Submodules reach handler-level siblings via **relative** `super::super::<sibling>` paths, NOT absolute
  `crate::routes::service_ws::handler::...`.
- The facade (`mod.rs`) privately re-imports sibling NAMES (e.g. `use super::shared_types::{ProcessorResponse,
load_linked_host_ids};`) so submodules write `use super::{ProcessorResponse, load_linked_host_ids};`.
- **Never** write `super::shared_types::` inside a submodule — `shared_types` is a handler sibling, not under
  `messages`. (Same for `audit_service`, `renewal`, `message_processor`, `updates`, handler-`discovery`:
  facade re-imports by name; submodules use `super::<Name>`. Where a submodule needs a sibling the facade does
  NOT re-export, reach it via `super::super::<sibling>::<Name>`.)

**Visibility:**

- An item consumed OUTSIDE `messages` (by `message_processor.rs` / `session_authenticated.rs`) must be
  `pub(in super::super)` in its submodule **and** re-exported by the facade with `pub(super) use`.
  Applies to: `handle_ping` (shared.rs), `handle_renew_certificate` (certificate.rs),
  `handle_report_hosts` (hosts.rs), `handle_version_check_results` (version_check.rs),
  `handle_discovery_results` (discovery.rs), `handle_report_plugin_config` (plugin_config.rs).
- A `pub(super)` item + `pub(super) use` re-export does **NOT** compile (can't widen). Intra-`messages` helpers
  stay `pub(super)`/private and the facade pulls them with a private `use`.
- A handler-sibling that `handler/mod.rs` no longer uses itself is reached via `super::super::<sibling>`
  (relative — fine).

## Idiom & standards conformance (from `.superpowers/standards-snapshot.md`)

- Lint suppressions stay `#[expect(lint, reason="...")]` — never bare `#[allow]`, never a NEW suppression to
  dodge a gate. Preserve any existing `#[expect]` attrs on moved items verbatim.
- **File-level `#![expect]` attributes must move with their triggering code.** `messages.rs` carries two
  inner attributes (lines 8–12):
  `#![expect(clippy::expect_used, reason = "expect used for infallible operations; message documents the invariant")]`
  and `#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]`. The workspace sets
  `unfulfilled_lint_expectations = "deny"`, so leaving these on the now-codeless `mod.rs` facade is a **hard
  build error**. Relocate each to the submodule(s) that retain the triggering code and DELETE both from `mod.rs`:
  `clippy::expect_used` → `plugin_config.rs` (the `.expect("...JSON is always valid")` calls in
  `handle_report_plugin_config`); `clippy::indexing_slicing` → `version_check.rs` (the `group.hsi_ids[i]` /
  `group.items[i]` access in `build_enriched_display_overrides`). Verify final placement by compiling — clippy
  reports any unfulfilled expectation. Precedent: `updates/replay.rs` and `updates/result.rs` each carry a
  file-level `#![expect]` for exactly this reason.
- No `unwrap()` in production (parking_lot/RwLock excepted); errors via `rootcause::Report` / `report!` /
  `bail!`. This refactor moves code, does not author new error paths — preserve existing patterns.
- `parking_lot::Mutex`, drop guards before `.await` — preserve as-is in moved code.
- Conventional Commits at workspace level. Suggested commit sequence mirrors `updates/`:
  `git mv` + facade; move tests; extract each family submodule; dedups; bookkeeping; (optional) gated
  extraction. Each commit must compile and pass fmt/clippy.

## Quality gates (run BOTH feature permutations)

```bash
cargo fmt --all
cargo check  --no-default-features --features db-sqlite
cargo check  --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test   --all-features
cargo deny check
bash ci/verify_no_security_audit.sh        # audit-emit code moves across submodules
bash ci/verify_typed_audit_actions.sh      # typed audit actions in moved code
bash ci/verify_handler_state_contract.sh   # split restructures handler module layout
python3 ci/check_plugin_semantic_boundary.py   # blocking gate; production code path
python3 ci/verify_db_access_policy.py
markdownlint --config .markdownlint.json '**/*.md'   # AGENTS.md + this spec
# REQUIRED — messages.rs is service-lifecycle code (binding rule, no refactor carve-out):
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

Re-score gate: after the split (Steps 1–4), run CodeScene `code_health_score` on `messages/mod.rs` and every
new production submodule; confirm none-red. Only then attempt Step 5, re-scoring `version_check.rs` to gate it.

## Documentation deliverables

- **`AGENTS.md`** — handler-module table: replace the `messages.rs` row with per-submodule rows (Step 6). **Required.**
- **`db_access_policy.toml`** — per-file policy sections migrated (Step 6). Tracked gate, not prose docs. **Required.**
- **No ADR** — pure mechanical decomposition under existing `docs/adr/0001-web-api-decomposition-strategy.md`;
  no new architectural decision. Justified: behavior-preserving file split following an established pattern.
- **No `README.md` / `CONTEXT.md` change** — no externally observable behavior, surface, config, or
  architecture change.

## Sequencing

1. `git mv` → `messages/mod.rs` facade skeleton.
2. Move test module → `messages/tests.rs` (+ collapse ~26 dup test pairs).
3. Extract per-family production submodules; wire facade re-exports + visibility.
4. Apply harmless production dedup (`handle_renew_certificate` inner-fn).
5. Migrate `db_access_policy.toml` sections; update `AGENTS.md` rows.
6. **Re-score** every production submodule → confirm none-red.
7. **Only then** attempt the gated `build_enriched_display_overrides` extraction; keep only if it improves (or
   does not regress) the score with no new excess-args/large-method; otherwise revert.
8. Full quality gates incl. Docker integration suite.

## Out of scope / deferred

- Decomposing the large dispatchers (`handle_version_check_results`, `handle_discovery_results`,
  `handle_report_plugin_config`) beyond the gated `build_enriched_display_overrides` attempt — high regression
  risk for no required score benefit; the updates.rs precedent reverted the analogous work.
- Any change to message semantics, audit JSON, DB queries, SSE/MQTT payloads.
- Touching sibling handler modules beyond the import/visibility wiring needed to keep them compiling.
- Pursuing green at the cost of redistributed complexity or new excess-args helpers.
