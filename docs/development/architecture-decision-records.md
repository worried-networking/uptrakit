# Architecture Decision Records

ADRs live in `docs/adr/` and are managed with the [`adrs` CLI](https://joshrotenberg.com/adrs/)
(adr-tools-compatible mode; pinned 0.10.1 in CI). Policy and rationale: see
[ADR-0036](../adr/0036-manage-adrs-with-adrs.md).

## Creating an ADR

```sh
cargo install adrs --locked   # one-time; hooks warn-skip without it, CI enforces
adrs new "Title Of The Decision"
```

`adrs new` allocates the next free number and prints the created file (`no_edit = true` in `adrs.toml` skips the
editor). Fill in the generated Context/Decision/Consequences sections, then regenerate the TOC:

```sh
scripts/regen-adr-toc.sh
```

Never hand-allocate an ADR number and never hand-edit `docs/adr/README.md` (generated). Supersessions:
`adrs new --supersedes N "New Title"` or `adrs link <new> Supersedes <old>`.

## Validation gates

| Gate                                                               | Where                                                                                                                                              | Needs adrs binary |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------- |
| `bash ci/verify_adr_numbers.sh` — duplicate numbers                | pre-commit (incl. conflicted-merge commits), pre-merge-commit, pre-rebase (`--against`, refuses the rebase), post-rewrite (warning only), pre-push | no                |
| `scripts/regen-adr-toc.sh --check` — TOC staleness + link validity | pre-commit, pre-push, CI `markdown` job                                                                                                            | no                |
| `adrs doctor` — format, duplicates, links                          | pre-commit + pre-push (warn-skip if not installed), CI `markdown` job (hard)                                                                       | yes               |

CI is the authoritative gate: local hooks are bypassable (`--no-verify`) and absent until husky-rs installs them.
The CI leg runs on every push and PR unconditionally.

## Config

`adrs.toml` at the repo root: `adr_dir = "docs/adr"` (required — it overrides `.adr-dir`), `no_edit`,
`default_status = "accepted"`, Nygard template format, and `[doctor] ignore = ["ADR011"]` (numbering gaps are
accepted policy — the 0014 gap is permanent; duplicates stay fatal via ADR012).

## Recovering from a number collision

Two branches can still mint the same number and merge out of order — CI on the second merge goes red. The later
lander renumbers: `git mv docs/adr/NNNN-name.md docs/adr/<next-free>-name.md`, update the file's title line,
`grep -rn 'NNNN-name\|ADR-NNNN'` to re-point references, regenerate the TOC, and confirm `adrs doctor` exits 0.
