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
   (`crates/ui/web-api/src/routes/service_ws/handler/messages/version_check.rs:127`)
   guards the version columns with `if let Some(ref installed_version) = result.installed_version`
   — when `None`, the write still fires (e.g. `update_category`) but skips `installed_version`,
   `installed_version_detected_at`, and `installed_display_version`, which freeze at their last
   detected values (2026-06-18 for the affected rows).
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

Uninstall detection is **universal and plugin-agnostic**: when a discovery plugin's error-free
runs (two consecutive, for glitch tolerance) no longer report software it previously reported on a
host, the controller automatically deactivates the host link — no plugin-side uninstall signaling
required. Deactivating the last
host link deactivates the software item itself. Rediscovery reactivates both. In-flight update
records for a link deactivated this way terminate instead of waiting forever.

## Chosen approach: controller-side discovery reconciliation

Reconcile inside the existing discovery-results processing
(`process_discovery_results`, `crates/ui/web-api-queries/src/queries/autodiscovery/mod.rs`), as a
new transactional step alongside the upserts (see §2). Discovery is already a **complete
per-discovery-source per-host snapshot** (scheduler every 6 h by default, plus manual trigger and
first enrollment), and the wire
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

### 1. Provenance: `last_discovered_at` + `discovery_source`

Two new nullable columns on `host_software_items`, set on every discovery create **and** every
discovery re-match of an existing link:

- `last_discovered_at TIMESTAMP NULL` — when this link was last present in a discovery snapshot.
- `discovery_source TEXT NULL` — the `plugin_type` of the **discovering** plugin
  (`DiscoveryPluginResult.plugin_type`), which is not derivable from the link's management plugin
  rows: PHS discovery (`discovery_proxmox_helper_scripts`) creates links whose
  `host_software_item_plugins` rows carry the _target_ plugin types (`releases_github`,
  `generic_shell`, `package_manager_apt`), so the discovering plugin must be recorded explicitly.

Reconciliation only ever touches rows where both columns are non-`NULL`. Manually created links
(via the software-items API) have them `NULL` and are permanently immune to auto-deactivation.
This implements the "previously reported by discovery" scoping directly in data rather than by
inferring provenance from creation paths.

### 2. Reconciliation step in `process_discovery_results`

For each `DiscoveryPluginResult` in the payload where `error IS NULL` (skip errored results; skip
discovery sources absent from the payload entirely — a missing plugin result means "did not run",
not "found nothing"):

1. Build the set of reported identifiers as **per-target effective package identifiers** —
   `target.package_identifier` override falling back to `DiscoveredSoftware.package_identifier`,
   the same resolution `find_or_create_software_item` applies when writing
   `host_software_items.package_identifier` (skills store the encoded
   `<source_url>#<skill_path>` form, PHS targets store `owner/repo`, not the raw discovery-level
   identifier). Build it **before ignore-list filtering**: ignore rules (`software_ignores`)
   suppress _creation_ of new items; an ignored-but-already-linked item that is still reported
   must stay active.
2. Select candidate links: active (`deactivated_at IS NULL`) `host_software_items` on that host
   with `discovery_source = result.plugin_type` and `last_discovered_at IS NOT NULL`.
   **Presence always clears first**: for every candidate whose `package_identifier` IS in the
   effective-identifier set, clear `missing_since` (and refresh `last_discovered_at`) before any
   skip logic — clearing is always safe. Then **skip** absent candidates that currently have an
   `InProgress` (or dispatch-pending `Pending`/`Queued`) `update_history` row from stamping and
   deactivation — an update may be rewriting the package state at snapshot time; the link stays
   eligible for the next cycle. `AwaitingRestart` does **not** defer: an uninstalled item never
   produces the version-check result that would resolve it, so deferring would strand it forever.
