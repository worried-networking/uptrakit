# Plugin-Guidelines Realignment — Design

- **Date:** 2026-07-12
- **Status:** Draft (spec)
- **Scope:** Documentation correctness. No code, no ADR, no wire/OpenAPI/frontend change, no new dependency.
- **Audit source:** `.superpowers/audit-2026-07-11.md` findings at L360 (HIGH), L388 (HIGH), L949 (MEDIUM), ~L22 (MEDIUM).

## Problem

`docs/development/plugin-guidelines.md` documents a plugin registration/authoring system that no longer
exists. Four independent drift areas, all in this one file, all describing APIs that were refactored out:

1. **Registration system deleted (HIGH, L360 / doc L962).** The doc's "Register in `PluginRegistry`"
   step, its ~40-line "`register_plugins!` macro" section, and the surrounding "package identifier
   validation" prose all reference a `register_plugins!` macro and a `struct PluginRegistry` that were
   deleted. A reader following these steps writes code that does not compile.

2. **`declare_plugin!` invocation syntax obsolete (HIGH, L388 / doc L266).** The "Syntax reference" and
   all five worked examples (APT, GitHub, Shell Hook, Webhook, Dashboard Icons) use a field-style
   `declare_plugin! { id:, name:, config:, plugin: }` form. The current macro is **positional**:
   `declare_plugin!($plugin, $config, $type_id, { … })`. Every copy-pasted example fails to compile. The
   `required_sudo_commands` prose (grep `required_sudo_commands`; ~L864) is likewise stale — it describes a `&self` method returning
   struct literals; the real form is a free `fn(&serde_json::Value) -> Vec<SudoCommandEntry>` wired via
   the macro's `sudo:` field.

3. **HTTP client guidance wrong (MEDIUM, L949 / doc L771).** The "HTTP Client Requirements" section teaches
   plugins to obtain a `reqwest::Client` by downcasting the controller runtime and calling
   `controller.http_client().clone()`. That method **does exist**
   (`ControllerRuntime::http_client() -> Option<&reqwest::Client>`, `descriptor.rs:832`), so the audit's
   "method doesn't exist" phrasing is imprecise — the real defect is that **no first-party plugin uses the
   downcast-for-HTTP pattern anymore** (grep for `.http_client()` callers across `crates/**` returns
   nothing; GitHub and peers build clients via `build_plugin_http_client`). The stale idiom
   (`controller.http_client().clone()`) recurs at **four doc sites** — guide lines **480, 611, 713, 783** —
   plus the `http_client: reqwest::Client` struct fields at 600/702. The implementer must correct every
   occurrence, not only the first. The sanctioned pattern is
   `build_plugin_http_client(PluginHttpClientConfig { … })`, which `crates/plugins/AGENTS.md` already
   documents as the rule — the long-form guide contradicts the subfolder guide.

