# Prevent Future Republication of Workspace-Internal DB/Crypto Crates

**Date:** 2026-06-09
**Status:** Approved

## Problem

Five workspace-internal crates are currently obligated to keep publishing new
versions to crates.io whenever their workspace-pinned version changes:

- `uptrakit-audit-log`
- `uptrakit-audit-log-derive`
- `uptrakit-shared-db`
- `uptrakit-crypto`
- `uptrakit-tenant-db`

They are workspace-internal database/encryption plumbing. They have no external
consumers and should not be on crates.io at all — yet today the publish flow for
the two genuinely-publishable crates (`uptrakit-service-sdk` and
`uptrakit-openapi-client`) forces these five to be present on the registry through
one transitive chain.

The existing `0.0.1` versions are already squatted on crates.io and will remain
there permanently (yanking is explicitly out of scope — see "Deferred"). What this
spec delivers is **bounded** — no future version churn, no implicit SemVer
contract with phantom external consumers, no release-plz noise around these
five — not name reclamation.

### The chain

The only two crates this project publishes to crates.io are `uptrakit-service-sdk` and
`uptrakit-openapi-client`. cargo's publish flow (and crates.io itself) validates every
named dependency entry in the published manifest, including dev-dependencies that carry
a `version` field, against the registry. Optional dependencies are validated the same way.

Walking the direct-dep graph of the two publishable crates:

```text
uptrakit-service-sdk (publish = true)
└── uptrakit-wire (direct)
    └── [dev-dependencies] uptrakit-audit-log = { workspace = true }
        └── [dependencies, optional] uptrakit-shared-db = { workspace = true }
            ├── uptrakit-crypto (direct)
            └── uptrakit-tenant-db (direct)
        └── uptrakit-audit-log-derive (path + version)

uptrakit-openapi-client (publish = true)
└── uptrakit-web-api-types (direct)
    └── uptrakit-wire (direct)  ← same chain from here
```

Because `wire` is a published crate, its manifest on crates.io must list
`uptrakit-audit-log = "0.0.1"` (the workspace-pinned version) under
`[dev-dependencies]`. That forces `audit-log` onto crates.io. Once `audit-log` is
on crates.io, its manifest references `uptrakit-shared-db` (optional, but still
validated). And so on down the chain.

Removing the single dev-dep from `wire` breaks the chain at its only load-bearing
edge: nothing else in the dep tree of either publishable crate references any of
the five subtree crates.

### Evidence

- `grep -rn "uptrakit-audit-log\|uptrakit-crypto\|uptrakit-shared-db\|uptrakit-tenant-db"
  crates/shared/{service-sdk,openapi-client,web-api-types,wire,types,surfaces,macros,
  build-info}/Cargo.toml` returns a single match: `wire/Cargo.toml` line 27
  (`[dev-dependencies]`).
- `cargo tree -p uptrakit-service-sdk --all-features -e normal` and the same for
  `openapi-client` show zero references to any of the five crates. They are forced
  onto crates.io only by the dev-dep edge, not by any runtime/build edge.
- `cargo publish -p uptrakit-service-sdk --dry-run --allow-dirty` fails today only
  on the orthogonal absence of `uptrakit-build-info` from crates.io. That precondition
  is not part of this spec's success criteria.

## Solution

Wire-side cut. Drop the single dev-dep from `wire`, replace the two test-fixture
`AuditActionType` references with a synthetic named constant local to wire's tests,
lock the five freed crates against accidental future republication, and add an
integration test in each publishable crate that fails if the chain ever re-forms.

## File-by-file changes

### 1. `crates/shared/wire/Cargo.toml`

Delete the dev-dep on `uptrakit-audit-log`:

```toml
[dev-dependencies]
serde_yaml_ng = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt", "time", "test-util"] }
# uptrakit-audit-log line is removed
```

### 2. `crates/shared/wire/src/tests.rs`

Remove the `use uptrakit_audit_log::AuditActionType;` import at line 10. Replace the
two references that consume it. Use a **synthetic, non-canonical** action_type value
rather than the audit-log catalog's real strings — the tests assert serde round-trip
shape of `AuditEventPayload`, which treats `action_type` as opaque `String`, so the
value is irrelevant to test coverage. A synthetic value eliminates the silent
catalog-rename drift risk that would otherwise exist if the audit-log constants are
renamed but the wire test's stale literals continue to compile and pass.

