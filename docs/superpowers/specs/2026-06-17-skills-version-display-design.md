# Skills Version Display Design

Date: 2026-06-17

## Summary

The `package_manager_skills` plugin currently exposes raw 40-character git tree SHAs as both `installed_version` and `latest_version` for
every installed LLM Skill. The Dashboard renders these SHAs verbatim, producing unreadable rows next to other plugins (Docker, GitHub) that
already show human-friendly labels.

This design replaces the displayed value with the corresponding git commit date in ISO 8601 UTC (e.g. `"2026-06-11T01:15:00Z"`), matching
the Docker row presentation. The canonical version values used for change detection — the tree SHAs — are not modified. Only the
`display_version` / `installed_display_version` fields change.

The agent does not have access to the global GitHub provider, so `installed_version` enrichment must happen on the controller. To keep the
web-api free of plugin-type-specific branches (per ADR-0018), a new typed plugin role `InstalledVersionEnricher` is introduced. The role
mirrors the existing controller-side `ReleaseFetcher` pattern from ADR-0015 and is exercised by the web-api through a generic registry
lookup.

## Goals

- Show a date instead of a SHA in the Skills row of the software item list, for both installed and latest columns.
- Make the canonical SHA values used for change detection unaffected.
- Keep web-api plugin-agnostic — no `match plugin_type` branches.
- Reuse the controller-side provider injection pattern already established for `ReleaseFetcher`.
- Reuse the existing frontend display pipeline (`resolveDisplayVersion` → `formatVersion`) without per-plugin special cases.

## Non-Goals

- Persistent commit-date cache across scheduler cycles.
- Re-enrichment endpoint to retry past misses without waiting for the next cycle.
- Display enrichment for plugins other than Skills (Docker already populates `display_version` via its own controller-side `batch_fetch`).
- Extending the `~/.agents/.skill-lock.json` lockfile format or coupling to the upstream skills CLI.
- DB schema changes — `installed_display_version` already exists; latest-side display is carried via `latest_release_metadata` as today.
- Backfill — the existing rows are overwritten on the next scheduler cycle by design.

## Current Context

`crates/plugins/package-managers/skills/src/releases.rs`:

- `ReleaseFetcher::batch_fetch` builds an `UpstreamRelease` from a GitHub Trees API call. It sets `version = Version::new(sha)`,
  `tag = sha`, leaves `display_version = None`.

`crates/plugins/package-managers/skills/src/detection.rs`:

- `VersionDetector::batch_detect` reads `skill_folder_hash` from `~/.agents/.skill-lock.json` and returns it as the installed `Version`. No
  `display_version` is produced — the agent has no GitHub provider.

`crates/core/scheduler-runtime/src/executors/fetch_releases.rs`:

- Phase A (controller-side fetch_releases) writes the serialized `UpstreamRelease` into `host_software_item.latest_release_metadata`.
  Frontend extracts `display_version` from this blob via `extract_release_info` in
  `crates/ui/web-api-queries/src/queries/software_states.rs`.

`crates/ui/web-api/src/routes/service_ws/handler/messages.rs`:

- `handle_version_check_results` aggregates detect_version results from the agent and writes `installed_version` +
  `installed_display_version` to `host_software_item`. Today there is no enrichment hook between aggregation and write.

`crates/shared/global-github-provider/src/lib.rs`:

- `GitHubProviderClient` trait exposes `fetch_repository_tree` only. Tree entries carry only `(path, kind, sha)` — no commit timestamps. To
  learn when a given tree-at-path SHA was produced, a separate `GET /repos/{owner}/{repo}/commits?path={skill_dir}&per_page=N` paged walk is
  required.

`frontend/src/lib/utils.ts`:

- `resolveDisplayVersion(version, displayVersion)` prefers `displayVersion ?? version`.
- `formatVersion` detects ISO 8601 (`YYYY-MM-DDTHH:MM:SSZ`) and renders via browser locale (`"3 Jun 2026 at 19:09"`). It already shortens
  `sha256:` digests to first 12 hex characters.

This creates two gaps:

- the controller-side `ReleaseFetcher` does not set `display_version` for Skills, so the frontend falls back to the raw SHA;
- the agent-side `VersionDetector` cannot set `display_version` because it has no upstream metadata.

