# Adopt `adrs` for ADR Management — Design

Date: 2026-07-30
Status: Approved for planning

## Problem

`docs/adr/` holds 33 free-form ADRs managed by hand. Numbering is convention-only: nothing validates it, and
two parallel branches both allocated `0033` on 2026-07-29 (`0033-effective-plugin-enablement-and-surface-visibility.md`
and `0033-shared-zeroconf-crate.md`), landing a live duplicate on `main`. There is no format validation, no
duplicate detection at commit/merge/rebase time, and no CI gate over the ADR corpus.

## Goal

Manage and validate ADRs with the `adrs` CLI (crates.io, pinned **0.10.1** — latest stable, verified
2026-07-30) while preserving the `docs/adr/` path. Import the existing corpus so `adrs doctor` passes, resolve
the duplicate, and enforce — locally via git hooks and authoritatively in CI — that duplicate ADR numbers
cannot land, including duplicates produced by merges and rebases. All future ADRs are created via `adrs new`.

## Verified current state (probed 2026-07-30)

- `adrs 0.10.1` installed at `~/.cargo/bin/adrs`; crates.io `max_stable_version` is 0.10.1.
- `adrs init docs/adr` on a copy of the corpus preserves all 33 files and writes only a `.adr-dir` marker
  (content `docs/adr`) — no files rewritten, no initial ADR injected.
- `adrs doctor` on the current corpus: **exit 1**. Errors: 1× ADR012 (duplicate number 33), 14× ADR001
  (title not `# N. Title`), 31× ADR002 (missing `## Status`), 25× ADR003 (missing `Date:`), 2× ADR004 /
  4× ADR005 / 3× ADR006 (missing Context/Decision/Consequences headings). Warnings: 16× ADR011
  (numbering gaps — partly spurious because doctor's number recognition keys on the title line, which most
  legacy files lack), 14× ADR014 (thin/placeholder-looking sections).
- Git hooks run from `.husky/` via `core.hooksPath=.husky` (set by `husky-rs` 0.4). Only `pre-commit`,
  `pre-push`, `commit-msg` exist; any additional hook file dropped into `.husky/` (e.g. `pre-rebase`,
  `pre-merge-commit`, `post-rewrite`) is picked up natively. `husky-rs` does not manage or clobber
  unknown files there.
- Reference counts: `0033-shared-zeroconf-crate` has ~8 repo references (6 path refs in two
  `docs/superpowers/` files + 2 zeroconf-context bare `ADR-0033` refs). The effective-plugin-enablement
  ADR has 30+ bare `ADR-0033` references (code comments, docs, plans). Renaming the zeroconf ADR is the
  cheap direction.

## Decisions (owner-confirmed)

1. **Normalization depth: mechanical headers only.** Retitle, insert Status/Date, add missing section
   headings above existing prose. Bodies untouched. No rule suppression for format rules, no full rewrite.
2. **Duplicate resolution: `0033-shared-zeroconf-crate.md` → `0034-shared-zeroconf-crate.md`.** The
   `0014` gap stays (doctor gap rule suppressed; see config). No mass renumbering.
3. **Missing local binary: warn-and-skip for `adrs doctor` in hooks; CI is the hard gate.** The
   duplicate-number guard is pure shell (no binary needed) and always hard-fails locally.
4. **Extras in scope:** meta-ADR for this adoption; supersession link 0033→0006; generated
   `docs/adr/README.md` TOC with staleness gate.
5. **Bonus:** update the user-global `~/.claude/commands/write-plan.md` and `review-plan.md` to route ADR
   creation through `adrs` when the target project uses it.

## Design

### 1. Tool integration (compatible mode, path preserved)

Committed at repo root:

- `.adr-dir` — content `docs/adr`. adr-tools-native marker; this is what makes every `adrs` command
  target the existing path.
- `adrs.toml`:

  ```toml
  no_edit = true               # agents/scripts create ADRs; contributors edit the file afterwards
  default_status = "accepted"  # repo practice: ADRs land accepted

  [templates]
  format = "nygard"            # matches the normalized corpus

  [doctor]
  ignore = ["ADR011"]          # numbering gaps are accepted policy (0014); duplicates stay fatal via ADR012
  warnings_as_errors = false
  ```

Compatible mode (no `--ng` frontmatter): the corpus stays plain markdown, adr-tools interoperable.

Local installation is `cargo install adrs --locked` (documented, not enforced — see gate layering).
CI installs the pinned 0.10.1 (prebuilt binary via `cargo-binstall` where available, fallback
`cargo install adrs --locked --version 0.10.1`, cached across runs). The pinned version lives in one
place in `ci.yml`; bumping it is a normal PR.

### 2. Corpus import (mechanical, content-preserving)

