# 0027 — Discovery-Based Uninstall Reconciliation

Date: 2026-07-11

## Status

Accepted

## Context

Uptrakit reported phantom "update available" badges for software already uninstalled from a Host. The
observed instance: several Skills plugin items stayed visible as pending updates after being removed
locally, because the frozen version-check write path never clears `installed_version` when a plugin
correctly reports "not installed" (`installed_version: None`, no error) — the shared package-manager
convention (apt, homebrew, npm behave identically). `latest_version` keeps refreshing on every release
check, so `installed != latest` forever, with nothing in the system ever noticing the uninstall.

More broadly, Software Discovery only ever creates and re-matches `host_software_items` links — it never
deactivates one that stops appearing in a discovery snapshot. A related strand: when software was
uninstalled mid-update, the `update_history` row could sit in `AwaitingRestart` indefinitely, because the
post-update version check that would normally resolve it never arrives (there is no "installed" version
to detect).

See `docs/superpowers/specs/2026-07-02-discovery-uninstall-reconciliation-design.md` for the full root-cause
analysis and the rejected alternatives (per-item version-check `None` handling, and a hybrid of both
mechanisms).

## Decision

### Controller-side reconciliation, keyed on discovery provenance

Two nullable columns on `host_software_items` — `last_discovered_at` and `discovery_source` — record,
for every discovery create and every discovery re-match, when a link was last seen and which discovering
plugin (`DiscoveryPluginResult.plugin_type`) reported it. This is recorded independently of the link's
management-plugin role rows, since a discovery source and a target's execution plugin can differ (PVE
Helper Scripts discovery creates links whose role assignments target `releases.github`/`generic.shell`
plugins, for example).

Reconciliation only ever touches a link where both columns are non-`NULL`. A link created through the
software-items API directly has them permanently `NULL` and is therefore immune to auto-deactivation —
this is how "previously reported by discovery" scoping is expressed in data rather than inferred from a
creation code path.

Reconciliation runs once per error-free `DiscoveryPluginResult` (`result.error.is_none()`), including an
empty snapshot — an empty, error-free result is a positive assertion of "nothing installed" for that
source, not an absence of information. A result with `error` set, or a discovery source missing from the
payload entirely, is skipped: absence of a result means "did not run," never "found nothing."

### Presence-clear runs unconditionally, before any skip logic

For every candidate link whose `package_identifier` is present in the plugin's reported identifier set,
`missing_since` is cleared (if set) and `last_discovered_at` is refreshed. This happens before the
active-update defer check below, so an in-progress update never blocks the routine refresh of a link
that is still present.

### Two-miss hysteresis with a one-hour age floor

An absent candidate is not deactivated on the first miss. `missing_since` is stamped `now` only if unset;
a link is deactivated only once a **later** error-free snapshot also misses it **and** `now - missing_since`
has reached `RECONCILE_MIN_MISSING_AGE` (one hour). A miss recorded before the floor elapses leaves the
link untouched (`missing_since` retains its original stamp). This tolerates a single transient discovery
glitch and a reasonably fast re-check without producing a false deactivation, at the cost of a bounded
detection delay.

### Active-update defer, with `AwaitingRestart` excepted

An absent candidate with an `update_history` row in `InProgress`, `Pending`, or `Queued` is deferred
entirely — neither stamped nor deactivated — because an update may legitimately be rewriting the
package's installed state at snapshot time. `AwaitingRestart` deliberately does **not** defer: an
uninstalled item never produces the version-check result that would resolve an `AwaitingRestart` row, so
deferring it would strand it forever. This is why deactivation force-terminates non-terminal
`update_history` rows explicitly (see below) rather than relying on the defer set to eventually clear.

### Deactivation cascades to the last active link, and force-terminates stranded updates

Deactivating a link sets `deactivated_at`. If no other active link remains for its `software_item`, the
`software_item` itself is deactivated the same way, in the same transaction. Any `update_history` row
still in `UpdateStatus::unfinished()` for that link (in practice only `AwaitingRestart`, since the
active-update defer already excludes `InProgress`/`Pending`/`Queued` from reaching this point — the
broader filter is retained as defense-in-depth) is force-transitioned to `Failed`, with `completed_at`
stamped and `output` set to the fixed string `"software no longer installed on host"`.

### In-place reactivation on rediscovery, both collision rules included

Rediscovery of a previously-deactivated identity reactivates in place rather than creating a duplicate,
subject to two collision rules:

1. **Link-level.** If an active `host_software_item` already exists for the target
   `(host, software_item, qualifier)` key, that active row is preferred and updated; the deactivated
   duplicate is left untouched rather than reactivated alongside it.