## Decision

Introduce a new typed plugin role, `InstalledVersionEnricher`, that runs controller-side and post-processes agent-reported installed
versions. Web-api dispatches by descriptor slot, not by plugin type. The Skills plugin implements both `ReleaseFetcher` (latest) and
`InstalledVersionEnricher` (installed) and shares a single new GitHub provider primitive (`list_recent_commit_dates_for_path`) between them.
The frontend gains one line of fallback for raw 40-hex SHAs.

## Architecture

### 1. New plugin role: `InstalledVersionEnricher`

Located in `crates/plugins/infrastructure/core/src/roles.rs`. Controller-only async trait, signature:

```rust
#[async_trait]
pub trait InstalledVersionEnricher: Send + Sync {
    /// Returns a `Vec` of the same length and order as `items`. The
    /// dispatcher zips input ↔ output **by index**, not by `package_identifier`,
    /// so two items sharing a `package_identifier` (e.g. the same Skill
    /// installed on two hosts with different SHAs) are kept distinct.
    /// Implementors must preserve order; the dispatcher checks length on return
    /// and treats a mismatch as a fatal contract violation (warn + drop the
    /// whole batch's display values to `None`).
    ///
    /// **`None`-input contract**: if `items[i].installed_version` is `None`,
    /// implementors MUST return `display_version = None` for that index.
    /// The dispatcher does not police this beyond the echo check, but
    /// returning a phantom display for an unknown installed SHA is a
    /// trait-contract violation.
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>>;
}

#[non_exhaustive]
pub struct InstalledVersionItem {
    pub package_identifier: String,
    pub installed_version: Option<String>,
}

#[non_exhaustive]
pub struct InstalledVersionDisplay {
    /// Echo of the input `package_identifier` for sanity-check logging. The
    /// dispatcher does not key on it — see the trait doc — but a mismatch is
    /// logged at `warn!` as an impl-contract violation.
    pub package_identifier: String,
    /// Echo of the `installed_version` the dispatcher passed in. The dispatcher
    /// verifies `installed_version_echo == items[i].installed_version` before
    /// writing `display_version`. Mismatch is impossible under the
    /// per-`host_software_item` serialization invariant `handle_version_check_results`
    /// already enforces (a single UPDATE atomically writes `installed_version`
    /// and `installed_display_version` together — see `messages.rs:758-789`),
    /// but the echo doubles as a contract assertion: if it ever mismatches,
    /// the dispatcher writes `display_version = None` with reason tag
    /// `race_skipped`, never pairing a stale display with a fresh SHA.
    pub installed_version_echo: Option<String>,
    pub display_version: Option<String>,
}
```

Both new structs carry `#[non_exhaustive]` per the standards-snapshot binding rule on extensible public structs. The trait uses
`async_trait` per workspace convention. Errors propagate as `rootcause::Report<PluginError>` via the existing `Result<T>` alias.

The role slot is a bespoke struct, **not** a `RoleSlot<R>` instantiation. `RoleSlot<R>` in
`crates/plugins/infrastructure/core/src/descriptor.rs:453` carries a 2-arg `CreateRoleFn<R>` (config + runtime). `InstalledVersionEnricher`
needs a 3-arg factory (config + runtime + context), exactly mirroring how `ReleaseFetcher` already does it via `ReleaseFetcherSlot` +
`CreateReleaseFetcherFn` (`descriptor.rs:250-269`):