- Rename `0033-shared-zeroconf-crate.md` → `0034-shared-zeroconf-crate.md`. Reference sweep is derived at
  plan time by grep, not from memory, covering the site classes: exact path literal
  (`0033-shared-zeroconf-crate`), bare `ADR-0033` refs whose context means zeroconf (known: 2 in
  `docs/superpowers/plans/2026-07-29-zeroconf-b-cli-discovery.md` /
  `docs/superpowers/specs/2026-07-28-cli-zeroconf-discovery-design.md`), and a final bare-word
  `\bADR-0033\b` + `0033-shared` sweep to confirm zero zeroconf-attributed leftovers.
- Every ADR file gets, where missing:
  - Title rewritten to `# N. Title` (e.g. `# 11. SPIFFE Service Identity`). Existing `# ADR-0011: …` /
    `# ADR 0016 — …` prefixes are dropped in favor of the doctor-parseable form.
  - `Date: YYYY-MM-DD` — the file's first-commit date (`git log --diff-filter=A --follow --format=%as`).
  - `## Status` section with `Accepted`.
  - Missing `## Context` / `## Decision` / `## Consequences` headings (9 insertions across ~5 files)
    placed above the existing prose that already serves that role. **No prose is rewritten, merged, or
    deleted** — headings and header lines are inserted only.
- Supersession link: `adrs link 33 Amends 6` (ADR-0033's effective-enablement predicate supersedes
  ADR-0006's visibility predicate — already stated in both files' prose; this makes it structural).
- Acceptance: `adrs doctor` exits 0. Residual ADR014 warnings (~14 thin sections) are tolerated and
  documented in the meta-ADR — fixing them means writing content, out of scope.
- Post-normalization, re-run doctor and confirm the warning set empirically (doctor's number parsing keys
  on titles; the pre-normalization gap/duplicate warnings are not a reliable predictor of the
  post-normalization set).

### 3. Enforcement layers

Two independent mechanisms, layered:

- **Duplicate-number guard** — `ci/verify_adr_numbers.sh`, pure POSIX shell + git, no `adrs` binary
  needed. Always hard-fails. Two modes:
  - no args: scan `docs/adr/[0-9]*.md` in the working tree for duplicate 4-digit prefixes;
  - `--against <rev>`: numbers introduced relative to `<rev>` (`git diff --name-only --diff-filter=A`)
    intersected with numbers present in `<rev>`'s tree (`git ls-tree`) — used by `pre-rebase` to predict
    the post-rebase collision before any commit is replayed.

  The script mirrors the `ci/verify_*.sh` family idiom (allowlist-free variant): same output shape, same
  strictness; the plan diffs it against a sibling (`verify_no_security_audit.sh`) for structure parity.
  It must fail loudly on empty/garbled input rather than pass vacuously (empty ADR dir → explicit error).

- **`adrs doctor`** — full format/duplicate/link validation. In hooks: if `adrs` is not on `PATH`,
  print a one-line warning naming `cargo install adrs --locked` and skip (owner decision — local
  friction traded for CI enforcement). In CI: binary always installed, failure is fatal.

Wiring (all hook additions live in `.husky/`, picked up via `core.hooksPath`):

| Hook / gate | Trigger | Checks | Failure semantics |
| --- | --- | --- | --- |
| `pre-commit` (extend) | staged files under `docs/adr/` | dup guard; doctor; TOC staleness | guard hard; doctor/TOC warn-skip w/o binary. Also covers the concluding commit of a conflicted merge. |
| `pre-merge-commit` (new) | clean (non-conflicted, non-FF) `git merge` | dup guard; doctor | aborts the merge before the merge commit exists. FF merges cannot introduce new duplicates (ancestor relation). |
| `pre-rebase` (new) | before any commit is replayed; args: `$1`=upstream, `$2`=branch (empty ⇒ current) | dup guard `--against $1` | non-zero exit **refuses the rebase**. Covers `git pull --rebase`. Known seam: `--onto <other>` rebases and interactive edits that renumber mid-flight are not predictable here — caught by `post-rewrite`/`pre-push`/CI. |
| `post-rewrite` (new) | after rebase/amend completes | dup guard | git ignores the exit code — **advisory loud warning only**; the blocking layers are pre-rebase before and pre-push/CI after. |
| `pre-push` (extend) | always | dup guard; doctor; TOC staleness | guard hard; doctor/TOC warn-skip w/o binary. |
| CI (new step) | every PR/push, alongside the other verify gates | install pinned adrs; `adrs doctor`; TOC staleness diff; dup guard | hard fail — the authoritative gate. Hooks are bypassable (`--no-verify`); CI is not. |

Every gate lands with an observed-RED probe in the plan: synthetic duplicate ADR → each layer fails with
its named message → revert → green. The pre-rebase probe stages a real throwaway rebase.

### 4. Generated TOC

`docs/adr/README.md` is generated by `adrs generate toc` and committed. Staleness gate (pre-push
warn-skip, CI hard): regenerate to a temp file, `diff` against the committed copy — same model as the
OpenAPI/AsyncAPI regen gates. The file is never hand-edited. The plan verifies the generated output
passes `markdownlint` with the repo config as-is; if it does not, adjust `[generate] toc_prefix` /
generation flags — **no `.markdownlintignore` additions**.

