# ST-0031 Track C Production Semantic Gate

## Goal

Establish a blocking zero-tolerance semantic-boundary check for non-plugin production consumers so that, outside the boundary-owner surfaces,
production code does not:

- depend on plugin crates directly
- depend on `uptrakit_plugin_infrastructure_core` directly
- reference concrete plugin IDs via `plugin_ids::...`
- use plugin-identity helper methods such as `PluginTypeId::is_package_manager()` or `display_name()`
- hardcode concrete plugin-type identifiers such as `"releases_github"` or `"enhancement_dashboard_icons"`

This is the Track C follow-on to the earlier static-edge cleanup and semantic cleanup work.

## Production scope

### In scope

The blocking gate applies to these production consumer surfaces:

- non-plugin Rust source under `crates/**`
- production frontend source under `frontend/src/**`
- `Cargo.toml` manifests for in-scope non-plugin Rust crates

### Boundary-owner surfaces

These production surfaces are allowed to own plugin-specific knowledge and are therefore excluded from the Track C gate:

- plugin implementation crates under `crates/plugins/**`
- the unified registry/catalogue implementation under `crates/plugins/infrastructure/registry/**`
- the canonical plugin-type definition module at `crates/shared/types/src/plugin_type_id.rs` only for exact plugin ID constant definitions; semantic
  helper definitions and uses remain in scope there

### Non-production exclusions

The gate does not apply to:

- `docs/**`
- tests
- examples
- migrations

These non-production areas may continue to reference plugin-specific identifiers and plugin crates where useful for documentation, fixtures, or
targeted verification.

The exclusions must be operationally defined in the checker:

- dedicated non-production paths must be excluded by path pattern
- inline Rust test modules inside otherwise in-scope files must be excluded by prefix scanning or an equivalent syntax-aware mechanism
- frontend test/story/fixture files inside `frontend/src/**` must be excluded by path pattern
- the checker must not rely on whole-file carve-outs when only an inline test section is exempt
- prefix scanning is acceptable only as a compatibility bridge for current known mixed files; as coverage expands, new mixed files should use
  block-aware or syntax-aware filtering rather than a growing hand-maintained prefix list
- exclusion patterns must be path-scoped rather than broad filename suffixes alone, because some production files use names like `config_test.rs`
  without being test-only code

## Boundary policy

### Allowed production dependency surface

- for in-scope production consumer code, the unified plugins registry/catalogue surface

### Disallowed production dependency surface

- concrete plugin crates
- `uptrakit_plugin_infrastructure_core`
- ad hoc semantic knowledge about specific plugins or plugin families outside the registry/catalogue

If in-scope production consumer code needs plugin metadata, capability classification, display labels, or type-specific dispatch decisions, it must
obtain that information through the registry/catalogue surface rather than encoding it locally.

This also applies to execution-substrate crates such as `crates/shared/agent-core/**` and `crates/shared/scheduler-engine/**`: they remain in-scope
consumers, not boundary owners. If they still require plugin execution primitives, Track C must extend the registry/catalogue surface or add an
approved registry-owned re-export layer rather than exempting those crates.

## Enforcement shape

Use the existing enforcement entrypoints:

- `ci/check_plugin_semantic_boundary.sh`
- `.husky/pre-push`

Track C should start from this existing gate and substantially extend it rather than introduce a second parallel checker.

The checker should block these rule families in in-scope production consumer code:

1. Direct imports of concrete plugin crates outside the boundary-owner surfaces.
2. Direct imports of `uptrakit_plugin_infrastructure_core` outside the boundary-owner surfaces.
3. `plugin_ids::...` references outside the boundary-owner surfaces.
4. Definitions and callsites of the forbidden semantic helper APIs `PluginTypeId::is_package_manager()` and `PluginTypeId::display_name()` outside the
   boundary-owner surfaces, including helper definitions in `crates/shared/types/src/plugin_type_id.rs`, which remain explicitly in scope and must be
   rejected there.
5. Direct crate manifest dependencies on concrete plugin crates or `uptrakit_plugin_infrastructure_core` in `Cargo.toml` files for in-scope non-plugin
   crates.
6. Raw hardcoded plugin-type string literals outside the boundary-owner surfaces.

The string-literal rule must be exact and context-aware rather than prefix-based:

- it must match only quoted string literals, not arbitrary identifiers or substrings
- it must match only in plugin-identity contexts rather than arbitrary text
- plugin-identity contexts include:
  - assignments, comparisons, or serialized payload values for plugin-typed fields such as `plugin_type` and `channel_type`
  - arguments passed to `PluginTypeId::new(...)` or `PluginTypeId::from_static(...)`
  - plugin-type path construction such as plugin type settings/config routes
- it must match against the canonical concrete plugin ID set in `plugin_ids::ALL`
- it must not flag the canonical constant definitions in `crates/shared/types/src/plugin_type_id.rs`
- it must use a syntax-aware helper for literal extraction and surrounding context rather than a plain substring grep
- for Rust, that helper must understand normal and raw string literals
- for frontend code, that helper must understand single-quoted, double-quoted, and non-interpolated template string literals in `.ts`, `.js`, and
  `.svelte` files

