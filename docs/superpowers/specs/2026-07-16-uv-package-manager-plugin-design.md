# uv Package-Manager Plugin (Discovery-Capable, Tools-Only)

**Date:** 2026-07-16
**Status:** Approved (user interview, 2 rounds + contrarian pass; empirical uv CLI verification on uv 0.11.29)
**Scope:** new crate `crates/plugins/package-managers/uv/`, registry/type-id registration, docs.
No frontend, web-api, wire-protocol, or migration changes.

## Problem

Hosts using [uv](https://docs.astral.sh/uv/) to install Python CLI tools (`uv tool install ruff`) have no Uptrakit
coverage: no discovery, no version tracking, no update path. Each uv tool is a standalone application in its own
isolated venv — exactly the kind of per-item software Uptrakit tracks.

## Decisions (locked with user)

1. **Tools only.** The plugin covers `uv tool` installs exclusively. `uv pip` / system-environment packages are
   **out of scope**: `uv pip` is a pip-interface reimplementation over standard Python environments — packages
   there are not attributable to uv (pip-installed and uv-installed rows are indistinguishable; uv keeps no
   install receipt in those environments). The originally requested "global packages behind a default-off
   setting" was dropped on this basis. If uv ever grows an attributable global-install concept, the extension
   path is Homebrew's `package_type` Select pattern (`crates/plugins/package-managers/homebrew/src/config.rs`).
2. **`featured: true`** on every discovered tool (`DiscoveredSoftware.featured`). Each uv tool becomes an
   individual software item row with role assignments (and an individual Home Assistant entity), not a per-host
   aggregate. Live precedents for featured + package-manager classification: cargo
   (`crates/plugins/package-managers/cargo/src/plugin.rs:221`), skills, routeros. Persistence honors the
   plugin-provided flag (`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs:223`).
3. **Full role set:** `Discoverer`, `VersionDetector`, `ReleaseFetcher` (controller-side), `UpdateExecutor`.
4. **Releases fetched controller-side from the PyPI Simple API** (PEP 691 JSON + PEP 700 `versions`), default
   index `https://pypi.org/simple`, index URL configurable for self-hosted mirrors. Agent-side
   `uv tool list --outdated` rejected: yields only "latest", no version list for the UI, and one network fetch
   per host instead of one per package.
5. **Agent-user scope.** Discovery sees the agent user's `uv tool list` only. Tools installed by other users are
   invisible — documented limitation, consistent with the unprivileged-agent invariant.
6. **No sudo.** All `uv tool` operations are per-user. `required_sudo_commands()` stays empty (descriptor
   declares none, like cargo).

## Current reality (verified)

- **Primary precedent: cargo** (`crates/plugins/package-managers/cargo/`) — the only existing plugin combining
  discovery + version detect + controller-side release fetch + update + `type_settings: true`. uv has the same
  shape (per-user, sudo-free, upstream index).
- `uv tool list` has **no JSON output**. Stdout format (verified on uv 0.11.29):

  ```text
  ruff v0.6.8
  - ruff
  ```

  Empty state: stdout empty, `No tools installed` on **stderr**, exit 0.
- `uv tool list --show-with` is **lossy** (verified empirically): `--with 'httpx[http2]' --with 'rich>=13,<14'`
  renders as `[with: httpx, rich>=13, <14]` — extras and environment markers are dropped, and specifier
  commas collide with the list separator. It must NOT be used as the source for `--with` preservation. The
  faithful ground truth is `{uv tool dir}/<name>/uv-receipt.toml`, whose `[tool].requirements` array carries
  structured `name`/`extras`/`specifier`/`marker` fields. `uv tool dir` prints the absolute tools directory.
- `CommandOutput.output` **concatenates stdout and stderr** (`crates/shared/command/src/command.rs:167-180`).
  Any parser sees the merged stream; uv writes warnings and notices to stderr.
- Update semantics (verified empirically in an isolated `UV_TOOL_DIR`):
  - `uv tool install 'pkg==ver'` replaces the existing install at the pinned version, exit 0 (also downgrades).
  - `uv tool upgrade 'pkg==ver'` **fails** (exit 1) when the pin conflicts with the install-time specifier —
    unusable as the update command.
  - Reinstall without `--with` **silently drops** the original extra requirements; re-passing the receipt's
    requirements as `--with` preserves them.
  - The receipt's `[tool]` level also records `python = "<request>"` (for `--python` installs) and a
    structured `[tool.options] index = [{ url = …, default = true, … }]` array (for `--default-index`/
    `--index` installs); a reinstall without the corresponding flag silently drops each (probed on
    uv 0.11.29 — the rewritten receipt loses `python`). Flag-free installs on an unconfigured host record
    neither — but a host-level default index (`UV_DEFAULT_INDEX` env or `uv.toml` `[[index]]
    default = true`, i.e. exactly how a mirror host configures uv) bakes `[tool.options].index` into
    **every** plain-install receipt (probed).
  - uv PEP 503-normalizes the tool name consistently across `uv tool list`, the tools-dir entry, and the
    receipt's primary requirement (probed: `ruamel.yaml.cmd` → `ruamel-yaml-cmd` in all three).
- Shared `Version` (`crates/plugins/infrastructure/core/src/version.rs`) parses strict semver only and falls
  back to raw-string ordering. PEP 440 versions (`1.0`, `1.2.3.post1`, `2024.1.1`, `1.2.3rc1`) fail semver parse
  and mis-sort lexically (`"1.9" > "1.10"`). The scheduler trusts plugin-provided order verbatim
  (`crates/core/scheduler-runtime/src/executors/fetch_releases.rs:405-418` — `find(!is_prerelease).or(first)`,
  no re-sort), so correct descending order must be produced inside the plugin. Cargo's prerelease heuristic
  (`contains('-')`, `cargo/src/plugin.rs:48-50`) is wrong for PEP 440 (`1.0rc1` has no hyphen).
- pypi.org serves PEP 700 `versions` (verified live, api-version 1.4). Self-hosted indexes (devpi, Artifactory,
  Nexus, GitLab) often serve HTML-only Simple or PEP 691 JSON **without** `versions` — a filename fallback is
  required for the configurable-index scope to actually work.

## Design

### Crate and descriptor

New crate `crates/plugins/package-managers/uv/`, package `name = "uptrakit-plugin-package-manager-uv"`,
modeled on cargo's file layout (`config.rs`, `plugin.rs` + discovery, `detection.rs`, `releases.rs`,
`update.rs`) — with named deviations where cargo predates current mandates: command execution goes through
`execute_and_capture` (not cargo's hand-rolled `execute_quiet` pattern), tests use the shared
`infrastructure-core::testing` doubles (not cargo's local mock), and `index_url` validation is written fresh
(see Config section). Descriptor mirrors cargo's `declare_plugin!` (`cargo/src/plugin.rs`) verbatim in shape:

```rust
declare_plugin!(UvPlugin, UvConfig, "package_manager_uv", {
    display_name: "uv Tools",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    type_settings: true,
    roles: [Discoverer, VersionDetector, ReleaseFetcher, UpdateExecutor],
    extra_capabilities: [PluginCapability::ControllerSideFetchReleases],
});
```

Constructor mirrors `CargoPlugin::new` (`cargo/src/plugin.rs:118-148`) exactly, including the SSRF mode fork:
`SsrfMode::Permissive` when `index_url` is set (self-hosted/LAN mirror), `SsrfMode::Strict` for the pypi.org
default; HTTP client via `build_plugin_http_client(PluginHttpClientConfig { .. })` — never a raw
`reqwest::Client::builder()`.

### Config / type settings

```rust
pub struct UvConfig {
    include_prereleases: bool,      // default false
    index_url: Option<String>,      // default None => https://pypi.org/simple
}
```

Both fields exposed in `TypeSettings::type_settings_form_schema()` (Toggle + Text), following
`CargoConfig` (`cargo/src/config.rs`) for the `PluginConfig`/`TypeSettings` impl shapes and sample.
`index_url` validation is **written fresh** — no existing plugin config validates this combination (cargo's
`registry_url` validate body is an empty-string check only; `GitLabConfig::validate_inner`
(`crates/plugins/releases/gitlab/src/config.rs`) is the `url::Url::parse` + scheme-check shape to follow, but
it is https-only and rejects private hosts, both of which uv must NOT copy): require non-empty, parseable
`url::Url`, scheme `http` or `https` (http allowed — self-hosted LAN mirrors are the point of the field, and
`SsrfMode::Permissive` deliberately admits private hosts), and reject embedded credentials
(`url.username() != "" || url.password().is_some()`). Security-relevant: this field also selects the SSRF
mode fork. No new UI work: type settings render schema-driven in `PluginConfigsTab.svelte`.

### Discovery

`discover_software()` runs `uv tool list` via the **mandatory shared helper**
`execute_and_capture(executor, cmd, context)` (`infrastructure-core/src/command.rs` — new package-manager
plugins must use it instead of hand-rolling `execute_quiet` → `map_err` → exit-code checks; cargo's manual
pattern predates the helper and is NOT the template here; uv has no rpm-style meaningful non-zero exit, so
the helper's uniform-failure contract fits; real executors return `Err(CommandError::CommandFailed)` on
non-zero exit (`crates/shared/command/src/command.rs:190-192`) and the helper folds any executor `Err` into
`PluginError::PluginInternal` — its own `CommandFailed` re-bail arm is reachable only for doubles that
return `Ok` with a non-zero exit). Note the returned string is the **merged
stdout+stderr stream** (`crates/shared/command/src/command.rs:167-180`), so the parser must be hard-anchored
(contrarian finding — loose matching can false-match stderr noise like `warning: foo v2 ...`).

Parser: a **manual line-oriented parser** in the family of `parse_cargo_install_list`
(`cargo/src/plugin.rs:73-98`, which uses `strip_suffix`/`find(" v")`/`split_whitespace` — no `regex`
dependency; every free-text pm parser in the codebase is manual, `regex` is used only by the release-API
plugins). For uv prefer `split_once(" v")` over cargo's `find` + string range slicing (avoids the
`string_slice` deny lint and the file-level `#![expect(clippy::string_slice, …)]` cargo's plugin.rs needs
for exactly that pattern). Per line, in order: split on the first `" v"`; accept only if the name part
is non-empty, starts at column 0, and
passes the identifier charset; the version part must start with an ASCII digit (PEP 440 normalized forms
cannot start otherwise) and contain no whitespace; any trailing content rejects the line.

- Entrypoint bullets (`- ruff`), `No tools installed`, and uv warning/notice lines fail these checks and are
  skipped without error.
- Non-matching lines are skipped; there is no "malformed payload" bail for this text format (unlike JSON). The
  degenerate all-noise case yields an empty result, which is indistinguishable from "no tools" by design.

Each parsed tool emits one `DiscoveredSoftware` with `featured: true`, `package_identifier` = tool name (uv
prints PEP 503-normalized names), `installed_version` = version string, and one `DiscoveryTarget` carrying
`plugin_type: package_manager_uv`, empty config `{}` (package-manager types use type settings; no
`plugin_config` row is created — `discovery_items.rs:203-216`), and all three roles
(`detect_version`, `fetch_releases`, `execute_update`). Document the exact emission shape in
`docs/development/autodiscovery-internals.md` §"Plugin-driven discovery targets" (new `### uv` entry).

`detect_host_compatibility()` copies cargo's `which` probe (`cargo/src/plugin.rs:232-243`) with `"uv"`.
Operator note (docs): the agent user's `PATH` must include uv's bin dir (typically `~/.local/bin`) — same
pre-existing limitation as cargo's `~/.cargo/bin`.

### Version detection

`detect_installed_version(package_identifier)` runs the same `execute_and_capture` + parser and selects
the row matching the identifier (uv has no per-package query). Absent row ⇒ `Ok(None)` — the standard
not-installed outcome in cargo's `detection.rs` (`installed.get(..).map(Version::new)`; never an error —
no package-manager plugin in the repo treats "not installed" as an error). This is the scheduled-check path that keeps `featured` items'
versions current (rediscovery intentionally skips `installed_version` for featured items,
`discovery_items.rs:400-403`).

