# Agent Skills Plugin Design

Date: 2026-05-13

## Summary

Add a new tenant-scoped package-manager plugin (`package_manager_skills`) that discovers, tracks, and updates
LLM-agent Skills installed globally on a Host via the `skills` CLI (`npx skills@latest …`).

Each Skill is a `Software Item`. The plugin's `skillFolderHash` (a git tree SHA from `~/.agents/.skill-lock.json`)
serves as the `installed_version`. The matching upstream `Release` is the same tree SHA, resolved on the
Controller by querying the GitHub git-trees API through the existing `uptrakit-global-github-provider`.
Updates run on the Agent via `npx skills@latest update -g <name> -y`.

The plugin reuses the existing `package-manager` plugin family (mirrors `package_manager_npm`) and the
existing `GlobalProviderConsumer` injection model (mirrors `dashboard-icons`). It introduces one narrow
contract change so that controller-side `ReleaseFetcher` instances may consume the global GitHub provider
in the same way singleton enhancement plugins already do today.

## Goals

- Discover globally-installed Skills on a Host, with one `Software Item` per Skill, attributed to the
  Host's Agent.
- Track each Skill's `installed_version` as the git tree SHA recorded in `~/.agents/.skill-lock.json`.
- Fetch the upstream Release on the Controller using the configured GitHub Provider credentials, falling
  back to anonymous GitHub API when no token is configured.
- Execute updates per-Skill via `npx skills@latest update -g <name> -y`, and let the next detection cycle
  observe the new tree SHA.
- Follow existing plugin idioms (`declare_plugin!`, `rootcause::Report`, `parking_lot` locks,
  `#[non_exhaustive]`, structured `tracing`).
- Reuse the existing GitHub-Provider auth path; do not introduce a parallel token-storage surface.

## Non-Goals

- Project-scoped Skills under `./.agents/`. Global only.
- Non-GitHub source URLs in `.skill-lock.json`. The plugin treats non-`github.com` sources as unsupported
  and emits a clean release-fetch error.
- Pinning to a specific Skill version via `execute_update(to_version)`. The CLI does not support pinning;
  `to_version` is recorded for audit but the executed command always moves the Skill to its upstream HEAD
  tree SHA. The detection cycle reconciles `installed_version` after the update lands.
- A new generic provider-pool abstraction. The plugin consumes the existing
  `uptrakit-global-github-provider` (option C from the design conversation), not a new pool.
- Hot-reload of GitHub Provider credentials. Existing reload semantics (catalog snapshot at boot) apply.
- Per-tenant override of the GitHub Provider token. Token is instance-level, exactly as today.

## Current Context

Today:

- `skills` (the npm CLI) stores per-host install metadata in `~/.agents/.skill-lock.json`. Each entry is keyed
  by Skill name and records: `source` (e.g. `obra/superpowers`), `sourceUrl`, `sourceType`
  (`github`), `skillPath` (e.g. `skills/brainstorming/SKILL.md`), `skillFolderHash` (git tree SHA),
  `installedAt`, `updatedAt`.
- `npx skills@latest list -g --json` enumerates installed Skills; `update -g [name] -y` upgrades them.
  No version pinning is supported.
- A GitHub Provider singleton already exists (`crates/ui/web-api/src/global_providers/github.rs`),
  governed by an instance owner with `ManageGlobalSettings`. It exposes a
  `GitHubProviderClient::fetch_repository_tree` contract through `uptrakit-global-github-provider`.
- The provider is currently injected only into singleton `SoftwareItemLifecycle` plugins via `CatalogConfig`
  (dashboard-icons today). `ReleaseFetcher` slot factories receive only `(config_json, runtime)` and have
  no path to the provider.
- The shared `RepositoryTreeEntry` / `GitHubTreeEntry` types carry `path` and `kind`, but drop the
  per-entry `sha`. The skills plugin needs that `sha` to resolve a Skill folder's git tree hash.

This creates two gaps that block the plugin from following the existing idiom:

1. `RepositoryTreeEntry.sha` is dropped by the github-client DTO parse and the shared global-github-provider
   contract.