```rust
pub type CreateInstalledVersionEnricherFn = fn(
    &serde_json::Value,                              // merged plugin config
    Arc<dyn HostRuntime>,
    &InstalledVersionEnrichmentContext,
) -> Result<Box<dyn InstalledVersionEnricher>>;

#[non_exhaustive]
pub struct InstalledVersionEnricherSlot {
    pub create: CreateInstalledVersionEnricherFn,
    pub host_requirements: HostRequirements,
}

impl InstalledVersionEnricherSlot {
    pub const fn new(
        create: CreateInstalledVersionEnricherFn,
        host_requirements: HostRequirements,
    ) -> Self { Self { create, host_requirements } }
}

// Container struct is `RoleCreators` (not `PluginRoleSlots`), defined at
// `crates/plugins/infrastructure/core/src/descriptor.rs:499`. It is NOT
// `#[non_exhaustive]`; only the `declare_plugin!` macro constructs it, so
// adding a field is safe.
pub struct RoleCreators {
    // ... existing fields ...
    pub installed_version_enricher: Option<InstalledVersionEnricherSlot>,
}
```

### 2. New capability bit: `EnrichInstalledVersion`

Added to `PluginCapability` in `crates/shared/types/src/plugin_capability.rs`. The web-api uses this bit to filter eligible plugins before
dispatching to the slot, parallel to how `ControllerSideFetchReleases` is used in
`crates/core/scheduler-runtime/src/executors/fetch_releases.rs:287-293`.

Plugins declare the capability via `extra_capabilities` in `declare_plugin!`, the same way they declare `ControllerSideFetchReleases` today.

### 3. New context: `InstalledVersionEnrichmentContext`

Mirrors `ReleaseFetchContext` from ADR-0015:

```rust
#[non_exhaustive]
pub struct InstalledVersionEnrichmentContext {
    pub global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
}
```

`with_lookup_opt` constructor follows the existing `ReleaseFetchContext::with_lookup_opt` shape. Behind `#[cfg(feature = "catalog")]`
exactly like `ReleaseFetchContext`.

### 4. `declare_plugin!` macro extensions

`crates/plugins/infrastructure/core/src/macros.rs`:

- Add `InstalledVersionEnricher { host_requirements: ... }` role arm to `__accumulate_role_caps!`. The arm contributes **no** implicit
  capability — the gating bit `EnrichInstalledVersion` is declared explicitly via `extra_capabilities:` in `declare_plugin!`. This matches
  the `ReleaseFetcher` precedent (the implicit `ReleaseFetching` cap is purely informational; `ControllerSideFetchReleases` — which actually
  drives routing — is also declared via `extra_capabilities:`).
- Add new top-level field `installed_version_enricher_create:` parallel to `release_fetcher_create:`. The macro wires this to the
  descriptor's `roles.installed_version_enricher` slot.

The macro change is mechanical and additive; existing plugins that do not declare the role compile unchanged.

### 5. GitHub provider primitive: `list_recent_commit_dates_for_path`

`crates/shared/global-github-provider/src/lib.rs`:

```rust
#[non_exhaustive]
pub struct TreeCommit {
    pub tree_sha_at_path: String,
    pub committed_at: OffsetDateTime,
}

#[async_trait]
pub trait GitHubProviderClient: Send + Sync {
    // existing methods ...

    /// Default impl returns `Misconfigured("list_recent_commit_dates_for_path not implemented")`
    /// so existing implementors (production `GitHubProviderRuntime`, plus the eight test
    /// doubles enumerated below) compile unchanged. Skills's enricher treats this error
    /// like any other provider failure: log + write `None`.
    async fn list_recent_commit_dates_for_path(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        limit: usize,
        expected_shas: &std::collections::HashSet<String>,
    ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
        let _ = (consumer, owner, repo, path, limit, expected_shas);
        Err(GitHubProviderError::Misconfigured(
            "list_recent_commit_dates_for_path not implemented".to_string(),
        ))
    }
}
```

Production impl: `GitHubProviderRuntime` (`crates/ui/web-api/src/global_providers/github.rs:445`). Test doubles required to override:
`FakeProvider` / `CountingProvider` / `ThrottledProvider` / `AuthFailedProvider` in
`crates/plugins/package-managers/skills/src/releases.rs:359-545`; the four impls in
`crates/plugins/enhancements/dashboard-icons/src/cache.rs` and `plugin.rs` keep the default. Reuse the existing `PACKAGE_MANAGER_SKILLS`
consumer constant (`crates/shared/global-github-provider/src/lib.rs:28`); do not introduce a new ID.

Implementation:

1. Paged walk of `GET /repos/{owner}/{repo}/commits?path={path}&per_page=30`, capped at **3 pages × 30 = 90 commits**. Each returned commit
   carries `commit.committer.date` and `commit.author.date`, plus `commit.tree.sha` (the commit's **root** tree SHA, not the subtree at
   `path`). Use **committer date** (`commit.committer.date`) as `committed_at` — author date is sensitive to rebases and cherry-picks,
   committer date reflects when the SHA actually entered the upstream branch and matches Skill update cadence semantics.