### Release fetching (controller-side)

`fetch_releases(package_identifier)`:

1. Normalize the name per PEP 503 (lowercase; collapse runs of `.`, `_`, `-` to `-`) for the URL only.
2. `GET {index_url}/{normalized_name}/` with header `Accept: application/vnd.pypi.simple.v1+json` — join
   with the configured `index_url` trimmed of trailing `/` (`trim_end_matches('/')`): operators commonly
   configure mirrors pip-style with a trailing slash, and a naive join produces `…/simple//name/`, which
   self-hosted index servers are not guaranteed to collapse.
3. Version extraction, in order:
   - PEP 700 `versions` array when present;
   - **fallback** (contrarian finding — most self-hosted indexes lack PEP 700): derive the version set from
     `files[].filename`, **anchored on the known name, never naive-split** — hyphenated projects make
     `{name}-{version}` splitting ambiguous (`zope-interface-4.5.0.tar.gz`: is the version `4.5.0` or
     `interface-4.5.0`?). Match the prefix under **full PEP 503 normalization** (lowercase, collapse
     `[._-]+` runs to `-`) with an index-tracking walk over the stem: normalize characters until the
     output equals `{normalized_name}-`, then take the **raw** remainder's next `-`- or
     extension-delimited segment as the version. A `_`→`-`-only replacement misses dotted legacy sdists
     (`zope.interface-5.4.0.tar.gz` never matches prefix `zope-interface-` → that version silently
     vanishes → wrong "latest" when no wheel exists), while normalizing the whole stem would corrupt the
     version's own dots (`5.4.0` → `5-4-0`) — hence normalized matching for the name region, raw text for
     the version region (`str::get`/`split_at`, never range-indexing — `string_slice`/`indexing_slicing`
     are denied). Wheels are preferred over sdists when both exist for a version (PEP 427 underscore-escapes the
     name, so wheels are unambiguous); legacy pre-PEP-625 sdists are exactly what old self-hosted mirrors
     serve. Deduplicate.
   - JSON body that parses but yields zero versions via both paths ⇒ `bail!` (`PluginError`-typed), never a
     silent empty list — this spec's own contract decision: a valid Simple-API project page always carries at
     least one version or file, so zero extractable versions signals a wrong index URL or lossy negotiation.
     Non-JSON (HTML-negotiated) responses and HTTP errors also `bail!`; the npm parse-contract precedent
     (commit `61c7d4358`) covers exactly that unparseable-body case (it does NOT bail on valid-but-empty).