4. **Hand-maintained inventories drifted (MEDIUM, ~L22 / doc L22).** Three prose lists that enumerate
   registered plugins have fallen behind `all_descriptors()`: the `PluginTypeId` string list (doc L22–23),
   the "Plugin crates" table (doc L904–920), and the "current plugins with this capability" list for
   `ControllerSideFetchReleases` (doc L215). Each names a subset and omits crates added since. These are
   exactly the "counts / inventory tables that mirror code" the repo's own `AGENTS.md` § _Maintaining this
   file_ bans, and the same class of drift the common-mistakes ledger records (row 5: "Hardcoded counts in
   agent docs rot silently").

All four are the same underlying problem — **the guide duplicates code facts and drifts from them** — so a
single coherent editing pass is more maintainable than four piecemeal ones.

### Verified current reality

Confirmed against the tree (2026-07-12):

- **Registry surface** (`crates/plugins/infrastructure/registry/src/registry.rs`,
  `.../registry/src/lib.rs`):
  - `pub fn all_descriptors() -> Vec<&'static PluginDescriptor>` (registry.rs:27) — the single authoritative list.
  - `pub fn get_descriptor(type_id: &str) -> Option<&'static PluginDescriptor>` (registry.rs:84).
  - `pub fn all_required_sudo_commands() -> Vec<(PluginTypeId, Vec<SudoCommandEntry>)>` (registry.rs:144).
  - `pub fn build_catalog(config, instance_states) -> Result<PluginCatalog>` (lib.rs:108).
  - `PluginOps` and sibling ops traits (`PluginConfigOps`, `PluginMetadataOps`, `NotificationOps`,
    `PluginSurfaceOps`, `PluginSurfaceActionOps`, `SoftwareItemLifecycleOps`, `ControllerUpdateHookOps`,
    `ControllerUpdateProtectionOps`) re-exported from the registry crate (lib.rs:17–40).
  - No `register_plugins!` macro, no `PluginRegistry` struct anywhere in code (only in CHANGELOG history +
    the two stale doc references below).

- **`declare_plugin!` grammar** (`crates/plugins/infrastructure/core/src/macros.rs:30–75`): positional
  `$plugin:ty, $config:ty, $type_id:expr` followed by a `{ … }` block. The block's fields (all optional
  except `display_name`, `family`, `config_model`, `roles`): `host_requirements`, `config_test`,
  `type_settings`, `scope`, `instance_config`, `roles`, `extra_capabilities`, `notification_transport`,
  `software_item_lifecycle`, `controller_update_protection`, `controller_update_hook`, `infra`,
  `release_fetcher_create`, `installed_version_enricher_create`, `owned_surface_ids`, `raw_settings_keys`,
  `global_provider_consumers`, `sudo`, `surface_actions`, `surfaces`, `migrations`, `reset_tenant_data`,
  `db_migrate_tables`. The macro emits **compile-time trait assertions** (`__assert_role_impl!` per role,
  macros.rs:84–94) — capabilities are _asserted from_ the declared roles, not "auto-derived": the plugin
  struct must `impl` each role trait or the crate fails to compile.

- **Canonical example** (`crates/plugins/package-managers/apt/src/plugin.rs:143–158`):

  ```rust
  declare_plugin!(AptPlugin, AptConfig, "package_manager_apt", {
      display_name: "APT Package Manager",
      family: PluginFamily::Software,
      config_model: ConfigModel::PluginConfig,
      host_requirements: HostRequirements::POSIX,
      config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
      type_settings: true,
      roles: [
          Discoverer,
          VersionDetector,
          ReleaseFetcher,
          PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },
          UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED },
      ],
      sudo: AptPlugin::required_sudo_commands,
  });
  ```

- **Sudo commands** (`crates/plugins/package-managers/apt/src/plugin.rs:113–138`): a free
  `pub fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry>` returning
  `SudoCommandEntry::new(program, description).with_args_suffix(…).with_setenv()` builder chains, wired
  via `sudo: AptPlugin::required_sudo_commands`.

- **HTTP client** (`crates/plugins/releases/github/src/plugin.rs:129–159`): GitHub builds its client via
  `build_plugin_http_client(PluginHttpClientConfig { user_agent, default_headers: Some(headers), ..Default::default() })`.
  The `Authorization: Bearer …` token is placed in the client's **default headers** — safe _only because_
  `PluginHttpClientConfig` defaults to `redirect::Policy::none()`
  (`crates/plugins/infrastructure/core/src/http_client.rs:65`), so the auth header can never be replayed to
  a redirect target. The doc omits this rationale today.

## Goal

Make `docs/development/plugin-guidelines.md` describe the code that exists, and stop it re-drifting by
removing the hand-mirrored inventories in favour of a pointer to the single source of truth
(`all_descriptors()`). Fold in the two same-class stale references discovered in sibling docs.

Non-goals: restructuring the guide, changing any code, adding CI machinery, or touching the out-of-scope
docs-drift findings in other files.

