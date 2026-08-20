# Tag-Series Version Handling — Design

**Date:** 2026-08-19
**Status:** Design (pending plan)
**Origin:** promotes bug bead `uptrakit-zgw04`

All `file:line` references are locator hints against `main` @ `f51c04cce`; verify before editing.

## Problem

For the live PHS-installed Uptrakit instance, the Software page shows:

```text
installed:  uptrakit-controller-standalone-v0.0.7
latest:   ↑ uptrakit-service-sdk-v0.0.4
```

Two defects in one row, produced by a verified end-to-end chain:

1. **Raw release tags shown where a version belongs.** The PHS installer records the full monorepo
   tag in `/root/.<app>` (`scripts/pvehs/install/uptrakit-install.sh:19-21`, prefix
   `uptrakit-controller-standalone-v`); PHS detection returns it verbatim (`parse_version_file`,
   `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs:962-968` — trim only). The
   GitHub plugin's `tag_strip_prefix` default `"v"` (`crates/plugins/releases/github/src/config.rs:49`)
   does not match component-namespaced tags, so `strip_tag_prefix` no-ops
   (`src/tag.rs:5-10`, applied at `plugin.rs:199`) and the full tag becomes the "version".
2. **No tag-series constraint on the latest-version lookup.** `fetch_releases` scans
   `/repos/{owner}/{repo}/releases?per_page=100` (`plugin.rs:174-180`, paginated to
   `MAX_PAGES = 10`) with **no tag filtering at any stage**; a release survives `convert_release`
   even when zero assets match `asset_patterns` (`plugin.rs:214-241` returns the release with an
   empty asset list). "Latest" is positional — first non-prerelease in GitHub API order
   (`crates/core/scheduler-runtime/src/executors/fetch_releases.rs:406-410` and
   `crates/ui/web-api/src/routes/software_items/controller_fetch.rs:140-190`) — so a sibling
   crate's tag (`uptrakit-service-sdk-v0.0.4`) can win. The update-available check is a raw string
   inequality (`host_update_available`,
   `crates/ui/web-api-queries/src/queries/software_items/mod.rs:241-249`), so any cross-series
   mismatch renders as "update available".

A release with no matching asset is already un-updatable — `execute_update` errors when no asset
matches (`plugin.rs:593-621`) — so reporting one as "latest" offers an update guaranteed to fail.

The PHS-synthesized fetch target (`github_fetch_target()`,
`crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs:237-250`) sets only
`tag_strip_prefix: "v"`, `include_prereleases: false`, `asset_patterns: []` — the series knowledge
the installer had is discarded. The Codeberg/Forgejo twin (`plugin.rs:262-277`) has the same gap,
and `ForgejoConfig` mirrors `GitHubConfig` (`crates/plugins/releases/forgejo/src/config.rs:18-37`:
`tag_strip_prefix`, `asset_patterns`, no tag filter).