2. `ReleaseFetcher` factories have no way to receive the global GitHub provider handle.

Both gaps are narrow and resolvable additively without altering existing plugin behaviour.

## Decision

Build `uptrakit-plugin-package-manager-skills` as a new crate at
`crates/plugins/package-managers/skills/`, following the layout of
`crates/plugins/package-managers/npm/`. The plugin is `PluginFamily::Software`, tenant-scoped, with the
following roles:

| Role              | Site        | Notes                                                                  |
| ----------------- | ----------- | ---------------------------------------------------------------------- |
| `Discoverer`      | Agent       | Reads `~/.agents/.skill-lock.json` to enumerate Skills.                |
| `VersionDetector` | Agent       | Reads the same lock file to resolve a Skill's `installed_version`.     |
| `ReleaseFetcher`  | Controller  | Uses the global GitHub Provider to fetch each source repo's git tree.  |
| `UpdateExecutor`  | Agent       | Runs `npx skills@latest update -g <name> -y`.                          |

To unblock the controller-side `ReleaseFetcher` from consuming the GitHub Provider, extend three existing
contracts narrowly (see §4 Cross-cutting changes).

## Architecture

### 1. Plugin shape

Crate name: `uptrakit-plugin-package-manager-skills`.
Plugin type id: `package_manager_skills` (registered in `plugin_ids::ALL`).
Display name: `Agent Skills`.
Family: `PluginFamily::Software`.
Scope: `Tenant` (default).
Capabilities: `DiscoverLocalSoftware`, `DetectHostCompatibility`, `VersionDetection`, `ReleaseFetching`,
`UpdateExecution`.
Sudo: none (`~/.agents/` is user-owned).
Interactive dispatch: not required; `npx skills update` is non-interactive.

Source layout mirrors the `npm` crate:

```text
crates/plugins/package-managers/skills/
├── Cargo.toml
├── CODEREVIEW.md          # left empty/skeleton until first review
└── src/
    ├── config.rs          # SkillsConfig (no auth fields)
    ├── detection.rs       # VersionDetector + batch_detect via lock file
    ├── discovery.rs       # Discoverer + detect_host_compatibility
    ├── error.rs           # SkillsError + Result alias + impl_report_conversion!
    ├── lib.rs             # pub uses
    ├── lock.rs            # parse_skill_lock(json) -> Vec<SkillEntry>
    ├── plugin.rs          # SkillsPlugin struct, declare_plugin!, helpers
    ├── releases.rs        # ReleaseFetcher using GitHubProviderClient
    └── update.rs          # UpdateExecutor running `npx skills update -g <name> -y`
```

### 2. Identity model

Identifier shape mirrors the `proxmox-helper-scripts` pattern, which already overrides
`DiscoveryTarget.package_identifier` per target:

- `DiscoveredSoftware.package_identifier` = Skill name (e.g. `brainstorming`). Clean for UI.
- `DiscoveredSoftware.name` = Skill name (same value; UI display).
- `DiscoveryTarget.package_identifier` = `Some("{source_url}#{skill_path}")`.
  This is the value stored in `host_software_item_plugin.package_identifier` and the value passed back to
  the plugin for Detect/Fetch/Update.
- `DiscoveredSoftware.extra` = `{ "source_url", "skill_path", "agents", "lock_name" }` for diagnostics.
- `qualifier` = `None`; `installed_display_version` = `None`.

Encoded identifier validator (`validate_identifier`):

- starts with `https://` or `http://`,
- contains exactly one `#` separating the URL from the path,
- the URL prefix (the portion before `#`) parses as a valid `url::Url` — `url::Url` is applied
  only to the prefix, not to the full encoded string (the `#` is a custom delimiter here, not an
  RFC 3986 fragment separator),
- the path is non-empty, length ≤ 512 bytes, no control chars, no `..` segments, no leading `/`,
- total length ≤ 1024 bytes.

A free-standing `parse_skill_identifier(&str) -> Result<(Url, String), _>` is the single decode site.

### 3. Version model

`installed_version` is the 40-char hex git tree SHA recorded as `skillFolderHash` in
`~/.agents/.skill-lock.json`.

