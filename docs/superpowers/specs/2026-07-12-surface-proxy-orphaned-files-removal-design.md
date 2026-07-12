# Remove Orphaned surface-proxy Module Files — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** delete five uncompiled files under `crates/ui/surface-proxy/src/proxy/`. No code change, no ADR, no
deps, no wire, no test.

## Problem

Audit `audit-2026-07-11` L1105 (MEDIUM · maintainability · ui-cli-surface-proxy · verified): five module files in
`crates/ui/surface-proxy/src/proxy/` are git-tracked but declared by **no `mod` statement anywhere in the crate**
— they are **never compiled**. They are an abandoned decomposition refactor: a near-verbatim parallel extraction
of logic that lives inline in the live `proxy.rs`. The hazard is concrete — a developer fixing a bug (e.g.
editing `dispatch.rs`) can watch tests pass and **ship nothing**, because the compiler never sees those files;
they also silently drift from the live copy.

## Verified current reality (byte-checked, 2026-07-12)

- The five orphaned files (all dated **May 3**, stale): `proxy/bookkeeping.rs` (276 lines), `proxy/dispatch.rs`
  (217), `proxy/idempotency.rs` (43), `proxy/prepared.rs` (70), `proxy/validation.rs` (248) — ~950 lines total.
- `lib.rs` declares only `mod proxy;` + `mod registry;`. `proxy.rs` declares only `mod controller_local;`,
  `mod local_executor;`, `mod tests;`, `pub mod entity_enrichment;` (`proxy.rs:17-28`). **None** of the five
  files is named by any `mod` statement anywhere in the crate.
- The only references to these modules are **within the island itself** (`prepared.rs` → `super::idempotency` /
  `super::validation`; `dispatch.rs` → `super::validation`). Nothing **compiled** references them (the
  `controller_local/*.rs` uses of `validation::Validate` resolve to the *different crate*
  `uptrakit_web_api_types::validation`, not the local `validation.rs`). They form a self-referential, uncompiled
  island.
- **The live `proxy.rs` is the authoritative copy; the dead files duplicate it** (confirmed by a diff during
  review):
  - The dead **helper** functions **do exist live under identical names** in `proxy.rs` — e.g.
    `timeout_pending_request` (`:447`), `fail_pending_request` (`:473`), `record_provider_failure` (`:483`),
    `validate_result_schema` (`:851`), `build_idempotency_key` (`:897`), and the `bookkeeping.rs` methods
    (`register_pending`/`take_pending`/`reserve_idempotency`/`cached_response`/… at `:522-668`) + the
    `validation.rs` helpers (`:677-935`). So most of the dead code is a same-named duplicate of live functions.
  - The two **top-level dispatch** functions the dead `dispatch.rs` defines — `execute_local_invocation` /
    `execute_proxied_invocation` (`dispatch.rs:5-154`) — are **not** named in the live path; their logic lives
    **inline** in `invoke_inner`'s two `match &resolved.interaction.transport` arms (`proxy.rs:279-407`,
    near-verbatim). So the dead files are an *unwired, partially-extracted* copy of the live code, not the code the
    crate runs. Either way, every dead-file capability has a behaviorally-equivalent live counterpart.

## Approach (chosen — delete the dead files, YAGNI)

`git rm` the five orphaned files. They are uncompiled (no `mod`), referenced only by each other, stale (drifted
since May 3), and duplicate the authoritative inline logic in `proxy.rs`. Deletion is **zero-risk** — the compiler
never sees them and nothing depends on them — and it removes ~950 lines of dead, silently-drifting code plus the
silent-no-op-edit footgun.

```sh
git rm crates/ui/surface-proxy/src/proxy/{bookkeeping,dispatch,idempotency,prepared,validation}.rs
```

Commit the removal in its own `refactor(surface-proxy): …` (or `chore(surface-proxy): …`) commit, matching the
repo's established dead-code-removal convention (e.g. `58ec792a6` deleting the 12-file `surface_runtime/` dead
dir, `a2536ab3c`/`335ea0402`/`cc48e3156`).

### Pre-deletion responsible-deletion gate (required in the plan, BEFORE deleting)

