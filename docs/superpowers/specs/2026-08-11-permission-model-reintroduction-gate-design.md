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
collect_findings "permission_model" \
  '\bPermission\b|permission_extractor|\brole_permissions\b|\bRolePermission\b|entity::(permission|role_permission)\b' \
  crates \
  --glob '**/*.rs' \
  --glob '!**/migration/**'
```

Plus the three mechanical touch-points the script's structure requires: `permission_model` added to the allowlist
rule-name validation, a `COUNTS` entry, and a violation print block with the message
`legacy permission-model identifiers remain outside allowlist:`.

Pattern rationale, per component:

- `\bPermission\b` — the deleted enum in any shape: declaration, path (`Permission::`), type position, import. Does
  not match `Permissions` (`std::fs::Permissions` sites in `crates/shared/directories`/`service-sdk` — no word
  boundary before the `s`) or `permission_extractor` (lowercase).
- `permission_extractor` — the deleted macro, any invocation or definition.
- `\brole_permissions\b` — the dropped join table's name in code (sea_query `Alias::new`, string literals, entity
  refs).
- `\bRolePermission\b` — the deleted entity struct name (`entity/role_permission.rs` exported it).
- `entity::(permission|role_permission)\b` — path references to the deleted entity modules. `\b` after the group
  keeps `entity::permission_x` unmatched.

Scope is `crates/**/*.rs` excluding `**/migration/**`, matching the `security_audit` family's scoping. The bare table
name `permissions` is deliberately not a pattern component — too generic (matches filenames, prose, unrelated
identifiers); reintroduction of that table is caught via `role_permissions`, `Permission`, and the entity-path shapes.

### Allowlist rows (exactly 2)

Dry-run of the full union pattern against main (2026-08-11) returns 9 hits: 7 comment-only lines (module docs in
`crates/shared/service-sdk/src/dirs.rs` and `crates/shared/directories/src/lib.rs` about Unix _permission hardening_,
one comment in the drop-regression test), which the existing comment filter skips, and exactly 2 code lines:

```text
permission_model|crates/core/agent-ssh-runtime/src/operations/bootstrap.rs|Permission denied
permission_model|crates/core/integration-tests/tests/database/migrations.rs|role_permissions
```

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
- `AGENTS.md` line 43 (quick-start block) — description becomes
  `# No legacy security_audit / raw action / permission-model identifiers`
- `docs/development/quality-gates.md` line 54 (canonical command list) — same description; AGENTS.md and
  quality-gates.md update in the same commit per the AGENTS.md maintenance rule
- `ci/verify_no_new_cfg_not_feature.sh` line 39 — a comment citing the script as its allowlist-parsing precedent;
  update the name in the comment

Historical mentions under `docs/superpowers/specs/*.md` are immutable records and stay untouched (same convention M1.9
applied to its own doc sweep).

### Enforcement surfaces

Corpus-shaped gate: no baseline, no git-history comparison, no resolution modes, no skip paths — it greps the checkout
it runs in. Behavior is therefore identical across pre-push, CI push/PR events, and shallow checkouts; the
inert-baseline failure class of history-comparing gates does not apply.

## Verification (done-when)

1. `bash ci/verify_no_legacy_identifiers.sh` exits 0 on main (zero fix cost claim holds).
2. Negative self-test, run once as evidence and not committed: append `pub enum Permission { A }` to any in-scope
   `.rs` file → gate exits 1 naming the line under `permission_model`; same for a `permission_extractor!` token and an
   `Alias::new("role_permissions")` line outside `migration/`; revert.
3. Allowlist non-blinding check: the two allowlist rows are text-pattern-scoped (`Permission denied`,
   `role_permissions`), so a genuine `Permission` type reference added to either allowlisted file still fails the
   gate. Evidence: temporary `use x::Permission;` line in `bootstrap.rs` → gate exits 1; revert.
4. Old name fully retired: `rg verify_no_security_audit` over the repo returns hits only under
   `docs/superpowers/specs/` (historical records).
5. `.husky/pre-push` and CI run the renamed script (hook run + CI green on the branch).
6. `markdownlint` green on the touched markdown; `python3 ci/verify_db_access_policy.py` untouched families all green
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

## Dependencies

None — bash + ripgrep, both already required by the script being extended.