## Decisions (settled with owner, 2026-08-19)

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Version-ordering comparison is out of scope.** `host_update_available` stays `installed != latest`; correctness is restored by series filtering plus both sides normalizing to bare versions. A pinning test guards `2.9.0 → 2.10.0` (the lexicographic trap: `"2.10.0" < "2.9.0"` as strings). Real ordering (parse-both-else-fall-back-to-`!=`, reusing the `Version {raw, parsed}` semantics from `crates/plugins/infrastructure/core/src/version.rs:60-67`, which would need extraction to `uptrakit-shared-types` for dependency-direction reasons) is a **separate follow-up bead**, filed `discovered-from` during implementation.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| D2  | **New `tag_prefix` config field on the GitHub and Forgejo release plugins.** Optional literal string prefix (no regex, no glob — no ReDoS surface, no glob-to-regex translation trap); length-capped at validation. When non-empty, releases whose `tag_name` does not start with `tag_prefix` are dropped during `fetch_releases` scanning (no extra API calls **within a fetch job** — the list is already in hand; the job-count effect of per-item overrides is a known limitation, see "Deferred / out of scope"). Version extraction strips `tag_prefix` first, then applies the existing `tag_strip_prefix` to the remainder (so `tag_prefix = "uptrakit-controller-standalone-"` + `tag_strip_prefix = "v"` and `tag_prefix = "uptrakit-controller-standalone-v"` both yield `0.0.7`). Forgejo is in scope because PHS synthesizes Codeberg targets with the same shape and an unknown field in an override would be silently ignored — a silent no-filter.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| D3  | **Asset-gating.** When `asset_patterns` is non-empty and a release's filtered asset list comes out empty, `convert_release` returns `None` (release dropped) instead of an `UpstreamRelease` with empty assets. Guarded on `asset_patterns` being set so source-only-tag repos are unaffected. Aligns `fetch_releases` with what `execute_update` can actually do. Same change in both plugins. Not load-bearing for the PHS heal chain (PHS synthesizes `asset_patterns: []` and updates via the shell plugin) — general hygiene for operator-configured patterns, bundled here deliberately.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| D4  | **Zero survivors is an error, not empty success.** If the raw fetch returned ≥ 1 release but the D2/D3 filters dropped all of them, `fetch_releases` returns a typed error naming the active filter values (logged like every other fetch error; note that today NO fetch error surfaces in the UI or API — per-item persistence/surfacing is the tracked follow-up bead uptrakit-z4j50, owner-accepted 2026-08-20). This follows the plugin rule that `fetch_releases()` failures must set an error, never silently degrade. The error text must additionally (a) state how many releases were scanned and that the scan window is bounded (`MAX_PAGES = 10` × `per_page=100`) — on a busy monorepo an in-series release may exist beyond the window, so "all filtered out" and "window exhausted before the series appeared" must not read identically; and (b) note that `tag_prefix` may originate from a discovery-synthesized per-host assignment override and say where to change it — without this, an upstream series rename leaves the item permanently erroring with no path back (the stored frozen prefix equals the still-inferred prefix, so no D9 conflict ever fires; the operator's only lever is editing/clearing the overrides, and the error is the only place that tells them so). A rename stales **both** override sides, not just fetch: the detect row's frozen `version_strip_prefix` stops matching the new raw tag, D6's conservative rule passes it through verbatim, and DetectVersion (sole writer, overwrite permitted) writes the raw tag back into `installed_version` — the error text must name both overrides. The text must also warn against the naive half-fix of setting `tag_strip_prefix` to the full prefix (strips without filtering — recreates the phantom this spec fixes). Pre-existing emptiness semantics (repo with no releases; all-prerelease repos under `include_prereleases: false`) are unchanged and not errors.                                                                                                                                                                                                                                                                                                                                  |
| D5  | **PHS prefix/version inference is a pure function.** `split_tag_version(raw: &str) -> Option<(&str, &str)>` in the PHS crate: scan left to right for the earliest position where a version shape (`\d+(\.\d+)+` optionally followed by a pre-release/build suffix) matches **through end of string**; everything before it is the prefix, the match is the version. `uptrakit-controller-standalone-v0.0.7` → (`uptrakit-controller-standalone-v`, `0.0.7`); `v1.2.3` → (`v`, `1.2.3`); `1.2.3` → (``, `1.2.3`); `app2-v1.2.3` → (`app2-v`, `1.2.3`) (the digit run `2-v…` fails the run-to-end rule); `k3s-v1.28.3+k3s1` → (`k3s-v`, `1.28.3+k3s1`). At least one dot required — a bare integer never splits. No match ⇒ `None` ⇒ today's verbatim behavior, no override synthesized (conservative no-match-no-change). Test matrix on the pure function is mandatory.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| D6  | **Detection stores the bare version — via a new generic-shell `version_strip_prefix` field.** (Revised 2026-08-19 after review found the original wording fatal: PHS declares `roles: [Discoverer]` only — `declare_plugin!`, `proxmox-helper-scripts/src/plugin.rs:445-453` — and there is no "PHS `DetectVersion`". The recurring detect path for PHS GitHub-managed apps is the **generic Shell plugin**: `phs_shell_target()` (`plugin.rs:299-313`) assigns `DetectVersion`/`ExecuteUpdate` to `plugin_ids::GENERIC_SHELL` with `version_command` = the sudo version helper, which returns the raw tag verbatim — so an unmodified Shell plugin would rewrite `installed_version` back to the raw tag on every version check, making the phantom permanent.) Fix: `ShellConfig` gains `version_strip_prefix: Option<String>` — literal prefix, same no-regex/no-glob/length-cap rules as D2's `tag_prefix`. After the existing first-trimmed-line extraction (`generic/shell/src/plugin.rs:60-79`), when set and the value starts with the prefix, strip it; otherwise return verbatim (conservative, mirrors D5's no-match-no-change). PHS synthesizes it into the shell target's `config_override` from the same D5 inference (see D7); layer-3 merge delivers it on the recurring detect path (`resolve_effective_config` merges `assignment_config` last — `scheduler-runtime/src/executors/detect_version.rs:69-73`; same call in the manual `version_check` route). Discovery-path `installed_version` writes (NULL-fill only, ADR-0037) still run D5 directly. The raw installed tag is discarded after normalization — nothing downstream needs it (the update path resolves the download from `latest_release_metadata`, not from the installed value). `installed_display_version` stays untouched by this spec (bare canonical is already displayable; `resolveDisplayVersion` in `frontend/src/lib/utils.ts:115-120` falls back to canonical).                                                                                                                                                                                                                                                                                                                                                |
| D7  | **Auto-population goes through `DiscoveryTarget.config_override`, not the shared profile.** The synthesized config named "GitHub Releases" is one row per `(tenant_id, plugin_type, name)` shared by every PHS item of that type (`find_or_create_default_plugin_config`, `crates/ui/web-api-queries/src/queries/autodiscovery/default_configs.rs:43-91`), so a per-item `tag_prefix` cannot live there. PHS sets `config_override: {"tag_prefix": "<inferred prefix>"}` on the github/forgejo fetch target **and** `config_override: {"version_strip_prefix": "<same inferred prefix>"}` on the shell detect/update target (D6). Synthesis conditions are asymmetric because the fields carry asymmetric risk: `tag_prefix` is a _filter_ that can hard-error (D4), so it is synthesized only when the inferred prefix adds series information — non-empty **and** not equal to the target's `tag_strip_prefix` (a bare-`v` prefix would install a filter and a D4 failure mode while changing nothing about extraction); `version_strip_prefix` only normalizes, so it is synthesized for any non-empty prefix. Outcomes stay consistent (`v1.2.3` → `1.2.3` on both sides either way). Note the override lands on **three assignment rows**, not two — the insert path clones `config_override` per role (`discovery_items.rs:322-343`) and the shell target declares `[DetectVersion, ExecuteUpdate]`, so the shell ExecuteUpdate row also carries `version_strip_prefix`; `execute_update` never reads it, but the row participates in D8 fill and D9 conflict uniformly (do not special-case it — per-role cloning is the existing insert behavior). Layer-3 merge applies each override last (narrowest wins). All three rows' `config` is NULL today — the shell target's per-app identity comes from the `{package_identifier}` placeholder in the shared profile's `version_command` (`plugin.rs:285-303`), not from per-item config — so D8's fill-if-NULL heals all three rows on the live deployment. Neither field is sensitive, so the layer-3 sensitive-field rejection does not apply. `asset_patterns` is **not** auto-populated (regex field; the installer's shell glob is not translatable without the glob-to-regex trap — D3 still bites whenever an operator sets patterns manually). |
| D8  | **Fill-if-NULL override refresh on re-discovery.** Today, existing items skip role-assignment processing entirely (`find_or_create_software_item` returns `None` → `continue`, `discovery_items.rs:271-277`; inserts swallow unique-violations at `:338-342`), so existing assignments never receive a new `config_override`. New rule: for an existing assignment row matching a target's `(host_software_item, plugin_type, role)`, discovery writes the target's `config_override` (after the same sensitive-strip as the insert path, `discovery_items.rs:279-319`) **only when the stored `config` is NULL**. Any non-NULL value — operator-set or previously discovery-set — is never touched. Mirrors ADR-0037's `installed_version` rule (never overwrite non-NULL; NULL-fill still writes). Known limitation, accepted: a future changed PHS default will not propagate to rows already holding an override — that is exactly the manual-edit protection wanted.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| D9  | **Conflict notification — per-proposed-key, not whole-object.** The proposed override is a partial object (one key); the stored override may legitimately carry unrelated operator keys (`asset_patterns`, `prefer_interactive`, …). A conflict exists **only when the stored non-NULL override contains the proposed key with a different value**. A stored non-NULL override _without_ the proposed key is a deliberate operator opt-out: no write, no notification (whole-object inequality would turn any unrelated override — or an operator clearing the key — into a permanent 6h notification loop with no way to silence it). A stored row whose `plugin_type` differs from the target's is also a conflict (operator reassigned the role — never write). On conflict, discovery emits a new `NotificationEvent` variant (working name `DiscoveryOverrideConflict`) carrying host, software item, plugin type, role, and a proposed-vs-stored summary — plus a `warn!` log. Fire-and-forget through the existing dispatcher (bounded mpsc, overflow-drop); values are non-sensitive by construction (layer-3 strip), HTML-escaped at render per notifications policy. It re-fires on every 6h discovery pass while the conflict persists — accepted for now ("notifications might not be working properly and it's fine for now" — owner); dedup/throttling is deferred.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| D10 | **Positional "latest" selection is out of scope.** The first-non-prerelease-else-first positional pick in both fetch paths, and the lexicographic `.max()` in `load_latest_version_for_item` (`software_items/mod.rs:294`), stay as-is; both are covered by the D1 follow-up bead (they need the same version-ordering primitive). With the series filter in place, GitHub's created-desc order makes the positional pick correct for the monorepo case.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| D11 | **No migration; healing is organic.** Next discovery run fill-if-NULLs the overrides on the live deployment's existing assignments (all three rows per item — fetch FetchReleases, shell DetectVersion, shell ExecuteUpdate; `config` is NULL today on each, see D7) → next scheduled fetch rewrites `latest_version` as a bare, in-series version and refreshes `latest_release_metadata` → next version check runs the Shell plugin with `version_strip_prefix` merged in and writes the bare `installed_version` (`apply_version_update_to_db`, `crates/ui/web-api/src/routes/service_ws/handler/messages/version_check.rs:107-163`; ADR-0037 permits — DetectVersion is the sole version writer for active items, and post-D6 that writer produces the bare version instead of re-clobbering with the raw tag). A transient false "update available" between the two writes is possible and self-resolves within one cycle of each task.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| D12 | **One short ADR** (via `adrs new`, never hand-allocated) for the new discovery-write invariant: _discovery never overwrites a non-NULL `host_software_item_plugins.config`; NULL-fill writes; conflicts notify._ Extends the ADR-0037 family to a second column.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| D13 | **GitLab release plugin is out of scope.** PHS does not synthesize GitLab targets; the tag-filter class applies there too but lands with the D1 follow-up or a later bead. Docker plugin is unaffected (image tags, different model).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

