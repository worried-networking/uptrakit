# Surface-Proxy Orphan Retirement, Fresh Decomposition, and Orphan-Module CI Gate — Design

**Date:** 2026-08-10
**Status:** Design (pending plan)
**Supersedes:** `docs/superpowers/specs/2026-07-12-surface-proxy-orphaned-files-removal-design.md` and its plan
(retired into this spec's own bead epic `uptrakit-spec-2026-08-10-surface-proxy-orphan-retirement-design` at the
beads migration 2026-08-16; full text at `pre-beads-archive`) — both removed by this spec, see
[Supersession](#supersession-of-the-2026-07-12-spec).

## Problem

`crates/ui/surface-proxy/src/proxy/` holds five git-tracked files that no `mod` declaration reaches, so they never
compile: `bookkeeping.rs` (276 lines), `validation.rs` (254), `dispatch.rs` (217), `prepared.rs` (73),
`idempotency.rs` (43) — 863 lines. `lib.rs` declares only `mod proxy; mod registry;`; `proxy.rs` declares only
`mod controller_local; mod local_executor; mod tests; pub mod entity_enrichment;` (`proxy.rs:17-30`).

Consequences:

- `warnings = "deny"` and `clippy::all = "deny"` never see these files. No lint, no type-check, no test coverage.
- The ADR-0040 provider-origin security predicate exists in two places — live `proxy.rs:313-321` and dead
  `prepared.rs:56-64` (byte-identical today only because two commits, `5c1c2357b` and `b20b1175e`, hand-mirrored it).
  The dead copy reads as authoritative.
- Two review agents in separate phases cited these files as live code; one proposed extracting the live gate INTO
  `validation.rs`, which would have removed enforcement from the compiled binary. Recorded twice in
  `.superpowers/common-mistakes.md` (orphan-cited-as-live row, count 2).
- Nothing in CI prevents recurrence: no gate checks that every `.rs` under `src/` is reachable from a `mod`
  declaration, and the file-scanning gates (`ci/verify_*`) scope to `routes/` or `crates/plugins/` only.

Additionally, no positive `ProviderKind::BuiltIn` admission test exists for the ADR-0040 rules — both admission gates
test `== ProviderKind::Service`, so BuiltIn permissiveness is structural, never asserted.

## Provenance (carried from the superseded spec, extended)

- Audit origin: `audit-2026-07-11` L1105 (MEDIUM · maintainability · ui-cli-surface-proxy · verified) first recorded
  the orphan island and the edit-the-dead-copy-tests-pass-ship-nothing footgun.
- Family history: all five (plus `controller_local.rs`, `local_executor.rs`) were created 2026-04-18 by `1590272aa`
  as the SurfaceProxy half of `docs/superpowers/specs/2026-04-17-surface-runtime-decomposition-design.md`
  (Finding 1: "reduce `invoke_inner` to a coordinator"). The wiring step was abandoned: `controller_local` and
  `local_executor` were later declared; the five siblings never were. The crate move `1d614943c` (2026-05-01) carried
  them along verbatim.
- Same-track precedent: the SSH-runtime half of that spec produced an identical orphan — the 12-file, 3229-line
  `surface_runtime/` directory deleted by `58ec792a6` (2026-06-05). The repo has already chosen deletion for this
  track's orphans once.
- Post-scaffold provenance: exactly six commits touched the orphans after the crate move (`git log 1d614943c..HEAD`):
  `8f57ea013` (repo-wide lint-attr sweep, orphan-only, attribute-only), `fda763cea`, `f03bbec02`, `0e342bbe0`,
  `b20b1175e` (each also landed the real change in `proxy.rs`), and `5c1c2357b` ("align the orphaned prepare gate" —
  its real fix landed in `crates/shared/surfaces/src/surface.rs`). No fix ever landed only in a dead file.
- Island topology (verified safe to recreate): the orphans only referenced each other (`prepared.rs` →
  `super::{idempotency,validation}`, `dispatch.rs` → `super::validation`); no compiled sibling collides with the
  recreated module names (`controller_local/notifications.rs` uses the fully-qualified
  `uptrakit_web_api_types::validation::Validate`, not a local `validation`).
- Dead-code-removal commit convention the Phase 1 commit mirrors: `refactor(<crate>):` per `58ec792a6`, `a2536ab3c`,
  `335ea0402`, `cc48e3156`.
- Mirror-maintenance policy: two plans (`2026-08-03-access-mcp-surfaces-2-required-action-sweep.md:336`,
  `2026-08-10-provider-origin-descriptor-gate.md:486`) explicitly instructed implementers to mirror predicate changes
  into the orphan `prepared.rs` (a third, `2026-07-16-proxmox-guest-flow-provider-invocable.md:219`, gave the opposite
  and correct instruction: "DO NOT TOUCH"). **This spec voids the mirror policy.** No future plan may schedule an
  edit to a file that is not reachable from a `mod` declaration; the CI gate below makes such files impossible to
  keep.

## Decision summary

1. **Delete all five orphans** (per-file justification below). Reintegration-by-reconciliation is rejected: the live
   code is a verified strict superset of every orphan, and the orphans encode the pre-cancellation-safety design.
2. **Decompose the current `proxy.rs` fresh** into `mod`-declared responsibility modules, honoring the still-valid
   origin intent (`rust-idioms.md` § Module Design names surface/action dispatchers as canonical extraction
   candidates) with today's code, not the stale fork. Zero behavior change; the ADR-0040 gate stays in `invoke_inner`.
3. **Add a blocking CI gate** `ci/verify_no_orphan_modules.py`: every tracked `.rs` file must be reachable from a
   crate target root via `mod` resolution.
4. **Add BuiltIn-kind positive admission tests** for the two ADR-0040 admission gates.
5. **Fix the one live-doc stale reference** and supersede the 2026-07-12 spec/plan pair.

No ADR: nothing changes compiled-path behavior (deletion removes uncompiled files; decomposition is a mechanical
move; the gate is tooling, consistent with ADR-0022's preference for team-owned scripts). If implementation discovers
that any step requires a behavior change, stop and re-spec that step.

## Phase 1 — Delete the five orphans

### Per-file disposition (all: delete)

Duplication verified twice independently (explore agent + contrarian re-diff, 2026-08-10). Live is a strict superset
in every file; line references are hints against the 2026-08-10 tree.

| Orphan                 | Live counterpart                                                                           | Why delete, not wire                                                                                                                                                                                                                                                                                                                                     |
| ---------------------- | ------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bookkeeping.rs` (276) | `proxy.rs:41-48` (consts), `:171-217` (state structs), `:577-779` (`PendingState` methods) | Live adds owner-tagged idempotency (`IdempotencyInFlight.owner`), per-entry `deadline` + backstop sweep (`cleanup_expired`, `proxy.rs:754-768`), `PendingRegistration` param-object, RAII guards. Orphan's unconditional `release_idempotency` and owner-less `take_pending` are the pre-cancellation-safety design; wiring would regress a shipped fix. |
| `validation.rs` (254)  | `proxy.rs:850-1165` (all ten helpers, same names)                                          | Live adds `SurfaceProviderVisibility` threading, the provider self-target branch (`:932-937`), `select_available_provider_for_surface`, and the per-declared-param validation loop (`:998-1014`) the orphan lacks — the orphan's `validate_input_schema` accepts requests missing required params. Wiring = security downgrade.                          |
| `dispatch.rs` (217)    | inline transport arms of `invoke_inner` (`proxy.rs:328-464`)                               | Orphan lacks both RAII guards, reserves idempotency anonymously, and has no `method` field on the outbound `SurfaceActionRequest` — it does not compile against today's wire type.                                                                                                                                                                       |
| `prepared.rs` (73)     | `invoke_inner` prologue (`proxy.rs:280-321`)                                               | `PreparedInvocation`/`prepare_invocation` never shipped. Only live-equal content is the ADR-0040 predicate, byte-identical to `proxy.rs:313-321` because it was hand-mirrored; deleting it collapses the dual-home security predicate to one home.                                                                                                       |
| `idempotency.rs` (43)  | `proxy.rs:1116-1152`                                                                       | Both functions byte-identical live. 100% redundant.                                                                                                                                                                                                                                                                                                      |

Do **not** touch `crates/ui/surface-proxy/src/proxy/tests/bookkeeping.rs` — a distinct, compiled test file declared by
`proxy/tests.rs`. This same-stem/different-directory collision is why the CI gate below must resolve directories, not
match stems.

### Pre-deletion gate (commit-provenance, replaces the old per-function diff)

For each of the five post-scaffold commits above, confirm the substantive change also exists in compiled code. Three
(`fda763cea`, `f03bbec02`, `0e342bbe0`) touch `proxy.rs` in the same commit — direct confirmation. `8f57ea013` touched
only the orphan `dispatch.rs`, converting a bare `#[allow]` to `#[expect(reason = …)]` on `execute_proxied_invocation`;
there is nothing to port — the live equivalent is an inline `match` arm in `invoke_inner` that carries no such
attribute, and a lint attribute on dead code has no compiled counterpart by nature. `5c1c2357b` landed its fix in
`crates/shared/surfaces/src/surface.rs` — confirm that file carries it. If any other orphan-only change surfaces with
no compiled counterpart: stop, port it into live code first, then delete.

Tripwire: deletion must require **no `Cargo.toml` change and no code change**. If it does, the orphan claim is wrong
for that file — stop.

## Phase 2 — Fresh decomposition of `proxy.rs`

Mechanical move of current live code into `mod`-declared submodules; content comes only from today's `proxy.rs`,
never from the deleted orphans. **Ships as its own PR, after the Phase 1+3+4 PR lands** — deletion, gate, and tests
are zero-risk and independently valuable; the refactor must not hold them hostage, and it lands against the gate
already enforcing atomicity. Target layout (module charters deliberately sharper than the origin spec's — a module
named `validation` must not hold resolution logic, or it recreates the semantic magnet that misled reviewers):

- `proxy/validation.rs` — pure input/output validation only: `validate_input_schema`, `validate_sensitive_fields`,
  `resolve_timeout`, `validate_result_schema`, `validate_result_limits`, `schema_matches` — plus their consts, used
  nowhere else: `DEFAULT_TIMEOUT_SECONDS`/`MIN_TIMEOUT_SECONDS`/`MAX_TIMEOUT_SECONDS` (`proxy.rs:38-40`) and
  `MAX_RESULT_BYTES`/`MAX_RESULT_ROWS` (`proxy.rs:43-44`).
- `proxy/resolution.rs` — registry-lookup adaptation plus caller-origin/provider resolution:
  `caller_origin_for_request`, `implicit_target_provider_for_request`, `select_available_provider_for_surface`,
  `provider_is_available` (these consult live registries), and `map_lookup_error` (a pure lookup-error → proxy-error
  mapper, placed here because it adapts the same lookup step the resolvers serve). This is a different concern
  family from validation; naming it `validation` is what invited the historic extract-the-gate-into-validation.rs
  proposal.
- `proxy/bookkeeping.rs` — the in-flight state machine: budget/failure/idempotency consts, `PendingState` and its
  methods, `PendingRequest`, `PendingRegistration`, `IdempotencyKey`, `IdempotencyInFlight`, `CachedIdempotent`,
  `ProviderFailureState`, `decrement_counter`, and the RAII guards `PendingGuard`/`IdempotencyGuard`
  (cancellation-safety lives with the state it guards).
- `proxy/idempotency.rs` — `build_idempotency_key`, `fingerprint_request` (`proxy.rs:1116-1152`).
- `proxy/dispatch.rs` — a second `impl SurfaceProxy` block: the two transport arms extracted as
  `execute_local_invocation` / `execute_proxied_invocation` (from `invoke_inner`'s `match` at `proxy.rs:328-464`),
  plus the pending-request lifecycle family `complete` (`proxy.rs:474-479` — public entry point, callers in
  `routes/service_ws`, `routes/surfaces.rs`, `config_test_proxy.rs` are unaffected by an impl-block move),
  `timeout_pending_request`, `fail_pending_request`, `record_provider_failure`, `try_get_cached_response`,
  `store_cached_response`.
- `proxy.rs` keeps: the crate's public request/error surface — `SurfaceProxyError` (+ `Display`/`Error` impls),
  `SurfaceCallerOrigin`, `SurfaceInvokeRequest` (`proxy.rs:57-171`; all three are re-exported by `lib.rs:5-10`,
  which therefore needs no path change) — `SurfaceProxy` struct + builder, `invoke`/`invoke_inner` as the coordinator
  (resolution prologue,
  **the ADR-0040 provider-origin gate at its current home**, cache probe, delegation to the two execute methods),
  `fail_in_flight_for_provider`, and the existing `mod` declarations plus the five new ones. `proxy.rs` (and the
  sibling modules) reach moved `pub(super)` items via plain `use` of the new module paths — no `pub use` re-exports
  (a re-export cannot widen `pub(super)` visibility, and none is needed).
- `prepared.rs` is **not** recreated: the prepare/execute struct split never shipped and the coordinator prologue is
  clear inline. The security predicate stays in `invoke_inner` — extracting it into a helper module is exactly the
  refactor shape that previously pointed at dead code; it remains in the coordinator by design.

Rules:

- Zero behavior change. Existing tests (`proxy/tests/*`) are the behavioral anchor and must pass unmodified except
  for import-path adjustments. Exactly one test file imports moving symbols directly:
  `proxy/tests/bookkeeping.rs:5` (`use super::super::{IdempotencyKey, PendingRegistration, PendingRequest,
PendingState}`) — adjust to `use super::super::bookkeeping::{…}` (no re-export from `proxy.rs`; imports point at
  the new single home). All other test files import only `SurfaceProxy`/error/request types that do not move
  (verified by grep during spec review).
- Each moved item gets the narrowest visibility that compiles (`pub(super)`/`pub(crate)`); `unreachable_pub = "deny"`
  is the check.
- Each new module lands **with** its `mod` declaration in the same commit (the CI gate enforces atomicity; a plan
  step must never leave the tree with an undeclared file).
- Every deletion/move step names the imports it orphans and prunes them in the same step (deny-level unused-import
  ledger row).
- The module-level `#![expect(clippy::indexing_slicing, …)]` attributes at `proxy.rs:1-8` must NOT ride along
  wholesale: push each down to the narrowest module — **existing or new** — that actually triggers it;
  `unfulfilled_lint_expectations = "deny"` verifies placement for free. Expected outcome: `let_underscore_must_use`
  moves down into `dispatch.rs`/`bookkeeping.rs` (triggers at today's `proxy.rs:477,496,515` and `:765,811`);
  `indexing_slicing` has zero triggers in `proxy.rs`'s own body — its triggers live in the pre-existing children
  (`tests/*`, `entity_enrichment.rs`), so it stays at `proxy.rs` (or moves into those children), and deleting it
  outright would fail `cargo clippy --all-targets`. Never resolve a misplacement with `#[allow]`.
- Reword `IN_FLIGHT_SWEEP_MARGIN`'s doc comment where it names `MAX_TIMEOUT_SECONDS` — post-split they live in
  different modules; name the module in the comment.
- Phase 2 lands in commits separate from Phase 1 (never combined), so blame on the recreated paths starts clean; the
  Phase 2 commit body states: "these paths were previously held by never-compiled orphans deleted in
  \<phase-1 sha\>; earlier history for these paths is unrelated."

## Phase 3 — CI gate: `ci/verify_no_orphan_modules.py`

Blocking from day one. A throwaway prototype run during spec review (2026-08-10) over all workspace-member sources
found exactly the five known orphans, zero false positives, and zero unresolvable declarations, in under 0.2 s. The
plan re-establishes these results on the real gate; do not carry hardcoded file/crate counts forward — they drift.

Design (each point is load-bearing for the ADR-0022 false-positive bar):

- **Candidates from `git ls-files -- '*.rs'`**, never a filesystem walk — untracked scratch files can never fail the
  gate, and `target/` needs no exclusion logic.
- **Target roots from `cargo metadata --no-deps --offline --format-version 1`** (target `src_path`s need no
  dependency resolution; bare `cargo metadata` can hit the network on a stale lockfile and false-fail an offline
  pre-push). A `cargo metadata` invocation failure is reported on exit code `2` with an `environment error:` message
  prefix — distinguishable from a Class B `resolver gap:` by prefix, never by inventing a new exit code. Roots
  cover lib/bin/test/example/bench/build-script targets per workspace member — handles multi-bin crates,
  `[[bin]] path = …`, lib+bin crates, `xtask/`, and the `frontend` member without hardcoded layout assumptions.
  `tests/`, `examples/`, `benches/` roots are scanned too: `tests/common/helper.rs` is reachable only via `mod` and
  is where the next orphan would appear.
- **Resolver:** from each root, walk `mod name;` declarations (regex over source, comment/string-aware enough for
  this codebase) to `name.rs` / `name/mod.rs` in the declaring file's module directory; honor `#[path = "…"]` (4
  in-tree uses) and treat `include!("literal.rs")` as a visit. **Cfg-agnostic:** visit every declaration regardless
  of `#[cfg(...)]` — this is what makes the gate immune to feature/platform false positives.
- **Two failure classes, reported separately:**
  - Class A — tracked `.rs` file visited by no resolution walk: "orphan module" (author-actionable). Exit code 1.
  - Class B — `mod` declaration that resolves to no file: "resolver gap" (gate-actionable, fails loudly instead of
    silently making the target look orphaned). Exit code 2, shared with config errors. Zero occurrences in-tree
    today.
- **CLI + allowlist conventions** follow `ci/check_plugin_semantic_boundary.py` (the closer precedent — the
  `verify_db_access_policy.py` no-args/two-exit-code style does not fit a two-class gate): `argparse` with `--root`
  and `--allowlist` defaults, exit scheme `0` pass / `1` violations / `2` resolver-gap-or-config-error. Allowlist is
  one TOML file, `ci/no_orphan_modules_allowlist.toml`, entries carrying `path` (Class A) or `decl` (Class B), and a
  **mandatory non-empty `reason`** (enforced; missing-reason gets a fail fixture, mirroring
  `ci/testdata/plugin_semantic_boundary/fail/allowlist_missing_reason/`). Ships empty.
- **Exclusions:** `ci/testdata/` (fixture crates), plus the gate's own fixtures.
- **Fixtures + tests:** `ci/testdata/no_orphan_modules/{pass,fail}` fixture trees checked by
  `ci/test_verify_no_orphan_modules.py` (pattern: `ci/check_plugin_semantic_boundary.py` +
  `ci/testdata/plugin_semantic_boundary/` + `ci/test_check_plugin_semantic_boundary.py` — the repo's
  fixture-directory precedent; `verify_db_access_policy.py`'s inline-tempfile style is not it). The `fail` tree must
  reproduce the historic same-stem collision — a live `tests/bookkeeping.rs` beside an orphan
  `src/proxy/bookkeeping.rs` — proving stem-matching would miss it and directory-aware resolution catches it. Each
  fixture root carries its own `[workspace]` table and zero dependencies — without it, `cargo metadata` inside a
  fixture nested under the repo root errors ("not a member of workspace") and every fail fixture would surface the
  environment error instead of Class A.
- **Wiring (same commit):** `.husky/pre-push`, two `ci.yml` steps (unittest run + gate run, mirroring
  `ci.yml`'s `check_plugin_semantic_boundary` pair), `docs/development/quality-gates.md` (canonical list) **and** the
  AGENTS.md Quick-start block.
- Python 3 stdlib only; no new dependencies. TOML parsing uses the repo's established fallback idiom —
  `import tomllib` / `except ImportError: import tomli as tomllib` — as in `ci/verify_db_access_policy.py:19-27`
  (`ci/check_plugin_semantic_boundary.py:11-17` is the same idiom via the narrower `ModuleNotFoundError` subclass;
  CI pins no Python version; `tomllib` is stdlib only from 3.11).

Stated consequences and limits:

- Decompositions must land atomically (file + `mod` declaration in one commit). Intended discipline; say it in the
  gate's header comment.
- Non-goal: the gate cannot detect "`mod`-declared but never compiled under any cfg" rot (a cfg-agnostic walk visits
  those). An advisory depinfo cross-check (`target/**/*.d` vs `git ls-files`) is named as possible future work, not
  built — platform-cfg modules and `embed-frontend` guarantee false positives if made blocking.

## Phase 4 — BuiltIn admission regression tests

The invoke-time gate (`proxy.rs:313-321`) keys on `CallerOrigin::Provider` and is provider-kind-agnostic — a
BuiltIn-kind invoke test would assert nothing the existing `ProviderKind::Plugin` allow-tests
(`provider_proxied/mod.rs`) don't already cover. The untested surface is admission-side:

- `InteractionDescriptor::validate_for_provider` (`crates/shared/surfaces/src/interaction.rs:256-270`): add a
  BuiltIn arm to the accept-side test — gated (`required_action` set) + `provider_invocable` + `ProviderKind::BuiltIn`
  ⇒ `Ok`.
- `validate_descriptor_gated_provider_invocable` (`crates/shared/surfaces/src/protocol.rs:420-435`): add an
  end-to-end registry test via `SurfaceRegistry::bootstrap_builtin`
  (`crates/ui/surface-proxy/src/registry.rs:394`; provider id must start with `builtin.`) — descriptor-gated +
  `provider_invocable` BuiltIn registration succeeds, mirroring
  `bootstrap_plugin_accepts_provider_invocable_under_gated_descriptor` (same file, `:2390`).

Known BuiltIn-permissive branches beyond these two — in `crates/ui/surface-proxy/src/registry.rs`: `:410`, `:806`
(id-prefix rule, already tested), `:980`, `:1482-1483`; plus `protocol.rs:227` (priority-range bypass) — are
consciously deferred; this spec pins the
ADR-0040 admission posture only. Document in `docs/security/surfaces.md` (provider-origin section) that the invoke
gate is kind-agnostic by design and BuiltIn/Plugin permissiveness at admission is now test-pinned.

No time-dependent tests are added; none of the new tests call `tokio::time` APIs (admission validation is
synchronous), so `start_paused` does not apply — verified against the code paths under test, not just the test
bodies.

## Documentation deliverables

- `docs/development/surfaces.md:199` — `resolve_timeout` attribution: the Phase 1+3+4 PR points it at the live home
  (`proxy.rs`); the Phase 2 PR re-points it at `proxy/validation.rs` when the symbol moves. Never leave it naming the
  orphan.
- `docs/security/surfaces.md` — **PR1:** the kind-agnostic invoke-gate note (Phase 4); **PR2:** file-pointer table
  row for `proxy.rs` (`:200`) updated to the new module layout.
- `docs/architecture/surfaces.md:46,108` — **PR2:** re-check file pointers after decomposition; update if they name
  `proxy.rs` for moved concerns.
- `docs/development/quality-gates.md` + root `AGENTS.md` Quick-start block — **PR1**, new gate command (same commit
  as the gate).
- **PR2:** grep the whole docs tree post-decomposition for `proxy.rs` attributions of moved symbols (prose-grep
  ledger row — identifier greps alone are insufficient).
- Reference rule for both `bookkeeping.rs` files: after Phase 2, `src/proxy/bookkeeping.rs` and
  `src/proxy/tests/bookkeeping.rs` are both live; every doc/plan/tracker reference to either must use the full path,
  never the bare filename — the bare-filename grep is exactly what misled reviewers.
- No ADR. No wire/OpenAPI/asyncapi/frontend change.

## Supersession of the 2026-07-12 spec

- `git rm docs/superpowers/specs/2026-07-12-surface-proxy-orphaned-files-removal-design.md`; delete
  the stale-named plan (retired into this spec's own bead epic
  `uptrakit-spec-2026-08-10-surface-proxy-orphan-retirement-design`; plan files were untracked local
  artifacts — plain `rm`; full text at `pre-beads-archive`). Both executed 2026-08-10 alongside this
  spec's review.
- Rewrite inbound references to point here: `.superpowers/pending-specs.md:615, 702, 704` (tracker entry replaced per
  tracker procedure) and the plan retired into bead epic
  `uptrakit-spec-2026-07-16-interaction-system-unification-design` (beads migration 2026-08-16; full
  text at `pre-beads-archive`), formerly at line 22 of that plan.
  Tracker row 704 carries an open question ("`prepared.rs` denial may contradict documented `provider_invocable`
  opt-in") — close it as **resolved, no contradiction**: the live predicate ends with
  `&& !resolved.interaction.provider_invocable` (`proxy.rs:315`), so the opt-in is honored; verified 2026-08-10.
  Write the answer into the tracker rewrite, don't just delete the question with the row.
- Two things the deleted files carried are restated in this spec so they survive: the `audit-2026-07-11 L1105`
  provenance (see Provenance) and the recorded rejection of reintegrating stale copies (see per-file table — the
  2026-07-12 "decompose fresh later" deferral is discharged by Phase 2, not re-deferred).
- Other dated historical specs/plans that mention the orphan paths stay untouched (user decision 2026-08-10): the
  deletion makes those paths obviously dead, and dated plans are records, not living docs.

## Verification

- Phase 1: `cargo check -p uptrakit-surface-proxy` (and `--all-features`) identical before/after; `git status` shows
  exactly five deletions plus doc/tracker edits.
- Phase 2: `cargo test -p uptrakit-surface-proxy` — full existing suite green with no assertion changes;
  `cargo clippy --all-targets` clean; `unreachable_pub` clean.
- Phase 3: `python3 ci/verify_no_orphan_modules.py` green on the final tree; `ci/test_verify_no_orphan_modules.py`
  green, including the same-stem-collision fail fixture; gate red when run against a tree with the orphans present
  (spot-check: `git worktree add <temp-path> <pre-deletion-sha>`, run the gate with `--root <temp-path>`, then
  `git worktree remove <temp-path>`; never bare `git stash`).
- Phase 4: new admission tests green; test names greppable as `builtin` + `provider_invocable`.
- Docs: `markdownlint --config .markdownlint.json` on touched files; `npx prettier --write` for formatting fixes.

## Out of scope

- Decomposing `registry.rs` (157 KB — the larger debt, separate concern).
- The four deferred BuiltIn-permissive branches outside the ADR-0040 gates.
- The advisory depinfo dead-under-all-cfgs cross-check.
- Any behavior change to invocation, admission, or the ADR-0040 predicate.
- Editing dated historical specs/plans beyond the one inbound reference named above.
