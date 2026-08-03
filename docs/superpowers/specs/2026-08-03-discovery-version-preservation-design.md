# Discovery Version Preservation — Design

- **Date:** 2026-08-03
- **Status:** Approved (owner sign-off in grilling session, 2026-08-03)
- **Scope:** Autodiscovery must not overwrite the detected `installed_version` of an
  already-registered, active software item. Version freshness for active items becomes the sole
  responsibility of the scheduled `DetectVersion` check.

## Problem

Autodiscovery writes `host_software_items.installed_version` unconditionally on every run, for
every matched item — including items whose `detect_version` role assignment the operator has
manually customized. The inline comment in
`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs` (in
`find_or_create_software_item`, near the Phase-1 write block) declares discovery "the source of
truth for the installed version", and tests assert that even manually-created and featured items
are overwritten.

Observed failure (live deployment): `grocy`, discovered by `discovery.proxmox-helper-scripts`,
reports its version from the PHS-written `/root/.grocy` version file. That file goes stale when
the software is updated by other means, so every 6-hour discovery run re-stamps a wrong
`installed_version`. The operator's corrected `detect_version` assignment (`generic.shell` reading
`/var/www/html/version.json`) runs on the daily `DetectVersion` schedule and fixes the value —
producing a permanent flip-flop between wrong (discovery) and right (version check).

The AGENTS.md claim "periodic re-discovery only updates versions for autodiscovery-created items"
is not enforced by any provenance check; it is an emergent property of discovery's matching keys,
and the grocy case (a discovery-created item with a customized detection assignment) falls inside
the overwrite path.

## Decision

Discovery never overwrites a non-`NULL` `installed_version` on a continuously-active
`host_software_items` row. The version-bearing fields become conditional; presence/provenance
stamps stay unconditional.

### Behavior rule

At each site in the autodiscovery module that updates an **existing** `host_software_items` row
(`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`,
`find_or_create_software_item` — the Phase-1 matched-update block (~line 439) and the Phase-3
existing-HSI block (~line 624); anchor by function and phase comments, not line numbers):

- **Skip** writing `installed_version`, `installed_display_version`, and
  `installed_version_detected_at` **iff** the matched row itself was already active before this
  discovery pass (`deactivated_at IS NULL` on the `host_software_items` row) **and**
  `installed_version IS NOT NULL`.
- **Write** them when any of:
  - fresh insert (new `host_software_items` row, ~line 653);
  - link-level reactivation — the matched row had `deactivated_at` set (revival ≈
    re-registration; the stored version predates removal);
  - `installed_version IS NULL` — filling an empty value overrides nothing.
- **Always** write, exactly as today: `last_discovered_at`, `discovery_source`,
  `missing_since = NULL`, `deactivated_at = NULL`. Presence tracking and the
  `missing_since`/soft-delete reconciliation in `reconcile.rs` are unchanged.