4. Parse every version with `pep440_rs`; skip unparseable strings (item-level tolerance, homebrew-style).
5. `is_prerelease` = `Version::any_prerelease()` (pre-release **or** dev segment; verified present in
   pep440_rs 0.7.3). Filter prereleases unless `include_prereleases`. **Post-filter emptiness is NOT an
   error**: a package with only pre/dev releases under `include_prereleases: false` returns `Ok(vec![])`
   (the fetch succeeded; there is legitimately no stable Release) — the zero-version `bail!` above applies
   only to the pre-filter extraction.
6. Sort **descending by the parsed PEP 440 key, before building** `UpstreamRelease` values (raw version
   string into the shared `Version` for display only). Do **not** copy cargo's
   `releases.sort_by(|a, b| b.version.cmp(&a.version))` (`releases.rs:107`) — that sorts by the shared
   `Version` whose string fallback is exactly the mis-ordering pep440_rs exists to fix; never re-sort the
   built vec by `Version`. `release_url` = the project's index page URL.
7. `batch_fetch` mirrors cargo's bounded `buffer_unordered(10)` (`cargo/src/releases.rs`).

Yanked handling: **not filtered in v1** (PEP 700 has no version-level yank; per-file scanning adds complexity
for marginal value on a user-triggered update flow). Deferred.

