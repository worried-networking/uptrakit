# Controller Crate Merge Design

**Date:** 2026-08-20

**Origin:** promoted from task bead `uptrakit-vp793` (changelog leak investigation, Option F).

## Problem

The `uptrakit-controller` / `uptrakit-controller-standalone` split (2026-04-28,
`0f664116b` / `eeffd06b3`) exists solely so `cargo binstall` can address two
feature flavors as distinct Cargo packages. The cost is structural:

1. **Changelog leak.** release-plz attributes commits at package granularity via
   `changelog_include`; `uptrakit-controller-runtime` (and `controller-core`)
   appear in both bin packages' include arrays, so every runtime commit is
   duplicated across two changelogs and two GitHub releases. There is no
   path- or feature-level filtering to scope them correctly.
2. **Two release lineages** (`uptrakit-controller-v*`,
   `uptrakit-controller-standalone-v*`) for what is one binary built with two
   feature sets — double tags, double release entries, doubled CI selectors,
   and a changelog-scoping apparatus that keeps drifting.

## History (verified 2026-08-20)

- **Shape A** (one package, two feature-flavored CI builds, both archives on one
  release — essentially this spec's target) was introduced with the release-plz
  migration (`6b4c54969`, 2026-04-20/21) and replaced one week later. **Zero
  releases ever shipped under Shape A**; the two-binary upload path never ran to
  completion.
- The split rationale, documented in
  [2026-04-28-controller-crate-split-design.md](2026-04-28-controller-crate-split-design.md)
  (Goals 1–2), was exclusively **binstall addressability** — a proactive tooling
  requirement, not operational pain.
- All real release pain occurred **after** the split and was unrelated to binary
  topology: HTTP 422 release-body cap (~160 KB body, 2026-06-04, run
  26969114640), unbounded commit-walk changelog re-dumps (release-plz bot PRs
  #131/#132), and the `git_only`+`publish` contradiction wedge (#136). All are
  fixed by the two-anchor model and the 100 KB body truncation, both of which
  remain in place.

Returning to Shape A therefore re-imports exactly one known cost — binstall can
address only one flavor — which this design resolves by **dropping the binstall
goal outright** (owner decision, review round 2026-08-20): the workspace
publishes only to the fake registry `uptrakit-private` (root `Cargo.toml`
`publish = [...]`), so `cargo binstall uptrakit-controller` /
`cargo install uptrakit-controller` **by name cannot resolve today and never
could** — the split's sole rationale defended a channel that does not exist.
See Install flavors for the recorded drop.

## Goals

1. One bin crate `uptrakit-controller`; delete `uptrakit-controller-standalone`.
2. Keep `uptrakit-controller-runtime` a separate lib crate — the dispatch seam
   the future unified god-binary spec
   (`uptrakit-spec-2026-04-18-unified-god-binary`) depends on.
3. One release lineage `uptrakit-controller-v*` carrying **both** archives.
4. **One flavor model, per-channel feature deltas**: the full (standalone)
   flavor is the default install everywhere — default cargo features and
   `cargo install --path`/`--git` yield it; lean is opt-in. Exact feature
   sets still differ per channel (released artifacts add `db-all,nats`;
   Docker always passes explicit `--no-default-features` lists; plain
   `cargo install` is SQLite-only full) — "default" is a flavor guarantee,
   not a feature-set guarantee. Binstall is explicitly **not** a channel
   (goal dropped — see Install flavors).
5. Changelog covers the full flavor's surface (embedded service runtimes
   included), eliminating the controller-vs-standalone double-posting.
   Precisely that and no more: shared libs like `uptrakit-agent-core` remain
   in multiple lineages' `changelog_include` arrays
   (`release-plz.toml:554,606,619`) — cross-lineage overlap with
   agent/mqtt/scheduler releases is a pre-existing release-plz limitation
   (no path/feature scoping) that this merge does not and cannot fix, and
   the union actually enlarges it. Do not reopen this as "the leak" later.

## Non-goals

- Merging `controller-runtime` into the bin crate (god-binary seam).
- Deduplicating historical changelog entries (deferred, see below).
- Tombstone releases or tag deletion on the standalone lineage — historical
  tags and releases stay as-is.
- Any change to agent / mqtt / scheduler service binaries or their releases.

## Design

### Crate and feature layout

- Delete `crates/core/controller-standalone/` entirely. Workspace `members`
  glob auto-drops it; nothing in `[workspace.dependencies]` references either
  bin crate (verified).
- `crates/core/controller/` restructures its features around two additive
  meta-features (**standalone/full is the default flavor everywhere** — owner
  decision, review round 2026-08-20):
  - **`lean`** meta-feature = the ten features the crate currently forces
    (embedded-frontend, db-sqlite, oidc, zeroconf, interactive,
    notifications-all, reset-data, dashboard-icons, mcp, plugin-ops).
    **Wiring reality check:** these are NOT a cargo `default` feature today —
    the controller crate has no `[features] default` at all; they are
    unconditional entries in the `uptrakit-controller-runtime` dependency's
    `features = [...]` list, which is **immune to `--no-default-features`**.
    Making them a real optional feature is therefore a behavioral change to
    the whole workspace's minimal-feature gates, not a rename (see the gate
    consequences bullet below). Deliberate naming deviation from the house
    `<category>-all` rollup convention: `lean` is a flavor selector, not a
    category rollup (no precedent exists for flavor features), and a single
    named feature keeps the CI lean invocation
    (`--no-default-features --features lean`) drift-free instead of
    re-listing ten features in the workflow.
  - **`embedded-all`** meta-feature = `["embedded-scheduler", "embedded-mqtt",
"embedded-agent", "embedded-ssh-agent"]` (purely additive —
    ADR-0032-compliant; exact forwarded composition verified during planning
    against the standalone crate's current dependency spec).
  - **`default = ["lean", "embedded-all"]`** — plain `cargo build` and
    `cargo install --path`/`--git` yield the full flavor. The lean flavor is
    produced with `--no-default-features --features lean`.
  - **`test-utils`** forwarding feature (currently only on the standalone
    crate; required by `docker/Dockerfile.test`).
  - **Minimal-gate consequences (day-one failure risk):** because the ten
    lean features are forced today, the workspace gates
    `cargo check/clippy --no-default-features --features db-sqlite`
    currently still compile oidc/mcp/plugin-ops/notifications/etc. through
    the controller's dep spec. Moving them under an optional `lean` feature
    makes the minimal gate genuinely minimal **for the first time** — a
    never-before-compiled configuration under `deny(warnings)`, where
    neither `#[allow]` nor `#[cfg(not(feature))]` is an available fix.
    Mitigations, all required: (a) the feature restructure lands as its
    **own first PR** with zero release/workflow changes, running the full
    gate matrix in isolation; (b) a new explicit gate
    `cargo clippy --all-targets -p uptrakit-controller --no-default-features --features lean`
    preserves the coverage the workspace minimal gate silently loses;
    (c) `docs/development/quality-gates.md:110-111` (which already
    mischaracterizes the controller's "default features") is corrected in
    the same commit.
- Exactly **one `[[bin]]` target** named `uptrakit-controller`. Never two bin
  targets with `required-features`: any workspace build enabling embedded
  features would unify them into the lean bin too.

### CI build matrix (release-plz.yml)

Two invocations of the same package, per target:

| Flavor         | Invocation                                                                                       | Archive prefix                   | Inner binary name                |
| -------------- | ------------------------------------------------------------------------------------------------ | -------------------------------- | -------------------------------- |
| Lean           | `cargo build -p uptrakit-controller --release --no-default-features --features lean,db-all,nats` | `uptrakit-controller`            | `uptrakit-controller`            |
| Full (default) | `cargo build -p uptrakit-controller --release --features db-all,nats`                            | `uptrakit-controller-standalone` | `uptrakit-controller-standalone` |

Both release builds keep today's explicit `db-all,nats` addition
(`release-plz.yml:410-413`, `:421-424`) — dropping it would remove
Postgres/NATS support from shipped artifacts. (Source installs with plain
default features get SQLite-only full flavor; released artifacts are
defaults + `db-all,nats`.)

**Output collision:** both invocations write
`target/{target}/release/uptrakit-controller`. This is a **new hazard with no
existing precedent** — no current workflow step builds the same package twice
(every binary today has a unique name). The workflow must copy/rename the
first flavor's binary to a distinct path before running the second build,
then hand each captured path to `package_and_upload`.

Both archives attach to the same `uptrakit-controller-v{version}` release.
`package_and_upload(pkg src arc_prefix inner_name)` already supports
arc_prefix/inner_name divergence (precedent: `uptrakit-cli` packages inner
binary `uptrakit`); it takes an arbitrary `src` path, so calling it twice for
the same package works. The full flavor's build output (`uptrakit-controller`)
is renamed to `uptrakit-controller-standalone` at packaging time — systemd
units, PVEHS install scripts, and existing deployments see unchanged file
names inside the archive.

Workflow package selectors — four `IN("uptrakit-controller","uptrakit-controller-standalone")`
call sites (`release-plz.yml:74-76`, `:78-80`, `:96-100`, `:264-266`) — drop
the standalone entry; the backfill job and
`ci/release-plz/parse-backfill-tags.sh` must **tolerate historical
`uptrakit-controller-standalone-v*` tags without generating new ones** (tags
remain on the remote permanently).

**Backfill job (workflow_dispatch matrix, `release-plz.yml:~700-919`):** it
has its own hardcoded `Build controller-standalone` step
(`cargo build -p uptrakit-controller-standalone`, `:762-769`) and a Package
archives step that assumes `archive-prefix == package_name` (sole exception:
cli). Post-merge, a backfill run against a historical standalone tag would
hard-fail at cargo package resolution. Decision: the backfill path **skips**
historical `uptrakit-controller-standalone-v*` tags with an explicit log line
(backfilling the retired lineage is unsupported — those releases already
exist); the hardcoded standalone build step is deleted. The skip lives in
`ci/release-plz/parse-backfill-tags.sh` itself — it **excludes**
`uptrakit-controller-standalone` entries from the emitted `PLAN` JSON (the
script is the sole plan builder, invoked once at `release-plz.yml:639` with
no downstream filter; leaving the entry in while deleting the build step
would instead hard-fail the Package archives step at `:806-841` with
`missing source binary`). Its test case 3
(`test_parse-backfill-tags.sh:128-133`) flips from an inclusion assertion to
a skip assertion.

The backfill pipeline is 1-package→1-archive **end to end** — the `PLAN`
JSON's unit is `(package_name, version)`, and the build step (`:751-759`),
the Package-archives loop (`:806-841`, `archive="${pkg}-${ver}-${TARGET}"`
with `inner_name_for()`), and the digest pre-check all key on it. Left
unmodified post-merge, the surviving `Build controller` step
(`--features db-all,nats`, defaults on) would build the **full** flavor and
upload it under the **lean** archive prefix — a mislabeled binary — and
the standalone archive of a post-merge release could not be backfilled at
all. Rather than special-casing the controller in three places, the
`PLAN`'s unit changes from _package_ to _artifact_
(`{package, flavor/features, archive_prefix, inner_name}`): this cleanly
absorbs the existing `uptrakit-cli` inner-name exception and pre-aligns
with `uptrakit-spec-2026-08-06-release-build-dedup`'s `binaries.json`
manifest (planning must check whether that spec should simply land first —
see Dependencies).

### Install flavors

One model: **the default is full, everywhere**.

- **binstall goal DROPPED (owner decision, review round 2026-08-20 —
  recorded here explicitly so it is never silently resurrected):** the
  workspace publishes only to the fake registry `uptrakit-private` (root
  `Cargo.toml` `publish = ["uptrakit-private"]`), so name-based
  `cargo binstall uptrakit-controller` / `cargo install uptrakit-controller`
  **cannot resolve — the channel does not exist** and no success criterion
  can be verified against it. The `uptrakit-cli` binstall overrides block
  (`[[package.metadata.binstall.overrides]]` in `crates/ui/cli/Cargo.toml`)
  does not match binstall's documented schema and is inert — there is no
  working precedent to preserve either. Consequences: the merged
  `uptrakit-controller` crate carries **no** `[package.metadata.binstall]`
  block (the controller crate's existing block, which points at the lean
  archive, is deleted with the restructure); no binstall success criterion;
  supported install paths are release-asset download, PVEHS scripts, Docker,
  and source (`--path`/`--git`). Revisit only if crates.io publication ever
  becomes a real goal — that is its own spec cycle.
- **`cargo install --path crates/core/controller`** (or `--git`, default
  features) produces the **full** flavor — source installs match the
  release default.
- **Lean flavor**: opt-in only — direct release-asset download
  (`uptrakit-controller-*` archive) or source build with
  `--no-default-features --features lean`. Documented in
  `docs/development/releases.md`.

### release-plz configuration

- Delete the `uptrakit-controller-standalone` package entry
  (`release-plz.toml`).
- `uptrakit-controller` stays `git_only = true` (two-anchor model unchanged).
- Its `changelog_include` becomes the **union** of both current arrays — the
  standalone extras (`uptrakit-controller-runtime` already present, plus
  `uptrakit-agent-runtime`, `uptrakit-agent-ssh-runtime`,
  `uptrakit-agent-core`, `uptrakit-mqtt-runtime`,
  `uptrakit-scheduler-runtime`) join the controller array. The changelog
  covers embedded services — they are compiled into the default (full) flavor,
  and the one changelog describes the whole release, which ships both flavors.
- The former "residual leak" of runtime commits (boot / service_host / reload)
  into two changelogs dissolves by design: one changelog is now the intended
  destination.
- **Body-size interaction:** the union grows release bodies; the existing
  100 KB truncation guard — the workspace-level `git_release_body` Jinja
  template at `release-plz.toml:39-47` — is the mitigation and must be
  confirmed still active during planning.

### Changelog backfill

Replace `crates/core/controller/CHANGELOG.md` wholesale with the standalone
crate's changelog (the fuller lineage), then delete the standalone crate's
file with the crate. Known duplicate entries are copied **as-is**; dedupe is
deferred (bead below). Version lineages are in lockstep (both crates and both
tag series at 0.0.7), so the merged crate continues from 0.0.7 with no
downgrade or renumbering.

### Update-path migration (plugins + scripts)

The standalone flavor's _release tag prefix_ changes
(`uptrakit-controller-standalone-v*` → `uptrakit-controller-v*`); asset names
and inner binary names do **not** change. Consumers to update:

1. **Self-update discovery plugin**
   (`crates/plugins/discovery/uptrakit-self-update/`): release lookup is
   `build_fetch_releases_target` (`src/discovery.rs:135-150`) — a
   `releases.github` target with `tag_strip_prefix: "v"` and
   `asset_filter: <service_name>`; it has **no tag-prefix scoping**, so the
   likely outcome is **no code change**: the merged release still carries the
   `uptrakit-controller-standalone-*` asset the filter matches. Planning
   verifies the github release plugin's version inference against
   `uptrakit-controller-v*` tag names. **Asset-filter ambiguity (new hazard,
   required deliverable — not "verify during planning"):** `asset_filter` is
   a **regex** matched unanchored (github release plugin,
   `crates/plugins/releases/github/src/plugin.rs:100,221,599`), and
   `execute_update` **hard-bails when more than one asset matches**
   (`:611-619`). Today the two flavors live on separate releases so
   `uptrakit-controller` is unambiguous; post-merge both archives (plus
   their `.sha256` files) sit on one release, and `uptrakit-controller` is a
   strict prefix of `uptrakit-controller-standalone` — a lean deployment's
   filter matches everything and the update path bails. Required: anchor the
   filter (`^uptrakit-controller(-standalone)?-\d...` style, flavor- and
   target-scoped) **and** add a test asserting exactly-one-match against a
   fixture release carrying both flavors × targets × `.sha256` assets.
   **Persisted-config migration:** the `fetch_releases` target is a
   DB-persisted row from a past discovery run, and re-discovery never
   rewrites existing targets (autodiscovery invariant) — a corrected plugin
   alone leaves the live deployment on the stale filter. The spec requires
   an explicit path for the live row: data migration, admin re-target, or a
   documented manual step (choice is a planning detail; silence is not).
   The `src/plugin.rs:357,395` fixtures
   encode the **service/binary name** (`uptrakit-controller-standalone`,
   unchanged by this spec), not tag names — expected no fixture change;
   confirm during planning.
   Observation (out of scope, verify during planning): the target hardcodes
   `owner: "uptrakit", repo: "uptrakit"` — if that repo slug is wrong,
   self-update is broken today independently of this spec; file a follow-up
   bead if confirmed.
2. **PVEHS discovery plugin**
   (`crates/plugins/discovery/proxmox-helper-scripts/`): likely **no code
   change** — the script parser (`src/discovery.rs:693`) only recognizes
   `check_for_gh_release` / `fetch_and_deploy_gh_release`, while the uptrakit
   CT script uses `check_for_gh_tag` / `get_latest_gh_tag`, so the plugin
   never captures this tag prefix; the prefix lives in the scripts (item 3).
   Planning confirms no plugin-side inference depends on the tag scheme and
   updates any test fixtures encoding the old prefix.
3. **PVEHS install scripts** (`scripts/pvehs/install/uptrakit-install.sh:21`
   tag prefix + download paths; `scripts/pvehs/ct/uptrakit.sh:50-51,66-67` —
   `check_for_gh_tag` / `get_latest_gh_tag` prefix args
   `uptrakit-controller-standalone-v` → `uptrakit-controller-v`): fetch latest
   `uptrakit-controller-v*` release, download the `-standalone` asset. systemd
   `ExecStart` unchanged.
4. **Existing deployment** (single live instance, owner-managed): manual
   migration — owner redeploys/upgrades once against the new tag scheme. No
   tombstone release on the old lineage.

Old versions of the plugins/scripts pointed at the retired
`uptrakit-controller-standalone-v*` prefix will see no new releases (silent
staleness, not breakage); the manual migration covers the one live deployment.

**Landing order (binding for the plan — each step is a clean revert
point):** (1) **feature restructure alone** (`lean`/`embedded-all`/`default`
on the controller crate, zero release/workflow changes) — full gate matrix
green, minimal-gate fallout fixed in isolation; (2) **self-update filter
anchoring + live-config migration**; (3) **topology**: crate deletion,
release-plz config, workflows, docs; (4) verify the first post-merge
release-plz PR and the first `uptrakit-controller-v*` release carrying both
assets; (5) only then flip the script/plugin tag prefixes in a follow-up
commit. Flipping consumers before a release exists on the new lineage would
point them at a lineage with zero releases while the old one has gone
quiet; and once the first merged release ships, reverting the topology is
expensive — hence the ordering.

### Docker / test infrastructure

**Feature mechanism caveat:** `docker/Dockerfile` (`:63`, `:75`) builds with
`--no-default-features --features "${FEATURES}"` whenever `FEATURES` is
non-empty — cargo defaults **never** apply to Docker builds. Every docker-side
FEATURES list must therefore name its flavor tokens explicitly; "defaults
yield full" claims do not hold here.

- `docker/Dockerfile.test` (`:58`, `:69`, `:95`): build
  `-p uptrakit-controller` with an explicit feature list including
  `nats,test-utils` plus whatever flavor tokens the current standalone test
  build implies (exact list is a planning detail — same explicit-features
  caveat applies if this file uses the `--no-default-features` pattern); COPY
  path updated to the `uptrakit-controller` binary (or rename at COPY if the
  test image's entrypoint expects the standalone name — pick whichever keeps
  `crates/core/integration-tests/tests/helpers/containers.rs` minimal).
- `.github/workflows/docker.yml`: remove the dead
  `uptrakit-controller-standalone-v*` tag trigger (`:7-8`). **Docker goes
  standalone-only (owner decision, review round 2026-08-20):** of the two
  controller matrix rows (`:59-84`) the **lean-image row is deleted** — its
  image is retired (no new tags pushed; historical registry tags remain).
  The surviving standalone-image row keeps its image name, builds
  `-p uptrakit-controller` with `features: lean,embedded-all,db-all,nats`
  (explicit tokens — cargo defaults never apply, see caveat above), and sets
  `binary: uptrakit-controller` (cargo only ever emits that name; consumed
  as `BINARY=${{ matrix.binary }}` at `:166`). Its Docker tag-matching logic
  (`:279-282`, `type=match,pattern=uptrakit-${{ matrix.name }}-v(.+)` + the
  `stable` enable) currently keys on the retired
  `uptrakit-controller-standalone-v(.+)` lineage and would never match again
  — it must key on the single `uptrakit-controller-v*` lineage. The
  **`build-swagger` job** is a separate breakage point: its tag gate
  (`:192`, `:300`, `:323-326`) already keys on `uptrakit-controller-v` and
  is fine, but its build-args (`:227-229`) hardcode
  `PACKAGE=uptrakit-controller-standalone` / `BINARY=uptrakit-controller-standalone`
  — post-merge these must become `PACKAGE=uptrakit-controller`,
  `BINARY=uptrakit-controller`,
  `FEATURES=lean,embedded-all,db-all,nats,swagger-ui`, or the swagger image
  build fails on the next release.
- `docker-compose.yml` (`:30-31`): the controller `FEATURES` arg (currently
  `embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all`)
  must be rewritten with the new flavor tokens — e.g.
  `lean,embedded-all,db-all,nats` (full flavor; superset of the previous
  lean+embedded-scheduler build) — functionally compatible; accepted build
  weight.
- `crates/core/functional-tests/tests/release_config_invariants.rs`: the
  existing `BINARY_TARGETS` list (`:22-30`) drops the standalone name; add a
  **new** test asserting the merged entry's release-plz shape (git_only +
  union `changelog_include`) — no such assertions exist today.

## Deliverables

### Code

- Delete `crates/core/controller-standalone/`.
- `crates/core/controller/Cargo.toml`: `lean` + `embedded-all` meta-features,
  `default = ["lean", "embedded-all"]`, `test-utils` forwarding; the existing
  `[package.metadata.binstall]` block is **deleted** (binstall goal dropped —
  see Install flavors).
- `release-plz.toml`: standalone entry removed; controller
  `changelog_include` union.
- `.github/workflows/release-plz.yml`: build matrix (two invocations, one
  upload target), selectors, backfill job (delete the hardcoded
  standalone build step `:762-769`; skip historical standalone tags with a
  log line).
- `ci/release-plz/parse-backfill-tags.sh` + its test: the script gains the
  skip — historical `uptrakit-controller-standalone-v*` tags are excluded
  from the emitted plan with a log line; test case 3
  (`test_parse-backfill-tags.sh:128-133`) changes from inclusion to skip
  assertion.
- `docker/Dockerfile.test`, `.github/workflows/docker.yml`,
  `docker-compose.yml` (explicit flavor-token FEATURES lists — see Docker
  section).
- Plugins: **asset-filter anchoring** — github release plugin
  (`crates/plugins/releases/github/src/plugin.rs:221`) and/or the
  self-update `asset_filter` value so a lean deployment cannot match the
  standalone archive on the merged release (see Update-path migration).
  Otherwise self-update + PVEHS discovery keep their mechanisms; planning
  confirms fixtures (self-update `plugin.rs:357,395` encode the unchanged
  service name; any PVEHS fixtures with the old tag prefix get updated).
- Scripts: `scripts/pvehs/install/uptrakit-install.sh`,
  `scripts/pvehs/ct/uptrakit.sh`.
- Tests: `release_config_invariants.rs`,
  `integration-tests/tests/helpers/containers.rs`, plugin tests touching the
  tag prefix.
- `crates/core/controller/CHANGELOG.md` replaced with standalone lineage.

### Documentation (non-optional)

- **New ADR** (created via `adrs new`, never hand-numbered): record the return
  to Shape A — one package, two feature-flavored assets, one release lineage;
  cite the 2026-04-28 split spec and the verified history above; record the
  **binstall-goal drop** (channel never existed — fake registry) and the
  **Docker standalone-only** decision (lean image retired). Supersedes the
  split spec's rationale.
- `docs/development/releases.md`: the binaries-per-release table (`:41-49`)
  must gain the artifact≠package case — one package row producing two archive
  prefixes (`uptrakit-controller`, `uptrakit-controller-standalone`) — plus an
  explicit asset-naming rule for archive-prefix ≠ package-name (today the
  table assumes 1:1); changelog-scoping section (`:174+`); install-flavor
  divergence note (source default vs released `db-all,nats` extras); an
  explicit note that **binstall is not a supported install channel** (goal
  dropped, no crates.io publication) and that the **lean Docker image is
  retired** (standalone-only; historical tags remain); the
  "Installing from source" example block (`:132`, `:136`) — its commented
  `cargo install --path crates/core/controller-standalone` line references
  the deleted crate and must be rewritten in flavor terms. Planning sweeps
  end-user/deployment docs for references to the retired lean Docker image.
- `AGENTS.md`: codebase-layout tree (crates/core lines — stale on multiple
  counts: still claims embedded infra lives under the controller bin's `src/`
  at line 100, omits `controller-runtime`); one-line crate-removal edit.
- `docs/development/embedded-frontend.md:6`,
  `docs/security/sudoers-management.md:97`,
  `docs/end-user/deployment/proxmox-helper-scripts.md:18,92`: standalone
  package references → merged package + flavor language.
- `CONTEXT.md:81`: the **Embedded Mode** glossary entry currently reads
  "built via the `controller-standalone` crate" — reword to describe the
  feature flavor (embedded features of the merged `uptrakit-controller`
  crate); the term itself and its meaning are unchanged.
- No wire-protocol or OpenAPI impact (no endpoint or wire-type changes).

## Dependencies

- **Soft relation:** `uptrakit-spec-2026-08-06-release-build-dedup` —
  proposes extracting `release-plz.yml` build jobs into a `workflow_call`
  workflow driven by a `ci/release-plz/binaries.json` manifest, plus a
  `docker.yml` drift gate. Same files, no design conflict: whichever
  implements second rebases onto the other's workflow structure (the merged
  package simply becomes one manifest entry with two flavor invocations).
  `/write-plan` must check that spec's state and, if its plan exists or is in
  flight, wire an implementation-stage ordering edge.