Alternatives rejected during grilling: `tag_pattern` regex field (ReDoS review for no shown need;
prefix covers the monorepo case exactly); auto-populating `asset_patterns` from the installer's
shell glob (glob compiles as regex today — `standalone-*` parses as "standalone" + zero-or-more
hyphens — silent wrong matches); asset-gating alone without a tag filter (indirect, and requires
the untranslatable glob); a `latest_display_version` split instead of normalizing at source (the
comparison would then compare across spaces: full tag vs bare version); installer-side recording of
the prefix via a PHS upstream convention (no such convention exists; inference is self-contained
and covers third-party PHS apps); unconditional override refresh (clobbers operator edits — owner
rejected); provenance column for override ownership (heavier than fill-if-NULL for the same
protection). Detect-path alternatives rejected in the D6 revision: giving the PHS plugin its own
`DetectVersion` role (existing live rows hold the Shell assignment; D8 fill-if-NULL never
reassigns `plugin_type` and D9 treats a differing `plugin_type` as a conflict, so this needs a
role-reassignment/migration story the spec otherwise avoids); normalizing inside the
`uptrakit-phs-version` helper script (reimplements D5 in shell, drifts from the Rust test matrix,
and only heals hosts after the helper redeploys). The chosen `version_strip_prefix` reuses the
exact mechanisms the spec already builds — D5 inference, `config_override` synthesis, layer-3
merge, D8 fill-if-NULL — with no role or script changes.

