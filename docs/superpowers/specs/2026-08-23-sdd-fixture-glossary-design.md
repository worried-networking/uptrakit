# SDD Fixture Glossary Page — Design

Status: spec written 2026-08-23. Source: bead `uptrakit-sdd-spec-fixture-task`.
This is a THROWAWAY SMOKE FIXTURE for the SDD pipeline. Keep every artifact minimal.

## Problem and goal

The SDD pipeline needs a small, safe change to exercise its spec → review → plan → implement stages.
Goal: add one glossary page for three SDD fixture terms (convoy, workflow root, step bead) under `docs/`.
No code changes.

The glossary page is a throwaway pipeline fixture. It may be deleted at any time. Nothing in the
repository may depend on it.

## Chosen approach

- Create one new page: `docs/superpowers/sdd-fixture-glossary.md`.
- Format the three entries as one GFM table with two columns: Term and Definition.
- Do not link the page from `docs/README.md`. The page is a pipeline fixture, not part of the
  audience-facing documentation catalogue. (Removal covers the page plus the spec-cycle
  leftovers — see Deferred.)
- Page structure: H1 title, one marker sentence, the table. The marker sentence must state that
  the page is a throwaway fixture for the SDD pipeline and not an authoritative vocabulary
  source, and must point to `CONTEXT.md` as the repo's controlled vocabulary.
- Definition text is fixed by this spec. The terms come from the Gas City / beads workflow domain;
  no tracked repo file defines them (untracked tooling material under `.gc/` and `.claude/skills/`
  describes their usage, and these definitions paraphrase that usage). Each Definition cell
  contains exactly this text:
  - convoy: "A bead that groups one or more work beads (via `tracks` dependencies) so tooling can
    address and monitor them as one unit."
  - workflow root: "The bead that represents one workflow (formula) run; step beads attach to it."
  - step bead: "A bead that represents one step of a workflow run, routed to an agent for
    execution."

No alternatives carry a real tradeoff at this size. The two open decisions went through the grill
(see Decision log).

## Decision log

| Question | Answer | Rationale |
| --- | --- | --- |
| Where does the page live? | `docs/superpowers/sdd-fixture-glossary.md` | `docs/` root contains only `README.md`; all pages live in subdirectories. `docs/superpowers/` already holds pipeline-internal documents and sits outside the audience catalogue in `docs/README.md`. A throwaway fixture is easy to remove from there. (Gate `uptrakit-n84dp`, answer A.) |
| Entry format? | GFM table (Term, Definition) | GitHub renders tables natively. `.markdownlint.json` exempts tables from the MD013 line-length rule. Three terms fit one small table. CommonMark/GFM has no true definition-list syntax. (Gate `uptrakit-sc3jc`, answer A.) |

## Non-goals / out of scope

- No code changes.
- No entry in `docs/README.md` or any other index page.
- No terms beyond the three named in the source bead.
- No `zola check` impact: `docs/superpowers/` is not symlinked into `website/content/docs/`.

## Documentation deliverables

- `docs/superpowers/sdd-fixture-glossary.md` — the deliverable itself (new page).
- No other doc impact: the page is self-contained, unlinked, and touches no existing document.

## Dependencies

- Cross-cycle predecessors: none. No open spec/plan epic touches `docs/superpowers/` glossary files.
- New external dependencies: none.

## Deferred

- Remove the fixture page and its spec cycle leftovers after the pipeline smoke run is validated —
  bead `uptrakit-def-remove-sdd-fixture-glossary`.

## Success criteria

- `docs/superpowers/sdd-fixture-glossary.md` exists and defines exactly three terms:
  convoy, workflow root, step bead — with the definition text fixed in "Chosen approach".
- The entries render as one GFM table.
- The page contains the marker sentence required by "Chosen approach" (throwaway fixture,
  not authoritative, pointer to `CONTEXT.md`).
- `markdownlint --config .markdownlint.json docs/superpowers/sdd-fixture-glossary.md` passes.