- **Soft relation:** `uptrakit-spec-2026-07-12-ci-workflow-hardening`
  (in progress) — its plan splits/reorders `package_and_upload()` in
  `release-plz.yml` (attest-before-upload). Same function this spec's build
  matrix touches. `/write-plan` must wire this spec's plan blocked-by
  `uptrakit-plan-2026-07-13-ci-workflow-hardening` if still open, else rebase
  onto the landed shape.
- **Soft relation:** `uptrakit-spec-2026-08-19-tag-series-version-handling`
  (in progress) — its plan 2 touches PHS-discovery tag-prefix inference in
  the same `proxmox-helper-scripts` plugin this spec retargets. Different
  mechanism (per-item `tag_prefix` config vs release-lineage rename); sanity
  check during planning that the new `uptrakit-controller-v*` prefix does not
  break its inference assumptions, and vice versa.
- **Soft relation:** `uptrakit-spec-2026-04-18-unified-god-binary` — this spec
  deliberately preserves the `controller-runtime` lib seam that spec needs;
  that spec is stale against the current tree (predates the runtime
  extraction) and will need a refresh before implementation. Relation only, no
  blocking edge in either direction.

## Deferred / Out of scope

- **Dedupe historical changelog entries** — duplicate entries produced by the
  dual-package era are copied verbatim into the merged CHANGELOG; a follow-up
  pass removes them. (Bead: `uptrakit-def-controller-changelog-dedupe`.)
