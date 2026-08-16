# Durable Gate Against Permission-Model Reintroduction

Date: 2026-08-11
Status: Approved (owner round 2026-08-11: extend + rename the existing gate; broad token pattern with allowlist; registered identifier families only)

## Problem

M1.8 ([2026-08-07-m18-delete-permission-design.md](2026-08-07-m18-delete-permission-design.md)) deleted the `Permission`
enum, the `Permission`→`Action` shim, and the `permissions`/`role_permissions` entities; M1.7 had already deleted
`permission_extractor!`. M1.9 ([2026-08-09-m19-docs-adr-closure-design.md](2026-08-09-m19-docs-adr-closure-design.md))
closed the docs, but both milestones shipped **one-shot verification greps only**. The single durable check,
`ci/verify_action_security_declarations.py`, covers only the `x-required-permission` OpenAPI extension. Nothing stops a
future change (or an AI agent pattern-matching on git history) from reintroducing the deleted model. Greps are clean on
main today, so a durable gate lands at zero fix cost. This is the registered follow-up in the m18/m19 rows of
`.superpowers/pending-specs.md`.

## Decision

Extend `ci/verify_no_security_audit.sh` with a third rule family, `permission_model`, and rename the script to
`ci/verify_no_legacy_identifiers.sh` (its allowlist to `ci/verify_no_legacy_identifiers_allowlist.txt`) so the name
covers all three banned-identifier families: the legacy `target: "security_audit"` literal, raw audit-action literals,
and the deleted permission model. The existing script already has everything the gate needs — comment-only-line skip,
a rule-validated `rule|path|text-regex` allowlist, `|| true` on the `rg` pipe (zero-match is the desired steady
state), and wiring in both CI and pre-push — so this is a rule addition plus a rename, not new machinery.

### Alternatives considered

- **New dedicated `ci/verify_no_permission_model.sh`** — semantically clean name, but duplicates the allowlist/comment
  machinery and needs fresh wiring in `ci.yml`, `.husky/pre-push`, `quality-gates.md`, and `AGENTS.md`. Rejected: the
  precedent script is the canonical home for banned-identifier families; rename fixes the name drift instead.
- **Type-shaped pattern union** (`enum Permission|Permission::|: Permission|<Permission…`) instead of the broad token —
  avoids allowlist rows but goes silently blind to every unenumerated shape (impl block, turbofish, use-tree rename).
  Rejected per the recorded gate-narrowing failure class: with allowlist machinery available, broad-plus-allowlist makes
  every future legit collision a visible allowlist addition instead of a silent hole.
- **Keep the old script name** — no churn, but the name/content drift compounds with every family added. Rejected by
  owner.

## Design

### Rule family

One new `collect_findings` invocation, mirroring the two existing families:

```bash
collect_findings "permission_model" '\bPermission\b|permission_extractor|\brole_permissions\b|\bRolePermission\b|entity::(permission|role_permission)\b|table_name\s*=\s*"(permissions|role_permissions)"|Alias::new\("permissions"\)' \
  crates \
  --glob '**/*.rs' \
  --glob '!**/migration/**'
```

(Rule name and pattern on one line, matching both sibling invocations' formatting.)

Plus the three mechanical touch-points the script's structure requires: `permission_model` added to the allowlist
rule-name validation, a `COUNTS` entry, and a violation print block with the message
`legacy permission-model identifiers remain outside allowlist:`. The blank-line separator before the third block must
check **any prior family fired** — `(( COUNTS["security_audit"] > 0 || COUNTS["raw_action"] > 0 ))` — not just the
adjacent family, or a `security_audit`-only + `permission_model` run loses its separator (the existing two-block tail
at `ci/verify_no_security_audit.sh:190-193` checks only the immediately preceding count, so copying it verbatim is
wrong here).

One hardening requirement on the shared machinery (protects all three families): `collect_findings` currently ends in
`rg … 2>/dev/null || true`, which swallows `rg` exit code 2 — a malformed pattern or glob silently yields zero
findings and the gate prints `OK` (reproduced during review). Since this change adds the most complex pattern in the
script, `collect_findings` must capture `rg`'s exit status and hard-fail the script on status ≥ 2 (status 1 =
no matches stays legal). The fix shape is prescriptive because the obvious variants are each broken: keeping
`|| true` makes any status capture read 0 unconditionally (hardening becomes a no-op); converting to `rg … | while`
moves the loop into a subshell that discards every `FINDINGS+=` (gate permanently green); and a
`wait $!`-after-process-substitution form requires bash ≥ 5.1 (`$!` is not set for process substitutions before
that — older bash hard-fails on a clean tree). Use this tested, version-independent temp-file form — `|| true`
removed, `2>/dev/null` removed so the fatal path prints rg's own diagnostic, status checked **before** the loop:

```bash
  local tmp rc=0
  tmp="$(mktemp)"
  rg -n --no-heading "$pattern" "$@" >"$tmp" || rc=$?
  (( rc <= 1 )) || { echo "verify_no_legacy_identifiers: rg failed (rc=${rc}) for rule '${rule}'" >&2; rm -f "$tmp"; exit 1; }
  while IFS= read -r line; do
    # ... existing body unchanged ...
  done <"$tmp"
  rm -f "$tmp"
```

(`(( rc <= 1 ))` is `set -e`-safe as the left operand of `||`; `exit` inside the function reaches the script — both
call sites are plain top-level invocations. Cosmetic side effect, accepted: non-fatal rg stderr, previously
suppressed, can now print above `OK`.)

Pattern rationale, per component:

- `\bPermission\b` — the deleted enum in any shape: declaration, path (`Permission::`), type position, import. Does
  not match `Permissions` (`std::fs::Permissions` sites in `crates/shared/directories`/`service-sdk` — no word
  boundary before the `s`) or `permission_extractor` (lowercase).
- `permission_extractor` — the deleted macro, any invocation or definition.
- `\brole_permissions\b` — the dropped join table's name in code (sea_query `Alias::new`, string literals, entity
  refs).
- `\bRolePermission\b` — the deleted entity struct name (`entity/role_permission.rs` exported it).
- `entity::(permission|role_permission)\b` — single-path references to the deleted entity modules. `\b` after the
  group keeps `entity::permission_x` unmatched. **Known limitation**: the codebase's dominant brace-grouped import
  idiom (`use crate::entity::{permission, role};`) and lowercase use sites (`permission::Entity::find()`) never
  produce this literal — this component is a secondary net, not the primary catch.
- `table_name\s*=\s*"(permissions|role_permissions)"` — the SeaORM table-declaration attribute in the idiomatic
  derive form every entity in `crates/shared/db/src/entity/` uses; the primary catch for a `permissions`-table entity
  that dodges every identifier-shaped component via brace imports (contrarian-round addition, zero hits today). A
  hand-written `EntityName` impl or a deliberately singular table name would evade it — that is deliberate-evasion
  territory, outside a tripwire's scope.
- `Alias::new\("permissions"\)` — query-builder references to the dropped parent table outside migrations (the
  `role_permissions` equivalent is already covered by the bare token; zero hits today).

Scope is `crates/**/*.rs` excluding `**/migration/**`, matching the `security_audit` family's scoping. The bare table
name `permissions` is deliberately not a token component — dry-run returns 125 hits (OS-permission prose,
`.permissions()` std calls, doc comments), unusable noise; the two table-literal components above cover that table's
reintroduction shapes surgically instead.

### Allowlist rows (2 at landing)

Dry-run of the full union pattern against main (2026-08-11) returns 9 hits: 7 comment-only lines (module docs in
`crates/shared/service-sdk/src/dirs.rs` and `crates/shared/directories/src/lib.rs` about Unix _permission hardening_,
one comment in the drop-regression test), which the existing comment filter skips, and exactly 2 code lines:

```text
permission_model|crates/core/agent-ssh-runtime/src/operations/bootstrap.rs|Permission denied
permission_model|crates/core/integration-tests/tests/database/migrations.rs|TABLES.*role_permissions
```

