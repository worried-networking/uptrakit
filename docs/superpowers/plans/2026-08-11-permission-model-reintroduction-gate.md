# Permission-Model Reintroduction Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Durable CI grep gate against reintroduction of the deleted permission model (`Permission` enum,
`permission_extractor!`, `permissions`/`role_permissions` entities), per
`docs/superpowers/specs/2026-08-11-permission-model-reintroduction-gate-design.md`.

**Architecture:** Extend `ci/verify_no_security_audit.sh` with a third `permission_model` rule family reusing its
existing rule-validation/COUNTS/print-block machinery, harden `collect_findings` against silently swallowed `rg` errors,
then rename the script to `ci/verify_no_legacy_identifiers.sh` sweeping all five live references.

**Tech Stack:** Bash + ripgrep only. No Rust code changes in any commit — no cargo gates required; the gate script
itself plus markdownlint are the quality gates for this branch.

## Global Constraints

- No new dependencies (spec §Dependencies: bash + ripgrep, both already required).
- Conventional Commits (per `docs/development/commit-messages.md`).
- NEVER pass `--no-verify`, `--no-gpg-sign`, `SKIP=`, or `NO_HUSKY_HOOKS=1` to any git command. If a hook fails, report
  its output and STOP without committing.
- All in-place file edits in shell steps use `perl -pi -e` (BSD `sed -i` is not portable on macOS); file appends use
  single-quoted `printf '%s\n' '…' >> file` (no heredocs, no scalar-var argument lists — zsh does not word-split).
- Every temporary probe edit to a `.rs` file is reverted with `git checkout -- <file>` in the same step; no probe
  content is ever committed.
- Snapshot rules in force: "Git hooks via husky-rs enforce a gate subset on commit/push"; "root AGENTS.md ≤500
  lines/≤60KB … no hardcoded counts"; markdownlint MD013 line_length=150 (code blocks exempt).
- All snippets below were executed against the live tree on 2026-08-11 and produced exactly the Expected outputs shown
  (probe line numbers will differ — match on content, not line number).

---

### Task 1: Harden `collect_findings` against swallowed rg errors

**Files:**

- Modify: `ci/verify_no_security_audit.sh` (the `collect_findings` function, currently lines 128–146)

**Interfaces:**

- Consumes: nothing from other tasks.
- Produces: `collect_findings` with temp-file exit-status capture — Task 2 adds a third caller of it unchanged.

- [ ] **Step 1: Demonstrate the current bug (red, phase 1 — the gate lies)**

Temporarily corrupt the first rule pattern (adds a dangling `(`):

```bash
perl -pi -e 's/collect_findings "security_audit" \x27target:/collect_findings "security_audit" \x27\(target:/' ci/verify_no_security_audit.sh
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
```

Expected (the bug): `verify_no_security_audit: OK` and `EXIT:0` — a broken pattern silently passes. Revert:

```bash
git checkout -- ci/verify_no_security_audit.sh
```

- [ ] **Step 2: Replace the function body**

In `ci/verify_no_security_audit.sh`, replace the entire `collect_findings` function with exactly:

```bash
collect_findings() {
  local rule="$1"
  local pattern="$2"
  shift 2
  local line path rest line_no text
  local tmp rc=0

  tmp="$(mktemp)"
  rg -n --no-heading "$pattern" "$@" >"$tmp" || rc=$?
  if (( rc > 1 )); then
    echo "verify_no_security_audit: rg failed (rc=${rc}) for rule '${rule}'" >&2
    rm -f "$tmp"
    exit 1
  fi

  while IFS= read -r line; do
    path="${line%%:*}"
    rest="${line#*:}"
    line_no="${rest%%:*}"
    text="${rest#*:}"

    if is_comment_only_line "$text"; then
      continue
    fi

    FINDINGS+=("${rule}${SEP}${path}${SEP}${line_no}${SEP}${text}")
  done <"$tmp"
  rm -f "$tmp"
}
```