### Update execution

`execute_update(package_identifier, to_version, _release_info, output_tx)`:

1. **Preflight — read the tool's receipt** (`uv tool list --show-with` is lossy and must not be used here;
   see Current reality):
   - `execute_and_capture` `uv tool dir` → absolute tools directory. The stream is merged stdout+stderr
     (uv notices/warnings are `warning:`/`error:`/`note:`-prefixed lines), so select the **unique** line
     matching the path shape — starts with `/`, no whitespace — and typed-error on zero or multiple
     matches; never blind-trim the whole output (robust against notices before or after the path).
   - `execute_and_capture` `cat <tools_dir>/<package_identifier>/uv-receipt.toml`. Path traversal is
     structurally excluded: `package_identifier` has already passed `IDENTIFIER_RULES` (charset has no `/`;
     `reject_double_dot` blocks `..`).
   - Parse with the `toml` crate (already in `[workspace.dependencies]` as `toml = "1"` with
     `parse`/`serde` features; add `toml = { workspace = true }` to the plugin manifest) into a typed
     `#[derive(Deserialize)]` struct covering `[tool].requirements` entries: `name: String`,
     `extras: Option<Vec<String>>`, `specifier: Option<String>`, `marker: Option<String>`, **plus
     `#[serde(deny_unknown_fields)]`** — uv records non-index sources as extra fields (`url`, `path`,
     `editable`, `git`/`rev`/`tag`/`branch`; verified: `--with 'idna @ https://…whl'` records
     `{ name = "idna", url = "…" }`), and ignoring them would reconstruct a bare name ⇒ **silent source
     swap** from the pinned artifact to the index — exactly the silent-strip class this preflight exists to
     kill. Any unknown field ⇒ typed error, update fails loud. Non-index `--with` sources are a documented
     unsupported case (deferred). Deliberate trade: a future benign per-requirement field in uv's receipt
     schema fails updates loudly until the struct learns it — preferred over a source-field allowlist that
     could silently admit a future source-discriminating field.
   - **The `[tool]` table carries two more install-determining fields the struct must read** (probed on
     uv 0.11.29; the `[tool]` level itself stays tolerant of unknown keys — `entrypoints` etc.):
     - `python: Option<String>` — `uv tool install --python 3.12` records `python = "3.12"`, and a
       reinstall without `--python` **silently drops it** (verified: the receipt loses the field),
       reinterpreting the tool against the agent's default interpreter. When present, re-pass it as
       `--python <value>` in the update argv, guarded like every other receipt string (no leading `-`,
       no control characters, no whitespace, length-bounded).
     - `[tool.options]` — `uv tool install --default-index <url>` records a structured
       `index = [{ url = …, default = true, … }]` array; `--no-binary` records `no-binary = true`;
       `--index-strategy` records `index-strategy = …`. A bare pinned reinstall **drops the whole table**
       (probed: the rewritten receipt loses `[tool.options]`) — a silent index swap is the same
       dependency-confusion class the requirement-level `deny_unknown_fields` kills, and a dropped
       `no-binary` silently swaps a from-source build for a wheel at the same version (the `==<version>`
       pin dominates only the version, not the artifact form or resolution mode). v1 policy: **any
       non-empty `[tool.options]` table ⇒ typed error, update fails loud** — enumerating
       "pin-dominated" keys could misjudge a future option — with exactly one carve-out, because a
       host-level default index bakes `index` into every receipt on mirror-configured hosts (see Current
       reality) and blanket loud-fail would block updates on the very hosts the configurable `index_url`
       serves: when the table contains **only** the `index` key, with a **single** entry whose
       `default = true` and whose `url` (trailing-slash-trimmed) equals the plugin's trimmed
       `effective_index_url()`, proceed and re-pass `--default-index <effective_index_url>` (round-trip
       probed: exit 0, receipt re-records the index). Any other shape — mismatched URL, `explicit`
       entries, extra keys (`no-binary`, `index-strategy`, …) — ⇒ typed error (re-passing deferred).
   - The tool's own entry is **index 0** (verified: uv always records the primary requirement first), and
     it must be identified **by position, guarded by name**: take it via `requirements.split_first()` —
     never `entries[0]`, which is the workspace-denied `indexing_slicing` lint, and `split_first()`'s
     `None` arm is the typed-error path for an empty `requirements` array — then assert the primary's
     `name == package_identifier`, typed error on mismatch (converts a future uv reordering into a loud
     failure instead of silent argv corruption). Name-equality alone cannot identify it:
     `uv tool install celery --with 'celery[redis,auth]'` records two entries both named `celery`.
     Exact equality is safe across name spellings: uv PEP 503-normalizes identically in `uv tool list`,
     the tools-dir directory name, and the receipt (probed on uv 0.11.29:
     `uv tool install 'ruamel.yaml.cmd'` yields `ruamel-yaml-cmd` in all three).
   - **Index 0's payload is NOT discarded** — it feeds the primary install argument. The dominant
     extras form is `uv tool install 'celery[redis]'`, which records
     `{ name = "celery", extras = ["redis"] }` at index 0; a bare pinned reinstall would silently strip
     `[redis]` (verified empirically: uv uninstalls `redis` and rewrites the receipt). Index 0 goes through
     the same `deny_unknown_fields` typed parse (a URL-sourced primary fails loud too); its `extras` are
     re-applied to the pin, its recorded `specifier` is replaced by the user-selected version.
   - Reconstruct one PEP 508 string per remaining entry: `name[e1,e2]<specifier>; <marker>` (each part only
     when present; round-trip through `uv tool install --with` verified empirically incl. extras, ranges,
     markers).
   - Guard each reconstructed string before argv reuse (defense in depth on top of the structured source):
     non-empty `name` passing the identifier charset, no leading `-` anywhere (argument-injection guard — a
     recorded requirement must never become a new `uv` flag), no control characters, length ≤ 256 (the
     plugin-input-validation bound).
   - Any failure in this chain (dir resolution, missing/unreadable receipt, TOML parse, guard) ⇒ fail the
     update with a typed error, do not skip-and-continue — never proceed blind and strip extras.