### 5. Future workflow

- New ADR: `adrs new "Title"` (numbering = local max + 1; `no_edit` returns the path; author fills
  sections, commits). Supersessions/links: `adrs new --supersedes N`, `adrs link`.
- Renumbering after a collision that slipped past local gates (e.g. two PRs merged out of order — CI on
  the second merge/PR goes red): `git mv` the newer file to the next free number and sweep its references;
  doctor confirms.
- The meta-ADR records: tool + pinned version, compatible mode, numbering policy (gaps accepted,
  duplicates fatal), gate layering, warn-skip rationale, residual ADR014 warnings.

### 6. Bonus: user-global command updates

`~/.claude/commands/write-plan.md` (doc-deliverables item 6) and `~/.claude/commands/review-plan.md`
(fix-step doc check) currently say "docs/adr/* — add new ADR for architectural decisions". Both gain a
conditional clause: *if the project manages ADRs with `adrs` (an `.adr-dir` file is present), create the
ADR via `adrs new --no-edit` and validate with `adrs doctor`; never hand-allocate a number.* These files
are user-global (outside the repo) — updated in place, not part of the repo commit.

## Alternatives considered

- **Suppress format rules for legacy files** — rejected: `[doctor].ignore` is global, so new ADRs would
  also escape validation, gutting the adoption.
- **Full Nygard rewrite of legacy bodies** — rejected: high content-loss risk on load-bearing invariant
  docs; mechanical header insertion achieves doctor-clean without touching prose.
- **Rename the effective-plugin-enablement ADR instead of zeroconf** — rejected: 30+ references vs ~8.
- **Close the 0014 gap** — rejected: renumbering 20 files churns 100+ references for zero value; doctor's
  gap rule is suppressed instead (duplicates remain fatal via ADR012).
- **Hard-fail hooks when `adrs` missing** — owner chose warn-and-skip; the always-hard pure-shell dup
  guard keeps the collision property enforced locally regardless.
- **Custom format validation scripts** — rejected: `adrs doctor` owns per-file format validation; the
  only custom shell is the cross-branch/cross-tree duplicate prediction the tool cannot do.

## Deliverables

Code / config:

1. `.adr-dir`, `adrs.toml` (repo root, committed).
2. Corpus normalization commit(s): 0033→0034 rename + reference sweep; header normalization across the
   corpus (each file gains only what it lacks); `adrs link 33 Amends 6`.
3. `ci/verify_adr_numbers.sh` (dup guard, two modes, RED-probed).
4. `.husky/pre-rebase`, `.husky/pre-merge-commit`, `.husky/post-rewrite` (new);
   `.husky/pre-commit`, `.husky/pre-push` (extended).
5. `ci.yml`: pinned adrs install + doctor + TOC staleness + dup guard step.
6. `docs/adr/README.md` (generated TOC).

Documentation (all repo docs must pass markdownlint):

1. Meta-ADR `0035-manage-adrs-with-adrs.md` — created via `adrs new`.
2. `docs/development/architecture-decision-records.md` — process doc: creating ADRs with `adrs`,
   config meaning, gate layering, collision recovery.
3. `docs/development/quality-gates.md` — new gate entries (canonical source) **and** the AGENTS.md
   quick-start block in the same commit (AGENTS.md maintenance rule).
4. `AGENTS.md` — quick-start gate line; pointer to the process doc where ADRs are mentioned.
5. `docs/README.md` — catalogue entry for the process doc.
6. `CONTRIBUTING.md` — one-line pointer (create ADRs via `adrs new`), if it enumerates doc workflows
   (verify at plan time; skip if it defers to docs/development).
7. User-global `~/.claude/commands/write-plan.md`, `review-plan.md` (outside repo).

## Out of scope

- Closing the 0014 numbering gap.
- Rewriting thin legacy sections (residual ADR014 warnings).
- `adrs mcp` server integration.
- NextGen (`--ng`) YAML-frontmatter mode.
- Fixing `--onto`/interactive-rebase blind spot beyond the post-rewrite warning + pre-push/CI backstop.

## Risks and notes

- **Doctor parsing is title-keyed.** Pre-normalization gap/duplicate output is partly spurious; the plan
  re-measures doctor output after normalization instead of trusting today's numbers.
- **Rename sweep classes.** The 0033→0034 sweep greps exact literals, composed forms, and bare-word
  `\bADR-0033\b` mentions (comments/help text), scoped per hit to zeroconf attribution — never a
  remembered file list.
- **Hook bypass honesty.** `--no-verify` skips every local hook; `post-rewrite` cannot abort. The spec's
  guarantee is precisely: clean merges and standard rebases fail locally at creation time; everything
  else fails at pre-push (if hooks run) and unconditionally in CI.
- **Generated TOC is a drift-gated artifact** — regenerate-and-diff, never hand-edit, same trust model
  as `openapi.json` (staleness detection + reviewable diff, not hard-red change detection).
- **markdownlint** runs on every touched/generated markdown file; no new ignores.
