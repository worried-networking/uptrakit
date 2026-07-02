# Discovery-Based Uninstall Reconciliation — Design

**Date:** 2026-07-02 **Status:** Draft (pending plan)
**Crates:** `uptrakit-web-api-queries` (autodiscovery), `uptrakit-web-api` (service_ws handlers),
`uptrakit-plugin-skills`, `uptrakit-migration`

## Problem

Uptrakit reports phantom "update available" for software that was uninstalled from a host.

Observed instance (controller DB `~/Library/Application Support/org.uptrakit.controller/uptrakit.db`):
the skills plugin shows `LLM Skill: caveman` and `LLM Skill: zoom-out` as pending updates, although
both were uninstalled locally (absent from `~/.agents/.skill-lock.json` and `~/.claude/skills/`;
`caveman` was migrated to a Claude Code plugin, which the skills CLI does not manage). A third
skill, `diagnose`, is in the same uninstalled state but shows no badge only because its frozen
installed hash happens to equal the current latest hash.

Root cause chain (all verified in code):

1. The skills plugin's `batch_detect` correctly returns
   `BatchDetectResult { installed_version: None, error: None }` for a skill absent from the lock
   file — the shared package-manager convention for "not installed" (apt, homebrew, npm behave
   identically).
2. The controller's version-check handler
   (`crates/ui/web-api/src/routes/service_ws/handler/messages/version_check.rs:127`) guards the DB
   write with `if let Some(ref installed_version) = result.installed_version` — when `None`,
   **no write happens at all**. `installed_version` and `installed_version_detected_at` freeze at
   their last detected values (2026-06-18 for the affected rows).
3. `latest_version` keeps refreshing on every release check, so
   `installed != latest` (`software_states.rs:174-180`) reports a phantom pending update forever.
4. The autodiscovery flow (`crates/ui/web-api-queries/src/queries/autodiscovery/`) creates and
   matches items but **never deactivates** items that stop appearing in discovery results. Nothing
   in the system ever notices an uninstall.

Two adjacent defects compound this:

- **Skills plugin conflates failure with emptiness.** Both `discovery.rs` and `detection.rs`
  return an empty/`None` result when `~/.agents/.skill-lock.json` is unreadable or fails to parse
  (`Ok(vec![])` / `Ok(None)`, no error set). Once "absent from snapshot" starts meaning
  "uninstalled", an unreadable lock file would masquerade as "everything uninstalled".
- **Uninstall during an update strands `update_history`.** `AwaitingRestart` resolution
  (`crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:1210`) stays in
  `AwaitingRestart` indefinitely when the post-update version check returns
  `installed_version: None`.

## Goal

Uninstall detection is **universal and plugin-agnostic**: when a discovery plugin's error-free run
no longer reports software it previously reported on a host, the controller automatically
deactivates the host link — no plugin-side uninstall signaling required. Deactivating the last
host link deactivates the software item itself. Rediscovery reactivates both. In-flight update
records for a link deactivated this way terminate instead of waiting forever.

## Chosen approach: controller-side discovery reconciliation

Reconcile inside the existing discovery-results processing
(`process_discovery_results`, `crates/ui/web-api-queries/src/queries/autodiscovery/mod.rs`), in the
same transaction as the upserts. Discovery is already a **complete per-plugin-scope per-host
snapshot** (scheduler every 6 h by default, plus manual trigger and first enrollment), and the wire
format already distinguishes failure from emptiness
(`DiscoveryPluginResult { discoveries, error: Option<String>, .. }`) — the foundation exists;
only the reconciliation step is missing.

### Rejected alternatives

- **Version-check `None` handling** (clear `installed_version` when detection returns
  `(None, no error)`): per-item only — leaves zombie rows visible in lists, cannot cascade to
  `software_items`, and spreads uninstall semantics across two code paths. Explicitly rejected by
  the user in favor of the universal discovery-side mechanism.
- **Hybrid (both)**: the version-check half adds nothing once reconciliation deactivates rows
  within one discovery cycle; two mechanisms for one concept is a maintenance liability.

## Design

### 1. Provenance: `host_software_items.last_discovered_at`

New nullable timestamp column via SeaORM migration, set on every discovery create **and** every
discovery re-match of an existing link.

Reconciliation only ever touches rows where `last_discovered_at IS NOT NULL`. Manually created
links (via the software-items API) have it `NULL` and are permanently immune to auto-deactivation.
This implements the "previously reported by discovery" scoping directly in data rather than by
inferring provenance from creation paths.

### 2. Reconciliation step in `process_discovery_results`

For each `DiscoveryPluginResult` in the payload where `error IS NULL` (skip errored results; skip
plugin scopes absent from the payload entirely — a missing plugin result means "did not run", not
"found nothing"):

1. Build the set of reported `package_identifier`s from the **raw snapshot, before ignore-list
   filtering**. Ignore rules (`software_ignores`) suppress _creation_ of new items; an
   ignored-but-already-linked item that is still reported must stay active.
2. Select candidate links: active (`deactivated_at IS NULL`) `host_software_items` on that host
   belonging to that plugin scope (matched via `host_software_item_plugins` / `plugin_config_id`,
   same scope resolution the existing match phases use) with `last_discovered_at IS NOT NULL`.
3. Candidates whose `package_identifier` is not in the snapshot → set `deactivated_at = now`.
4. For each link deactivated in (3): if the `software_item` has no remaining active host links,
   set `software_items.deactivated_at = now` as well.
5. Terminate in-flight updates for each deactivated link: `update_history` rows in
   non-terminal states (`InProgress`, `AwaitingRestart`; `Pending`/`Queued` batch members are
   resolved the way the existing batch machinery expects — exact handling settled in the plan) →
   `Failed` with error message "software no longer installed on host". This also closes the
   `dispatch.rs:1210` stranded-`AwaitingRestart` case for uninstall-during-update.