## Component design

### 1. Release plugins (`crates/plugins/releases/github`, `crates/plugins/releases/forgejo`)

- `tag_prefix: Option<String>` on `GitHubConfig`/`ForgejoConfig` with serde default `None`;
  `FormFieldDescriptor` entry alongside `tag_strip_prefix`; validation rejects over-long values
  via a **new** length-cap constant. Existing conventions: `validate_command_length()` /
  `MAX_COMMAND_LENGTH` (`uptrakit-shared-types::command_validation`) is for command strings only
  — prefixes are not commands, so it does not literally apply — and `MAX_INSTALL_PATH_LENGTH` on
  `install_path` (`github/src/config.rs:127-198`) is crate-local precedent. Three crates need the
  cap (github, forgejo, shell §3): define **one shared prefix-length constant** in
  `uptrakit-shared-types` as a new dedicated sibling module (the codebase idiom is one module per
  validation concern — `package_identifier.rs` beside `command_validation.rs` in
  `crates/shared/types/src/lib.rs`; never append non-command limits into `command_validation.rs`,
  whose scope is command strings only) rather than three near-duplicate crate-local constants;
  follow the `MAX_INSTALL_PATH_LENGTH` validation structure, value sized for tag prefixes.
- Filter applied inside the existing release-conversion pipeline (`convert_release` or its caller):
  tag-prefix mismatch ⇒ skip; then D3 asset-gating; both before the release enters the collected
  list. Stripping order: `tag_name` → strip `tag_prefix` → strip `tag_strip_prefix` → `Version::new`.
- D4 error: the counters must be staged so pre-existing skips are not misattributed to the new
  filters. Draft-skip and prerelease-skip are already `return None` arms inside `convert_release`
  (`plugin.rs:184-196`) — the same function D2/D3 extend — so a naive raw-vs-surviving comparison
  would hard-error any all-draft or all-prerelease repo the moment `tag_prefix` is set,
  contradicting D4's "pre-existing emptiness semantics are unchanged". Track three counts:
  `raw_count` (releases returned by the API scan), `baseline_count` (survivors of the existing
  draft/prerelease filtering, before D2/D3), `surviving_count` (after D2/D3). Error condition:
  `baseline_count > 0 && surviving_count == 0` with either new filter active ⇒ typed
  `PluginError` naming the filters, the scanned-release count (`raw_count` — used only for the
  scan-window clause), the bounded window (window-exhausted must read differently from
  filtered-out — D4), and the possible override origin of `tag_prefix` with the recovery lever
  (edit/clear the per-host assignment override). Not retryable. `baseline_count == 0` keeps
  today's empty-success behavior regardless of the new filters.