## Approach (primary recommendation)

Pure documentation correction + de-duplication, one editing pass, four correction areas + two sibling
fold-ins. This mirrors the repo's own anti-drift philosophy (`AGENTS.md` § _Maintaining this file_: "no
hardcoded counts", "no inventory tables that mirror code", "one canonical home; link, don't copy").

**Locator convention (important).** `plugin-guidelines.md` is ~1836 lines and every edit shifts the lines
below it, so the line numbers in this spec are **informational hints, not addresses** — some are already
off by 20–30 lines against the live file. Each correction below names a **stable anchor**: a section
heading and/or a grep-defined string set (e.g. all `register_plugins!` hits). The implementer locates work
by heading + grep, and edits **bottom-up** (highest line number first) so earlier edits do not invalidate
later anchors.

### Correction 1 — registration / `PluginRegistry` references (grep-defined; doc ~L864–L1068)

**Locator:** every occurrence of `register_plugins!` and `PluginRegistry` in the guide (grep-defined — not
one section; currently ~18 hits). They cluster in **three distinct sub-areas**, all of which must be
corrected — an implementer who fixes only the macro section will leave two behind:

1. the **sudo-aggregator prose** (~L864) that references the deleted registry;
2. the **"Register in `PluginRegistry`" step + the `register_plugins!` macro section** (~L962–L1000), incl.
   the "Add your plugin to the `register_plugins!` macro" registration step;
3. the **package-identifier-validation subsection** (~L1013–L1068), which calls
   `PluginRegistry::validate_package_identifier` in its own code fence — a separate ~55-line block far below
   the macro section, easy to miss.

Replace every `register_plugins!` / `PluginRegistry` reference with the descriptor/catalog model:

- Registration step becomes: "add `&my_plugin::DESCRIPTOR` to the `all_descriptors()` list in
  `crates/plugins/infrastructure/registry/src/registry.rs`." (`declare_plugin!` emits the `DESCRIPTOR`
  static; the registry aggregates them.)
- Note that `build_catalog()` consumes `all_descriptors()` to produce the `PluginCatalog`, and that
  runtime dispatch goes through the `PluginOps` trait objects (the doc already half-uses `plugin_ops` at
  ~L1000/1022 — make the whole section consistent with that).
- **Delete** the "the macro generates these methods" list (it enumerated methods of the deleted
  `PluginRegistry`).
- Per ledger row 7 (merge-and-delete risks silent invariant loss): before deleting the surrounding
  "package identifier validation" prose, preserve any still-true guidance it carries about _how_ package
  identifiers are validated (the validation itself derives from `PluginCatalog` and still exists) — correct
  the mechanism reference, do not drop the concept.

### Correction 2 — `declare_plugin!` macro + five examples (`## The declare_plugin! Macro` / `## declare_plugin! Examples` headings)

- Rewrite the "Syntax reference" to the positional form and enumerate the block fields by pointing at
  `macros.rs` as the authoritative grammar (list the common fields inline, but name the macro source as
  the exhaustive reference rather than reproducing all 24 optional arms — a reproduced grammar is another
  drift surface).
- Replace all five worked examples with the **actual current invocations**, copied verbatim (ledger row 44)
  from these exact source files — do not hand-transcribe from memory or paraphrase:
  - APT — `crates/plugins/package-managers/apt/src/plugin.rs` (quoted in _Verified current reality_ above)
  - GitHub — `crates/plugins/releases/github/src/plugin.rs`
  - Shell Hook — `crates/plugins/hooks/shell/src/plugin.rs`
  - Webhook — `crates/plugins/notifications/webhook/src/plugin.rs`
  - Dashboard Icons — `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`

  Locate each with `grep -n "declare_plugin!" <file>` and copy the whole invocation through its closing
  `);` so the reader copies a form that compiles.
- Correct the "capabilities auto-derived from roles" claim: the macro emits compile-time `impl` assertions
  per declared role; the plugin must implement each role trait.