Add a module-level constant at the top of `tests.rs` (after the existing imports):

```rust
/// Synthetic action_type for `AuditEventPayload` serde tests. Not a real audit-log
/// catalog entry — the tests here verify wire serde round-trip shape, not catalog
/// correctness. Using a synthetic value avoids a workspace dep on `uptrakit-audit-log`
/// (which would re-form the crates.io squat chain documented in
/// docs/development/coding-standards.md "Publishable Crate Dependency Hygiene")
/// and avoids silent drift if the real catalog's constant names ever change.
const TEST_ACTION_TYPE: &str = "test.wire.synthetic_action";
```

Then replace:

- Line 229 (in `audit_event_serialization_roundtrip`):
  `action_type: AuditActionType::SOFTWARE_UPDATE_STARTED.to_string(),`
  becomes
  `action_type: TEST_ACTION_TYPE.to_string(),`

- Line 255 (in `audit_event_payload_round_trips_correlation_id`):
  `action_type: AuditActionType::SOFTWARE_UPDATE_FINALIZED.to_string(),`
  becomes
  `action_type: TEST_ACTION_TYPE.to_string(),`

The single doc comment on the constant carries the full rationale; no per-test
inline comments are needed since both call sites reference the named constant
whose doc-comment is one jump away in any editor.

### 3. Publish lock — five `Cargo.toml` files

Add `publish = false` to the `[package]` table of each:

- `crates/shared/audit-log/Cargo.toml`
- `crates/shared/audit-log-derive/Cargo.toml`
- `crates/shared/db/Cargo.toml` (package name `uptrakit-shared-db`)
- `crates/shared/crypto/Cargo.toml`
- `crates/shared/tenant-db/Cargo.toml`

Placement: directly after the existing `version = "0.0.1"` line, matching the
existing convention in `service-sdk/Cargo.toml` (`version = "0.0.2"\npublish = true`).

`release-plz.toml` already declares `release = false` for all five, so release-plz
behavior is unchanged. The `publish = false` field is belt-and-suspenders against a
contributor running `cargo publish -p uptrakit-shared-db` directly.

### 4. `Cargo.toml` (workspace) — register `cargo_metadata`

Add to `[workspace.dependencies]`:

```toml
cargo_metadata = "0.23.1"
```

`0.23.1` is the latest stable on crates.io as of 2026-06-09 (verified via
`cargo search cargo_metadata --limit 1`).

### 5. Two new integration tests

**`crates/shared/service-sdk/Cargo.toml`** — add to `[dev-dependencies]`:

```toml
cargo_metadata = { workspace = true }
```

**`crates/shared/service-sdk/tests/no_workspace_db_deps.rs`** — new file. See
"Guardrail test design" below for the implementation contract.

Mirror both changes for `crates/shared/openapi-client/`.

### 6. `docs/development/coding-standards.md`

Append a new section, placed alphabetically/topically appropriate to the existing
structure (suggested: near other crate-level discipline sections). See
"Documentation deliverables" below for full text.

## Guardrail test design

Both `no_workspace_db_deps.rs` files share the same shape. They MUST:

1. Invoke `cargo_metadata::MetadataCommand` directly. No `Command::new("cargo")`,
   no shell-out. The crate exposes a typed API; using it keeps the test resistant
   to PATH and `CARGO` env quirks.

2. Use **two separate `MetadataCommand::new()` instances** — one for the default-features
   run and one for the all-features run. Calling `.features(CargoOpt::AllFeatures)` twice
   on the same builder panics (`MetadataCommand` asserts `!self.all_features`). The two
   runs are wrapped in a single helper inside the test file:

   ```rust
   fn metadata(all_features: bool) -> cargo_metadata::Metadata {
       let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
       let mut cmd = cargo_metadata::MetadataCommand::new();
       cmd.manifest_path(manifest);
       if all_features {
           cmd.features(cargo_metadata::CargoOpt::AllFeatures);
       }
       cmd.exec().expect("cargo metadata")
   }
   ```

   The single `#[test] fn no_workspace_db_deps()` calls `assert_clean(metadata(false))`
   and `assert_clean(metadata(true))`.