2. Walk the returned commits **oldest→newest** while keeping a `HashMap<root_tree_sha, subtree_sha_at_path>` cache scoped to the entire
   `list_recent_commit_dates_for_path` call. For each commit, if its `root_tree_sha` is in the cache, reuse; otherwise issue a
   **non-recursive** `GET /repos/{owner}/{repo}/git/trees/{root_tree_sha}` and descend the path segments one level at a time, populating
   intermediate `root_tree_sha → child_tree_sha` cache entries on the way down. Non-recursive tree fetches return ~50–500 entries (one
   directory level) instead of the entire repo, dramatically cheaper than recursive — and consecutive commits frequently share unchanged
   intermediate trees, so the cache hit rate on a hot path is high.
3. If callers can pass a non-empty `expected_shas: &HashSet<String>` (the InstalledVersionEnricher does — it knows the installed SHAs ahead
   of time), short-circuit the walk as soon as every entry in `expected_shas` has been bound to a `(subtree_sha_at_path, committed_at)`
   pair. The latest-release caller passes an empty set and gets the full 90-entry walk.

Cost envelope (worst case, no shared intermediate trees): up to 1 paged-commits call (3 pages) + up to 90 non-recursive tree calls per
`(owner, repo, path)` per scheduler cycle. Real-world case (shared intermediate trees + short-circuit on bound installed SHAs): closer to 1
paged-commits + ~3–10 tree calls per call. Authenticated GitHub rate limit is 5000/hour; the cost envelope coexists with `releases-github`
and `dashboard-icons` consumers without depleting the budget. Unauthenticated deployments are out of scope — Skills enrichment requires the
global GitHub provider, which mandates auth.

`limit` parameter is bounded — callers pass `90`. The implementation clamps to `min(limit, 90)` defensively so a future caller cannot
accidentally request an unbounded walk.

Result: a list of `TreeCommit` records ordered newest-first.

`GitHubProviderError` already carries `Throttled`, `AuthFailed`, `Misconfigured`, plus a generic variant — no error-enum changes needed.

The primitive is added to the trait once and reused by both `ReleaseFetcher::batch_fetch` (top entry → latest commit date) and
`InstalledVersionEnricher::enrich_installed_versions` (map all 90 entries by SHA → installed commit date).

### 6. Skills plugin changes

`crates/plugins/package-managers/skills/src/releases.rs` (`ReleaseFetcher::batch_fetch`):

- For each `(owner, repo, skill_dir)` group, invoke `provider.list_recent_commit_dates_for_path(consumer, owner, repo, skill_dir, 90)`
  instead of (or in addition to) the existing `fetch_repository_tree` call. Cache the result in a
  `HashMap<(String, String, String), Result<Vec<TreeCommit>, String>>` for the duration of the batch.
- For the latest release, use entry `[0]` (newest commit) — `tree_sha_at_path` becomes `Version`, `tag`, and `release_url` source;
  `committed_at.format(DISPLAY_FMT)?` becomes `display_version`. `DISPLAY_FMT` is an explicit
  `time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z")` — do **not** use
  `time::format_description::well_known::Rfc3339`, which emits subsecond precision when the underlying `OffsetDateTime` carries it and
  breaks the frontend regex.
- Behavior when the call fails: existing `map_provider_error` posture — log `warn!`, return `BatchFetchResult::error`, never panic.

`crates/plugins/package-managers/skills/src/releases.rs` (new file or inline impl — `InstalledVersionEnricher`):

```rust
#[async_trait]
impl InstalledVersionEnricher for SkillsPlugin {
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>> { ... }
}
```

- For each item, parse the `package_identifier` into `(owner, repo, skill_dir)` via the existing `parse_skill_identifier` +
  `parse_github_owner_repo` + `derive_skill_dir` helpers in `releases.rs`.
- Group by `(owner, repo, skill_dir)`; one provider call per group, capped at 90 entries.
- Build `HashMap<&str, &OffsetDateTime>` keyed on `tree_sha_at_path`. For each item's `installed_version`, look up — hit → format as RFC
  3339 UTC string; miss → `None`.
- Provider absent (standalone-scheduler deployments without `catalog` feature) → all items return `display_version = None`. Same posture as
  `ReleaseFetcher` in the same scenario.