Row 2's text pattern is deliberately anchored on `TABLES` (the const declaration) rather than bare `role_permissions`,
so the row does not blind the whole file to future `role_permissions` code lines. Future legit OS-prose sites
(`"Permission denied"`-class strings in SSH/sudo/file-op code) will add rows over time — the accepted cost of the
broad-token choice; historical rate is one such site in the last 300 commits.

The first is the `output.contains("Permission denied")` match guard (OS error prose, not the type). The second is the
M1.8 drop-regression test, which permanently probes that `permissions`/`role_permissions` do not exist at tip and that
the drop migration's `down()` recreates them in FK order — the one place in live code that must keep naming the
dropped tables.

### Rename

`git mv ci/verify_no_security_audit.sh ci/verify_no_legacy_identifiers.sh` and
`git mv ci/verify_no_security_audit_allowlist.txt ci/verify_no_legacy_identifiers_allowlist.txt`, updating the
script's `ALLOWLIST_FILE` constant and every `verify_no_security_audit:` message prefix to
`verify_no_legacy_identifiers:`. Reference sweep on the literal name (grep-derived, complete for live surfaces):

- `.husky/pre-push` line 78 (`bash ci/verify_no_security_audit.sh`)
- `.github/workflows/ci.yml` line 80
- `docs/development/quality-gates.md` line 54 — the **canonical source** for gate command definitions; its
  description becomes `# No legacy security_audit / raw action / permission-model identifiers`