2. Run `uv tool install '<pkg>[<primary extras>]==<to_version>'` (extras segment only when index 0 recorded
   extras; verified round-trip: `uv tool install 'celery[redis]==5.6.3'` re-installs `redis` and the receipt
   keeps `extras = ["redis"]`) plus `--python <recorded>` when the receipt carries `[tool].python`, plus one
   `--with <req>` per reconstructed remaining requirement, through the
   mandatory shared helper `execute_command_update(CommandUpdateParams { binary: "uv", privileged: false,
   .. }, output_tx)` — copy the call shape from `cargo/src/update.rs`. `resumable` is NOT a
   `CommandUpdateParams` field: the trait result is built afterwards as
   `Ok(ExecuteUpdateResult::new(output, false))` (`cargo/src/update.rs:59`).

This closes the contrarian-flagged silent-extras drop for **index-sourced requirements**, on both the
primary entry (`celery[redis]` installed directly) and `--with` entries (`httpx[http2]`, ranges
`rich>=13,<14`, environment markers) — all reconstructed from the receipt's structured fields, never
re-parsed from lossy display output. It also closes the two silent swaps the receipt's `[tool]` level
carries: a recorded `python` pin is re-passed, and a recorded custom index fails loud. Non-index
sources (URL/path/git/editable `--with` entries), non-empty recorded `[tool.options]` tables (save the
matched-mirror default-index carve-out), and receipts missing entirely (tools installed by a pre-receipt
uv) fail the update **loudly** with a typed error — never a silent downgrade or silent
source/interpreter/artifact-form swap.

### Identifier rules

```rust
const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 128,
    first_char_valid: |c| c.is_ascii_alphanumeric(),
    char_valid: |c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-',
    reject_double_dot: true,
};
```

