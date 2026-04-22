# UI Design-Language Rollout — Implementation Order

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

Canonical implementation order for the 19 sub-specs dated 2026-04-21. Derived from each sub-spec's `Blocks on` +
`Parallel-safe with` lines. Each wave lists specs whose prerequisites are satisfied by the end of the previous wave;
specs inside a wave can run in parallel.

Waves are **strictly** dependency-ordered — a spec appears in the earliest wave where all its prerequisites are met.

## Wave 1 — Token foundation ✅ Shipped

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #1 | tokens-adapter | `2026-04-21-tokens-adapter-migration-design.md` | — |

Ships `tokens.ts` adapter + `--bg-hover` token consumed downstream.

## Wave 2 — Base Button primitive ✅ Shipped

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #2 | shared-button-terminal-theme | `2026-04-21-shared-button-terminal-theme-design.md` | #1 PR1 + PR2 |

Ships `<Button>` (primary/ghost/danger, `leadingIcon`, `trailingIcon`, `loading`, `href|onclick` discriminated union)
and `<UpdateAllButton>` with standalone `<button>` + `ariaLabel`.

## Wave 3 — Primitive extensions + first consumer migrations (active)

All run in parallel. All prerequisites are satisfied by the end of Wave 2.

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #2b | input-checkbox-link primitives | `2026-04-21-input-checkbox-link-primitives-design.md` | #1 PR2 |
| #2c | button-primitive-updates (`secondary` variant + `ariaLabel` prop + `--bg-hover` token) | `2026-04-21-button-primitive-updates-design.md` | #2 |
| #2d | textarea primitive | `2026-04-21-textarea-primitive-design.md` | #1 PR2 |
| #3a | public-entry Button migration | `2026-04-21-public-entry-button-migration-design.md` | #2 PR1 |

`#2b` and `#2d` technically only block on `#1 PR2` (not `#2`), so they could have shipped in Wave 2. They are
grouped here because their consumers land in Wave 4 and they do not block any other Wave-3 work.

## Wave 4 — After #2b, #2c, #2d, and #3a

All run in parallel.

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #3a2 | public-entry forms migration | `2026-04-21-public-entry-forms-migration-design.md` | #2b, #3a |
| #3b | layout shell + home migration | `2026-04-21-layout-shell-home-migration-design.md` | #2 PR1, #2c |
| #3f | software area migration | `2026-04-21-software-area-migration-design.md` | #2, #2c |
| #3i | services + system-services Button migration | `2026-04-21-services-system-services-migration-design.md` | #2, #2c |
| #3j | host-tags + audit + profile Button migration | `2026-04-21-host-tags-audit-profile-migration-design.md` | #2, #2c |
| #3k | shared modals + dialogs migration | `2026-04-21-shared-modals-dialogs-migration-design.md` | #2, #2c |

`#3a2` additionally needs `#3a` (Wave 3). `#3b` establishes the layout baseline required by Wave 5 migrations.

## Wave 5 — After #3b and #3k

All run in parallel.

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #3c | settings shell + auth + registration + danger-zone | `2026-04-21-settings-shell-auth-migration-design.md` | #2, #2c, #3b |
| #3g | history migration | `2026-04-21-history-migration-design.md` | #2, #2c, #3b |
| #3h | hosts Button migration | `2026-04-21-hosts-migration-design.md` | #2, #2c, #3b |
| #4 | surface-layer parity | `2026-04-21-surface-layer-parity-design.md` | #2, #2b, #2c, #2d, #3k |

`#3h` uses `variant="secondary"` (from `#2c`) for reversible actions and `ariaLabel` (also from `#2c`) for the
icon-only context-menu trigger. It also requires the `#3b` layout baseline.

`#4` is the first consumer of the full primitive set (Button + Input + Checkbox + Link + Textarea) plus the
`confirmVariant` rename shipped by `#3k`. Its latest prerequisite is `#3k` (Wave 4).

## Wave 6 — After #3c

All run in parallel.

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #3d | settings plugins + scheduler | `2026-04-21-settings-plugins-scheduler-migration-design.md` | #2, #2c, #3c |
| #3e | settings notifications + OIDC | `2026-04-21-settings-notifications-oidc-migration-design.md` | #2, #2c, #3c |

## Wave 7 — Final consolidation

| # | Name | File | Blocks on |
| --- | --- | --- | --- |
| #5 | fixture backfill | `2026-04-21-fixture-backfill-design.md` | every preceding sub-spec |

Platform pin: regeneration must run on macOS + Chromium per `frontend/playwright.config.ts` parity-suite guard.

## Critical path

Longest chain (minimum waves any single PR must wait through):

`#1 → #2 → #2c → #3b → #3c → #3d|#3e → #5`

Seven sequential merges across seven waves. Everything else parallelizes off that spine.

`#3h` is a Wave-5 leaf (parallel with `#3c`/`#3g`/`#4`) — it does not extend the critical path.

## Dependency index

Quick reference — what each sub-spec waits for before its PR can merge:

| # | Blocks on (explicit) |
| --- | --- |
| #1 | — |
| #2 | #1 PR1 + PR2 |
| #2b | #1 PR2 |
| #2c | #2 |
| #2d | #1 PR2 |
| #3a | #2 PR1 |
| #3a2 | #2b, #3a |
| #3b | #2 PR1, #2c |
| #3c | #2, #2c, #3b |
| #3d | #2, #2c, #3c |
| #3e | #2, #2c, #3c |
| #3f | #2, #2c |
| #3g | #2, #2c, #3b |
| #3h | #2, #2c, #3b |
| #3i | #2, #2c |
| #3j | #2, #2c |
| #3k | #2, #2c |
| #4 | #2, #2b, #2c, #2d, #3k |
| #5 | all preceding |

## Notes

- "#2 PR1" means the first PR of sub-spec #2 (Button + UpdateAllButton primitives). "#2 PR2" means the second PR
  (canary consumer migration). "Blocks on #2" without qualification means PR1.
- Deferred form-input passes (#3c2, #3e2, #3g2, #3h2, etc.) are not in scope — they follow after #2b + #2d land
  and are tracked as separate sub-specs when scheduled.