- **Self-update release-scoping defect (pre-existing)** — the
  `releases.github` target returns every repo release with no tag-lineage
  scoping, so e.g. a `uptrakit-cli-v0.1.0` tag's version outranks the
  controller's `0.0.8`; the hardcoded `owner: "uptrakit", repo: "uptrakit"`
  slug also needs verification. Predates this spec; not fixed here.
  (Bead: `uptrakit-def-selfupdate-release-scoping`.)
- God-binary spec refresh — tracked by its own spec epic, not this cycle.

## Success Criteria

1. `crates/core/controller-standalone/` gone; workspace builds green with
   `cargo check --all-features` and the minimal-feature clippy/check gates.
2. One release lineage: next release is `uptrakit-controller-v0.0.8` (or per
   release-plz bump) carrying both archives (`uptrakit-controller-*`,
   `uptrakit-controller-standalone-*`) per target.
3. `cargo install --path crates/core/controller` with default features
   builds the full flavor; `--no-default-features --features lean` builds
   the lean flavor. No binstall criterion (goal dropped); the controller
   crate carries no `[package.metadata.binstall]` block.
4. Self-update is **no worse than the pre-merge baseline**, measured by an
   explicit before/after resolution trace (the plugin's release-selection
   machinery has pre-existing defects — no tag-lineage scoping, so any repo
   release's version competes; tracked separately, see Deferred — and this
   merge must not be gated on fixing them); the anchored asset filter
   selects exactly one asset per flavor on a merged release. PVEHS plugins
   detect and offer updates from the new lineage; PVEHS install script
   installs successfully end-to-end (file:// test loop per existing ops
   notes).
