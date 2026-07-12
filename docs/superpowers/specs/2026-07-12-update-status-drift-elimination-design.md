# UpdateStatus Duplication Drift Elimination

**Date:** 2026-07-12
**Status:** Design
**Audit finding:** audit-2026-07-11 HIGH · stability · shared-core
**Sites:** `crates/ui/web-api-queries/src/queries/update_history.rs:18-31`,
`crates/shared/web-api-types/src/update_history.rs:11`

## Problem

`UpdateStatus` is defined **twice** with structurally identical seven-variant bodies:

- Canonical: `uptrakit_shared_types::UpdateStatus`
  (`crates/shared/types/src/update_status.rs`). `Copy + Eq + Hash`, `sea-orm`
  `DeriveActiveEnum`, `openapi` `ToSchema`, `strum::EnumIter`, `as_str` /
  `Display` / `FromStr` / `ParseUpdateStatusError`, plus the domain helpers
  `unfinished()` and `host_blocking()`.
- Duplicate: `uptrakit_web_api_types::update_history::UpdateStatus`
  (`crates/shared/web-api-types/src/update_history.rs:11`). Hand-written,
  `Clone + Debug + PartialEq` only, with its own `as_str` / `Display` /
  `FromStr` / `ParseUpdateStatusError`.

The DB entity already re-exports the canonical enum
(`crates/shared/db/src/entity/update_history.rs:3`:
`pub use uptrakit_shared_types::UpdateStatus;`), so the read path bridges the two
copies through a private converter, `db_status_to_api()`
(`update_history.rs:18-31`):

```rust
fn db_status_to_api(status: &update_history::UpdateStatus) -> UpdateStatus {
    match status {
        update_history::UpdateStatus::Queued => UpdateStatus::Queued,
        update_history::UpdateStatus::Pending => UpdateStatus::Pending,
        update_history::UpdateStatus::InProgress => UpdateStatus::InProgress,
        update_history::UpdateStatus::Completed => UpdateStatus::Completed,
        update_history::UpdateStatus::Failed => UpdateStatus::Failed,
        update_history::UpdateStatus::Interrupted => UpdateStatus::Interrupted,
        _ => {
            tracing::warn!("Unknown update status encountered, defaulting to Pending");
            UpdateStatus::Pending
        }
    }
}
```

Both enums are `#[non_exhaustive]`, so the `_ =>` arm is mandatory at this
cross-crate boundary — and it **silently swallows a missing `AwaitingRestart`
arm**. `AwaitingRestart` is a real, production-written DB state (dispatch and the
update reaper both `Set` it — see Out of Scope), so every awaiting-restart
record is returned to the API and Dashboard as **`pending`**, accompanied by a
spurious `"Unknown update status encountered"` warn.

The guard test `db_status_to_api_maps_all_variants` (`update_history.rs:705`)
omits `AwaitingRestart`, so CI never saw the gap. The bug is invisible precisely
because the wildcard makes the mapping _look_ total.

The frontend already renders an `awaiting_restart` badge (its status union
includes the variant), so the only broken link is this server-side mapping.

The `_ =>` arm is **not** a corruption canary, despite appearances: a genuinely
unknown DB string never reaches it — SeaORM's `DeriveActiveEnum` `TryGetable`
impl errors at row-load (`.all()`/`.one()` returns `DbErr`) before
`build_response` runs. The arm's only live trigger is the valid, in-range
`AwaitingRestart` variant, so its `warn!` fires on correct data. Removing it
removes a false alarm, not a diagnostic.

## Root cause

Two enums that must stay identical, bridged by a hand-maintained per-variant
converter with a lossy catch-all. This is a **drift class**, not a single bug:
any future variant added to the canonical enum will again fall through the
wildcard to `Pending`. Patching in one `AwaitingRestart => AwaitingRestart` arm
fixes today's symptom and leaves the trap armed for tomorrow's variant.

## Chosen approach — delete the duplicate, re-export the canonical enum

Collapse the two enums into one. The web-api-types copy becomes a re-export of
the canonical enum; the converter disappears because entity status and response
status become **the same type**, making the conversion identity and the drift
class _structurally impossible_ rather than merely patched.

This is the audit's own "longer term" recommendation, the most maintainable
option, and the smaller diff — it deletes code rather than adding an arm plus a
strum-iterating test.

### Change 1 — `crates/shared/web-api-types/src/update_history.rs`

Delete the local `UpdateStatus` enum, its `impl` blocks (`as_str`, `Display`,
`FromStr`), and its `ParseUpdateStatusError`. Replace with:

```rust
pub use uptrakit_shared_types::{ParseUpdateStatusError, UpdateStatus};
```