6. Emit Audit V2 events (`emit_stateful`, in-transaction) for every link deactivation, item
   cascade, and update termination.

Transaction discipline: the reconciliation reads rows then writes them, so the enclosing
transaction must use `BEGIN IMMEDIATE` (`SqliteTransactionMode::Immediate`) per the workspace
SQLite rule; no-op on Postgres.

Tenant isolation: all queries scope through the tenant as the existing autodiscovery queries do
(`host_software_items` reached via its tenant-scoped parents; no raw unscoped `find()`).

### 3. Reactivation on rediscovery

Extend the existing match phases (`discovery_items.rs` phases 1–2): when a discovery result matches
a **deactivated** link or item, clear `deactivated_at` on both the link and (if deactivated) the
`software_item`, refresh `last_discovered_at`, and emit a reactivation audit event.

Edge case the plan must handle: the partial unique indexes (`uix_hsi_unqualified`,
`uix_hsi_qualified`) exclude deactivated rows, so a deactivated link can coexist with a newer
active link for the same `(host, software_item, qualifier)`. Reactivation must prefer the existing
active row and leave the deactivated duplicate untouched (never resurrect into a unique-index
collision).

### 4. Skills plugin: failure ≠ empty

Change `crates/plugins/package-managers/skills/src/discovery.rs` and `detection.rs`:

- **Lock file missing** (`test -f` fails): legitimate empty snapshot — the skills CLI manages
  nothing on this host. Discovery returns `Ok(vec![])`; batch detect returns `None` per item.
  Reconciliation deactivating all previously discovered skills is then _correct_.
- **Lock file present but unreadable, or JSON parse failure**: return an error. Discovery →
  `Err(...)` so `DiscoveryPluginResult.error` is set and reconciliation skips the scope; batch
  detect → per-item `BatchDetectResult::error(...)` so the version-check handler preserves DB
  state (its existing error path).

The `sh -c "cat ..."` invocation is replaced/augmented so the two cases are distinguishable (e.g.
`test -f` probe before `cat`, or exit-code discrimination — plan decides the exact command shape
within the existing `CommandSpec` executor pattern).

Document the general contract in `docs/development/plugin-guidelines.md`: _discovery and batch
detection must report failures as errors; empty results assert "nothing installed" and, after this
change, trigger deactivation of previously discovered items._

### 5. Out of scope / deferred

- **Version-check-based uninstall detection** (the `version_check.rs:127` skip-on-`None` behavior
  stays as-is): a phantom update may persist up to one discovery interval (≤ 6 h by default) after
  an uninstall before reconciliation clears it. Accepted.
- **`awaiting_restart_timeout` machinery** (column exists, unused): general timeout for stuck
  `AwaitingRestart` records independent of uninstalls.
- **Tracking Claude Code plugin-installed skills** (e.g. `caveman` post-migration): separate
  discovery source, separate feature.
- **UI changes**: deactivated rows are already filtered from active list views; no frontend work.
- **Manual host-link deactivation API**: reconciliation is automatic only; no new endpoint.

## Data model changes

- Migration `mYYYYMMDD_NNNNNN_add_hsi_last_discovered_at`: add
  `host_software_items.last_discovered_at TIMESTAMP NULL`. No backfill — existing discovery-created
  rows gain provenance on their next discovery match, becoming reconciliation-eligible one cycle
  later. (Backfill is impossible anyway: creation provenance was never recorded.)

## Error handling

- Errored `DiscoveryPluginResult` → scope skipped, existing warn-level logging retained; DB state
  untouched.
- Reconciliation failures abort the transaction with the rest of discovery processing
  (`rootcause::Report`, no `unwrap()`).
- Skills plugin lock-file errors surface as typed `SkillsError` variants through the existing
  error plumbing.

## Testing

- **Reconciliation unit tests** (SQLite in-memory, FK parents inserted per workspace test rules):
  - previously discovered link absent from snapshot → deactivated; `last_discovered_at NULL`
    (manual) link absent → untouched
  - last-host cascade deactivates `software_item`; multi-host does not
  - errored plugin result → no deactivation; plugin scope absent from payload → no deactivation
  - raw-snapshot vs ignore-list: ignored-but-linked item still reported → stays active
  - qualifier-bearing links reconcile within their qualifier
  - rediscovery reactivates deactivated link + item, refreshes `last_discovered_at`; reactivation
    with an existing active duplicate prefers the active row
  - in-flight `update_history` (`InProgress`, `AwaitingRestart`) → `Failed` with reason;
    terminal rows untouched
  - audit events emitted for deactivate / cascade / reactivate / terminate
- **Skills plugin tests** (`FixedOutputExecutor`): missing lock file → empty/`None` without error;
  unreadable/corrupt lock file → discovery `Err` and per-item detect errors.
- Full workspace quality gates (fmt, check ×2 feature sets, clippy ×2, test, deny, markdownlint).

## Documentation deliverables

- New ADR in `docs/adr/` — _Discovery-based software uninstall reconciliation_ (lifecycle:
  auto-deactivation, cascade, reactivation, provenance column, failure-vs-empty contract). Number
  assigned at implementation time (0026 is reserved by the pending OpenAPI-client drift-guard
  spec).
- `docs/development/plugin-guidelines.md` — discovery/detection error-vs-empty contract for plugin
  authors.
- No README/user-guide impact: behavior change is "stale uninstalled software disappears", which is
  the documented intent of discovery; no new config surface.

## Dependencies

None added.