- Rewrite the `required_sudo_commands` prose to the free-`fn(&serde_json::Value) -> Vec<SudoCommandEntry>`
  form wired via `sudo:`, using the APT `SudoCommandEntry::new(…).with_args_suffix(…).with_setenv()`
  builder chain as the example.

### Correction 3 — HTTP client requirements (`## HTTP Client Requirements` heading; doc ~L771)

**Locator:** the `## HTTP Client Requirements` section heading, plus every `http_client().clone()` occurrence
in the guide (grep-defined; currently four — L480, L611, L713, L783 — but grep, do not trust the numbers).
Rewrite the whole downcast-to-`ControllerRuntime`-then-`.http_client().clone()` idiom, not just prose around
it, around `build_plugin_http_client(PluginHttpClientConfig { … })`:

- State that plugins obtain a client via `build_plugin_http_client` (re-exported from the registry crate),
  which enforces the SSRF-safe resolver, connect/read timeouts, and `redirect::Policy::none()` by default.
- Keep the "never call `reqwest::Client::builder()` directly" rule but point it at the helper (aligning the
  guide with `crates/plugins/AGENTS.md` § _HTTP clients_).
- Document `default_headers` semantics: auth may live in default headers **because** redirects are disabled
  by default — name this as the reason, so nobody re-enables redirects while keeping auth in default
  headers. Use the GitHub `build_client` as the worked example.

### Correction 4 — drifted inventories (doc L22, L215, L904)

Delete the three hand-mirrored lists; replace with a pointer to the source, keeping 2–3 illustrative rows
only:

- **`PluginTypeId` list (L22):** replace the enumeration with prose naming a few representative IDs
  (e.g. `package_manager_apt`, `releases_github`, `hook_shell`) and pointing at `all_descriptors()` in
  `registry.rs` as the complete, authoritative list. No count.
- **"Plugin crates" table (L904):** this table carries two _different_ kinds of content, and only one may
  be deleted. The bare **name → crate-path** mapping is the drifted inventory (root `AGENTS.md` already
  carries the crate-layout tree) — replace it with a pointer to `crates/plugins/**` and `all_descriptors()`.
  But several rows also carry **per-row conceptual guidance** that no other doc restates and that
  `all_descriptors()` cannot reproduce as human prose — e.g. which auth model a release plugin uses
  ("PRIVATE-TOKEN auth", "supports nested namespaces"), which trait a package-manager plugin implements
  ("Implements `HostCompatibilityPlugin`"), and each plugin's execution site. Per ledger row 7
  (merge-and-delete risks silent invariant loss), that conceptual content must be **preserved** — relocated
  into per-plugin prose, not trimmed away with the inventory. Do not reduce this to "a couple of example
  rows": the load-bearing per-plugin notes survive; only the name→path enumeration is removed. Root
  `AGENTS.md` line 91 is a bare pointer and does not carry this prose, so there is no other home for it.
- **`ControllerSideFetchReleases` "current plugins" list (L215):** delete the drifted enumeration; keep the
  prose explaining what the capability _means_ and point readers to grep
  `extra_capabilities: [… ControllerSideFetchReleases …]` across `crates/plugins/**` (or read
  `all_descriptors()`). Per ledger row 7, preserve the capability's conceptual explanation — only the
  "current plugins: X, Y, Z" enumeration is removed.

### Sibling fold-ins (same drift class, discovered during verification)

- **`crates/plugins/CODEREVIEW.md:168`** — "Extension handler registration is compile-time via the
  `register_plugins!` macro" is stale. Correct to `declare_plugin!` / the descriptor model. (Same deleted
  system; leaving it re-seeds the drift.)