**"Reactivation" is defined at the link level, not the software-item level.** Both write sites
already compute `let was_deactivated = hsi.deactivated_at.is_some();` (~lines 441, 623) — the
skip predicate is `!was_deactivated && hsi.installed_version.is_some()`, evaluated on the model
before `.into()`. Consequence: when only the parent `software_items` row was deactivated (or a
stale identity is cascade-repointed onto an already-active link via the Phase-1 "collision rule
1" `active_target_hsi` branch, ~lines 413–421), the link itself never lost presence, so its
non-NULL version is **preserved**, not refreshed — the daily `DetectVersion` check corrects it
within its cadence. This keeps the predicate uniform across all branches: the `active_target_hsi`
selection specifically filters `DeactivatedAt.is_null()`, so that branch can never take the
reactivation exception — while the `orig_hsi_query` fallback at the same Phase-1 write site
(~lines 429–436) carries no such filter and legitimately can (and does) reactivate.

**Implementation idiom:** under the skip predicate, simply omit the three `Set(...)` calls —
`Model::into()` yields `Unchanged` for every field, matching the file's existing
partial-`ActiveModel` convention. Do not fetch-then-reassign `Set(old_value)`.

Plan-writing instruction (inventory freshness): re-run
`grep -n "InstalledVersion\|installed_version" crates/ui/web-api-queries/src/queries/autodiscovery/`
at plan time and gate every version-write site found — the line anchors above are indicative, not
exhaustive-by-authority. The grep output includes test-fixture `Set(...)` literals (e.g. in
`mod.rs` `tests_common`) that need visual triage; only production write sites get the gate. Also
inventory **assertion** sites: test functions asserting `installed_version` values live in both
`discovery_items.rs` and `mod.rs` test modules and break when the write is gated — enumerate them
at plan time, not just the write sites.

### What does not change

- **Schema:** none. No migration, no new columns, no flags.
- **API/wire:** none. `DiscoveryResults` payloads, REST endpoints, and OpenAPI/AsyncAPI artifacts
  are untouched (no `regen-api.sh` / `regen-asyncapi.sh` run needed).
- **`DetectVersion` scheduled path:**
  `crates/ui/web-api/src/routes/service_ws/handler/messages/version_check.rs` →
  `apply_version_update_to_db` keeps writing `installed_version`,
  `installed_version_detected_at`, `installed_display_version`, `update_category`, and
  `latest_version` as today. It becomes the only writer of version fields for active items.
- **Audit:** no new state-changing site; existing discovery/reconcile audit emissions unchanged.
  `cargo xtask audit-coverage-check` must stay green without catalog edits.

### Accepted trade-off

The freshness regression is **universal, not a subset**: every active discovered item — including
the majority whose discovery-reported version is live and correct (e.g. APT reading dpkg) and
which never exhibited the flip-flop bug — moves from every 6-hour discovery run to the daily
`DetectVersion` schedule (`scheduled_tasks.detect_version`, default `interval_seconds = 86400`,
user-configurable). Out-of-band changes (e.g. a manual `apt upgrade` on the host) may show a stale
version for up to 24 hours on any item. Owner explicitly accepted this trade-off (the rejected
manual-override alternative would have kept 6-hour freshness for un-customized items); the
default interval is not changed by this work. In-app updates are unaffected: `UpdateResult` finalization writes `InstalledVersion` +
`InstalledVersionDetectedAt` directly (`crates/ui/web-api/src/routes/service_ws/handler/updates/result.rs`,
~lines 106–110), so post-update UI freshness does not depend on discovery or the daily check.

Freeze-safety caveat: an active item whose version can never refresh again would require it to
lack a working `detect_version` assignment. This is **not structurally impossible** — the
target-based registration path (`process_targets_discovery`, `discovery_items.rs` ~238–259)
creates only the roles listed on each `DiscoveryTarget`, and some targets carry no
`DetectVersion` role (per-item coverage comes from sibling targets). The live deployment was
audited point-in-time: all 33,677 active rows have a `detect_version` assignment. The residual
risks — a future item registered without one, a chronically-offline host missing the daily
connected-only dispatch, or a silently-failing detection command — leave the last-written value
in place indefinitely (previously masked by the 6-hour discovery overwrite). Accepted for this
single-owner deployment; detection-staleness observability is listed as deferred follow-up.

Reactivation caveat: the link-level reactivation exception re-admits the discovery-reported value
once per revival, so an item that flaps (deactivated then re-discovered) can briefly show a wrong
discovery-sourced version until the next daily check. Owner chose refresh-on-reactivate over
preserve-always knowingly; the flip-flop is bounded to revival events instead of every 6-hour
run.

Package-manager plugins (APT, Homebrew, npm) implement native batch detection
(`crates/shared/agent-core/src/version_check.rs`), so the daily check does not degrade into
per-item shell executions.

## Tests

Affected tests span **two** `#[cfg(test)]` modules — `discovery_items.rs` and `mod.rs` — in the
autodiscovery module; use their current fixture idioms (copy setup from sibling tests in the same
module). Line anchors indicative; re-derive the full list at plan time via the assertion-site
inventory above.

1. **Flip** the tests that currently assert the overwrite — each must now assert the version is
   preserved, and each fails against the old code (true RED, executes the changed statement):
   - `process_one_discovery_active_link_updates_version` (`discovery_items.rs` ~1076) → active
     link with non-NULL version keeps its value; rename to reflect preservation.
   - `process_one_discovery_featured_item_updates_version_and_provenance` (~1140) → version
     preserved; provenance stamps (`discovery_source`, `last_discovered_at`) still asserted
     written.
   - `process_one_discovery_manual_featured_item_updates_version_and_provenance` (~1229) →
     same for the manually-created item.
   - `process_one_discovery_deactivated_item_reactivates_in_place` (~890) → its fixture
     deactivates only the parent `software_items` row; the link stays active, so under the
     link-level rule the version is now **preserved** (item still reactivates; presence stamps
     still written). Adjust assertions accordingly.
   - `reactivation_prefers_existing_active_link_row` (~2227) → the preferred active link keeps
     its non-NULL version.
   - `cascade_reactivation_case_a_reconciles_plugin_links_onto_active_target_hsi` (~2486) →
     the already-active target link keeps its version through the cascade repoint.
   - `target_based_idempotent_on_second_run` (`mod.rs` ~779, assertion ~866) → second discovery
     run no longer bumps the version of the active row; assert the first-run value is preserved.
2. **Keep green:** `process_one_discovery_deactivated_link_emits_reactivate_audit` (~1001) and
   any other test whose matched **link** row has `deactivated_at` set — link-level reactivation
   still refreshes the version.
3. **Add:**
   - NULL-fill: active row with `installed_version = NULL` gets the discovered version written.
   - Presence-without-version: active row with non-NULL version — after discovery, version fields
     unchanged (positive assertion on the preserved value, not just "not equal to new"), while
     `last_discovered_at`/`discovery_source`/`missing_since = NULL` are updated.
   - Link-level reactivation refresh: a row with `deactivated_at` set and a stale non-NULL
     version is re-discovered → version refreshed (if no existing test drives exactly this
     shape, add it).

Assertions must check the concrete preserved/written value (positive content), not only
inequality.

## Documentation deliverables

Implementation is incomplete without all of these:

1. **New ADR** — created with `adrs new "Discovery never overwrites detected versions of active
items"` (never hand-allocate the number; re-verify the allocated number against `ls docs/adr/`
   immediately before writing, and run `bash ci/verify_adr_numbers.sh` +
   `bash scripts/regen-adr-toc.sh --check`). Records the invariant change, the reactivation/NULL
   exceptions, the accepted universal freshness trade-off, and — as an **active invariant gap**,
   not a nicety — that nothing structurally guarantees every registered item a `detect_version`
   assignment, so a discovery-only item registered in the future would freeze silently until the
   deferred observability follow-up lands.
2. **`docs/development/autodiscovery-internals.md`** — §2 (periodic re-discovery semantics):
   replace "only updates versions for autodiscovery-created items" with the new rule (never
   overwrites non-NULL versions of active items; creation/reactivation/NULL-fill exceptions;
   presence stamps unconditional).
3. **`AGENTS.md`** — Autodiscovery subsystem stub: update the invariant one-liner to match.
   Run `bash ci/verify_agents_md_budget.sh` and `markdownlint` after the edit.
4. **Inline comment** in `discovery_items.rs` (Phase-1 write block, currently "discovery is the
   source of truth for the installed version") — rewrite to state the preservation rule and cite
   the ADR.

No README, wire-protocol, API-doc, or frontend-doc impact: the change is server-internal behavior
with no surface, endpoint, or schema delta.

## Out of scope (deferred by owner decision, 2026-08-03)

- `reconcile_stale_plugin_links` repointing/deleting role rows during cascade reactivation
  (possible clobbering of manual role assignments) — separate concern, noted as known follow-up.
- Manual-override/origin tracking on `host_software_item_plugins` (rejected alternative).
- Any user-facing "version lock" toggle or UI surface (rejected alternative).
- Tightening the default `detect_version` interval.
- Detection-staleness observability (surfacing `installed_version_detected_at` age so a frozen
  item is visible) and any structural guarantee that every registered item carries a
  `detect_version` assignment — the safety net previously provided incidentally by the 6-hour
  discovery overwrite.

## Alternatives considered

- **Manual-override gate** (skip only when the `detect_version` assignment was manually
  created/edited, tracked via a new origin marker): would have confined the freshness regression
  to exactly the customized items — the un-customized majority (which never exhibited the bug)
  would have kept 6-hour discovery refresh. Cost: new schema, an edit-handler change, and
  backfill for pre-existing overrides. Owner chose the simpler universal rule, accepting the
  universal regression.
- **Explicit per-item lock flag**: most explicit, but adds UI/API surface and relies on the
  operator remembering to set it. Rejected.

## Quality gates for implementation

Run the **entire** Backend (Rust) command block from
[`docs/development/quality-gates.md`](../../development/quality-gates.md) — that document is the
canonical gate list; do not work from a copied subset (this spec deliberately does not enumerate
it, to avoid drift). Notes: `cargo test -p uptrakit-web-api-queries` is the tight development
loop; one whole-workspace `cargo test --all-features` runs before merge; `--all-features` worlds
require `frontend/build/` (build the frontend first); `cargo deny check` still runs even though
no new dependencies are expected.

Additional gates beyond that block:

- `cargo xtask audit-coverage-check` (per `docs/development/audit-logs.md` — routers/emitting
  handlers untouched, must stay green without catalog edits).
- Docs/ADR gates: `markdownlint --config .markdownlint.json '**/*.md'`,
  `bash ci/verify_agents_md_budget.sh` (AGENTS.md is edited),
  `bash ci/verify_adr_numbers.sh`, `bash scripts/regen-adr-toc.sh --check`, `adrs doctor`
  (hard-fail mode).