- `AGENTS.md` line 43 (quick-start block) — **byte-identical copy** of the quality-gates.md description (today the
  two files word this gate's comment differently; this change unifies them, anchored on the canonical file); both
  files update in the same commit per the AGENTS.md maintenance rule
- `ci/verify_no_new_cfg_not_feature.sh` line 39 — a comment citing the script as its allowlist-parsing precedent;
  update the name in the comment

Historical mentions under `docs/superpowers/specs/*.md` are immutable records and stay untouched (same
convention M1.9 applied to its own doc sweep); historical plan text now lives at the `pre-beads-archive`
git ref, since that plans directory no longer exists. Note for verifiers: a machine-local global
gitignore may still exclude `docs/superpowers/specs/` from `rg`'s default walk — use `--no-ignore` when
sweeping.

### Enforcement surfaces

Corpus-shaped gate: no baseline, no git-history comparison, no resolution modes, no skip paths — it greps the checkout
it runs in. Behavior is therefore identical across pre-push, CI push/PR events, and shallow checkouts; the
**baseline-drift** failure class of history-comparing gates does not apply. The remaining inertness vector — `rg`
exit-2 swallowed by `|| true` — is closed by the hardening requirement above.

The gate is an **identifier-hygiene tripwire**, not architectural enforcement: it catches the deleted model coming
back under its own names. A semantically identical model reintroduced under fresh names (`Privilege`, a new
role-mapping table) passes clean — that class is only catchable by human/agent review against ADR-0039.

## Verification (done-when)

1. `bash ci/verify_no_legacy_identifiers.sh` exits 0 on main (zero fix cost claim holds).
2. Negative self-test, run once as evidence and not committed: append `pub enum Permission { A }` to any in-scope
   `.rs` file → gate exits 1 naming the line under `permission_model`; same for a `permission_extractor!` token and an
   `Alias::new("role_permissions")` line outside `migration/`; revert.
   Separator leg: with BOTH a `target: "security_audit"` line and a `Permission` line temporarily present (and no
   `raw_action` violation), the gate prints both violation blocks separated by exactly one blank line — proving the
   widened any-prior-family separator condition; revert.
   Exit-2 leg: temporarily corrupt one rule pattern into an invalid regex (Rust-regex dialect — rule patterns go
   straight to `rg`; `is_valid_ere` guards allowlist rows only; e.g. a dangling `(`) → the script must exit nonzero
   printing rg's parse error, not `OK` — proving the `collect_findings` exit-status hardening; revert.
3. Allowlist non-blinding check: the two allowlist rows are text-pattern-scoped (`Permission denied`,
   `TABLES.*role_permissions`), so a genuine `Permission` type reference added to either allowlisted file still
   fails the gate. Evidence: temporary `use x::Permission;` line in `bootstrap.rs` → gate exits 1; revert.
4. Old name fully retired:
   `rg --no-ignore --hidden --glob '!.git/**' --glob '!.superpowers/**' verify_no_security_audit` over the repo
   returns hits only under `docs/superpowers/` (historical spec records; plan records now live at the
   `pre-beads-archive` git ref, not this working tree, since that plans directory no longer exists).
   Flag rationale: `--no-ignore` guards against any machine-local global gitignore hiding
   `docs/superpowers/specs/`; `--hidden` because the two enforcement surfaces (`.husky/pre-push`,
   `.github/workflows/ci.yml`) live in hidden directories ripgrep skips by default even with
   `--no-ignore`; `!.superpowers/**` because local session state (gitignored) carries historical
   mentions; `!.git/**` because `--hidden` would otherwise walk git internals.
5. Description strings unified: the gate's inline comment in `docs/development/quality-gates.md` and `AGENTS.md` is
   byte-identical (`grep` the line from each file, `diff` the extracted comments — empty diff).
6. `.husky/pre-push` and CI run the renamed script (hook run + CI green on the branch).
7. `markdownlint` green on the touched markdown; `python3 ci/verify_db_access_policy.py` untouched families all green
   (no handler changes — expected no-op).

## Deliverables

- `ci/verify_no_legacy_identifiers.sh` (renamed + extended), `ci/verify_no_legacy_identifiers_allowlist.txt`
  (renamed + 2 rows)
- `.husky/pre-push`, `.github/workflows/ci.yml` — renamed invocation lines
- `docs/development/quality-gates.md`, `AGENTS.md` — renamed row/line + updated description, same commit
- `ci/verify_no_new_cfg_not_feature.sh` — precedent-comment name update
- No new ADR: no architectural decision — this enforces existing ADR-0039/M1.8 decisions. No CONTEXT.md change: no new
  vocabulary. No README/API/frontend impact: gate is CI-internal.

## Out of scope / accepted residuals

- **New migrations recreating the dropped tables pass unseen** — `**/migration/**` is excluded because historical
  migrations legitimately create, seed, and drop these tables, and a grep cannot distinguish a historical reference
  from a new one. Any code _using_ recreated tables is still caught.
- **Commented-out reintroductions are skipped** by the comment-only-line filter — the gate targets code, and the
  filter is what keeps the 6 permission-hardening doc comments out of the allowlist.
- **A bare `mod permission;` declaration alone is unmatched** — caught at any use site (`entity::permission::` path,
  `RolePermission`, table-name literal); an unused module declaration is inert.
- **Adjacent deleted identifiers** (`has_permission`, `PermissionsResponse`, `actions_for_permission`,
  `seed_permissions_for_owner`) — all zero-hit on main today, but outside the registered follow-up's three families;
  owner kept scope to the registered families.
- The `required_permission` serde alias and `missing_required_permission` reason_code literal in surfaces code remain
  deliberate non-defects (documented at `docs/security/surfaces.md`); neither matches any pattern component
  (lowercase, no `Permission` token, no `role_permissions` token).
- **A renamed equivalent model passes clean** — the gate bans the deleted names, not the architecture; reintroduction
  under fresh vocabulary is a review concern (ADR-0039), not a grep concern.
- **Allowlist rows are exact-path with no staleness detection** (pre-existing property of the script, shared by all
  families): if an allowlisted file moves in a future crate split, the row lingers unmatched or the gate fires on the
  new path — both surface loudly at the next gate run, neither is silent.
- **`rg` exit 2 also covers per-file I/O errors**, not only broken patterns — the hardening deliberately fails the
  gate there too: a partially scanned tree is unknown coverage, and the unsuppressed stderr says which file.

## Dependencies

None — bash + ripgrep, both already required by the script being extended.