3. **Two-miss hysteresis with minimum age**: absent candidates get `missing_since = now` stamped
   on first miss; a later error-free snapshot that also misses a candidate whose `missing_since`
   is already set **and older than a minimum age** (a fixed floor, e.g. 1 h — plan fixes the
   constant) → set `deactivated_at = now`. The age floor prevents manual double-triggers (or a
   manual trigger racing the scheduled run) from collapsing "two misses" into one transient
   condition lasting seconds. This absorbs single-snapshot glitches (transient environment races,
   agents predating the error-contract fixes whose failures deserialize as error-free empties via
   `#[serde(default)]` on `DiscoveryPluginResult.error`).
4. For each link deactivated in (3): if the `software_item` has no remaining active host links,
   set `software_items.deactivated_at = now` as well.
5. Terminate in-flight updates for each deactivated link: `update_history` rows in
   non-terminal states (`AwaitingRestart`; `InProgress`/`Pending`/`Queued` cannot occur here by
   the step-2 skip — handled defensively by mapping to `Failed` the same way, no bespoke design)
   → `Failed` with error message "software no longer installed on host". This also closes the
   `dispatch.rs:1210` stranded-`AwaitingRestart` case for uninstall-during-update.
6. Emit Audit V2 events (`emit_stateful`, in-transaction) for every link deactivation, item
   cascade, and update termination.
7. `discovery_source` ownership is **last-writer-wins**: if two discovery sources ever match the
   same link, the most recent stamper owns reconciliation for it. In practice identifier
   encodings differ per source (skills encoded URLs, PHS `owner/repo`, apt package names); the
   plan verifies no real collision exists among current plugins.
8. The "absent from the payload = did not run" rule assumes a **disabled or unassigned** discovery
   plugin is omitted from the payload rather than reporting an error-free empty result — otherwise
   disabling a source would mass-deactivate its links. The plan verifies disabled sources take the
   absence path.

Two required changes to the existing `process_discovery_results` structure:

- **Remove the empty-snapshot early-`continue`** (`mod.rs:77-84`): today an empty error-free
  `discoveries` list skips processing entirely. Empty + no error is the primary reconciliation
  trigger ("everything of mine was uninstalled") and must reach the reconciliation step. The
  creation/upsert path may still skip empty results.
- **Introduce a transaction** — `process_discovery_results` currently runs against
  `&DatabaseConnection` with no transaction at all. Reconciliation is read-then-write and emits
  stateful audit entries, so it runs in its own `BEGIN IMMEDIATE` transaction
  (`SqliteTransactionMode::Immediate`; no-op on Postgres) with `emit_stateful(&tx, ...)`,
  following the codebase's `_in_tx` function convention. Wrapping the pre-existing upsert flow in
  the same transaction is not required by this spec; the plan decides the exact boundary
  (per-plugin-result or per-payload).

Tenant isolation: all queries scope through the tenant as the existing autodiscovery queries do
(`host_software_items` reached via its tenant-scoped parents; no raw unscoped `find()`).

Downstream consumer fix this state change requires: update-batch candidate selection
(`update_batches/candidates.rs`) filters `software_items.deactivated_at` but **not**
`host_software_items.deactivated_at`. Deactivated links become a routine state with this feature —
a multi-host item stays active while one host's link is deactivated, and that host must not be
selected for updates. Add the link-level filter.

### 3. Reactivation on rediscovery

Extend the existing match phases (`discovery_items.rs` phases 1–2): when a discovery result matches
a **deactivated** link or item, clear `deactivated_at` on both the link and (if deactivated) the
`software_item`, refresh `last_discovered_at`/`discovery_source`, reset `missing_since = NULL`
(a reactivated link must require two fresh misses), and emit a reactivation audit event.
Reactivation is **update-in-place** — the match phases must match deactivated rows too instead of
filtering them out and falling through to insert.