(PEP 503/508 project-name charset.) Crate-level `pub fn validate_identifier` + `PluginConfig` delegation, copying
cargo's shape (`cargo/src/config.rs`). Do **not** copy homebrew/npm's hand-rolled validation (flagged legacy in
`crates/plugins/AGENTS.md`).

### New dependency

`pep440_rs = "0.7.3"` — added to `[workspace.dependencies]` first (dependency-registration rule), referenced
`workspace = true` in the plugin crate. Full three-component pin — the workspace's plurality style (71
three-component vs 37 two-component `version =` entries in the root `Cargo.toml` at review time); 0.7.3 is
the latest stable on crates.io (verified 2026-07-17). License
`Apache-2.0 OR BSD-2-Clause` — cargo-deny resolves clean via the **Apache-2.0** branch of the OR
(BSD-2-Clause itself is not in the `deny.toml` allowlist; BSD-3-Clause is). Maintenance status,
recorded for future audits: last publish 2024-12; PEP 440 is a frozen standard, so a complete parser does not
need churn. The actively-published `uv-pep440` (0.0.62) was rejected: 0.0.x weekly releases, no stability
contract, `cargo deny` multiple-versions risk. Run `cargo deny check` after adding.

### Registration checklist (all CI-gated)

1. `crates/shared/types/src/plugin_type_id.rs`: add `PACKAGE_MANAGER_UV` const in `plugin_ids`, append to
   `ALL`, and bump the `all_constants_count` test assertion (currently `26`; verified at write time — re-count
   at edit time).
2. `crates/plugins/infrastructure/registry/src/registry.rs`: `&DESCRIPTOR` in `all_descriptors()`
   (package-managers block, unconditional — pm plugins are not feature-gated), `PACKAGE_MANAGER_UV` in
   `PACKAGE_MANAGER_IDS`, **and** in the manually-maintained array inside the
   `package_manager_lookup_covers_all_current_package_managers` test (same file — it duplicates
   `PACKAGE_MANAGER_IDS` and drifts silently if skipped). Also add the id to the `always_on` array in the
   `always_on_ids_have_descriptors` test (same file, third hand-maintained list; one-directional, so
   omission passes CI but drifts).
3. `crates/plugins/infrastructure/registry/Cargo.toml`: plain (non-optional) dependency on the new crate.
4. Root `Cargo.toml` `[workspace.dependencies]`: path entry for the new crate + `pep440_rs` (`toml` is
   already registered there; the plugin manifest just references it `workspace = true`).
5. Workspace member: picked up automatically by the `crates/*/*` glob — no member-list edit.
6. Boundary: the new crate must only be reachable via the registry (`ci/check_plugin_semantic_boundary.py`);
   no other manifest gains a dependency on it.
7. `release-plz.toml`: new `[[package]]` entry `name = "uptrakit-plugin-package-manager-uv"` with
   `git_only = true`, `git_release_enable = false`, `publish = false` (sibling-plugin shape — the cargo
   entry), **and** add the crate name to the `changelog_include` arrays of `uptrakit-controller` and
   `uptrakit-controller-standalone` (the two arrays enumerating every package-manager plugin; cargo appears
   in both). Every workspace package has a `[[package]]` entry in this file; omitting it drops uv commits
   from the controller changelogs.

### Error contract

- uv binary absent ⇒ `HostCompatibility::Incompatible` from the probe; discovery on an incompatible host is
  not an error.
- `uv tool list` failure (non-zero exit or spawn error) ⇒ `Err` from `execute_and_capture`, surfacing as
  `PluginError::PluginInternal` (tool malfunction, not "empty"). `LocalCommandExecutor::execute_quiet`
  returns `Err(CommandError::CommandFailed)` on non-zero exit, so the helper's own `CommandFailed` re-bail
  is production-unreachable — never assert `PluginError::CommandFailed` for this path in tests.
- Empty tool list (exit 0) ⇒ `Ok(vec![])`.
- Release fetch: HTTP error, non-JSON body, or zero-version JSON ⇒ typed `bail!`; per-version parse failures
  skipped item-level.
- Module error enum + `Result` alias per the error-handling standard (`rootcause` `report!`/`bail!`,
  `thiserror`), mirroring `CargoError`.

### Testing

Use `uptrakit_plugin_infrastructure_core::testing` doubles (`FixedOutputExecutor`, `RoutedOutputExecutor`,
`test_runtime_with_executor`) — dev-dep `features = ["testing"]`; never local mocks (cargo's local mock is the
flagged legacy exception, npm/homebrew are the pattern). Before relying on a double, check its behavior on the
*specific* trait method the code under test calls (`execute` vs `execute_quiet` differ on non-zero exits).
For "`uv tool list` fails" coverage use `FixedOutputExecutor::failure(n)` — its `execute_quiet` `Err`s on
non-zero, matching `LocalCommandExecutor` — and assert `PluginError::PluginInternal`;
`RoutedOutputExecutor`'s `Ok`-with-non-zero shape would exercise `execute_and_capture`'s
production-unreachable `CommandFailed` arm.