Confirm the dead files strand **nothing unique** — no bug fix, feature, or edge case present *only* in the dead
copy and missing from the live path. For the same-named helpers this is a direct fn-vs-fn diff (`proxy.rs`
`timeout_pending_request`/`validate_result_schema`/`build_idempotency_key`/… vs the dead copies); for the two
top-level dispatch fns, compare `dispatch.rs`'s `execute_*_invocation` bodies against `invoke_inner`'s inline
`match` arms (`proxy.rs:279-407`). **This gate was spot-verified during spec review**: every dead-file capability
has a named, behaviorally-equivalent live counterpart and nothing appeared stranded (the files are older than the
live code, so live is same-or-newer). The plan re-runs the per-function confirmation as the final pre-deletion
check; **if anything unique surfaces, port it into the live code first**, rather than blindly deleting.

## Verification

- `cargo check -p uptrakit-surface-proxy` (and `--all-features`) unchanged — the crate compiled **without** these
  files and still does; deletion removes nothing the build ever saw.
- Grep after deletion: zero `mod bookkeeping|dispatch|idempotency|prepared` / `proxy/validation` references remain
  (there were none to begin with — the island only self-referenced, and it is gone).
- `cargo clippy --all-targets` clean (no dead-code or unused-file churn).

## Tests

**None.** Deleting uncompiled files changes nothing the compiler or the test suite ever saw — there is no logic
to cover. `cargo check`/`clippy`/`test` all behave identically before and after. No `start_paused`.

## Deliverables

- Delete `crates/ui/surface-proxy/src/proxy/{bookkeeping,dispatch,idempotency,prepared,validation}.rs`.

### Documentation deliverables

- **No doc impact.** Grep found no doc / module-map / `CODEREVIEW.md` reference to these five files (they were
  never part of the compiled or public surface). The plan should re-grep `docs/` + any surface-proxy
  module-map/CODEREVIEW for the filenames and remove references if any surface; otherwise state "no doc impact."
- **No ADR, wire/OpenAPI/frontend/dependency change** — pure dead-file removal.

## Alternatives considered

- **Finish the decomposition** (declare the five modules + delete the inline copies from `proxy.rs`) — rejected:
  the orphaned files are **stale** and have silently drifted from the live copy, so adopting them risks
  reintroducing old behavior or dropping recent fixes; reconciling ~950 lines of stale parallel code against a
  working monolith is high-risk, high-effort churn. If `proxy.rs` genuinely warrants decomposition later, that is
  a separate, deliberate refactor done **fresh** against the current code — not resurrecting an abandoned May-3
  copy. Deleting the dead copy is the prerequisite either way ("do not leave both copies").
- **Leave them (add a lint/comment)** — rejected: they remain a silent-no-op-edit trap and keep drifting; a
  `#[path]`/comment does not stop a developer editing the wrong file. Deletion is the only real fix.

## Out of scope

Other unspecced immediate-Medium findings (core-mqtt-scheduler L911, plugins-infra L1042 frontend SSE param-nav,
web-api-routes L1226) — separate specs. Do **not** decompose `proxy.rs`, refactor the live proxy logic, or touch
the surface-proxy cancellation-safety issue (its own spec). This is purely dead-file removal.

**Sequencing (land this FIRST):** implement this deletion **before** the in-flight
`2026-07-11-surface-proxy-cancellation-safety-design.md`, which edits `proxy.rs` exactly where the dead files
duplicate it (`register_pending`/`take_pending`/idempotency) and itself warns implementers (its lines 94-98) not
to edit the orphan `bookkeeping.rs` by mistake. The two diffs are disjoint (deletion vs `proxy.rs` edits — they
rebase cleanly), but landing the deletion first **removes the exact footgun** that spec has to warn about. Git
history confirms the safety premise independently: the five files were last touched at the May-1/2 scaffold move
(`1d614943c`) and never after, while `proxy.rs` carried on — so no fix could have landed *only* in a dead file
(live is strictly same-or-newer).

**Decomposition is a separate open concern (not "never"):** deleting the abandoned decomposition does not fix the
946-line `proxy.rs` monolith the refactor was trying to break up — that remains a live maintainability concern
(the cancellation-safety spec has to navigate `proxy.rs`'s size). Any real decomposition is a separate, deliberate
refactor done fresh against current code; deleting the stale copy is the prerequisite either way. Ensure the
`proxy.rs`-size concern keeps a tracked home so "fresh later" is a plan, not a euphemism.