3. Restrict the banned-name check to the **root-reachable closure** of the host crate
   (not all `metadata.resolve.nodes`). `cargo metadata` invoked from a workspace
   member returns the full workspace package list in `metadata.packages` and the
   full workspace resolve graph in `metadata.resolve.nodes`. Walking
   `metadata.resolve.nodes` directly would flag banned crates that the host does
   not actually depend on (e.g. `uptrakit-audit-log` will still be a workspace
   member after the fix; it just won't be reachable from `service-sdk` or
   `openapi-client`). The test scopes correctly by BFS from `metadata.resolve.root`
   (or, if `root` is `None` in newer `cargo_metadata` versions because the manifest
   is a workspace member, by BFS from the `PackageId` whose `Package.name` equals
   the host crate — `env!("CARGO_PKG_NAME")`).

4. Resolve package names via the `metadata.packages` lookup, not via `PackageId.repr`.
   `PackageId.repr` is the full id URI (e.g.
   `"path+file:///…/crates/shared/audit-log#uptrakit-audit-log@0.0.1"`), so a naive
   equality check against bare names always fails silently. Iterate the
   `metadata.packages: Vec<Package>` and build a `HashMap<&PackageId, &str>` mapping
   `pkg.id → pkg.name.as_ref()` (the `AsRef<str>` impl on the `PackageName` newtype is
   the stable accessor; prefer it over `.as_str()`, which works today only via a
   `Deref<Target = String>` chain that future versions may break). Build the map
   once per `cargo metadata` invocation, then look up each visited node's id to
   recover its plain package name.

5. Walk the root-reachable subgraph (built in step 3). For each visited node, look up
   its package name (step 4) and check against:

   ```rust
   const BANNED: &[&str] = &[
       "uptrakit-audit-log",
       "uptrakit-audit-log-derive",
       "uptrakit-shared-db",
       "uptrakit-tenant-db",
       "uptrakit-crypto",
   ];
   ```

   `uptrakit-audit-log-derive` is included even though it's a proc-macro with no
   runtime deps of its own — if it appears in the reachable closure, the chain has
   re-formed via the audit-log path-version dep, and the test should catch it.

6. On any hit, panic with a message that reconstructs the dependency chain from the
   host crate to the banned package. The BFS in step 3 produces child edges only;
   the test must record the BFS parent for each visited node and walk parent
   pointers back from the banned node to the host crate. The error message names
   the offending crate and prints the chain (one node per line, host → … → banned)
   so a future maintainer can see the regression point immediately.

7. Use `#[test]` attributes — no special features, no nextest-only constructs, no
   tokio runtime. The tests run under `cargo test --all-features` from a clean
   checkout. `metadata.resolve.unwrap()` is acceptable because the project's
   `clippy.toml` sets `allow-unwrap-in-tests = true`.

8. The two test files are independent — each `no_workspace_db_deps.rs` file restricts
   its analysis to its own host crate. Do NOT factor the shared logic into a helper
   crate; duplication is acceptable (two ~120-line files), and a helper would itself
   need to be workspace-only and would re-raise the question of whether it should
   depend on `cargo_metadata`.

### Test runtime cost

`cargo metadata` for a single crate in this workspace runs in well under a second
on a warm cache. Cold-cache (e.g. CI first run after `cargo clean`) costs several
seconds because cargo resolves the full workspace graph. Two `MetadataCommand`
invocations per test binary × two test binaries adds a worst-case ~30s on a cold
CI run; warm-cache CI runs add well under a second total. Acceptable.

## Documentation deliverables

### `docs/development/coding-standards.md` — new section

Add a section titled "Publishable Crate Dependency Hygiene". Suggested contents:

> ### Publishable Crate Dependency Hygiene
>
> Two crates in this workspace are published to crates.io:
>
> - `uptrakit-service-sdk`
> - `uptrakit-openapi-client`
>
> Their transitive dep trees (including `[dev-dependencies]` of any crate they
> reach) must NOT contain any of:
>
> - `uptrakit-audit-log`
> - `uptrakit-audit-log-derive`
> - `uptrakit-shared-db`
> - `uptrakit-tenant-db`
> - `uptrakit-crypto`
>
> These five crates are workspace-internal database and encryption plumbing. They
> have no external consumers and must not be republished to crates.io.
>
> **Why this matters.** `cargo publish` (and crates.io's manifest validator) check
> every named dep entry in the published manifest — including `[dev-dependencies]`
> that carry a `version` field, and optional deps — against the registry. A
> dev-dep on `uptrakit-audit-log` from any crate that the publishable crates
> transitively reach is enough to force `audit-log` onto crates.io, and `audit-log`
> in turn forces `shared-db`, which forces `crypto` and `tenant-db`. The chain is
> load-bearing on every edge: cutting any link breaks all of it.
>
> **Enforcement.** Two integration tests guard this rule:
>
> - `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`
> - `crates/shared/openapi-client/tests/no_workspace_db_deps.rs`
>
> Each test walks the resolved cargo metadata graph (default features and
> `--all-features`) and panics if any banned name appears, naming the dep chain
> back to the publishable crate.
>
> **Why these five and not other internal crates?** Most workspace-internal crates
> (`uptrakit-build-info`, every plugin, every runtime, etc.) inherit `publish = true`
> from Cargo's defaults but are kept off crates.io by `release-plz.toml` declaring
> `release = false`. That is sufficient because release-plz is the only mechanism
> that publishes from this workspace. These five crates additionally carry the
> belt-and-suspenders `publish = false` in their own `Cargo.toml` because they are
> the *unique* failure case where the squat chain demonstrably reformed once
> before; locking them in their manifests defends against a contributor running
> `cargo publish -p uptrakit-shared-db` directly (bypassing release-plz) and
> resurrecting the chain.
>
> If you find yourself wanting to add one of these crates to anything in the
> service-sdk or openapi-client subtree (including dev-deps), stop and think about
> what you're actually testing. The wire-side fix for the historical version of
> this rule replaced two `AuditActionType::*` constants with a synthetic
> `TEST_ACTION_TYPE` constant in `crates/shared/wire/src/tests.rs` — the test was
> asserting serde round-trip shape, not catalog correctness, so the constant binding
> added no coverage.

### `crates/shared/wire/src/tests.rs` — inline comment

As specified under "File-by-file changes" §2.

### No ADR

This change fails the hard-to-reverse + real-tradeoff bar for an ADR. Reversal is
trivial (re-add the dev-dep); the tradeoff (catalog-bound constant vs. synthetic
test-local constant) is documented on the new `TEST_ACTION_TYPE` constant's
doc-comment and in the standards section.

## Verification

All quality gates must pass from a clean checkout. Per `.superpowers/standards-snapshot.md`:

- `cargo fmt --all`
- `cargo check --no-default-features --features db-sqlite`
- `cargo check --all-features`
- `cargo clippy --all-targets --no-default-features --features db-sqlite`
- `cargo clippy --all-targets --all-features`
- `cargo test --all-features`
- `cargo deny check`
- `markdownlint --config .markdownlint.json '**/*.md'`

Refactor-specific success criteria:

1. `cargo tree -p uptrakit-wire --edges dev` shows no `uptrakit-audit-log` entry.
   (Targeting `wire` directly is the correct spot-check: `cargo tree -p uptrakit-service-sdk
   --edges dev` only traverses `service-sdk`'s own dev-deps, not transitive normal deps'
   dev-deps — `service-sdk` has no direct dev-dep on any banned crate today, so that
   command always passes and is not a meaningful guard.)

2. Both new integration tests pass:
   `cargo test -p uptrakit-service-sdk --test no_workspace_db_deps --all-features`
   `cargo test -p uptrakit-openapi-client --test no_workspace_db_deps --all-features`
   These tests — not the `cargo tree` spot-check — are the load-bearing enforcement.

3. All existing `wire` tests pass with the dev-dep removed:
   `cargo test -p uptrakit-wire --all-features`. In particular,
   `audit_event_serialization_roundtrip` and `audit_event_payload_round_trips_correlation_id`
   continue to pass with the literal-string substitution.

4. release-plz still parses `release-plz.toml` cleanly with the five Cargo.toml
   files carrying `publish = false`. Verified by running
   `release-plz update --dry-run` (or whatever the project's existing release-plz
   verification command is — see `release-plz.toml` comments) and observing no new
   errors.

5. **Out of scope for this spec's success:** `cargo publish -p uptrakit-service-sdk
   --dry-run` and the same for `openapi-client` will still fail today on the
   absence of `uptrakit-build-info` and the other published library crates
   (`uptrakit-wire`, `uptrakit-shared-types`, `uptrakit-surfaces`,
   `uptrakit-shared-macros`, `uptrakit-web-api-types`) from crates.io. That is an
   orthogonal precondition for ever shipping a new release of the two publishable
   crates; resolving it is not blocked or unblocked by this spec.

## Commits

The work splits naturally into one or two Conventional Commits matching the
standards-snapshot rule (scope = crate name where natural, omit scope for
cross-cutting changes):

- `refactor(wire): drop uptrakit-audit-log dev-dep; synthetic action_type const`
- `chore: lock internal crates with publish=false; add publishable-dep guards`

The second commit is cross-cutting (touches five Cargo.toml files, two test files,
the workspace Cargo.toml, and the standards doc), so omitting the scope on the
subject line is per the multi-scope rule in `docs/development/commit-messages.md`.
Details go in the body.

## Deferred / Out of scope

- **Yanking the existing `0.0.1` versions of the five freed crates from
  crates.io.** The user has accepted these as permanent squats. The squat names
  remain reserved; only future republication is blocked. Yanking is an irreversible
  registry action and explicitly out of scope here.

- **Non-optional `sea-orm = { workspace = true }` dep in
  `crates/shared/web-api-types/Cargo.toml`.** This forces every consumer of
  `openapi-client` (an HTTP client crate that has no business touching `sea-orm`)
  to compile `sea-orm` transitively. The compile-time cost is real but the issue
  is distinct from the squat chain and has its own design tradeoffs. Deferred to a
  separate spec.

- **Workspace-wide audit of `publish` fields on other crates.** Many other crates
  (e.g. `uptrakit-build-info`, `uptrakit-controller-runtime`, every plugin crate,
  every runtime crate) default to `publish = true` in Cargo.toml despite
  release-plz marking them `release = false`. Locking those is a hygiene win but
  is not part of this spec. The five locked here are the ones the squat chain
  freed; broader cleanup waits.

## Rejected alternatives

**Split `uptrakit-audit-log` into `audit-log` (pure event types/emit traits, no
DB) + `audit-log-db` (persistence layer that owns the `shared-db` dep).** This was
the user's first-proposed approach during grilling. It addresses the architectural
smell — `audit-log` shouldn't carry an optional DB dep — but is strictly worse for
the stated goal:

- It locks only three crate names (`shared-db`, `crypto`, `tenant-db`) against future republication.
  `audit-log` and `audit-log-derive` remain forced onto crates.io by the wire
  dev-dep.
- Every consumer that today enables `uptrakit-audit-log/db` would need to switch
  to depending on the new `audit-log-db` crate. The blast radius spans the
  controller, web-api, every notification plugin, the agent, the agent-ssh-runtime,
  and the test fixtures. Migration risk and review burden are high.
- The smaller wire-side cut locks all five names with a single dev-dep deletion
  and two test-fixture string substitutions.

The split was rejected as a strictly larger fix that achieves a strictly smaller
result. If the architectural cleanliness of `audit-log` becomes a separate
concern (e.g. for external consumers), it can be revisited in its own spec.

## Risks

- **`cargo_metadata` API surface change in a future minor version.** Low risk:
  the crate has been stable for years and the API used (`MetadataCommand::new()`,
  `.manifest_path(…)`, `.features(CargoOpt::AllFeatures)`, `.exec()`,
  `metadata.resolve.unwrap().nodes`) is the public, documented surface. Workspace
  pinning to `0.23.1` plus the workspace's existing Cargo.lock keeps the version
  fixed until someone deliberately bumps it.

- **A future contributor adds an `uptrakit-audit-log` dependency to a NEW crate
  that ends up in the service-sdk or openapi-client subtree.** This is the
  primary regression vector. The guardrail tests catch it on `cargo test`. If a
  contributor runs `cargo check` only and skips tests, the regression lands and
  surfaces at release-plz time. Acceptable: the pre-push hook runs `cargo test`
  per the quality-gates standard.