`crates/plugins/package-managers/skills/src/plugin.rs`:

- Add `InstalledVersionEnricher { host_requirements: HostRequirements::CONTROLLER_ONLY }` to the `roles:` list in `declare_plugin!`.
- Extend `extra_capabilities` to `[ControllerSideFetchReleases, EnrichInstalledVersion]`.
- Add `installed_version_enricher_create:` factory parallel to `release_fetcher_create:`. The factory reuses
  `lookup_github_provider_from_ctx` to extract the provider client from `InstalledVersionEnrichmentContext`.

The two factories are nearly identical; consider extracting a shared helper if the duplication is noisy. Optional, not required for landing.

### 7. Web-api dispatch in `handle_version_check_results`

`crates/ui/web-api/src/routes/service_ws/handler/messages.rs::handle_version_check_results`:

**Plugin-type sourcing** — `VersionCheckResult` (`crates/shared/wire/src/payloads.rs:409`) does **not** carry `plugin_type`; the agent never
echoes it back. The controller therefore must resolve plugin_type per `(host_software_item_id, role = 'detect_version')` row before
dispatch. Add a single batched lookup at the top of `handle_version_check_results`:

```rust
// Pseudocode shape — actual query lives in web-api-queries.
let plugin_types: HashMap<Uuid /* host_software_item_id */, PluginTypeId> =
    queries::host_software_item_plugins::plugin_types_for_role(
        &state.db, &tenant_id, &hsi_ids, "detect_version",
    ).await?;
```

`host_software_item_plugin` already has `(host_software_item_id, role, plugin_type)`; the query is
`SELECT host_software_item_id, plugin_type FROM host_software_item_plugin WHERE host_software_item_id IN (...) AND role = 'detect_version'`.
Tenant isolation: `host_software_item_plugin::Entity` is **not** in `TenantScoped`, so the call must join through `software_item::Entity`
(which **is** `TenantScoped`) via `Relation::SoftwareItem`:

```rust
tenant_db.find_via_tenant_join::<host_software_item_plugin::Entity, software_item::Entity>(
    host_software_item_plugin::Relation::SoftwareItem.def(),
)
```

Per the standards-snapshot binding rule, no raw `host_software_item_plugin::Entity::find()` calls — they would bypass tenant scoping.

**Dispatch (after aggregation, before write)** — index-keyed side-channel (NOT keyed on `package_identifier`, because two host_software_item
rows can share an identifier with different installed SHAs):

```rust
// Per host_software_item_id → enrichment outcome.
let mut enriched: HashMap<Uuid /* host_software_item_id */, Option<String>> = HashMap::new();
```

- Group the aggregated detect_version results by `plugin_type_id` (looked up via the map above). Within each group, build a
  `Vec<(Uuid, InstalledVersionItem)>` preserving insertion order so the dispatcher can zip the enricher's returned `Vec` back to the per-row
  outcome by index.
- For each plugin_type, fetch the descriptor via the existing `uptrakit_plugin_infrastructure_registry::get_descriptor(plugin_type)`. Skip
  if the descriptor is missing, the capability list does not contain `EnrichInstalledVersion`, or `roles.installed_version_enricher` is
  `None`.
- Otherwise: build `InstalledVersionEnrichmentContext::with_lookup_opt(state.plugin.global_providers.clone())`, instantiate the enricher via
  the slot factory, and call `enrich_installed_versions(&items)` once with the per-plugin-type batch.
- On return, assert `output.len() == items.len()`; on mismatch log `warn!` with reason `race_skipped` and fold `None` for every row in the
  group. On length match, zip by index: for each `(hsi_id, item)` ↔ `display`, verify
  `display.installed_version_echo == item.installed_version` — match → insert `display.display_version` into `enriched[hsi_id]`; mismatch →
  log `warn!` reason `race_skipped` and insert `None`.
- On any enricher error: log `warn!` with reason `provider_error`, leave every entry in `enriched` for the group as `None`, continue. Never
  abort the batch. Same posture as `controller-side fetch_releases failed for package; skipping` at `fetch_releases.rs:400`.