Feasibility (verified): the crate already depends on `uptrakit-shared-types`
(`Cargo.toml:28`), and its `openapi` feature already forwards
`uptrakit-shared-types/openapi` (`Cargo.toml:13`), so the canonical enum's
`ToSchema` derive is available wherever the duplicate's was. The re-export keeps
the public path `uptrakit_web_api_types::update_history::UpdateStatus` valid, so
every downstream `use` continues to compile unchanged.

Delete the two now-redundant local tests `test_awaiting_restart_serde` and
`interrupted_roundtrips`. After the re-export they would be exercising another
crate's enum, which violates the "do not test upstream/other-crate behavior"
rule. The canonical enum already owns this coverage in its own crate
(`serde_round_trip`, `from_str_round_trip`, `test_awaiting_restart_round_trip`,
all iterating `strum::EnumIter`).

### Change 2 — `crates/ui/web-api-queries/src/queries/update_history.rs`

Delete `db_status_to_api` entirely. At its sole non-test call site
(`build_response`, line 82) pass the status directly:

```rust
// was: db_status_to_api(&record.status),
record.status,
```

`UpdateStatus` is `Copy`, so this is a plain move of the field value; entity and
response fields are now the same type, so the assignment type-checks only if they
are identical — the compiler now enforces what the converter used to approximate.

Remove the `update_history::UpdateStatus` import if it becomes unused, and drop
`UpdateStatus` from the `uptrakit_web_api_types::update_history::{…}` import only
if the regression test (below) does not reference it — keep it otherwise.

Delete the two tests whose subject no longer exists:
`db_status_to_api_maps_all_variants` (line 705) and
`db_interrupted_maps_to_api_interrupted` (line 728).

### Regression guard — drift-class tripwire, all variants

Add one test in the `update_history.rs` query module's `tests`, matching the
existing `build_response_*_status` pattern (`build_response_completed_status`,
`build_response_queued_status`, …). Iterate **every** variant through the real
`build_response` code path via `strum::EnumIter` and assert round-trip parity:

```rust
#[test]
fn build_response_preserves_every_status() {
    use strum::IntoEnumIterator;
    for status in update_history::UpdateStatus::iter() {
        let mut record = /* minimal valid Model */;
        record.status = status;
        let resp = build_response(&record, "h".into(), "s".into(), String::new(), None);
        assert_eq!(resp.status.as_str(), status.as_str(),
            "status {status:?} must survive build_response unchanged");
    }
}
```

`update_history::UpdateStatus` (= the canonical enum) already derives
`strum::EnumIter` in the unified build graph (see the feature-unification note in
Risks), so `iter()` is available. This exercises the actual mapping-bearing code
path for all seven variants — it is not "testing the compiler": if a future
refactor re-inserts a lossy converter, DTO mapping, or `match` with a wildcard,
this test fails for the dropped variant. That guards the **drift class** the fix
eliminates, not just the one `AwaitingRestart` instance that triggered the audit.
`AwaitingRestart` is called out by name in the assertion message as the
documented-bug anchor (it yielded `Pending` before this fix).

This uses the mandated strum-iteration pattern
(`docs/development/coding-standards.md#exhaustive-enum-test-coverage`) applied to
the response code path, superseding the audit's narrower "iterate `strum` over
the DB enum in the converter test" suggestion — there is no converter left, so
the guard attaches to `build_response` instead.

## Alternatives considered