`Release.tag` is the git tree SHA of the matching folder in the source repo's HEAD tree, fetched by the
Controller via the GitHub Provider's `fetch_repository_tree(owner, repo, "HEAD", recursive=true)`. The
plugin walks the returned entries and selects the `Tree` entry whose `path` equals the Skill folder
(`skill_path` with the trailing `/SKILL.md` stripped). That entry's `sha` is the Release tag.

`Release.url` = `https://github.com/{owner}/{repo}/tree/{branch_or_HEAD}/{skill_dir}` (best-effort link).
`Release.published_at` = `None` (the git-trees API does not surface commit timestamps; out-of-scope to
derive).

### 4. Cross-cutting changes

Three narrow extensions to existing contracts. Each is additive; no existing call site changes behaviour.

#### 4.a `RepositoryTreeEntry.sha`

In `crates/shared/github-client/src/lib.rs`:

- Add `pub sha: String` to `RepositoryTreeEntry`.
- Add `sha: String` to `RepositoryTreeEntryDto` (parsed verbatim from the GitHub `git/trees` response).
- Threaded through `into_model`. Existing tests get a `"sha": "<hex>"` field added to mock JSON bodies.

#### 4.b `GitHubTreeEntry.sha` and new consumer constant

In `crates/shared/global-github-provider/src/lib.rs`:

- Add `pub sha: String` to `GitHubTreeEntry`.
- Add `pub const PACKAGE_MANAGER_SKILLS: GlobalProviderConsumerId = GlobalProviderConsumerId::new("package-manager-skills");`.
  This constant is passed as the `consumer` argument in `fetch_repository_tree` calls for rate-limit
  attribution. It is NOT a catalog-gating mechanism — the catalog's `global_provider_consumers`
  satisfaction check (which uses the separate `GlobalProviderConsumerDecl` type from `descriptor.rs`)
  applies only to singleton `SoftwareItemLifecycle` slots, not to `ReleaseFetcher` roles. The
  `global_provider_consumers` field in the plugin's `PluginDescriptor` must remain empty (`&[]`);
  listing the GitHub provider there would incorrectly block plugin activation when the provider is
  absent. The plugin handles `None` gracefully and returns a clear error (see §7).

In `crates/ui/web-api/src/global_providers/github.rs`:

- `map_repository_tree_response` carries `entry.sha` through to the `GitHubTreeEntry`.
- Test mocks (`tests.rs`, `default_test_tree`) add the new field with placeholder SHAs. Logic asserted
  in those tests is unchanged.

In `crates/plugins/enhancements/dashboard-icons/src/cache.rs` and `plugin.rs`:

- Test mocks add `sha: "<placeholder>".into()` to every `GitHubTreeEntry` literal. Runtime behaviour
  unchanged (the cache ignores `sha`).

#### 4.c `ReleaseFetchContext` — provider injection without touching `HostRuntime`

Rather than adding a method to the generic `HostRuntime` trait (which is a host-execution abstraction
and should not carry provider-registry concerns), introduce a `ReleaseFetchContext` struct passed
directly to `ReleaseFetcher` factory functions:

In `crates/plugins/infrastructure/core/src/roles.rs`:

```rust
/// Context available to ReleaseFetcher factories at construction time.
/// Passed alongside the config JSON and HostRuntime when the scheduler creates a fetcher.
#[non_exhaustive]
pub struct ReleaseFetchContext {
    pub global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
}
```

The factory type for `ReleaseFetcher` roles changes from
`fn(&serde_json::Value, Arc<dyn HostRuntime>) -> Result<Box<dyn ReleaseFetcher>>`
to
`fn(&serde_json::Value, Arc<dyn HostRuntime>, &ReleaseFetchContext) -> Result<Box<dyn ReleaseFetcher>>`.

All existing `ReleaseFetcher` plugin factories add `_ctx: &ReleaseFetchContext` and ignore it — a
mechanical, one-line change per plugin with no behaviour difference. `HostRuntime` and its
implementations (`StandardHostRuntime`, `MetadataAwareHostRuntime`, `RouterOsHostRuntime`,
`ControllerRuntime`) are unchanged.

