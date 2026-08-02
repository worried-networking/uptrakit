# 0021 — Installed Version Enrichment Role

Date: 2026-06-17

## Status

Accepted.

## Context

Some plugins represent installed software versions as opaque identifiers — for example, the LLM Skills plugin uses a git tree SHA
(`skillFolderHash` from `~/.agents/.skill-lock.json`) as its installed version. Rendering the raw identifier in the Dashboard produces
unreadable rows next to plugins like Docker that already show human-friendly labels (commit date, semver tag).

Translating the identifier to a display string sometimes requires upstream metadata (e.g. a GitHub commits-by-path API call) that only the
controller can reach — the agent has no global GitHub provider. The existing `ReleaseFetcher` role (controller-only, ADR-0015 context
injection) covers the _latest_ side. The _installed_ side previously had no controller-side hook.

A naïve fix would be to inline a `match plugin_type` in `handle_version_check_results` and call a Skills-specific helper. That violates
ADR-0018 (typed plugin extension boundary): the web-api would gain plugin-type knowledge.

## Decision

Add a new typed plugin role `InstalledVersionEnricher`:

- Controller-only async trait with one method,
  `enrich_installed_versions(items: &[InstalledVersionItem]) -> Result<Vec<InstalledVersionDisplay>>`.
- Returned `Vec` is the same length and order as `items`; dispatcher zips by index, not by `package_identifier`, so two host_software_item
  rows sharing a package_identifier with different SHAs stay distinct.
- `InstalledVersionDisplay` carries `installed_version_echo` for sanity-check; mismatch logs `race_skipped` and writes `None`.
- Bespoke `InstalledVersionEnricherSlot` mirroring `ReleaseFetcherSlot` (3-arg factory).
- New capability bit `PluginCapability::EnrichInstalledVersion` gates dispatch.
- `InstalledVersionEnrichmentContext` mirrors `ReleaseFetchContext` from ADR-0015 and carries the optional `GlobalProviderLookup`.

`handle_version_check_results` dispatches purely via the typed registry:

- Resolve `host_software_item_id → plugin_type` via `host_software_item_plugin` join (tenant-scoped through `software_item`).
- For each plugin_type with the capability + slot, instantiate the enricher and call once with the per-group batch.
- Verify length + echo per item; fold display values into a `HashMap<host_software_item_id, Option<String>>` side-channel.
- Thread the override into the existing single `update_many` that already writes `InstalledVersion` and `InstalledDisplayVersion` atomically
  (`messages.rs:758-789`).

## Write semantics

`installed_display_version` is always overwritten alongside `installed_version` in the same UPDATE. Enricher miss / throttle / out-of-window
→ write `None`. Prior display values are never preserved across a SHA change — they would map to the wrong SHA.

## Observability

`warn!` logs distinguish four reason tags:

- `provider_error` — Throttled, AuthFailed, transient network, non-404 HTTP.
- `upstream_gone` — strictly 404 from `commits?path=…`.
- `out_of_window` — walk completed but SHA never appeared (subsumes force-push past, path rename, fork-merge unreachability, SHA older than
  90 commits).
- `race_skipped` — length / echo / identifier mismatch from the enricher.

## Operational note: 90-commit ceiling

The Skills enricher caps the commits-by-path walk at 90 to bound API cost. If `out_of_window` becomes a common reason tag in production,
raise the cap or pair it with a persistent per-`(owner, repo, path)` SHA→date cache.

## Consequences

Consequences are captured in the Write semantics, Observability, and Operational note sections above.

## Alternatives considered

- **Plugin-type switch in web-api** — rejected: violates ADR-0018.
- **Agent-side lockfile extension** — rejected: couples the agent to upstream CLI internals; doesn't cover existing installs without
  controller-side backfill anyway.
- **Per-plugin-type ad-hoc roles** — rejected: same generic shape works for any plugin that surfaces opaque installed identifiers.
- **Storing `sha_history: Vec<{sha, committed_at}>` inside `latest_release_metadata`** — rejected: replaces the typed slot boundary with a
  stringly-typed JSON key; future plugin authors would couple via the blob shape rather than via the role trait.

## Deferred follow-ups

- **Per-plugin stored config resolution in dispatch.** The dispatch in `handle_version_check_results` currently constructs the enricher
  factory with `merged_cfg = serde_json::json!({})`. Skills works because `SkillsConfig` derives `Default` and all fields use
  `#[serde(default)]`. Any future enricher requiring stored config fields will need the dispatch path to resolve config from
  `plugin_configs` / `plugin_type_settings` via the same `merged_plugin_config` flow `scheduler-runtime::fetch_releases` uses for the
  latest side.
- **Persistent commit-date cache across cycles** so installed SHAs that did not change skip the per-cycle GitHub round-trip.
- **Re-enrichment endpoint** to retry past misses without waiting for the next scheduler cycle.

## Related

- ADR-0015 (Release-fetcher context) — sibling pattern for the latest side.
- ADR-0018 (Typed plugin extension boundary) — invariant preserved.
- Spec: `docs/superpowers/specs/2026-06-17-skills-version-display-design.md`.