Index semantics (verified): the partial unique indexes `uix_hsi_unqualified` / `uix_hsi_qualified`
carry `AND deactivated_at IS NULL` on **all** databases — the migration registry
(`migration/mod.rs`) runs `m20260318_000001_host_software_item_qualifier` (which creates them
without the condition) _before_ `m20260309_000003_unified_software_tracking` (which drops and
recreates them _with_ it), despite the file names suggesting otherwise. No index migration is
needed. Because uniqueness excludes deactivated rows, a deactivated link _can_ coexist with a
newer active link for the same `(host, software_item, qualifier)` if some path inserts instead of
reactivating; the match phases must prefer an existing active row and leave any deactivated
duplicate untouched (never resurrect into a unique-index collision).

The same rule applies one level up: `uq_software_items_active_name (tenant_id, name) WHERE
deactivated_at IS NULL` means an active item with the same name may have been created while the
original was deactivated. Cascade-reactivation must prefer the existing active item (re-point the
link, matching what phase 3's name-based upsert already does) and leave the deactivated item
dormant — never clear its `deactivated_at` into a name-index collision.

### 4. Discovery plugins: failure ≠ empty (contract audit)

Once "absent from an error-free snapshot" carries deactivation semantics, every discovery plugin
must uphold the contract: **failures set `error`; empty means empty; a partial result (any
discovered item silently skipped due to a per-item failure) must also set `error`** — partial is a
failure mode the error/empty dichotomy cannot otherwise express. This spec audits all current
discovery plugins (skills, apt, homebrew, npm, PHS, docker) and fixes the known violators:

**Skills** — change `crates/plugins/package-managers/skills/src/discovery.rs` and `detection.rs`:

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

**PHS** (`crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`) — same defect, worse
form: a failed read of `/usr/bin/update` returns `Ok(vec![])` (error-free empty → would mass-
deactivate every PHS link), and per-item remote-fetch failures `continue` silently (partial
error-free snapshot → would deactivate the unreachable subset). Both paths must return `Err` /
set `error`.

**Apt, homebrew, npm, docker** — audited in the plan against the same contract; fixed if found in
violation.

Document the general contract in `docs/development/plugin-guidelines.md`: _discovery and batch
detection must report failures as errors — including partial results with silently skipped items;
empty results assert "nothing installed" and, after this change, trigger deactivation of
previously discovered items._

### 5. Out of scope / deferred

- **Version-check-based uninstall detection** (the `version_check.rs:127` skip-on-`None` behavior
  stays as-is): a phantom update may persist up to two discovery intervals (≤ 12 h by default,
  given the two-miss hysteresis) after an uninstall before reconciliation clears it. Accepted.
- **`awaiting_restart_timeout` machinery** (column exists, unused): general timeout for stuck
  `AwaitingRestart` records independent of uninstalls.
- **Tracking Claude Code plugin-installed skills** (e.g. `caveman` post-migration): separate
  discovery source, separate feature.
- **UI changes**: deactivated rows are already filtered from active list views; no frontend work.
- **Manual host-link deactivation API**: reconciliation is automatic only; no new endpoint.

Accepted residual behavior: a host whose discovery **persistently** fails or is persistently
partial (e.g. PHS under intermittent rate limiting, post-§4 setting `error` most cycles) never
reconciles — phantom updates persist there indefinitely. Correct tradeoff: never deactivate on
incomplete data.

Accepted residual risk: agents running plugin builds that predate the §4 error-contract fixes can
report persistent failures as error-free empty snapshots. The two-miss hysteresis absorbs
transient cases; a _persistently_ failing old agent still mass-deactivates after two cycles.
Self-healing (reactivation on rediscovery once fixed), audited, and bounded to the fleet-upgrade
window — no agent-version gating added.

## Data model changes

- Migration `mYYYYMMDD_NNNNNN_hsi_discovery_provenance` (use `add_column_if_not_exists`, the
  current pattern for nullable columns): add to `host_software_items` —
  `last_discovered_at TIMESTAMP NULL`, `discovery_source TEXT NULL` (provenance), and
  `missing_since TIMESTAMP NULL` (hysteresis state, §2 step 3). No backfill — existing
  discovery-created rows gain provenance on their next discovery match, becoming
  reconciliation-eligible one cycle later. (Backfill is impossible anyway: creation provenance was
  never recorded.) Consequence: rows whose software was uninstalled **before** deploy never get a
  discovery match, never gain provenance, and are never auto-deactivated — the incident rows
  (`caveman`/`zoom-out`/`diagnose`) require a one-time manual delete via the existing
  software-items delete endpoint. The feature prevents future phantoms; it does not clear
  pre-deploy ones.
- Entity update: `crates/shared/db/src/entity/host_software_item.rs` `Model` gains all three
  fields;
  all existing `ActiveModel` construction sites (including the `insert_host_link` test helper used
  by discovery tests) are updated.

## Error handling

- Errored `DiscoveryPluginResult` → reconciliation skipped for that discovery source, existing
  warn-level logging retained; DB state untouched.
- Reconciliation failures roll back the reconciliation transaction (`rootcause::Report`, no
  `unwrap()`); the upsert flow's results stand independently.
- Skills plugin lock-file errors surface as typed `SkillsError` variants through the existing
  error plumbing.

## Testing

- **Reconciliation unit tests** (SQLite in-memory, FK parents inserted per workspace test rules):
  - previously discovered link absent from two consecutive error-free snapshots (second miss past
    the minimum age) → deactivated; absent once then present again → `missing_since` cleared,
    stays active; two misses inside the age floor → not deactivated; provenance-`NULL` (manual)
    link absent → untouched
  - link with `InProgress` update at snapshot time: present → `missing_since` still cleared
    (presence-clear precedes the skip); absent → skipped this cycle (no `missing_since` stamp),
    reconciled after the update reaches a terminal state; link with `AwaitingRestart` →
    deactivated (not deferred)
  - reactivated link requires two fresh misses (`missing_since` reset on reactivation)
  - matching uses **effective** package identifiers (skills encoded-URL form, PHS target
    `owner/repo`) — a still-installed skill is never falsely deactivated
  - candidates selected by `discovery_source` — PHS-discovered links (management plugin types
    `releases_github` etc.) reconcile under the PHS discovery source
  - **empty error-free snapshot deactivates all previously discovered links of that source**
    (the early-`continue` removal)
  - last-host cascade deactivates `software_item`; multi-host does not
  - errored plugin result → no deactivation; discovery source absent from payload → no
    deactivation
  - effective-identifier set vs ignore-list: ignored-but-linked item still reported → stays active
  - qualifier-bearing links reconcile within their qualifier
  - rediscovery reactivates deactivated link + item in-place, refreshes provenance columns;
    reactivation with an existing active duplicate prefers the active row (no unique-index
    collision); cascade-reactivation with an existing active same-name item re-points the link
    (no `uq_software_items_active_name` collision)
  - deactivated links excluded from update-batch candidate selection
  - in-flight `update_history` (`InProgress`, `AwaitingRestart`) → `Failed` with reason;
    terminal rows untouched
  - audit events emitted for deactivate / cascade / reactivate / terminate
- **Skills plugin tests** (`FixedOutputExecutor`): missing lock file → empty/`None` without error;
  unreadable/corrupt lock file → discovery `Err` and per-item detect errors.
- **PHS plugin tests**: failed `/usr/bin/update` read → `Err` (not empty); per-item fetch failure
  → `error` set on the result (no silent partial snapshot).
- Full workspace quality gates (fmt, check ×2 feature sets, clippy ×2, test, deny, markdownlint).

## Documentation deliverables

- New ADR in `docs/adr/` — _Discovery-based software uninstall reconciliation_ (lifecycle:
  auto-deactivation, cascade, in-place reactivation, provenance columns, failure-vs-empty
  contract). Number
  assigned at implementation time (0026 is reserved by the pending OpenAPI-client drift-guard
  spec).
- `docs/development/plugin-guidelines.md` — discovery/detection error-vs-empty contract for plugin
  authors.
- No README/user-guide impact: behavior change is "stale uninstalled software disappears", which is
  the documented intent of discovery; no new config surface.

## Dependencies

None added.