Examples of literals that should be rejected outside the boundary-owner surfaces:

- `"releases_github"`
- `"package_manager_apt"`
- `"generic_shell"`
- `"hook_systemd"`
- `"webhook"`
- `"enhancement_dashboard_icons"`
- `"discovery_proxmox_helper_scripts"`
- `"infrastructure_proxmox"`

The checker should validate its own target globs and fail on misconfiguration if an expected target set matches zero files. That validation must
operate from the repository root so that both `crates/**` and `frontend/**` targets are covered.

Manifest scanning must be explicit rather than heuristic:

- it must inspect non-dev dependency sections in in-scope crate manifests, including renamed dependencies via `package = "..."`
- it must include target-specific non-dev dependency tables
- it must account for `workspace = true` / `[workspace.dependencies]` indirection when an in-scope crate consumes a banned dependency through the
  workspace
- it must ignore `dev-dependencies` and `target.'cfg(test)'.dependencies`
- it should resolve workspace indirection via a structured manifest parser rather than shell regexes

The current script coverage is only a narrow seed. Track C must expand target coverage beyond the current UI-only `plugin_ids` scan to all in-scope
non-plugin consumer surfaces.

The bash entrypoint may delegate to a helper script or binary where needed. In particular:

- the canonical plugin ID set must come from a structured source derived from `plugin_ids::ALL`, not from a duplicated hardcoded list in bash
- syntax-aware literal extraction for Rust/frontend and workspace-aware manifest resolution may use helper tooling rather than shell regexes alone
- structured TOML allowlist parsing, when needed, may use helper tooling rather than shell parsing
- comments, docs, and non-identity error strings that merely contain plugin ID substrings are not matches

The same semantic-boundary check must run in both places:

- CI must invoke the Track C boundary check as a blocking repository gate
- `.husky/pre-push` must invoke the same boundary check before push, so local pushes fail before CI when production semantic-boundary violations are
  present
- the pre-push hook should call the shared Track C check entrypoint rather than duplicate rule logic inline
- if the local hook intentionally skips because required tooling is unavailable, CI remains authoritative and blocking

## Exception mechanism

Default policy is zero-tolerance.

If a future exception is required, it should be added to an external checked-in allowlist file.

The allowlist format must be explicit and narrow. In v1:

- use a checked-in TOML file
- use these canonical rule identifiers:
  - `concrete-plugin-import`
  - `plugin-core-import`
  - `plugin-ids-reference`
  - `forbidden-plugin-helper`
  - `manifest-plugin-dependency`
  - `hardcoded-plugin-type-literal`
- each entry must contain:
  - exact file path
  - exact rule identifier
  - exact match kind
  - exact match value
  - reason
- globs are not allowed in paths
- regex is not allowed in the match value field

Allowed `match_kind` values are:

- `literal_string`
- `crate_name`
- `import_path`
- `api_name`
- `manifest_dependency`
- `module_token`

Do not use file-wide carve-outs for production code. The existing inline-test prefix-scan mechanism is acceptable because it narrows the scan to the
production prefix of a mixed file rather than exempting the whole file.

The allowlist is temporary bootstrap scaffolding only:

- do not create the file unless an exception is actually needed
- if the file is created and later reaches zero entries, delete it
- once the allowlist reaches zero, the default CI path must return to a bare zero-tolerance rule with no active allowlist file; any future exception
  requires an explicit follow-up change to re-enable the documented allowlist mechanism

## Current status

The production tree is not yet clean enough to flip the final Track C gate immediately.

Known production residue still exists in representative areas such as:

- `crates/shared/agent-core/src/client.rs`
- `crates/shared/scheduler-engine/src/executors/fetch_releases.rs`
- `frontend/src/routes/software/[id]/+page.svelte`

The earlier dashboard-icons production seam appears resolved, but broader semantic residue remains across production Rust and frontend code.

## One-shot rollout

Track C should land as a single blocking change:

1. Clean current production violations.
2. Extend `ci/check_plugin_semantic_boundary.sh` to the final strict production scope, including frontend and manifest scanning, plus any helper
   tooling required for canonical ID extraction, syntax-aware literal detection, workspace-aware manifest resolution, and structured allowlist
   parsing.
3. Add support for the external allowlist format, but only create the allowlist file if an exception is actually needed.
4. Wire the check into CI as a blocking gate and into `.husky/pre-push` as the local pre-push gate.
5. Add a short maintainer note explaining that docs, tests, examples, and migrations are exempt, but production code is not.

As part of step 1, run the expanded checker first and capture the full production residue inventory before cleanup, rather than treating the current
short list in this document as exhaustive.

## Acceptance

Track C is complete when:

- in-scope production Rust, frontend code, and in-scope crate manifests pass the semantic-boundary check with no unallowlisted violations
- `.husky/pre-push` invokes the shared Track C boundary check before push
- no allowlist file exists unless an active exception is required; if present, it is documented, bounded, and non-empty
- docs/tests/examples/migrations remain intentionally out of scope
- no production consumer code outside the boundary-owner surfaces depends on concrete plugin crates, `uptrakit_plugin_infrastructure_core`, concrete
  plugin IDs, semantic helper shortcuts, or hardcoded plugin-type identifiers