**Write-path threading** — `apply_version_update_to_db` (`messages.rs:747`) currently writes `result.installed_display_version.clone()`
straight from the wire payload. Change the signature to take an additional `Option<String>` override:

```rust
async fn apply_version_update_to_db(
    state: &AppState,
    result: &VersionCheckResult,
    installed_display_version_override: Option<String>,
) -> Result<...> { ... }
```

The caller passes the value looked up from `enriched` by `host_software_item_id`. When no enricher applies, the caller passes
`result.installed_display_version.clone()` — preserving today's behavior. When an enricher applies, the caller passes the enricher's value
(which may be `None` — that's the explicit overwrite-with-None case in section 8). No clone of `VersionCheckResult` required.

**Atomic-update invariant** — the existing `apply_version_update_to_db` builds a single `host_software_item::Entity::update_many()` that
sets `InstalledVersion` and `InstalledDisplayVersion` together (`messages.rs:758-789`). The new override threads through that same single
UPDATE, so the two columns remain atomically paired — no possibility of a `(SHA_new, display_old)` window between two queries.

No `plugin_type` string match anywhere in the dispatch — purely typed registry lookup. ADR-0018 compliance.

### 8. Write semantics

`installed_display_version` and the metadata-side `display_version` are always overwritten alongside the version column. When the enricher
returns `None`, write `None` — never preserve the prior value, since the prior display string corresponds to the prior SHA, not the new one.

`installed_version` write path in `messages.rs` already always writes both columns; the enrichment step replaces the previously-hardcoded
`None` with the enricher's result. Latest-side: Phase A in `fetch_releases.rs:438-470` already overwrites `latest_release_metadata` whole
every cycle; populating `UpstreamRelease.display_version` in Skills's `batch_fetch` makes the new field flow through naturally.

### 9. Frontend fallback in `formatVersion`

`frontend/src/lib/utils.ts`:

- Add one branch after the existing `sha256:` shortener: if the raw value matches `/^[0-9a-f]{40}$/i`, render `<first 12>…`. This is the
  fallback when an enricher misses (90-commit window exceeded, provider throttled, lookup fails). The change is plugin-agnostic — any plugin
  that surfaces a bare git SHA benefits.
- Unit test in vitest asserting the new branch.

No change to `resolveDisplayVersion` — the existing `displayVersion ?? canonicalVersion` already does the right thing.

### 10. Error handling and observability

- All new error types thread through `rootcause::Report<PluginError>` via the existing `Result<T>` alias in
  `uptrakit_plugin_infrastructure_core`.
- `GitHubProviderError::Throttled` continues to map to `PluginError::PluginInternal("GitHub rate limit exceeded")` via the existing
  `map_provider_error` helper.
- The web-api dispatch step does not surface enricher errors to the agent or client — it logs and writes `None`. The agent's detect_version
  result still lands in DB with the raw SHA.
- No `unwrap` / `expect` / `panic!` in production paths. Tests may use `expect` per the existing pattern.

**Observability** — distinguish miss reasons in `warn!` logs so out-of-window misses do not look like provider failures:

| Reason tag       | Trigger                                                                                                                                                                                                   | Frontend effect    |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| `provider_error` | `Throttled`, `AuthFailed`, transient network failure, any non-404 HTTP failure                                                                                                                            | Short-SHA fallback |
| `upstream_gone`  | Strictly: `commits?path=…` returns `404` (`Gone`/`Not Found`) — repo deleted or made private                                                                                                              | Short-SHA fallback |
| `out_of_window`  | Walk completed successfully but the installed SHA never appeared. Subsumes: SHA older than 90 commits, path renamed in history, force-push past the SHA, fork-merge tree SHA not reachable from this path | Short-SHA fallback |
| `race_skipped`   | Enricher returned wrong length, mismatched `installed_version_echo`, or mismatched `package_identifier` echo                                                                                              | Short-SHA fallback |

Reason field is a `&'static str` tag on the `warn!` event. Implementors add a counter (per-tenant or per-`(plugin_type, reason)`) only if
telemetry plumbing already exists; otherwise just the log is sufficient.

**Edge cases explicitly in scope** — all four reason tags above must be exercised by integration-test stubs (slice 6). Force-push, path
rename in history, fork-merge with non-reachable tree SHAs, and deleted/private upstream all collapse to one of `out_of_window` or
`upstream_gone` and never panic.

### 11. Testing strategy (TDD slices)

Slices to land in order, each red→green with the assertions inline:

1. **Provider primitive.** `crates/shared/global-github-provider/`: red test — `list_recent_commit_dates_for_path` returns a vector of
   `TreeCommit` in newest-first order, with stub HTTP responses covering single-page and three-page walks. Assert `limit` is honored. Assert
   `Throttled` propagates.
2. **Role infrastructure.** `crates/plugins/infrastructure/core/`: red test — `__accumulate_role_caps!` accepts
   `InstalledVersionEnricher { ... }` role arm; descriptor builder exposes the new slot; capability bit `EnrichInstalledVersion` round-trips
   through `PluginCapability` serde and `KNOWN_VARIANTS`.
3. **Skills `ReleaseFetcher::batch_fetch`.** Existing test pattern (`FakeProvider` in `releases.rs`): extend with a fake that returns
   `TreeCommit` entries; assert `UpstreamRelease.display_version` is **exactly** the string `"2026-06-11T01:15:00Z"` for the top entry —
   exact-string equality is load-bearing because the frontend regex requires no fractional seconds. Assert `version` and `tag` still carry
   the SHA.
4. **Skills `InstalledVersionEnricher::enrich_installed_versions`.** New unit test in `releases.rs` tests module: fake provider returns
   three `TreeCommit` entries; items request two SHAs (one match, one miss); assert one `Some(rfc3339)`, one `None`. Assert only one
   provider call per `(owner, repo, skill_dir)` (counter pattern from existing `CountingProvider`).
5. **Plugin-type lookup query + `apply_version_update_to_db` override param.** Prerequisite for slice 6. Red: unit test in
   `crates/ui/web-api-queries/` for `plugin_types_for_role` returning a map keyed by `host_software_item_id` for `role = 'detect_version'`,
   scoped via `TenantDb::find_via_tenant_join`. **Negative case required**: fixture must include a row from a _different_ tenant covering
   the same `host_software_item_id` namespace — assert that row is excluded. Red: change `apply_version_update_to_db`'s signature to accept
   `installed_display_version_override: Option<String>`; existing tests pass `result.installed_display_version.clone()` and remain green.
   Green: query + signature change land together.
6. **Web-api dispatch.** `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` tests module: existing `handle_version_check_results`
   test scaffolding. Add a case using a stub plugin descriptor declaring `EnrichInstalledVersion` + `InstalledVersionEnricher` slot; assert
   the enricher is invoked once, override flows into `apply_version_update_to_db`, and `installed_display_version` is written. Add a
   separate case where the enricher returns `None` for an item that previously had a non-null `installed_display_version` in DB — assert
   post-write column is `None` (the explicit overwrite-with-None requirement from section 8). Add a case for a plugin without the capability
   — assert the slot is not invoked and the original `result.installed_display_version` flows through unchanged.
7. **Skills descriptor capabilities.** Extend the existing `descriptor_capabilities` test in
   `crates/plugins/package-managers/skills/src/plugin.rs` to assert `EnrichInstalledVersion` is present, alongside
   `ControllerSideFetchReleases`.
8. **Frontend.** `frontend/src/lib/utils.test.ts` (or peer): assert `formatVersion("f260c775073816860fef8a37c032ac77e2ff5821")` returns
   `"f260c7750738…"`. Use the `…` escape in the TypeScript source, matching the existing `sha256:` shortener's style — not the literal `…`
   character.

All eight slices land with green tests before moving on. No skipped tests, no `--ignored` for the unit-level work.

### 12. Documentation deliverables

- **ADR-0021** (`docs/adr/0021-installed-version-enrichment-role.md`) — new. Covers the role, capability bit, context, web-api dispatch
  contract, write semantics, the 90-commit ceiling as an explicit known limitation (with the four reason tags from §10 covering why it might
  miss), and the operational note: if `out_of_window` becomes a common reason tag in production logs the cap should be revisited (raised, or
  paired with a persistent per-`(owner, repo, path)` SHA→date cache). Alternatives rejected: plugin-type switch in web-api, agent-side
  lockfile extension, splitting per-plugin-type roles, storing a `sha_history` blob inside `latest_release_metadata` — rejected because the
  JSON key would become a stringly-typed plugin contract, replacing the typed slot boundary the role pattern provides.
- **`docs/development/coding-standards.md`** — short subsection under the plugin section describing when to add `InstalledVersionEnricher`.
  One paragraph.
- **`crates/plugins/package-managers/skills/src/plugin.rs`** module doc comment — add one sentence noting the role declaration.
- **`CONTEXT.md`** — already updated inline during grilling (term: Installed Version Enricher).

No change to `frontend/AGENTS.md`, `AGENTS.md`, or README.

## Alternatives Considered

- **Plugin-type switch in web-api.** Cheapest delta — a single `if plugin_type == "package_manager_skills"` branch in
  `handle_version_check_results` would call a Skills-specific helper. Rejected: violates ADR-0018 (typed plugin extension boundary). Web-api
  stays plugin-agnostic.
- **Extend the agent's `.skill-lock.json`.** Persist commit date at install time so the agent's `batch_detect` returns it directly.
  Rejected: couples the agent to either upstream skills-CLI internals or a sidecar file; doesn't cover existing installs without backfill —
  backfill needs the same controller lookup we'd implement anyway.
- **Move `detect_version` to controller-side as a new role.** Architecturally clean but the host filesystem (`~/.agents/.skill-lock.json`)
  is only reachable by the agent; the controller would need an indirect "go ask the agent for the file" round-trip. Over-architecture for
  one plugin.

## Standards Snapshot Compliance

- `rootcause::Report` for all new error paths.
- `#[non_exhaustive]` on every new public struct (`InstalledVersionItem`, `InstalledVersionDisplay`, `TreeCommit`,
  `InstalledVersionEnrichmentContext`).
- No `unwrap` / `expect` / `panic!` in production code.
- `parking_lot::Mutex` would be required for any cross-task shared state; the per-batch cache is single-task scope and uses a plain
  `HashMap`, no lock needed.
- Trait methods use `#[async_trait]` per workspace convention.
- HTTP traffic goes through the global GitHub provider, which targets `api.github.com` (not user-controlled) — no `SsrfSafeResolver`
  requirement.
- No new HTTP request types, so no `Validate` trait impl required.
- No new external crates. `time::OffsetDateTime` and `time::macros::format_description!` (compile-time format) are already workspace
  dependencies.

## Verification

Quality gates (all must pass before merge):

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'

python3 ci/check_plugin_semantic_boundary.py
bash ci/verify_no_security_audit.sh
bash ci/verify_typed_audit_actions.sh
bash ci/verify_handler_state_contract.sh
python3 ci/verify_db_access_policy.py

cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Plus the sentrux architecture pass (`mcp__plugin_sentrux_sentrux__rescan` + `health`) before final commit, per AGENTS.md:320-333.

The new dispatch path in `handle_version_check_results` touches both handler state composition (`verify_handler_state_contract.sh`) and
plugin-boundary dispatch (`check_plugin_semantic_boundary.py`); both must pass.

Manual end-to-end check:

1. Rebuild controller (`cargo build -p uptrakit-controller`) and restart.
2. Trigger a scheduler `fetch_releases` cycle (wait or kick).
3. Confirm logs no longer emit the `"GitHub provider unavailable: skills release fetching requires the global GitHub provider"` error —
   already fixed in a prior change.
4. Confirm a Skills row in the Software Items list now shows a date like `"11 Jun 2026 at 01:15"` in the latest column instead of
   `f260c775073816860fef8a37c032ac77e2ff5821`.
5. Manually edit `~/.agents/.skill-lock.json` to roll a single skill back to an older `skillFolderHash` from the same path's history (within
   the last 90 commits) and trigger detect_version. Confirm the installed-version column shows an older date and the update arrow now points
   at a strictly newer date. Restore the lockfile after.
6. Throttle simulation: temporarily revoke the GitHub token and trigger a cycle. Confirm a warning is logged, `installed_display_version`
   and the metadata `display_version` are cleared to `NULL`, and the row falls back to short-SHA display (`f260c7750738…`) thanks to the new
   `formatVersion` branch. Re-enable the token.

Rollback: revert the eight slice commits in reverse order. No DB migration to undo. Old rows are re-populated on the next scheduler cycle.