- Empty post-strip remainder drops the release (mirrors §3's shell-side rule): a tag exactly
  equal to the composed prefix would otherwise survive the filter and yield `Version::new("")`
  as `latest_version`. A so-dropped release counts as filtered (does not increment
  `surviving_count`), so an all-marker-tag series feeds the D4 error rather than an empty
  string.
- The seam decisions (bail vs degrade-to-empty; divergence between the two plugins) each get their
  own test — a "covered by the pure layer" waiver is not acceptable here.

### 2. PHS discovery plugin (`crates/plugins/discovery/proxmox-helper-scripts`)

- `split_tag_version()` pure function per D5, unit-test matrix (prefixed, `v`-only, bare, digits in
  name, `+build`/`-rc` suffixes, no version, empty, dotless integer). Implementation may use the
  `regex` crate (already in `[workspace.dependencies]`, root `Cargo.toml:88`; the PHS crate adds a
  `workspace = true` reference — never a local version pin) or a manual scanner; a manual scanner
  must not add new lint suppressions (`indexing_slicing`/`string_slice` are workspace-denied;
  any `#[expect]` needs the standard justification).
- `parse_version_file` is defined in `discovery.rs:962-968`; its sole production caller is
  `phs_version()` (`plugin.rs:199-210`), invoked from the discovery loop at `plugin.rs:544-580`
  (GitHub) / `:588+` (Codeberg) — the site where `installed_version` and the target vector are
  built together, which is where D5 hooks in: run the inference once on the raw value
  `phs_version()` returned and feed both the normalized `installed_version` (NULL-fill only per
  ADR-0037) and the override synthesis below — do not re-read or re-derive per target. Note the
  existing basename asymmetry: `phs_version()` is called with the `unwrap_or(&script.slug)`
  fallback `vfb` while `phs_shell_target()` receives the raw
  `analysis.version_file_basename.as_deref()` — the inference input is the version _value_, not
  the basename, so this asymmetry is untouched. The recurring detect path is not PHS code — it
  is the Shell plugin (D6, §3).
- `github_fetch_target()` / codeberg twin: when D5 yields a prefix that is non-empty **and** not
  equal to the target's `tag_strip_prefix` (D7's filter-risk condition), set
  `config_override: Some(json!({"tag_prefix": prefix}))`; else `None` as today.
- `phs_shell_target()`: when the same D5 inference yields any non-empty prefix, set
  `config_override: Some(json!({"version_strip_prefix": prefix}))`; else `None` as today. One
  inference feeds both targets — same inference, different applicability (D7): the overrides may
  legitimately differ in presence (bare-`v` case synthesizes only the shell side) but never in
  value when both are present.

### 3. Generic Shell plugin (`crates/plugins/generic/shell`)

- `version_strip_prefix: Option<String>` on `ShellConfig` (`src/config.rs:21-29` region) with
  serde default `None`; `FormFieldDescriptor` entry alongside `version_command`; validation
  rejects over-long values via the shared prefix-length constant defined in §1.
- Applied in `detect_installed_version` after the existing first-non-empty-trimmed-line
  extraction (`src/plugin.rs:60-79`): value starts with the prefix ⇒ strip it; otherwise return
  the value verbatim (no error, no partial strip). Empty post-strip result is treated as no
  version detected (`Ok(None)`), not an empty string.
- `update_command`, `prefer_interactive`, and every other field are untouched; a config without
  the new field behaves exactly as today (dark-first, inert until discovery populates it).

### 4. Autodiscovery target processing (`crates/ui/web-api-queries/src/queries/autodiscovery`)

