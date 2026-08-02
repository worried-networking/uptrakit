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

1. **Normalization depth: mechanical headers only.** Retitle, insert Status/Date (converting existing
   `**Status:**`/`**Date:**` inline lines rather than duplicating them), add missing section headings
   above existing prose. Prose untouched save for explicitly-listed one-sentence bridges where a
   required section has no prose at all. No rule suppression for format rules, no full rewrite.
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

- `.adr-dir` — content `docs/adr`. adr-tools-compat marker for third-party tooling.
- `adrs.toml`:

  ```toml
  adr_dir = "docs/adr"         # REQUIRED: once adrs.toml exists it wins over .adr-dir — omitting
                               # this key silently retargets every command to the default doc/adr
                               # and breaks the toolchain (empirically verified)
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
CI installs the pinned 0.10.1 via the repo's established one-liner idiom,
`taiki-e/install-action@v2` with `tool: adrs@0.10.1` (already used 3× in `ci.yml` for cargo-deny /
llvm-cov / machete; unlisted tools fall back to `cargo-binstall`, and the adrs v0.10.1 GitHub release
ships prebuilt binaries — verified). No hand-rolled install/cache logic. The pinned version lives in
that one line; bumping it is a normal PR.

### 2. Corpus import (mechanical, content-preserving)

- Rename `0033-shared-zeroconf-crate.md` → `0034-shared-zeroconf-crate.md`. Reference sweep is derived at
  plan time by grep, not from memory, covering the site classes: exact path literal
  (`0033-shared-zeroconf-crate`), bare `ADR-0033` refs whose context means zeroconf (known: 2 in
  `docs/superpowers/plans/2026-07-29-zeroconf-b-cli-discovery.md` /
  `docs/superpowers/specs/2026-07-28-cli-zeroconf-discovery-design.md`), and a final bare-word
  `\bADR-0033\b` + `0033-shared` sweep to confirm zero zeroconf-attributed leftovers.
- Every ADR file gets, where missing or non-canonical:
  - Title rewritten to `# N. Title` (e.g. `# 2. RouterOS Non-POSIX Bootstrap Probe`), keeping
    standard technical notation ("P-384", "OAuth 2.0") — the TOC generator links by filename (§4),
    so titles carry no slug constraint. The exact retitle set is derived from live doctor output at
    plan time — the ADR001 check is more lenient than its stated pattern (e.g. `# 0033 — …` already
    passes), so the rule text is not a reliable predictor.
  - `Date: YYYY-MM-DD` — the file's first-commit date (`git log --diff-filter=A --follow --format=%as`).
  - `## Status` section with `Accepted`.
  - **Conversion, not duplication, of existing inline metadata:** 18 of 33 files carry
    `**Status:** …` / `**Date:** …` bold-inline lines that doctor does not recognize. These lines are
    converted in place to the canonical `## Status` section / `Date:` line (preserving their stated
    values, e.g. 0006's "superseded by ADR-0033" note) — never left alongside a freshly inserted
    duplicate.
  - Missing `## Context` / `## Decision` / `## Consequences` headings placed above the existing prose
    that already serves that role. **Existing prose is never rewritten, merged, or deleted.** Where a
    required section has no corresponding prose at all (e.g. ADR-0002, three self-contained
    sub-decisions and no consequences narrative: the sub-decisions live under one `## Decision`
    umbrella), at most a one-sentence bridge may be added, each such sentence explicitly listed in
    the plan for review.
- Supersession link: `adrs link 33 Supersedes 6` — matches the corpus's own prose (0033:
  "**Supersedes:** ADR-0006 Decision 4"; 0006: "superseded by ADR-0033"). Ordering is load-bearing:
  `adrs link` inserts below a literal `## Status` heading and **silently no-ops (exit 0) when the
  heading is absent** (empirically verified on these exact files), so the link step runs only after
  normalization, followed by a grep assertion that the link line landed in both files.