- **`crates/plugins/AGENTS.md:135`** — the full sentence is _"When a convention here becomes outdated,
  verify against the actual code in `crates/plugins/infrastructure/core/src/` before editing, since
  plugin-guidelines.md itself lags behind … `register_plugins!`/`PluginRegistry`."_ The
  **"verify against actual code before editing" lead-in is durable and must be preserved** — only the
  dependent _"since … lags behind … register_plugins!/PluginRegistry"_ tail is stale (the guide will be
  current once this spec lands). Trim the stale tail, keep the lead-in; do not delete the whole sentence and
  do not restructure the file (respect the AGENTS.md size budget).

### Rejected alternative — doc-drift guard test (YAGNI)

The audit suggests adding a **guard test** that renders the `PluginTypeId` list and asserts the doc matches,
"to keep them honest." Rejected as over-engineering: it adds CI machinery to keep a hand-mirrored prose list
synced with code, when the maintainable fix is to **stop mirroring code in prose** — exactly what the repo
already did when it slimmed `AGENTS.md` by deleting inventory tables (that work needed no guard test; it
needed deletion). A pointer to `all_descriptors()` cannot drift because it names the source instead of
copying it. No new test, no new CI script.

## Testing / verification

No code, so no unit tests. Verification is mechanical and belongs in the implementation plan:

- `markdownlint --config .markdownlint.json docs/development/plugin-guidelines.md crates/plugins/AGENTS.md`
  — the doc gate for these files. Do **not** run `npx prettier`: this repo scopes Prettier to `frontend/`
  only (no root Prettier config; the pre-commit path check never touches `docs/`), so prettier-formatting
  the guide would be a non-repo-conformant reflow.
- `crates/plugins/CODEREVIEW.md` is **globally excluded** from markdownlint via `.markdownlintignore` (by
  design, same class as `CHANGELOG.md`). Running markdownlint against it validates nothing (exit 0, no
  output). Verify that file's edit by grep + visual read, not by the linter.
- Grep the **guide** (`docs/development/plugin-guidelines.md`) after editing: `register_plugins!` and
  `PluginRegistry` must return 0 hits. **Do not** grep the whole tree for `ControllerRuntime::http_client` —
  that symbol legitimately still exists in code (`descriptor.rs:832`); a tree-wide "must not appear" check
  is unsatisfiable. Instead grep the **guide** for `http_client().clone()` → must return 0 hits (the stale
  downcast-for-HTTP idiom is gone).
- `bash ci/verify_agents_md_budget.sh` — the `crates/plugins/AGENTS.md` edit must stay within the scoped
  ≤250-line budget (it shrinks, so this is a safety check).
- **Verify each rewritten `declare_plugin!` example by diff, not eyeball** (ledger row 44): for each of the
  five, run `grep -n "declare_plugin!" <source-file>` (paths listed in Correction 2), read the invocation
  through its closing `);`, and confirm the doc fence is a byte-for-byte copy. "Spot-check" is not
  repeatable; a diff against the named source is.

## Documentation deliverables

This spec **is** a documentation change; the deliverables are the edited docs themselves:

- `docs/development/plugin-guidelines.md` — corrections 1–4 (primary).
- `crates/plugins/CODEREVIEW.md` — stale `register_plugins!` reference (L168).
- `crates/plugins/AGENTS.md` — stale "lags behind" note (L135).

No ADR (documentation accuracy is not an architectural decision). No README/CONTEXT change (the drift is
localised to the plugin guide and its two sibling references). No wire/OpenAPI/frontend/API-doc impact.

## Out of scope / deferred

- Other docs-drift findings in **different** files — `quality-gates.md` drift (×2),
  `scheduler-engine.md` documenting a deleted crate, `error-handling.md` dead example paths,
  `coding-standards.md` `non_exhaustive` enum inventory. Candidates for a separate "developer-docs drift
  sweep" spec; explicitly not touched here.
- Any new CI/guard machinery to enforce doc↔code correspondence (rejected above).
- Restructuring `plugin-guidelines.md` beyond the four correction areas.
- Any code change to the plugin macro, registry, or HTTP-client helper.