`GlobalProviderLookup` is defined in `crates/plugins/infrastructure/core/src/descriptor.rs` (same
crate; no circular dependency).

#### 4.d Scheduler wiring

In `crates/core/scheduler-runtime/src/executors/fetch_releases.rs`:

- `FetchReleasesExecutor` gains a field `provider_lookup: Option<Arc<dyn GlobalProviderLookup>>`.
- `FetchReleasesExecutor::new` gains a corresponding optional parameter. There is exactly one call site
  in `controller-runtime/src/scheduler/mod.rs` where `FetchReleasesExecutor` is constructed and passed
  to `run_embedded_scheduler`; that site must be updated to pass the lookup.
- When the executor builds a `ReleaseFetcher` for a plugin, it constructs a `ReleaseFetchContext`
  from its stored `provider_lookup` and passes it to the factory function (the new third argument).

In `EmbeddedSchedulerConfig` (or the embedded scheduler's startup path in `controller-runtime`):

- A new field `global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>` is added.
- The controller's startup code (which already has access to `GlobalProviders`) sets this field when
  constructing the embedded scheduler.
- `run_embedded_scheduler` threads the value into `FetchReleasesExecutor`.

The standalone scheduler (`scheduler-runtime/src/standalone.rs`) has no access to `GlobalProviders`;
it always passes `None`. Skills' `ReleaseFetcher` returns
`Err(PluginError::PluginInternal("global GitHub provider not available"))` in that deployment mode, which
is expected and logged. This is a known operational limitation (see Open questions).

No other ReleaseFetcher plugin reads `ctx.global_provider_lookup`, so behaviour is unchanged for them.

### 5. Discovery (Agent)

`Discoverer::discover_software` reads `~/.agents/.skill-lock.json` by running
`sh -c "cat ~/.agents/.skill-lock.json"` via `CommandExecutor::execute_quiet` and parsing stdout as
JSON. The shell invocation is required to expand the `~` tilde — passing `~` directly to a
`CommandSpec` path argument is NOT expanded by the kernel. Direct `tokio::fs` access is prohibited:
on Agent-SSH-managed hosts, `CommandExecutor` routes the command to the remote host, but `tokio::fs`
would silently read the controller's local filesystem. If the command exits non-zero or produces no
output, treat the lock file as absent and return an empty discovery result (Skill not yet installed).

For each entry whose `sourceType == "github"`:

- `DiscoveredSoftware.package_identifier` = Skill name (the JSON map key, e.g. `brainstorming`) — the
  clean UI display name stored in `host_software_item.package_identifier`.
- `installed_version` = `entry.skillFolderHash`.
- Emit one `DiscoveryTarget` with `plugin_type = PACKAGE_MANAGER_SKILLS`, `plugin_config = {}`,
  `plugin_config_name = "Agent Skills"`, `roles = [DetectVersion, FetchReleases, ExecuteUpdate]`,
  and `DiscoveryTarget.package_identifier = Some(encode(source_url, skill_path))` — the encoded form
  stored in `host_software_item_plugin.package_identifier` and passed back to the plugin for all
  subsequent Detect/Fetch/Update calls.

Entries with non-GitHub `sourceType` are logged at `warn` and skipped (Goal: "do not silently swallow
unsupported source types").

`detect_host_compatibility`:

- `which npx` exit 0 → `Compatible`.
- otherwise → `Incompatible("npx not found")`.

The plugin does NOT require `git` on the Agent or Controller (the GitHub Provider handles git semantics
through the HTTP API). The earlier discussion's `git`-availability concern is therefore moot.

### 6. Version detection (Agent)

`VersionDetector::detect_installed_version(package_identifier)`:

- Decode the identifier; if invalid, return `Err(PluginError::Configuration(...))`.
- Read `~/.agents/.skill-lock.json` via `sh -c "cat ~/.agents/.skill-lock.json"` through
  `CommandExecutor::execute_quiet`; if the command fails or produces no output, return `Ok(None)`
  (Skill not installed).
- Find the entry where `sourceUrl == decoded.url && skillPath == decoded.path`. If found, return
  `Ok(Some(Version::new(skillFolderHash)))`. Else `Ok(None)`.

`batch_detect` reads the lock file once and matches each item against the parsed entries. Items with
invalid identifiers return `BatchDetectResult::error(...)` for that item only; the batch as a whole
succeeds.

### 7. Release fetching (Controller)

`ReleaseFetcher::fetch_releases(package_identifier)`:

- Decode the identifier; on failure, `Err(PluginError::Configuration(...))`.
- If `sourceUrl` is not a `github.com` URL, return `Err(PluginError::PluginInternal("non-GitHub source not supported"))`.
- Parse `owner` and `repo` from the URL path.
- Fetch the provider handle from the `ReleaseFetchContext.global_provider_lookup` stored at
  construction time. If absent, return `Err(PluginError::PluginInternal("global GitHub provider not available"))`.
- Call `fetch_repository_tree(PACKAGE_MANAGER_SKILLS, owner, repo, "HEAD", recursive=true)`.
- Derive `skill_dir` from the decoded `skill_path` by stripping the last path component (the file
  name): split on `/`, drop the last segment, rejoin. Example: `skills/brainstorming/SKILL.md` →
  `skills/brainstorming`. If `skill_path` contains no `/` (bare filename), `skill_dir` is the empty
  string; in that case return zero releases (malformed identifier — the lock file's `skillPath` always
  has at least one directory segment in practice).
- Find the entry where `path == skill_dir && kind == Tree`. If found, emit one `UpstreamRelease`:
  `Version::new(entry.sha)`, `tag = entry.sha`, `is_prerelease = false`,
  `url = format!("https://github.com/{owner}/{repo}/tree/HEAD/{skill_dir}")`.
- If not found, return a `PluginError::PluginInternal` error whose message includes `owner`, `repo`,
  and `skill_dir` rather than silently returning zero releases. This distinguishes "skill removed
  upstream or moved repo" from "provider unavailable" in operator-visible logs.
- If `GitHubRepositoryTree.truncated == true`, log a `warn` (structured: `owner`, `repo`,
  `truncated = true`) and return zero releases for all items in that repo with an explanatory error;
  do not silently discard skills in large monorepos.

`batch_fetch` groups items by `(owner, repo)` and issues one tree call per repo, then walks the
shared tree to satisfy all items in the group. This reduces the call count from `O(skills)` to
`O(source_repos)`, which today is ~3–5. The trait method is `batch_fetch` (per `ReleaseFetcher` trait
in `infrastructure-core/src/roles.rs`), not a custom method name.

Provider-error mapping mirrors dashboard-icons:

- `GitHubProviderError::Throttled` → return `Err(PluginError::Throttled(_))` (whatever the existing
  contract requires; check at implementation time).
- `AuthFailed`/`Misconfigured` → return `Err(PluginError::Configuration(...))`.
- `UpstreamUnavailable`/`RequestFailed` → return `Err(PluginError::PluginInternal(...))` and let the
  scheduler's existing retry/backoff handle it.

### 8. Update execution (Agent)

`UpdateExecutor::execute_update(package_identifier, to_version, _release_info, output_tx)`:

- Decode the identifier; on failure → `Err(PluginError::Configuration(...))`.
- Read the lock file via `sh -c "cat ~/.agents/.skill-lock.json"` through
  `CommandExecutor::execute_quiet` and recover the Skill name (the JSON map key for the matching
  `sourceUrl + skillPath`). If the command fails or the entry is absent, return
  `Err(PluginError::PluginInternal("skill not installed"))`.
- Run `npx skills@latest update -g <name> -y` through `execute_command_update`.
- `privileged = false`. `to_version` is logged at `info` but not passed to the CLI (no pin support).
- `ExecuteUpdateResult::new(output, false)`.

`execute_batch_update` calls the single-item path per item (no native batch form in the `skills` CLI).
Batching at this layer is optional; the scheduler can interleave per-package calls in v1.

### 9. Configuration

`SkillsConfig` is intentionally empty in v1 (mirrors `NpmConfig` minimal form): no fields, no auth,
no registry URL. The GitHub Provider is the sole credential surface.

Future fields (e.g. a non-default branch toggle) are deferred to v2; they can be added without a
schema migration because plugin config is stored as JSON and unknown fields are ignored. Operators
configure the GitHub Provider once in Settings → GitHub Provider; the Skills plugin consumes it
implicitly.

`PluginConfig` impl:

- `validate_identifier` delegates to the encoded-identifier validator described in §2.
- `form_schema` returns an empty schema (no user-facing fields in v1).
- `validate()` returns `Ok(())` (no required fields).
- `with_secrets_masked()` is a no-op (no secrets).

### 10. Error handling

`SkillsError` enum (in `error.rs`):

- `LockFileMissing`, `LockFileMalformed(String)`, `LockEntryNotFound(String)`,
- `InvalidIdentifier(String)`, `UnsupportedSource(String)`,
- `ProviderUnavailable(String)`, `ProviderError(String)`,
- `CommandFailed(i32)`, `Configuration(String)`, `Plugin(String)`.

`Result<T> = std::result::Result<T, Report<SkillsError>>`.
Two separate `impl_report_conversion!` calls per the project pattern: one for `SkillsError => PluginError`
and one for `PluginError => SkillsError`. `SkillsError` is `pub(crate)`; `#[non_exhaustive]` is not
required on `pub(crate)` error enums (reference: `NpmError`, `DashboardIconsError`).

## Tests

In addition to the unit-test patterns already used by the `npm` and `cargo` plugins:

- `lock.rs` parsing tests cover: missing file (Ok with empty), malformed JSON (Err), the v3 lockfile shape
  shown in the example data, entries with non-GitHub source types, entries missing `skillFolderHash`.
- `discovery.rs` tests cover: empty lock → empty discovery; mixed GitHub and non-GitHub entries; the
  encoded identifier round-trips through `validate_identifier` + decoder.
- `detection.rs` tests cover: single-item match, single-item miss, batch with one valid + one missing,
  batch with invalid identifier (per-item error, batch succeeds).
- `releases.rs` tests use a `FakeGitHubProviderClient` returning canned trees:
  - Skill folder present as `Tree` → one `UpstreamRelease` with the entry's SHA.
  - Skill folder missing → zero releases.
  - Non-GitHub identifier → `Err`.
  - Provider returns `Throttled`/`AuthFailed`/`UpstreamUnavailable` → mapped error matches contract.
  - `batch_fetch` issues one tree call per `(owner, repo)` for N skills sharing a source.
- `update.rs` tests use a `FixedOutputExecutor` to assert the exact command form and exit-code handling.
- A descriptor test in the plugin registry asserts `PACKAGE_MANAGER_SKILLS` is present in
  `plugin_ids::ALL`, registered in `all_descriptors()`, and listed in
  `is_package_manager_plugin`'s static array.
- A unit test in `crates/plugins/infrastructure/core/src/roles.rs` asserts `ReleaseFetchContext`
  round-trips a `Some(lookup)` value through the factory call, confirming the new context plumbing.
- An end-to-end test on the scheduler ensures controller-side `ReleaseFetcher` creation populates
  `ReleaseFetchContext.global_provider_lookup` and passes it through to the factory.
- `releases.rs` tests assert `batch_fetch` (the actual `ReleaseFetcher` trait method) issues one tree
  call per `(owner, repo)` for N skills sharing a source.

Quality gates (per the snapshot, all required green before merge):

```text
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
python3 ci/check_plugin_semantic_boundary.py
markdownlint --config .markdownlint.json '**/*.md'
```

## Migrations

None. No new tables. The plugin uses existing `host_software_item`, `host_software_item_plugin`,
`software_item`, and release-cache tables exactly as `package_manager_npm` does.

## Documentation deliverables

Every doc affected by externally observable behavior, surface area, config, or architecture is listed;
none are optional.

| Doc | Status | Change |
| --- | --- | --- |
| `CONTEXT.md` | **No change** | Skills do not require new glossary terms — they are `Software Items` per the existing definition. |
| `docs/adr/` | **New ADR** | Record the `ReleaseFetchContext` factory-parameter extension as an additive contract change: describe rejected alternatives (adding `global_provider_lookup()` to `HostRuntime`, scheduler-side token injection, per-plugin `auth_token` field) and the rationale for passing context directly to the factory — avoids leaking provider concerns into a host-execution trait and eliminates delegation footguns. |
| `docs/development/plugin-guidelines.md` | **Update** | Add a short paragraph under a new "Consuming Global Providers in ReleaseFetcher" heading: `ReleaseFetcher` factory functions now receive a `&ReleaseFetchContext` third argument; access the GitHub provider via `ctx.global_provider_lookup`. Link to the Skills plugin as the reference implementation. |
| `docs/end-user/skills-plugin.md` | **New** | One-page operator guide: what gets discovered, how the GitHub Provider token affects rate limits, the no-pin update semantics, the GitHub-only source restriction. |
| `OpenAPI schema` | **Auto-generated** | The plugin descriptor surfaces through existing plugin-catalog endpoints; no manual schema work. Confirm by re-exporting and diffing. |
| `crates/plugins/CODEREVIEW.md` (per-plugin scaffold) | **New skeleton** | Created blank with a TODO header, populated at first code review. Same pattern as other plugin crates. |

## Out of scope

- **Project-scoped Skills.** `./.agents/` is ignored; only `~/.agents/` is read.
- **Skill pinning via `to_version`.** The `skills` CLI does not support a version-pin form. The plugin
  accepts `to_version` (for audit) but always runs `update -g <name> -y`.
- **Non-GitHub source repos** in the lockfile. Emit a clear error on release fetch; do not attempt git
  cloning or generic API fallback.
- **Surface registration** in the Dashboard beyond what the plugin family already provides
  (Discovery → Releases → Updates). No bespoke UI tab.
- **Sudo helper** for `npx skills update`. No privilege escalation is required for user-owned
  `~/.agents/`.
- **Hot-reload of the GitHub Provider** binding into the scheduler. Catalog snapshot at boot, per
  ADR-0006's existing decision.
- **Detection of a Skill being uninstalled** as a special `Update`. Discovery handles removal naturally.
- **Generalising `ReleaseFetchContext` to non-GitHub providers in v1.** The slot
  exists for future GitLab/Forgejo providers; the registry today only registers GitHub.
- **Refactor of `releases_github` to use the same lookup.** That plugin remains tenant-scoped with its
  own `auth_token`. Future work.

## Open questions and risks

| Item | Mitigation |
| --- | --- |
| **`HostRuntime` is unchanged.** Previous drafts proposed adding `global_provider_lookup()` to `HostRuntime`, which would leak provider concerns into a host-execution abstraction and create delegation footguns in `MetadataAwareHostRuntime`. | Resolved: `ReleaseFetchContext` is passed directly to the factory; `HostRuntime` and all its implementations are untouched. |
| **GitHub rate limits** for an unauthenticated controller fetching tree data for many skills. | One call per source repo per refresh cycle (best case when skills share monorepos; degrades to one call per skill if every skill is in a distinct repo). Unauthenticated baseline is 60 req/h — adequate for the monorepo-concentrated case. Configured token unlocks the standard 5000 req/h and is recommended for large or multi-repo deployments. |
| **Skill folders moving paths** in a source repo (e.g., `skills/foo` → `productivity/foo`). | Detection treats the relocated folder as a release miss → "no upstream release found"; UI shows the Skill at last-known version. Operator reinstalls via the `skills` CLI and the next discovery cycle reconciles. |
| **`.skill-lock.json` format change in `skills` v4+.** | Plugin reads only the field set `{ source, sourceUrl, sourceType, skillPath, skillFolderHash }`. Unknown fields ignored. A version assertion (`version == 3`) emits a warning but does not block. |
| **`skillFolderHash` SHA instability from force-push or mode-bit changes.** | Git tree SHAs change on content, mode, and line-ending changes, including force-pushes to HEAD. A force-push that rewrites history without changing file content still changes tree SHAs, producing a perpetual "update available" signal. Mitigation: the update executor runs `npx skills update -g <name> -y` which is idempotent (re-installs HEAD and records the new SHA). Document this as a known false-positive scenario in the end-user guide. No structural fix in v1. |
| **"Skill removed upstream" vs "skill moved to another repo" are indistinguishable.** | Both cases return zero releases. If the upstream repo URL changes, the stored identifier is stale; release-fetch returns zero releases with no distinguishing signal. Mitigation: when zero releases are returned, the release-fetch code must emit a structured `PluginInternal` error message that includes the encoded identifier (owner, repo, skill\_path) so operators can identify the stale entry. Do not silently return an empty release list. |
| **The shared `HostRuntime` extension may surprise plugin authors who do not need it.** | Method defaults to `None`. No existing plugin reads it. Documented in `plugin-guidelines.md` (see deliverables). |
| **Cross-cutting test churn** when adding `sha` to `RepositoryTreeEntry` / `GitHubTreeEntry`. | All sites identified in §4; all are test-fixture additions, not behaviour changes. |
| **Standalone scheduler has no `GlobalProviders` access.** Skills' `ReleaseFetcher` returns `Err("global GitHub provider not available")` in standalone deployments. | Known limitation; logged at `warn`. Operators using the standalone scheduler cannot use the Skills plugin's release-fetch path. Document in the end-user guide. |

## Snapshot conformance check

| Binding rule | Satisfied |
| --- | --- |
| Conventional Commits (type, optional scope) | All implementation PRs follow `feat(plugin-skills)`/`refactor(infrastructure-core)` style. |
| `cargo fmt --all` before commit | Quality gate enforced. |
| `cargo clippy --all-targets --all-features -- -D warnings` | Quality gate enforced. |
| `cargo test --all-features` before push | Quality gate enforced. |
| `cargo deny check` | Quality gate enforced; no new external deps beyond what the workspace already pins. |
| `#[non_exhaustive]` on extensible public enums and structs | `SkillsError` is `pub(crate)` — `#[non_exhaustive]` is not required on crate-private error enums (matches `NpmError`, `DashboardIconsError`). Any `pub` structs or enums exported by the plugin crate carry the attribute. |
| Wire-safe enums use `Other(String)` catch-all | No new wire enums introduced. |
| `parking_lot::Mutex` for sync locks in async code | Only ephemeral caches inside the plugin use locks; `parking_lot::Mutex` if needed. No `tokio::sync::Mutex`, no `std::sync::Mutex`. |
| `rootcause::Report` + `report!`/`bail!` + `impl_report_conversion!` | `SkillsError`/`PluginError` conversions follow the pattern. |
| `tracing::{error,warn,info,debug,trace}` with structured fields | All log sites use structured `tracing` calls; no `log` crate; `error!(error = %e, "...")` form. |
| No `thiserror` Display format-string tests | None planned. |
| Plugin HTTP clients require `.connect_timeout(10s)` + `.timeout(60s)` | The Skills plugin does not build its own HTTP client. It delegates to the GitHub Provider, which already enforces these timeouts. |
| Plugin clients to user-controlled URLs require `SsrfSafeResolver` | N/A — no plugin-local HTTP client. |
| `markdownlint` clean (line length ≤ 150) | This spec respects the rule. |
| Workspace lints (`-D warnings`) | New crate inherits `[workspace.lints]`. |

No deviations from the snapshot are necessary.

## References

- ADR-0006 — Instance-Scoped Plugins (`docs/adr/0006-instance-scoped-plugins.md`).
- Spec — Global GitHub Provider for Global Plugins
  (`docs/superpowers/specs/2026-04-17-global-github-provider-for-global-plugins-design.md`).
- Spec — Tiny GitHub Client (`docs/superpowers/specs/2026-04-17-tiny-github-client-design.md`).
- CONTEXT.md glossary — "Plugin", "Plugin Scope", "Software Item", "Release", "Software Discovery".
- Existing reference plugin — `crates/plugins/package-managers/npm/`.
- `skills` CLI — npm package `skills`; lockfile `~/.agents/.skill-lock.json` (v3).