Changes vs the old body: `2>/dev/null || true` on the `rg` process substitution is GONE (it made any status capture read
0 and hid rg's diagnostics); rg output goes to a `mktemp` file with `rc` captured, checked BEFORE the loop; status 1
(zero matches) stays legal, status ≥ 2 hard-fails. Do NOT use `wait $!` after a process substitution instead — that
requires bash ≥ 5.1. The message prefix stays `verify_no_security_audit:` here; Task 3's rename sweep updates every
prefix at once.

- [ ] **Step 3: Verify the fix (red, phase 2 — corrupt pattern now fails loudly)**

```bash
perl -pi -e 's/collect_findings "security_audit" \x27target:/collect_findings "security_audit" \x27\(target:/' ci/verify_no_security_audit.sh
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
```

Expected (named signature, verbatim):

```text
rg: regex parse error:
    (?:(target:\s*"security_audit")
    ^
error: unclosed group
verify_no_security_audit: rg failed (rc=2) for rule 'security_audit'
EXIT:1
```

Revert the corruption only (the function fix from Step 2 must survive — revert via perl, NOT `git checkout`):

```bash
perl -pi -e 's/collect_findings "security_audit" \x27\(target:/collect_findings "security_audit" \x27target:/' ci/verify_no_security_audit.sh
```

- [ ] **Step 4: Verify clean tree still passes**

```bash
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
```

Expected: `verify_no_security_audit: OK`, `EXIT:0`.

- [ ] **Step 5: Commit**

```bash
git add ci/verify_no_security_audit.sh
git commit --only ci/verify_no_security_audit.sh -m "fix(ci): fail verify_no_security_audit hard when rg itself errors

rg exit code 2 (malformed pattern, unreadable file) was swallowed by
2>/dev/null || true, so a broken rule pattern turned every family
silently green. Route rg output through a temp file, capture the exit
status, and hard-fail on status >= 2 before the parse loop; status 1
(zero matches) remains the legal steady state."
```

---

### Task 2: Add the `permission_model` rule family

**Files:**

- Modify: `ci/verify_no_security_audit.sh` (rule validation, collect_findings calls, COUNTS, violation print tail)
- Modify: `ci/verify_no_security_audit_allowlist.txt` (append 2 rows)

**Interfaces:**

- Consumes: Task 1's hardened `collect_findings` (same call signature as the two existing invocations).
- Produces: rule name `permission_model` (allowlist rows and print block key on this exact string); Task 3 renames
  files/prefixes but changes no rule names.

- [ ] **Step 1: Widen the allowlist rule validation**

In `ci/verify_no_security_audit.sh`, inside `load_allowlist()`, change:

```bash
    if [[ "$rule" != "security_audit" && "$rule" != "raw_action" ]]; then
```

to:

```bash
    if [[ "$rule" != "security_audit" && "$rule" != "raw_action" && "$rule" != "permission_model" ]]; then
```

- [ ] **Step 2: Add the third `collect_findings` invocation**

Immediately after the `collect_findings "raw_action" …` invocation (after its last `--glob '!**/docs/**'` line), insert:

```bash

collect_findings "permission_model" '\bPermission\b|permission_extractor|\brole_permissions\b|\bRolePermission\b|entity::(permission|role_permission)\b|table_name\s*=\s*"(permissions|role_permissions)"|Alias::new\("permissions"\)' \
  crates \
  --glob '**/*.rs' \
  --glob '!**/migration/**'
```

(Rule name + pattern on one line, matching both sibling invocations. `**/migration/**` is excluded because historical
migrations legitimately create/seed/drop these tables — spec §Out of scope.)

- [ ] **Step 3: Add the COUNTS entry**

Change:

```bash
declare -A COUNTS=(
  ["security_audit"]=0
  ["raw_action"]=0
)
```

to:

```bash
declare -A COUNTS=(
  ["security_audit"]=0
  ["raw_action"]=0
  ["permission_model"]=0
)
```

- [ ] **Step 4: Add the third violation print block**

Inside the `if (( ${#VIOLATIONS[@]} > 0 )); then` tail, after the closing `fi` of the `raw_action` block and before
`exit 1`, insert:

```bash

  if (( COUNTS["permission_model"] > 0 )); then
    if (( COUNTS["security_audit"] > 0 || COUNTS["raw_action"] > 0 )); then
      echo
    fi
    echo "verify_no_security_audit: legacy permission-model identifiers remain outside allowlist:"
    for entry in "${VIOLATIONS[@]}"; do
      IFS="$SEP" read -r rule path line_no text <<<"$entry"
      [[ "$rule" == "permission_model" ]] || continue
      echo "${path}:${line_no}:${text}"
    done
  fi
```

The separator condition is deliberately `security_audit > 0 || raw_action > 0` (ANY prior family), NOT a copy of the
raw_action block's adjacent-only check — copying that check verbatim drops the blank line when security_audit fires but
raw_action does not (spec §Rule family).

- [ ] **Step 5: Append the two allowlist rows**

```bash
printf '%s\n' 'permission_model|crates/core/agent-ssh-runtime/src/operations/bootstrap.rs|Permission denied' 'permission_model|crates/core/integration-tests/tests/database/migrations.rs|TABLES.*role_permissions' >> ci/verify_no_security_audit_allowlist.txt
```

Row 1: the `output.contains("Permission denied")` match guard (OS error prose). Row 2: the M1.8 drop-regression test's
`const TABLES` declaration — anchored on `TABLES` so the row does not blind the whole file to future `role_permissions`
lines.

- [ ] **Step 6: Clean-tree run (zero fix cost)**

```bash
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
```

Expected: `verify_no_security_audit: OK`, `EXIT:0`. (The 7 comment-only hits are skipped by `is_comment_only_line`; the
2 code-line hits match the new allowlist rows.)

- [ ] **Step 7: Negative self-test — all three registered families of token**

```bash
printf '%s\n' 'pub enum Permission { A }' 'pub fn permission_extractor_probe() {}' 'pub fn probe_alias() { let _ = r#"Alias::new("role_permissions")"#; }' >> crates/shared/types/src/lib.rs
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
git checkout -- crates/shared/types/src/lib.rs
```

Expected (line numbers will differ):

```text
verify_no_security_audit: legacy permission-model identifiers remain outside allowlist:
crates/shared/types/src/lib.rs:56:pub enum Permission { A }
crates/shared/types/src/lib.rs:57:pub fn permission_extractor_probe() {}
crates/shared/types/src/lib.rs:58:pub fn probe_alias() { let _ = r#"Alias::new("role_permissions")"#; }
EXIT:1
```

All three lines MUST appear — a missing line means a pattern component regressed.

- [ ] **Step 8: Negative self-test — two-family separator leg**

```bash
printf '%s\n' 'pub fn selftest_sep() { let _ = r#"target: "security_audit""#; }' 'pub enum Permission { A }' >> crates/shared/types/src/lib.rs
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
git checkout -- crates/shared/types/src/lib.rs
```

Expected: the `security_audit` block, then EXACTLY ONE blank line, then the `permission_model` block, `EXIT:1`:

```text
verify_no_security_audit: legacy security_audit callsites remain:
crates/shared/types/src/lib.rs:56:pub fn selftest_sep() { let _ = r#"target: "security_audit""#; }

verify_no_security_audit: legacy permission-model identifiers remain outside allowlist:
crates/shared/types/src/lib.rs:57:pub enum Permission { A }
EXIT:1
```

- [ ] **Step 9: Negative self-test — allowlist rows do not blind their files**

```bash
printf '%s\n' 'use x::Permission;' >> crates/core/agent-ssh-runtime/src/operations/bootstrap.rs
bash ci/verify_no_security_audit.sh; echo "EXIT:$?"
git checkout -- crates/core/agent-ssh-runtime/src/operations/bootstrap.rs
```

Expected: `permission_model` block naming
`crates/core/agent-ssh-runtime/src/operations/bootstrap.rs:…:use x::Permission;`, `EXIT:1` — the row's
`Permission denied` text pattern must not absorb a genuine type reference.

- [ ] **Step 10: Confirm working tree is clean and gate is green**

```bash
git status --short   # expected: only the two ci/ files modified
bash ci/verify_no_security_audit.sh   # expected: OK
```

- [ ] **Step 11: Commit**

```bash
git add ci/verify_no_security_audit.sh ci/verify_no_security_audit_allowlist.txt
git commit --only ci/verify_no_security_audit.sh --only ci/verify_no_security_audit_allowlist.txt -m "feat(ci): gate against reintroduction of the deleted permission model

Third rule family permission_model bans the M1.7/M1.8-deleted
identifiers in crates/**/*.rs (migrations excluded): the Permission
enum token, permission_extractor, the role_permissions/RolePermission
entity names, entity-module paths, and the SeaORM table-declaration
and query-builder literals for the dropped tables. Two allowlist rows
cover the only live collisions: an OS-error prose string and the M1.8
drop-regression test. Greps are clean on main, so the gate lands at
zero fix cost. Registered follow-up of the m18/m19 rows; spec at
docs/superpowers/specs/2026-08-11-permission-model-reintroduction-gate-design.md."
```

---

### Task 3: Rename to `verify_no_legacy_identifiers` and sweep all references

**Files:**

- Rename: `ci/verify_no_security_audit.sh` → `ci/verify_no_legacy_identifiers.sh`
- Rename: `ci/verify_no_security_audit_allowlist.txt` → `ci/verify_no_legacy_identifiers_allowlist.txt`
- Modify: `.husky/pre-push` (one line), `.github/workflows/ci.yml` (one line), `AGENTS.md` (line 43),
  `docs/development/quality-gates.md` (line 54), `ci/verify_no_new_cfg_not_feature.sh` (one comment line)

**Interfaces:**

- Consumes: Task 2's finished three-family script.
- Produces: final script path `ci/verify_no_legacy_identifiers.sh` (what CI and pre-push invoke from now on).

- [ ] **Step 1: git mv both files, sweep internal occurrences**

```bash
git mv ci/verify_no_security_audit.sh ci/verify_no_legacy_identifiers.sh
git mv ci/verify_no_security_audit_allowlist.txt ci/verify_no_legacy_identifiers_allowlist.txt
perl -pi -e 's/verify_no_security_audit/verify_no_legacy_identifiers/g' ci/verify_no_legacy_identifiers.sh
```

The perl sweep rewrites the `ALLOWLIST_FILE=` constant AND every message prefix in one pass. The allowlist txt contains
no self-reference (its only comment is `# rule|path|text-regex`) — no edit needed there.

- [ ] **Step 2: Update the two enforcement surfaces**

In `.husky/pre-push`, replace the line `bash ci/verify_no_security_audit.sh` with
`bash ci/verify_no_legacy_identifiers.sh` (sole occurrence). In `.github/workflows/ci.yml`, on the sole `- run:` line
invoking the old script, replace `verify_no_security_audit.sh` with `verify_no_legacy_identifiers.sh` (keep the line's
existing indentation).

- [ ] **Step 3: Update the two doc lines — byte-identical description**

`docs/development/quality-gates.md` is the canonical source; `AGENTS.md` carries a byte-identical copy (spec §Rename —
this deliberately unifies two currently-divergent comments; both files update in this same commit per the AGENTS.md
maintenance rule).

In `docs/development/quality-gates.md`, replace the line:

```text
bash ci/verify_no_security_audit.sh                                  # No legacy security_audit or raw semantic action literals
```

In `AGENTS.md`, replace the line:

```text
bash ci/verify_no_security_audit.sh                                  # No legacy security_audit / raw action literals
```

BOTH with this exact line (command is 4 chars longer, so the padding run shrinks by 4 to keep the `#` column aligned
with sibling rows):

```text
bash ci/verify_no_legacy_identifiers.sh                              # No legacy security_audit / raw action / permission-model identifiers
```

Verify byte-identity:

```bash
diff <(grep 'verify_no_legacy_identifiers.sh' AGENTS.md) <(grep 'verify_no_legacy_identifiers.sh' docs/development/quality-gates.md) && echo IDENTICAL
```

Expected: `IDENTICAL` (empty diff).

- [ ] **Step 4: Update the precedent comment**

In `ci/verify_no_new_cfg_not_feature.sh`, change the comment line:

```bash
# Validate + load every allowlist row up front (mirror verify_no_security_audit.sh:
```

to:

```bash
# Validate + load every allowlist row up front (mirror verify_no_legacy_identifiers.sh:
```

- [ ] **Step 5: Run the renamed gate + affected sibling gates + markdown lint**

```bash
bash ci/verify_no_legacy_identifiers.sh        # expected: verify_no_legacy_identifiers: OK
bash ci/verify_no_new_cfg_not_feature.sh       # expected: OK (comment-only change)
bash ci/verify_agents_md_budget.sh             # expected: OK (AGENTS.md line count unchanged)
markdownlint --config .markdownlint.json AGENTS.md docs/development/quality-gates.md
```

All four must pass.

- [ ] **Step 6: Old-name sweep**

```bash
rg --no-ignore --hidden --glob '!.git/**' --glob '!.superpowers/**' -l verify_no_security_audit | sort
```

Expected: a non-empty list where EVERY path starts with `docs/superpowers/` (historical spec + plan records, immutable
by convention — including the gate spec itself, which names the old script). Any hit outside `docs/superpowers/` = a
missed reference; fix it before committing. (`--no-ignore` because a machine-local global gitignore can hide
`docs/superpowers/plans/`; `--hidden` because `.husky`/`.github` are hidden dirs rg otherwise skips.)

- [ ] **Step 7: Commit**

```bash
git add ci/verify_no_legacy_identifiers.sh ci/verify_no_legacy_identifiers_allowlist.txt .husky/pre-push .github/workflows/ci.yml AGENTS.md docs/development/quality-gates.md ci/verify_no_new_cfg_not_feature.sh
git commit -m "refactor(ci): rename verify_no_security_audit to verify_no_legacy_identifiers

The script now bans three legacy identifier families (security_audit
target literal, raw action literals, deleted permission model), so the
old name undersold its coverage. git mv preserves history; the sweep
updates the pre-push hook, the CI workflow, the quality-gates.md
canonical row and its byte-identical AGENTS.md quick-start copy, and
the precedent comment in verify_no_new_cfg_not_feature.sh. Historical
mentions under docs/superpowers/ stay untouched by convention."
```

Note: `git status --short` before committing must show ONLY the seven paths listed above (the two renames show as `R`).
Anything else = investigate before committing.

---

## Documentation deliverables (covered in Task 3)

- `docs/development/quality-gates.md` — canonical gate row rename + new description (Task 3 Step 3).
- `AGENTS.md` — quick-start line, byte-identical copy, same commit (Task 3 Step 3).
- No new ADR: the gate enforces existing ADR-0039/M1.8 decisions — no architectural decision is made here (spec
  §Deliverables).
- No CONTEXT.md change (no new vocabulary), no README/API/frontend impact (gate is CI-internal) — carried from spec
  §Deliverables.

## Self-review notes

- Spec coverage: §Rule family → Task 2 Steps 1–4; §Allowlist rows → Task 2 Step 5; hardening requirement → Task 1;
  §Rename sweep → Task 3 Steps 1–4; done-when 1 → Task 2 Step 6; done-when 2 (three legs incl. separator + exit-2) →
  Task 2 Steps 7–8 + Task 1 Step 3; done-when 3 → Task 2 Step 9; done-when 4 → Task 3 Step 6; done-when 5 → Task 3 Step
  3; done-when 6 → Task 3 Step 5; done-when 7 → Task 3 Step 5 (markdownlint; no handler changes, so
  `verify_db_access_policy.py` is untouched and runs in pre-commit as usual).
- Every code snippet and Expected output in this plan was executed verbatim against the live tree on 2026-08-11
  (clean-tree OK, three-token leg, separator leg, non-blinding leg, exit-2 leg) and reverted; none is hand-derived.
- Intermediate-commit sanity: after Task 1 and Task 2 commits the script keeps its old name and old wiring — every
  commit deploys a consistent, green state; Task 3 flips the name and all references atomically in one commit.