5. Changelog for the next release includes embedded-runtime commits exactly
   once; release body under the 100 KB guard.
6. `release_config_invariants.rs` and all quality gates pass; docs updated per
   the deliverables list.

## Risks

- **Release body growth** from the union `changelog_include` — mitigated by
  the existing 100 KB truncation; verify during planning.
- **First post-merge release-plz run** must produce a sane PR against the
  replaced CHANGELOG — anchor is the existing `uptrakit-controller-v0.0.7`
  tag, which release-plz resolves independently of CHANGELOG content; verify
  in the release PR before merging it.
- **Stale external installers** (old script copies) silently stop seeing
  updates — accepted; single deployment migrates manually.
- **Default dev builds get heavier** — `default = ["lean", "embedded-all"]`
  compiles the four embedded runtimes on plain `cargo build`/`cargo test`
  and pulls the embedded runtimes into the default workspace graph
  (rust-analyzer included). Accepted (owner decision: one flavor model beats
  build speed); explicit `--no-default-features` invocations are unaffected.
  The **pre-push hook** already runs against a 10-minute ceiling: measure
  default-feature `cargo test` wall time before/after the feature
  restructure PR, and if it crosses the hook budget, pin the hook's
  invocations to explicit feature sets rather than defaults.
- **Flavor-token drift** — the two flavor feature strings are hand-copied
  across `docker/Dockerfile.test`, `docker-compose.yml`, the surviving
  `docker.yml` matrix row, and the swagger build-args, with no failure signal if one
  omits `embedded-all` (silently changes what integration tests exercise).
  Mitigation: define the two token strings once (workflow env var or the
  `binaries.json` manifest the release-build-dedup spec proposes) and bring
  that spec's contemplated drift gate into this cycle rather than after.