- Acceptance is mechanical, not eyeball review of 33 diffs: (a) `adrs doctor` exits 0; (b) a
  prose-preservation assertion — strip exactly the inserted/converted header lines (title, `Date:`,
  `## Status` + status value, inserted section headings + their MD022 blank lines, listed bridge
  sentences) from each normalized file and compare the remainder against the pre-normalization file
  with its old title/metadata lines stripped (byte-exact after those stated strips; no other
  whitespace normalization). Evaluated on the post-normalization / **pre-link** snapshot — the
  subsequent `adrs link` step inserts a Supersedes line into 0033/0006 that is outside (b)'s
  strip-set; (c) because (b) by construction cannot see the converted values
  themselves, a positive value assertion — every converted metadata value (each date, each status
  text including e.g. 0006's "superseded by ADR-0033" note) must appear verbatim in the normalized
  file. Residual ADR014 warnings (~14 thin sections) are tolerated and documented in the meta-ADR —
  fixing them means writing content, out of scope.
- Every `adrs` mutation is suspect-until-asserted: the tool's observed failure mode is silent
  success (`link` exits 0 doing nothing; doctor's pre-normalization number parsing is title-keyed).
  Each `adrs new`/`adrs link` step pairs with a grep/existence assertion of the artifact it claims
  to have produced.
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

  The script mirrors the `ci/verify_*.sh` family idiom: the structural-parity sibling is
  `verify_agents_md_budget.sh` (same allowlist-free shape — `ROOT` resolution, failure accumulator,
  `verify_<name>: message` / `OK` output), not the allowlist-bearing `verify_no_security_audit.sh`.
  It must fail loudly on empty/garbled input rather than pass vacuously (empty ADR dir → explicit error).

- **`adrs doctor`** — full format/duplicate/link validation. In hooks: if `adrs` is not on `PATH`,
  print a one-line warning naming `cargo install adrs --locked` and skip (owner decision — local
  friction traded for CI enforcement). In CI: binary always installed, failure is fatal.

Wiring (all hook additions live in `.husky/`, picked up via `core.hooksPath`):

| Hook / gate | Trigger | Checks | Failure semantics |
| --- | --- | --- | --- |
| `pre-commit` (extend) | staged files under `docs/adr/` | dup guard; doctor; TOC staleness | guard + TOC hard (pure shell); doctor warn-skip w/o binary. Merge-path nuance: the existing hook `exit 0`s early on `MERGE_HEAD`/`REVERT_HEAD` by design (staged-file lints don't apply to merge commits) — **only the dup guard** is added inside that merge branch, before its `exit 0`, with a comment stating why (the concluding commit of a conflicted merge must still fail on a duplicate); doctor/TOC stay in the normal non-merge path. Belt-and-suspenders leg, not a primary layer: two same-number files merge cleanly (no textual conflict), so the clean case fires `pre-merge-commit`, never this branch — this leg only catches a duplicate riding along with an unrelated conflict. |
| `pre-merge-commit` (new) | clean (non-conflicted, non-FF) `git merge` | dup guard; doctor | aborts the merge before the merge commit exists. FF merges cannot introduce new duplicates (ancestor relation). |
| `pre-rebase` (new) | before any commit is replayed; args: `$1`=upstream, `$2`=branch (empty ⇒ current) | dup guard `--against $1` | non-zero exit **refuses the rebase**. Covers `git pull --rebase`. Known seam: `--onto <other>` rebases and interactive edits that renumber mid-flight are not predictable here — caught by `post-rewrite`/`pre-push`/CI. |
| `post-rewrite` (new) | after rebase/amend completes | dup guard | git ignores the exit code — **advisory loud warning only**; the blocking layers are pre-rebase before and pre-push/CI after. |
| `pre-push` (extend) | always | dup guard; doctor; TOC staleness | guard + TOC hard (pure shell); doctor warn-skip w/o binary. |
| CI (new step) | **unconditional on both event legs** (`push: ['**']` and `pull_request`) | install pinned adrs; `adrs doctor`; TOC staleness | hard fail — the authoritative gate. Hooks are bypassable (`--no-verify`) and may be absent entirely (husky-rs installs on cargo build — a docs-only contributor may have none); the unconditional push leg is the actual guarantee, local hooks are latency reduction. Path-scoping was considered and rejected: this workflow fires push + pull_request on the same SHA for same-repo branches, so a scoped PR leg saves nothing (the push leg installs adrs anyway), and no existing gate in this repo path-skips (`check_deny.sh` always runs; its `base_ref` branch selects diff mode, not skipping). Accepted availability risk: an adrs install outage blocks CI the same way a RustSec advisory-DB outage already blocks `cargo deny`. No dup-guard invocation: doctor's ADR012 already detects duplicates and the binary is always present when the step runs — the shell guard exists solely as the no-binary local fallback. |

Every gate lands with an observed-RED probe in the plan: synthetic duplicate ADR → each layer fails with
its named message → revert → green. The pre-rebase probe stages a real throwaway rebase.

### 4. Generated TOC

`docs/adr/README.md` is generated by a new `scripts/regen-adr-toc.sh` (same invoke-and-commit shape
as `regen-api.sh` / `regen-asyncapi.sh`) and committed. The script is pure shell: a fixed H1
preamble, then one `- [<first-H1 title>](<filename>)` entry per `docs/adr/[0-9]*.md`, links keyed to
the **actual filenames**. The generator owns H1-extraction correctness: it escapes `[`/`]` in link
text (a future `adrs new "Fix [urgent] X"` title must not emit a malformed link) and ends by
asserting every emitted link target exists on disk — trivially true for filename-keyed links, kept
as a tripwire against generator bugs. `adrs generate toc` is deliberately not used here (see Alternatives): it
derives link targets from the title slug rather than the filename (empirically verified), which on
this pre-existing corpus either produces stable-but-broken links or forces titles into nonstandard
notation ("P384" for P-384, "OAuth 2" for OAuth 2.0) — filename-keyed links delete that entire
failure mode plus the link-validity assertion it would have required. Being adrs-independent, the
TOC staleness gate (re-run the script to a temp file, `diff` against the committed copy) is hard at
every layer, like the dup guard. Honest precedent note: the OpenAPI/AsyncAPI staleness gates are
CI-only and use in-place `git diff` / an in-test `assert_eq!` — this gate is a new mechanism, and
its local hook legs are a deliberate addition beyond that precedent, not a copy of it. The committed
file is never hand-edited; it must pass `markdownlint` with the repo config as-is — **no
`.markdownlintignore` additions**.

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
  custom shell is limited to what the tool cannot do (cross-branch/cross-tree duplicate prediction,
  filename-keyed TOC generation).
- **`adrs generate toc` for the TOC** — rejected after empirical probing: it derives link targets
  from title slugs, not filenames, so on this pre-existing corpus it yields broken links unless
  every legacy title is contorted to slug-match its filename (degrading e.g. "P-384"→"P384",
  "OAuth 2.0"→"OAuth 2"); its `--intro` shim also carries an MD012 trailing-newline footgun. A
  ~15-line filename-keyed shell generator deletes the constraint, the degraded titles, and the
  link-validity check, and makes the TOC gate binary-independent.
- **Abandon sequential numbering (date- or hash-prefixed IDs)** — rejected: collision-freedom is
  real, but `adrs` (the tool being adopted, per the task) is built around sequential Nygard
  numbering, the corpus and 100+ cross-references are number-keyed (`ADR-0033`), and the owner
  explicitly chose managed sequential numbers. The guard apparatus is the accepted price of keeping
  human-readable IDs.

## Deliverables

Code / config:

1. `.adr-dir`, `adrs.toml` (repo root, committed).
2. Corpus normalization commit(s): 0033→0034 rename + reference sweep; header normalization /
   inline-metadata conversion across the corpus (each file gains only what it lacks); then
   `adrs link 33 Supersedes 6` + link-landed assertion.
3. `ci/verify_adr_numbers.sh` (dup guard, two modes, RED-probed; parity sibling
   `verify_agents_md_budget.sh`).
4. `.husky/pre-rebase`, `.husky/pre-merge-commit`, `.husky/post-rewrite` (new);
   `.husky/pre-commit`, `.husky/pre-push` (extended).
5. `ci.yml`: unconditional ADR step — `taiki-e/install-action@v2` pinned adrs + doctor + TOC staleness.
6. `scripts/regen-adr-toc.sh` (pure shell, filename-keyed) + `docs/adr/README.md` (generated TOC).

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
- **Hook bypass honesty.** `--no-verify` skips every local hook; `post-rewrite` cannot abort; a
  docs-only contributor who never runs `cargo build`/`test` has no hooks installed at all (husky-rs
  installs on build). The spec's guarantee is precisely: local hooks reduce latency-to-detection for
  contributors who have them; the unconditional CI push leg is the guarantee.
- **Generated TOC is a drift-gated artifact** — regenerate-and-diff, never hand-edit, same trust model
  as `openapi.json` (staleness detection + reviewable diff, not hard-red change detection).
- **markdownlint** runs on every touched/generated markdown file; no new ignores.