Required coverage (success + failure per invariant):

- Parser: plain rows, entrypoint bullets skipped, `No tools installed` stderr-merged case, interleaved
  `warning: foo v2 is deprecated` line (must NOT produce a phantom package — merged-stream fixture),
  empty input.
- Discovery: emission shape (`featured: true`, target roles, empty config); command failure
  (`FixedOutputExecutor::failure`) surfaces `PluginError::PluginInternal`.
- Detection: found, not-found, command-failure paths.
- Releases (HTTP mocked at the parse layer like cargo's tests): PEP 700 body; PEP 691 body **without**
  `versions` (filename fallback yields non-empty, including a **hyphenated project name** —
  `zope-interface`-style — proving name-anchored parsing, not naive splitting, and a **dotted legacy
  sdist filename** — `zope.interface-4.5.0.tar.gz` under project `zope-interface` — proving
  full-PEP-503 prefix matching with the version's dots preserved); HTML body ⇒ error;
  prerelease filtering (`1.2.3rc1`, `1.0a1`, `.dev` — no-hyphen forms); all-prerelease package under
  `include_prereleases: false` ⇒ `Ok(vec![])`; descending PEP 440 order incl. `1.9` vs `1.10` and `.post1`.
- Update: receipt-based preservation — **primary-extras regression case** (`celery[redis]` at index 0 ⇒
  install arg `celery[redis]==<ver>`, extras never stripped); `--with` reconstruction fidelity for extras
  (`httpx[http2]`), specifier ranges (`rich>=13,<14`), and markers; same-name `--with` entry
  (`celery` + `celery[redis,auth]`) survives the positional primary identification;
  primary-entry `name != package_identifier` ⇒ typed error; empty `requirements` array ⇒ typed error; a `url`/`git`/`path`-sourced entry (primary OR
  `--with`) ⇒ typed error (deny_unknown_fields); `[tool].python` present ⇒ `--python <value>` in the
  argv (absent ⇒ no flag); `[tool.options]` cases — sole matched default index (url ==
  `effective_index_url()`) ⇒ proceeds with `--default-index` in the argv, mismatched url ⇒ typed error,
  extra key alongside a matched index (`no-binary = true`) ⇒ typed error; multi-line `uv tool dir`
  output (stderr-merged warning plus the path) resolves to the unique path-shaped line, zero or multiple
  path-shaped lines ⇒ typed error; missing/unparseable
  receipt aborts the update; pinned-install arg shape.
- Identifier rules: accepted/rejected names.
- No sudo entries declared (mirror cargo's descriptor test).

No integration-test (Docker) run required: no DB, migration, REST, or wire changes.

### Quality gates

Canonical AGENTS.md quick-start commands apply unchanged (the new crate is unconditional in the registry — no
extra feature flags to thread). `cargo deny check` required for the new dependency. No
`./scripts/regen-api.sh` (no endpoint changes).

## Documentation deliverables

1. `docs/development/autodiscovery-internals.md` — new `### uv` emission entry in §"Plugin-driven discovery
   targets"; adjust the capability-count prose in §"Plugin capabilities" **by re-counting from source at edit
   time** (hand-written counts there are pre-existing drift hazards).
2. `docs/end-user/plugin-configs.md` — new `package_manager_uv` row in the plugin table + a
   `### package_manager_uv configuration fields` subsection (fields, discovery behaviour, tools-only scope,
   agent-user visibility, `PATH`/`~/.local/bin` operator note, "No sudo required", and the
   update-preservation contract with its loud-failure cases: recorded `[tool.options]` beyond the
   matched-mirror default-index carve-out (`no-binary`, mismatched index, …), non-index sources, missing
   receipt, whitespace-bearing tools-dir or recorded-python paths, plus the mixed-index "latest"
   limitation) — cargo's subsection is the
   template. This is the primary end-user doc; no standalone `docs/end-user/uv-plugin.md` (follow the
   cargo-in-table precedent, not the npm/snap standalone-page one).
3. `docs/development/plugin-guidelines.md` — add uv to the "Featured flag routing" table with
   `featured = true`, and **fix the stale Cargo row** (table says `false`; code emits `true`,
   `cargo/src/plugin.rs:221`) so the table matches reality.
4. Root `AGENTS.md` — codebase-layout tree line for `package-managers/`: append `uv` (the list is already
   missing `routeros`/`skills`; fix those in the same edit).
5. `docs/development/plugin-system.md` — "First-Party Plugin Crates" table: new `package_manager_uv` row
   (`uptrakit-plugin-package-manager-uv`, Family Software, Discovery Yes, Controller-side fetch Yes, Host
   compat Yes, Lifecycle hooks No), and fix the stale `package_manager_cargo` "Controller-side fetch" cell
   in the same edit (table says No; cargo declares `PluginCapability::ControllerSideFetchReleases`,
   `cargo/src/plugin.rs:168`). The table is also missing rows for `package_manager_skills`,
   `package_manager_routeros`, and `discovery_uptrakit_self_update` — add them in the same edit (same drift
   class as the AGENTS.md tree-line fix), deriving each row's cells from that crate's `declare_plugin!`
   block at edit time, not from this spec.
6. Rustdoc on the new crate's public items (parser contract, PEP 503 normalization, `--with` preservation
   rationale).

No ADR: no new architecture decision — the plugin instantiates existing, documented extension points.
No `CONTEXT.md` change: no new domain vocabulary ("uv tool" is plugin-internal).

## Alternatives considered

- **`uv pip list --system` global-packages mode (default-off toggle):** dropped by user decision — packages in
  standard Python envs are not attributable to uv (see Decisions).
- **Agent-side `uv tool list --outdated` for releases:** latest-only, no version list for the UI, per-host
  network fan-out. Rejected.
- **Legacy PyPI JSON API (`/pypi/{name}/json`):** pypi.org-exclusive; defeats the configurable self-hosted
  index. Rejected in favor of the Simple API both uv itself and self-hosted mirrors speak.
- **Reading `uv-receipt.toml` files for discovery:** rejected — per-tool path enumeration when
  `uv tool list` answers the same question in one command. For **update preflight** the receipt IS the chosen
  source (one known path, structured fields): the initially preferred `uv tool list --show-with` proved lossy
  under contrarian testing (extras/markers dropped, specifier commas ambiguous) and was rejected.
- **`uv tool upgrade` as the update command:** fails against install-time pins (verified). Rejected.
- **Regex prerelease heuristic instead of `pep440_rs`:** cannot fix ordering (`1.9` vs `1.10`), which the
  scheduler depends on; a wrong "latest" is a functional bug, not a cosmetic one. Rejected.
- **`uv-pep440` instead of `pep440_rs`:** see New dependency.

## Deferred / out of scope

- `uv pip` / system-environment package tracking (dropped; revisit only if uv adds attributable global installs).
- Yanked-version filtering in release lists.
- Updating tools whose receipt carries non-index `--with` sources (URL/path/git/editable) — v1 fails these
  loudly; preserving them needs source-kind-aware reconstruction.
- Updating tools whose receipt records `[tool.options]` beyond the matched-mirror default-index
  carve-out (mismatched/explicit index entries, `no-binary`, `index-strategy`, …) — v1 fails these
  loudly; support needs option-aware reconstruction, and the index case interacts with the
  release-fetch limitation below.
- Per-tool release-fetch index: `fetch_releases` runs controller-side with only the package identifier —
  the agent-side receipt is unreachable there, so "latest" is always computed against the configured
  `index_url`. Hosts with tools from mixed indexes see `index_url`-relative latest for the off-index
  subset (documented limitation).
- Tools installed from non-index sources (git/path/URL primaries) still appear in `uv tool list` and are
  discovered as featured items; their release fetch errors each cycle (name absent from the index).
  Suppressing them needs receipt-aware discovery (rejected in Alternatives) — documented limitation.
- Discovery of other users' uv tools (would need sudo/root traversal; conflicts with the unprivileged-agent
  invariant).
- Tracking uv itself as a software item (it is installed via other managers — brew/curl — already covered or
  coverable by their plugins).
- Batch trait-method overrides (`execute_batch_update` etc.): uv has no genuine multi-package pinned-install
  call shape; the default sequential fallback is correct (per `crates/plugins/AGENTS.md`, do not fake batching).

## Verification (mechanical)

- `grep -rn "package_manager_uv" crates/shared/types/src/plugin_type_id.rs crates/plugins/infrastructure/registry/src/registry.rs`
  → const, `ALL`, descriptor, `PACKAGE_MANAGER_IDS` hits all present.
- `grep -c "featured: true" crates/plugins/package-managers/uv/src/*.rs` → non-zero.
- `grep -n "uv" docs/end-user/plugin-configs.md docs/development/autodiscovery-internals.md docs/development/plugin-guidelines.md`
  → table row, emission entry, featured-routing row present.
- `grep -n "package_manager_uv" docs/development/plugin-system.md` → First-Party Plugin Crates row present.
- `grep -c "uptrakit-plugin-package-manager-uv" release-plz.toml` → `3` (`[[package]]` entry + two
  `changelog_include` rows).
- Full canonical gate list from `docs/development/quality-gates.md`, plus `cargo deny check` and
  `python3 ci/check_plugin_semantic_boundary.py`.