- Existing-item path gains the D8 fill-if-NULL write and D9 conflict detection. The write applies
  the same sensitive-strip logic as the insert path — which today is an **inline** match block
  inside `process_targets_discovery` (`discovery_items.rs:279-319`), not a callable helper: the
  plan must first extract that block into a single helper function, then call it from both the
  insert path and the new fill path (widen, don't copy).
- **Reaching the existing-item branch requires a contract change.** Today
  `find_or_create_software_item` returns `Some((software_item_id, hsi_id))` only when a new link
  was created; the existing-link path returns `None` and the caller `continue`s
  (`discovery_items.rs:271-277`) — it never enters the per-role assignment loop and holds no
  reference to the matched link's assignment rows. The plan must either change the return contract
  to distinguish created-vs-existing (both carrying the ids) so the caller runs a per-role
  fill/conflict pass on both branches, or move that pass inside the function before it returns.
  Either way the base insert path's behavior is unchanged.
- Matching key for existing assignments: the table's live unique index —
  `uq_hsip_hsi_role_ordinal` on `(host_software_item_id, role, ordinal)` with `ordinal = 0`
  (discovery-created assignments only use ordinal 0 today;
  `m20260318_000001_host_software_item_qualifier.rs:681`). Do **not** match on
  `(host_id, software_item_id, …)` — the qualifier column means multiple `host_software_item`
  rows can share that pair, and the similarly-named `idx_hsip_host_item_role_ordinal`
  (`m20260326_000001`) is non-unique; the caller already holds the `hsi_id`. `plugin_type` is not
  part of the key — the stored row's `plugin_type` is compared as data: if it differs from the
  target's, the operator has reassigned the role — never write, treat as a D9 conflict.
- **The fill write is a single atomic conditional UPDATE**, never select-then-write for the write
  decision — the discovery write path runs on a plain `DatabaseConnection` with no wrapping
  transaction, and concurrent discovery passes must not race the NULL check. The WHERE clause is
  the full decision: `host_software_item_id = ? AND role = ? AND ordinal = 0 AND plugin_type = ?
AND config IS NULL` (via SeaORM builders). A 0-rows-affected result conflates three cases and
  the follow-up read must discriminate all of them: (a) row exists, `config` non-NULL ⇒ run the
  D9 per-key comparison; (b) row exists, `plugin_type` differs ⇒ D9 conflict (role reassigned);
  (c) row absent (assignment deleted) ⇒ no write, no notification. The read happens after the
  write decision, not before it (the conflict check is best-effort notification, not a guarded
  write).
- Equality for conflict detection: per-proposed-key (D9) — JSON value equality of the stripped
  proposed key's value vs the stored override's value at that key; stored override lacking the
  key ⇒ opt-out, no event.
- Audit: no `audit-catalog.toml` entries exist today for discovery-pipeline
  `host_software_item_plugins` writes, and `discovery_items.rs` opens no transaction — the only
  audit precedent in this path is Event-kind `emit_event` (`emit_reactivation_event`). Default:
  classify the fill-if-NULL write as an Event following that precedent (automated NULL-fill by
  discovery, old value NULL by definition — no snapshot pair to capture); if the audit-coverage
  gate or planning review instead demands Stateful, the write must gain `begin_immediate()`
  wrapping, which does not exist in this path today. The conflict notification is an Event either
  way.
- **Dispatch direction:** `uptrakit-web-api-queries` has no dependency path to
  `NotificationDispatcher` (it lives in `web-api`). Conflict data is returned from the discovery
  processing functions and dispatched by the web-api layer's discovery handler
  (`routes/service_ws/handler/messages/discovery.rs`), mirroring the existing
  `NewSoftwareDiscovered` flow — never dispatched from inside web-api-queries. This is a second
  explicit return-contract change: `process_discovery_results` currently returns `Result<()>`
  (`autodiscovery/mod.rs:110`) and must gain a payload carrying the detected conflicts for the
  handler to dispatch.

### 5. Notifications (`crates/shared/audit-log`-adjacent producer + `crates/plugins/notifications`)

- New internal `NotificationEvent` variant; producer emits from the web-api discovery handler
  post-processing (fire-and-forget; never blocks or fails the discovery run) — see §4 dispatch
  direction.
- The change surface spans two crates with exhaustive matches extended in lockstep: a new
  `NotificationEventDetails` variant + `event_type()` arm
  (`crates/plugins/notifications/delivery/src/event.rs`), a new **closed**
  `NotificationEventType` variant
  (`crates/shared/web-api-types/src/notifications/event_types.rs` — `Other(String)` is
  wire-forward-compat only, never used for locally-produced events), and a new `build_content()`
  arm (`crates/plugins/notifications/delivery/src/message_builder.rs`).
- Rendering in the core notification plugin follows existing patterns; `escape_html()` on all
  interpolated values.

### 6. Frontend

- Version rendering: no changes. Values themselves become bare versions; every render site
  (including the raw `{host.installed_version ?? 'unknown'} -> {host.latest_version}` at
  `frontend/src/routes/software/+page.svelte:1435`) is fixed by data. Deliverable-to-render
  trace: bare version reaches the pixel via existing `SoftwareItemResponse.installed_version` /
  `latest_version` fields — verified consumer chain, no dropped field.
- Notification event type: **changes required.** `NotificationEventType` is a REST contract type
  — it is the `event_type` field of `CreateNotificationRuleRequest` /
  `UpdateNotificationRuleRequest` (`crates/shared/web-api-types/src/notifications/rules.rs:14,31`)
  and lands in the generated client as a closed union
  (`frontend/src/lib/api/generated/types.gen.ts:1289-1301`). Adding the D9 variant requires
  `./scripts/regen-api.sh` (commit `crates/ui/web-api/openapi.json` +
  `frontend/src/lib/api/generated/` — CI gates on staleness) **and** a new key in the exhaustive
  `EVENT_TYPE_LABELS: Record<NotificationEventType, string>` map
  (`frontend/src/routes/settings/NotificationRulesSettings.svelte:26` — `npm run check` fails on
  the missing key otherwise); verify `NotificationLogView.svelte`, the other
  `NotificationEventType` consumer, in the same pass.

## Data flow (after)

```text
installer writes /root/.<app> = "uptrakit-controller-standalone-v0.0.7"   (unchanged)
  → PHS discovery: split_tag_version → installed_version NULL-fill = "0.0.7"
  → fetch target:  config_override = {"tag_prefix": "uptrakit-controller-standalone-v"}
    shell target:  config_override = {"version_strip_prefix": "uptrakit-controller-standalone-v"}
  → layer-3 merge → GitHubConfig { tag_prefix: Some(...), tag_strip_prefix: "v", ... }
                    ShellConfig  { version_command: <helper>, version_strip_prefix: Some(...) }
  → fetch_releases: filter tags by prefix → strip prefix(es) → latest_version = "0.0.7"
      (full tag preserved in latest_release_metadata.tag — update dispatch reads it:
       crates/ui/web-api-queries/src/queries/update_dispatch.rs:1178-1191)
  → recurring version check: shell helper prints raw tag → strip prefix →
      installed_version stays "0.0.7" (no re-clobber)
  → host_update_available("0.0.7", "0.0.7") = false  ✓
```

## Security

- No regex from user config in either new field (`tag_prefix`, `version_strip_prefix` — both
  literal prefixes); existing `asset_patterns` regex handling unchanged.
- `config_override` writes keep the fail-closed sensitive-strip (agents remain untrusted for
  layer-3 content); the fill-if-NULL path may not bypass it.
- Notification values HTML-escaped; no secrets can appear (layer-3 is sensitive-free by
  construction).
- No new sudo, SSRF, or wire surface. `DiscoveryTarget` shape is unchanged (existing
  `config_override` field carries new values only) — no asyncapi regen.

## Testing

- Pure inference matrix (D5) — success and failure shapes.
- GitHub + Forgejo plugin: prefix filtering, strip composition (both prefix variants), D3 drop,
  D4 error (and: filters inactive ⇒ no error), counter staging (all-prerelease repo under
  `include_prereleases: false` **with `tag_prefix` set** ⇒ empty success, not error —
  `baseline_count == 0`), tag equal to the composed prefix ⇒ release dropped (empty remainder,
  §1), prerelease emptiness unchanged, per-plugin (no shared-fixture waiver).
- Shell plugin `version_strip_prefix`: matching prefix stripped; non-matching output returned
  verbatim; field unset ⇒ behavior identical to today; output equal to the prefix (empty
  remainder) ⇒ `Ok(None)`; validation rejects over-long values.
- PHS target synthesis: prefixed tag ⇒ both overrides carry the same inferred prefix; bare-`v`
  inference ⇒ shell override only, fetch `config_override: None` (D7 asymmetry); no-prefix
  inference ⇒ both `None`.
- `host_update_available` pinning tests: `2.9.0` vs `2.10.0` ⇒ update; equal bare versions ⇒ none.
- Autodiscovery: fill-if-NULL writes on all three assignment rows (including shell
  ExecuteUpdate); non-NULL untouched; per-key conflict semantics (D9): stored override contains
  the proposed key with a different value ⇒ conflict event; equal value ⇒ no event; stored
  non-NULL override **without** the key ⇒ opt-out, no write, no event; stored `plugin_type`
  differing from the target's ⇒ no write + conflict event; assignment row absent ⇒ no write, no
  event; sensitive-strip applied on the fill path; existing-item version preservation (ADR-0037)
  unaffected.
- Round-trip: update dispatch still resolves the full tag from `latest_release_metadata` after
  normalization (regression test at the `update_dispatch` seam).
- No `start_paused` on DB tests; TestApp harness for any new endpoint-level assertions.

## Documentation deliverables

Implementation must grep, not hand-list — the greps below are the starting set; their result sets
are the edit list (`rg -l 'tag_strip_prefix|asset_patterns' docs/`, plus concept-prose greps for
"latest version"/"release tag" over `docs/` and root markdown):

- `docs/end-user/plugin-configs.md` — `tag_prefix` rows for GitHub and Forgejo tables (`:57`, `:91`
  region), `version_strip_prefix` row for the generic-shell table, asset-gating + zero-survivor
  error semantics, and an explicit `tag_prefix` vs `tag_strip_prefix` distinction (filter+strip
  vs strip-only — setting `tag_strip_prefix` to a full series prefix strips without filtering
  and recreates the cross-series phantom).
- `docs/development/plugin-guidelines.md` — release-plugin field table (`:1687` region) and the
  PHS GitHub-managed-apps note (`:1721` — currently says the config carries only three fields).
- `docs/end-user/notifications.md` — new event type in the catalog, plus the note that no
  existing rule matches it until the operator creates one (acceptance-criteria caveat).
- `docs/development/autodiscovery-internals.md` — fill-if-NULL override refresh, conflict
  notification, inference behavior; **also fix the stale unique-index prose** (`:412` region still
  cites `(host_id, software_item_id, role, ordinal)`; live index is `uq_hsip_hsi_role_ordinal` on
  `(host_software_item_id, role, ordinal)` since `m20260318`).
- `docs/architecture/software-item-entity.md` — check `:65` context (per-host `asset_patterns` /
  `tag_strip_prefix` prose) for accuracy after the change; comparison prose stays (D1).
- New ADR (D12) via `adrs new`.
- `AGENTS.md` autodiscovery stub — one invariant line for the override rule (subject to the
  size-budget gate).
- **OpenAPI regen required** for the new `NotificationEventType` variant (REST contract — §6):
  `./scripts/regen-api.sh`, commit `crates/ui/web-api/openapi.json` and
  `frontend/src/lib/api/generated/` in the notification commit. No asyncapi regen (wire types
  unchanged — `DiscoveryTarget` shape untouched). Plugin form schemas (`tag_prefix`,
  `version_strip_prefix` descriptors) are served dynamically, not via OpenAPI.

## Dependencies and sequencing

- **Cross-cycle:** `uptrakit-w9121` (PHS base-OS compatibility) — soft relation only (same-files
  overlap on `proxmox-helper-scripts/src/{discovery,plugin}.rs` and notification events; no
  decision or landed-code dependency either way). Whichever implements second rebases over the
  other's PHS-plugin changes.
- **Commit sequencing (dark-first):** release-plugin `tag_prefix`/asset-gating/error and shell
  `version_strip_prefix` land first (all inert until configured — no behavior change for existing
  configs with the fields unset); PHS inference + dual override population next; fill-if-NULL +
  notification last. The notification commit must bundle the backend variant, the OpenAPI regen
  outputs, and the frontend `EVENT_TYPE_LABELS` key (§6) — splitting them fails the generated-file
  staleness gate or `svelte-check` at the intermediate commit. Every intermediate commit's
  deployed behavior is baseline-or-better.

## Deferred / out of scope

- Version-ordering comparison (`host_update_available`, `load_latest_version_for_item` max,
  positional latest selection) — follow-up bead, filed `discovered-from` during implementation
  (D1/D10).
- Typed version value (raw_tag, extracted_version, series) end-to-end — direction note from the
  origin bead (fork 5); superseded structurally by D6/D7 for this bug class.
- GitLab plugin tag filter (D13).
- Notification dedup/throttling for repeated conflicts (D9).
- Auto-population of `asset_patterns` (D7 rationale).
- **Fetch-group splitting from per-item overrides (accepted limitation).** Controller-side
  scheduled fetch batches by `(plugin_config_id, assignment_config)`
  (`phase_a_group_key()`, `crates/core/scheduler-runtime/src/executors/fetch_releases.rs`), so N
  distinct `tag_prefix` overrides against one repo produce N paginated scans per fetch cycle
  instead of one — each scan is up to `MAX_PAGES = 10` API calls, so the multiplier is up to
  10×N against a 60 req/hr unauthenticated GitHub limit (identical overrides — same app on many
  hosts — still share a group). Bounded by the number of distinct components tracked per repo; a
  shared-raw-fetch optimization belongs with the D1 follow-up if rate limits ever bite. The same
  mechanism exists on the agent detect path: `version_check.rs` groups assignments by
  `(PluginTypeId, effective_config)` (`crates/shared/agent-core/src/version_check.rs:108-150`),
  so per-item `version_strip_prefix` overrides fragment the single PHS shell detect group into
  one per distinct prefix — accepted too (no external rate limit; the default `batch_detect`
  already loops per item).
- **Upstream series rename recovery (accepted limitation).** If upstream renames the tag series,
  the stored `tag_prefix` override keeps filtering everything and D4 errors persist; no D9
  conflict fires because PHS keeps inferring the old prefix from the unchanged `/root/.<app>`
  file (proposed == stored). The stale state spans both override sides: once the version file
  does change (e.g. an out-of-band installer re-run), the detect row's frozen
  `version_strip_prefix` stops matching and the raw tag re-enters `installed_version` verbatim —
  visually the original bug — until the overrides are fixed (that file change does flip the D9
  comparison to proposed ≠ stored, so a conflict notification fires then). Recovery is manual —
  clear/edit the assignment overrides — and the D4 error text is required to point there and
  name both (D4). Automatic recovery would need installer-refreshed state or prefix
  re-validation against live tags; out of scope.

## Acceptance criteria

- Software list and detail show bare versions (e.g. `0.0.7`), not release tags, for
  PHS-installed items carrying prefixed tags (after one heal cycle per D11).
- The latest-version lookup for an item with an inferred tag series only considers releases in
  that series; a sibling crate's tag is never offered as its update; cross-series mismatches do
  not render as "update available".
- `2.9.0 → 2.10.0` still reads as an update (pinned by test); no ordering introduced. Corollary
  (accepted, D1): within-series ordering stays `!=` — a series downgrade or re-published older
  release still reads as "update available"; fixed by the D1 follow-up bead, not this spec.
- Triggering an update after normalization still resolves the real downloadable tag
  (round-trip via `latest_release_metadata.tag`, pinned by test). Guards the non-PHS GitHub
  `execute_update` path — PHS updates run the shell command, which takes no `{tag}` placeholder.
- With `asset_patterns` set, a release with zero matching assets is never reported as latest; a
  filter set that eliminates every release surfaces an explicit error.
- Re-discovery fills NULL overrides on existing assignments, never touches non-NULL ones, and
  on a differing manual override emits the `warn!` log and dispatches the conflict event
  (asserted at unit level against the dispatcher seam). End-user delivery is **not** an
  acceptance criterion: notification rules are per-`event_type` rows the operator creates, so a
  brand-new variant matches zero existing rules until one is added (via the §6 frontend
  dropdown) — document this in the end-user notification docs.
- Live-deployment check: `ssh root@uptrakit` instance shows bare versions and no phantom update
  after one discovery + fetch + detect cycle, with the agent on the same build (an older agent
  ignores the unknown `version_strip_prefix` key — `ShellConfig` is not `deny_unknown_fields` —
  and keeps reporting the raw tag until upgraded).