2. **Item-level.** If cascade-reactivating a deactivated `software_item` would collide with an already
   active `software_item` of the same `(tenant_id, name)`, the link is re-pointed to the active item
   instead, and the originally deactivated item is left dormant.

Both rules exist because discovery match phases intentionally see deactivated rows — matching them is
what lets rediscovery reactivate in place instead of falling through to insert and creating a duplicate —
so a collision with something that reactivated or was created independently in the interim must be
resolved deterministically rather than by unconditionally clearing `deactivated_at`.

### Audit split: state transitions are Stateful, reactivation is Event

Deactivation, the software-item cascade, and update termination are state transitions this reconciliation
step owns end-to-end inside its own `BEGIN IMMEDIATE` transaction, so each emits a `Stateful` audit entry
in-tx with before/after snapshots. Reactivation happens inside the discovery upsert path
(`find_or_create_software_item`), which writes against a plain `DatabaseConnection` with no wrapping
transaction; it therefore cannot participate in an in-tx `Stateful` write without changing that path's
transaction shape, so it is emitted as an `Event` via the fire-and-forget path instead. Presence-clear
itself is not audited — it is a routine refresh, not a state transition.

### Update-batch candidate selection excludes deactivated links

Batch update candidate queries add a filter excluding `host_software_items` rows with `deactivated_at IS
NOT NULL`, alongside the pre-existing `software_items.deactivated_at` filter, so a deactivated link never
resurfaces as an update candidate between the moment it deactivates and any UI refresh.

### No backfill

Existing discovery-created rows only gain the new provenance columns on their next discovery match — they
become reconciliation-eligible one cycle later, not immediately on deploy. A row whose software was
uninstalled _before_ this feature deployed never produces a discovery match, never gains
`discovery_source`/`last_discovered_at`, and is therefore never auto-deactivated by this mechanism; such
pre-existing phantom rows require a one-time manual deletion via the existing software-item delete path.
This was an explicit trade-off against migrating provenance retroactively, which would have required
guessing a `discovery_source` for rows with no recorded discovery history.

### Plugin error contract

The universality of this mechanism depends on every discovery-capable plugin correctly distinguishing "no
error, nothing found" from "an error occurred" in its result — an empty, error-free snapshot is what
triggers deactivation. This contract is specified in `docs/development/plugin-guidelines.md`, not
duplicated here.

## Consequences

- Software uninstalled from a Host disappears from the Dashboard as a phantom pending update within at
  most two discovery intervals (default: 6 h scheduler cadence) after removal, bounded further by the
  one-hour age floor.
- A discovery plugin that fails silently and permanently — reporting an error on every run rather than an
  empty error-free result — never triggers reconciliation for its previously-discovered links; those links
  never deactivate through this mechanism. Accepted: the alternative (treating persistent errors as
  eventual uninstalls) would risk mass false deactivation from a broken plugin, which is a strictly worse
  failure mode than a stale badge.
- During a fleet upgrade where some Agents run an older build that predates this feature's discovery
  payload conventions, discovery results from those Agents behave as before — no regression, but no
  reconciliation either, until the Agent is upgraded. Accepted as a rollout-timing risk, not a defect.
- A link or item with no discovery provenance (manually added via the software-items API) is permanently
  immune to auto-deactivation, by construction of the `NULL`-provenance scoping — this is intentional, not
  an oversight.
- Pre-existing phantom rows from before this feature deployed are not retroactively cleaned up (see "No
  backfill") and require a one-time manual delete.

## Cross-references

- Spec: `docs/superpowers/specs/2026-07-02-discovery-uninstall-reconciliation-design.md`
- Plan: `uptrakit-spec-2026-07-02-discovery-uninstall-reconciliation-design` (plan retired at beads
  migration 2026-08-16; full text at `pre-beads-archive`)
- Reconciliation: `crates/ui/web-api-queries/src/queries/autodiscovery/reconcile.rs`
- Reactivation collision rules: `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`
  (`find_or_create_software_item`)
- Migration: `crates/shared/db/src/migration/m20260702_000001_hsi_discovery_provenance.rs`
- Update-batch candidate exclusion: `crates/ui/web-api-queries/src/queries/update_batches/candidates.rs`
- Discovery payload assembly: `crates/core/scheduler-runtime/src/executors/discover_software.rs`
  (`build_assignments`)
- Plugin error contract: `docs/development/plugin-guidelines.md`
- `CONTEXT.md` — Software Discovery glossary entry
