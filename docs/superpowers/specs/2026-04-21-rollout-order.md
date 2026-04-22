# UI Design-Language Rollout — Implementation Order

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

Canonical implementation order for the 19 sub-specs dated 2026-04-21. Derived from each sub-spec's `Blocks on` +
`Parallel-safe with` lines. Each wave lists specs whose prerequisites are satisfied by the end of the previous wave;
specs inside a wave can run in parallel.

## Wave 1 — Token foundation

| #   | Name           | File                                            |
| --- | -------------- | ----------------------------------------------- |
| #1  | tokens-adapter | `2026-04-21-tokens-adapter-migration-design.md` |

Blocks on: nothing. Ships `tokens.ts` adapter + `--bg-hover` token consumed downstream.

## Wave 2 — Base Button primitive

| #   | Name                         | File                                                |
| --- | ---------------------------- | --------------------------------------------------- |
| #2  | shared-button-terminal-theme | `2026-04-21-shared-button-terminal-theme-design.md` |

Blocks on: #1 PR1 + PR2 merged. Ships `<Button>` with `leadingIcon` + `trailingIcon` + `loading` + `href|onclick`
discriminated union.

## Wave 3 — Primitive extensions + Button-only consumer migrations

Runs in parallel after #2.

| #   | Name                                                                    | File                                                      | Blocks on      |
| --- | ----------------------------------------------------------------------- | --------------------------------------------------------- | -------------- |
| #2b | input-checkbox-link primitives                                          | `2026-04-21-input-checkbox-link-primitives-design.md`     | #1 PR2         |
| #2c | button-primitive-updates (a11y + disabled + `--bg-hover` + `ariaLabel`) | `2026-04-21-button-primitive-updates-design.md`           | #2             |
| #3a | public-entry Button                                                     | `2026-04-21-public-entry-button-migration-design.md`      | #2 PR1         |
| #3h | hosts Button migration                                                  | `2026-04-21-hosts-migration-design.md`                    | #2             |
| #3i | services + system-services                                              | `2026-04-21-services-system-services-migration-design.md` | #2, #2c (chip) |
| #3j | host-tags + audit + profile                                             | `2026-04-21-host-tags-audit-profile-migration-design.md`  | #2, #2c        |

Note: #3i and #3j list #2c as a prereq — they can start drafting before #2c merges, but final merge must wait for #2c.
If strict sequencing is desired, defer #3i + #3j to Wave 4.

## Wave 4 — After #2b + #2c

| #    | Name                    | File                                                   | Blocks on                  |
| ---- | ----------------------- | ------------------------------------------------------ | -------------------------- |
| #2d  | textarea primitive      | `2026-04-21-textarea-primitive-design.md`              | #1 PR2 (parallel with #2b) |
| #3a2 | public-entry forms      | `2026-04-21-public-entry-forms-migration-design.md`    | #2b, #3a                   |
| #3b  | layout shell + home     | `2026-04-21-layout-shell-home-migration-design.md`     | #2 PR1, #2c                |
| #3f  | software area           | `2026-04-21-software-area-migration-design.md`         | #2, #2c                    |
| #3k  | shared modals + dialogs | `2026-04-21-shared-modals-dialogs-migration-design.md` | #2, #2c                    |

Sub-spec #2d can technically start alongside #2b (both depend on #1 only), but its consumers are #3 sub-specs, so
slotting it here keeps the wave boundaries clean.

## Wave 5 — After #3b

| #   | Name                                               | File                                                 | Blocks on    |
| --- | -------------------------------------------------- | ---------------------------------------------------- | ------------ |
| #3c | settings shell + auth + registration + danger-zone | `2026-04-21-settings-shell-auth-migration-design.md` | #2, #2c, #3b |
| #3g | history                                            | `2026-04-21-history-migration-design.md`             | #2, #2c, #3b |

## Wave 6 — After #3c

| #   | Name                          | File                                                         | Blocks on    |
| --- | ----------------------------- | ------------------------------------------------------------ | ------------ |
| #3d | settings plugins + scheduler  | `2026-04-21-settings-plugins-scheduler-migration-design.md`  | #2, #2c, #3c |
| #3e | settings notifications + OIDC | `2026-04-21-settings-notifications-oidc-migration-design.md` | #2, #2c, #3c |

## Wave 7 — Surface layer parity

| #   | Name                 | File                                        | Blocks on              |
| --- | -------------------- | ------------------------------------------- | ---------------------- |
| #4  | surface-layer parity | `2026-04-21-surface-layer-parity-design.md` | #2, #2b, #2c, #2d, #3k |

Sub-spec #4 is the first consumer of the full primitive set (Button + Input + Textarea + Checkbox) plus the
`confirmVariant` rename shipped by #3k.

## Wave 8 — Final consolidation

| #   | Name             | File                                    | Blocks on                |
| --- | ---------------- | --------------------------------------- | ------------------------ |
| #5  | fixture backfill | `2026-04-21-fixture-backfill-design.md` | every preceding sub-spec |

Platform pin: regeneration must run on macOS + Chromium per `frontend/playwright.config.ts` parity-suite guard.

## Critical path

Longest chain (min waves any one PR must wait through):

`#1 → #2 → #2c → #3b → #3c → #3d|#3e → #5`

Seven sequential merges. Everything else parallelizes off that spine.

## Notes

- "Blocks on #2" always means #2 PR1 unless the entry names PR2.
- "#3a–k" in some specs' `Parallel-safe with` list is shorthand; individual dependencies above supersede the shorthand.
- Form-input deferred passes (#3c2, #3e2, #3g2, etc.) are not in scope for this rollout — they follow after #2b + #2d
  land and are tracked as their own sub-specs when scheduled.