- **Patch in the missing arm** (audit's short option): add
  `AwaitingRestart => AwaitingRestart` and extend the mapping test to iterate
  `strum`. Rejected: leaves the duplicate enum, the converter, and the wildcard
  trap in place — the next new variant drifts again. More code than the deletion,
  and it preserves the very structure that caused the bug.
- **Add a `From<shared::UpdateStatus>` impl** to make the conversion total and
  checked. Rejected (YAGNI): still two types to keep in lockstep and a converter
  to maintain, for zero benefit over making them one type. Note the "one type"
  choice is superior only _while the two are meant to be identical_, which they
  are today (all seven variants are user-visible; the wildcard was corrupting
  data, not redacting an internal state). If entity and API status ever must
  diverge — e.g. the DB gains an internal-only state the API must not expose
  verbatim — reintroduce a checked `From` (or a response DTO) at that point. The
  deletion does not preclude that future; it just declines to pre-build it.

## Documentation deliverables

- **Generated API surface (required):** run `./scripts/regen-api.sh`; commit
  `crates/ui/web-api/openapi.json` and `frontend/src/lib/api/generated/`. The
  `UpdateStatus` schema is now sourced from the canonical enum — same schema name
  `"UpdateStatus"`, same seven `snake_case` variants; only the doc-comment-derived
  `description` fields change, and `awaiting_restart` is now emitted where the
  response previously said `pending`. CI gates on staleness of both files.
- **No prose doc claims `awaiting_restart` is reported as `pending`** (checked:
  `docs/architecture/update-history-entity.md`, `docs/development/coding-standards.md`,
  end-user docs). No prose doc edit required.
- **No new public docstrings** — this is a net deletion.
- **No ADR** — consolidating a duplicate onto an existing re-export pattern is a
  bugfix, not an architectural decision.
- **No wire-protocol change** — `asyncapi.yaml`, `SoftwareStates`, and all wire
  enums are untouched; this is confined to the HTTP read/response path.

## Out of scope

- The two production **write** sites are correct and unchanged — they already
  `Set` the canonical entity enum, not the duplicate:
  `crates/ui/web-api-queries/src/queries/update_dispatch.rs` (dispatch) and the
  update reaper. Only the read/response path was lossy.
- No changes to `UpdateStatus`'s variants, serialization, domain helpers
  (`unfinished()`, `host_blocking()`), or the partial unique indexes that
  reference `awaiting_restart`.
- The `?status=` request filter (`update_history.rs:213`,
  `Column::Status.eq(status.as_str())`) is **not** affected — it compares the
  DB text column against `as_str()`, so `?status=awaiting_restart` already
  matched correctly. The bug was purely response-side (`db_status_to_api`
  relabeling the returned rows). Post-fix `as_str()` on the canonical enum is
  byte-identical, so filtering behavior is unchanged; no filter test is added.
- No `From` impl, no wrapper type, no compatibility shim.

## Risks

- **OpenAPI schema-name collision (low, self-checking):** `openapi.json` currently
  has exactly one `"UpdateStatus"` schema (`:15781`); after the change it is
  sourced from `shared-types`. If both crates somehow contributed a same-named
  schema, `utoipa`/regen fails loudly, so CI catches it — verify at implementation
  time that `./scripts/regen-api.sh` succeeds and yields a single `UpdateStatus`
  schema.
- **Hidden byte-copy consumer (low, mitigated):** removing the duplicate and the
  converter was scoped by grepping the **bare** symbol `UpdateStatus` across the
  workspace, not only the qualified path (common-mistakes ledger, row 14).
  Verified consumers of the web-api-types enum — all source-compatible with the
  re-export:
  - `crates/ui/web-api-queries/src/queries/update_history.rs` (this query file);
  - `crates/ui/mcp/src/tools/history.rs:250` (`s.parse::<…UpdateStatus>()`,
    satisfied by the canonical `FromStr` + `ParseUpdateStatusError`);
  - `crates/ui/web-api/src/routes/update_history.rs` (imports + `pub use` for the
    `utoipa` path, satisfied by the canonical `ToSchema`);
  - `crates/ui/web-api/src/routes/interactive_ws.rs` (SSE types, not the enum);
  - `crates/ui/cli/src/commands/history.rs` and
    `crates/shared/openapi-client/src/update_history.rs` (test-only) — both reach
    the type transitively through `openapi-client`'s glob re-export
    (`pub(crate) use uptrakit_web_api_types::*;`), and use only `Display` on
    `ParseUpdateStatusError` plus variant constructors, all present on the
    canonical enum.

  The canonical enum is a strict superset (adds `Copy + Eq + Hash` and the helper
  methods), so no consumer loses capability.

- **`sea-orm` feature-unification (informational, no risk):**
  `web-api-queries`/`web-api` depend on `uptrakit-shared-types` _without_ the
  `sea-orm` feature, but the DB crate pulls it in with `sea-orm`, so Cargo
  workspace feature-unification enables `sea-orm` for the canonical enum across
  the whole build graph. That means its `EnumIter` derives via the
  `#[cfg_attr(feature = "sea-orm", …)]` line, not the inert
  `#[cfg_attr(all(test, not(feature = "sea-orm")), …)]` line — no double-derive,
  no compile issue. Noted only because it is the kind of feature-unification
  detail that could shift under a different crate-graph shape.

## Quality gates

Rust: `cargo fmt --all`;
`cargo check --no-default-features --features db-sqlite`;
`cargo check --all-features`;
`cargo clippy --all-targets --all-features`;
`cargo test --all-features` (targeting the two touched crates plus a workspace
build). Generated surface: `./scripts/regen-api.sh` then verify `openapi.json` +
`frontend/src/lib/api/generated/` are staged and non-stale. Frontend: `npm run
build` to confirm the regenerated client type-checks. Markdown: `markdownlint`
on this spec.
