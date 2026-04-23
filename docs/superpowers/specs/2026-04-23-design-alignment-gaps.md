# Design Alignment Gaps — Audit Report

**Date:** 2026-04-23
**Audited against:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` +
`docs/development/ui-design-language.md`
**Method:** 10 parallel subagent sweep of entire `frontend/src/` tree post Wave-7 merge.

Gaps organised into four categories. Each category has a corresponding sub-spec in this
directory.

---

## Category A — Old Skeleton preset classes still in production

Sub-spec: `2026-04-23-token-migration-design.md`

| File | Line | Violation |
| --- | --- | --- |
| `lib/components/ToastNotifications.svelte` | 388 | `<a class="btn btn-sm preset-tonal">` |
| `lib/components/BatchActionBar.svelte` | 119 | Raw `<button>` with Skeleton hover tokens |
| `lib/components/AssignToHostModal.svelte` | 270 | `preset-filled-error-500` on `<aside>` |
| `lib/components/AssignToHostModal.svelte` | 274 | `preset-tonal-surface` on `<aside>` |
| `lib/components/EditHostAssignmentModal.svelte` | 685 | `preset-filled-error-500` on `<aside>` |
| `lib/components/EditHostAssignmentModal.svelte` | 699, 1015 | `badge preset-tonal` |
| `lib/components/EditHostAssignmentModal.svelte` | 838, 965, 1157 | `preset-filled-error-500` on error paragraphs |
| `lib/components/EditHostAssignmentModal.svelte` | 883, 1205 | `badge preset-tonal-warning` |
| `lib/components/SoftwareMergeWizard.svelte` | 323 | `badge preset-tonal-surface` |
| `lib/components/SoftwareMergeWizard.svelte` | 324, 361 | `badge preset-filled-primary-500` |
| `lib/components/SoftwareMergeWizard.svelte` | 349 | `card preset-tonal-primary` |
| `lib/components/SoftwareMergeWizard.svelte` | 374 | `badge preset-tonal-error` |
| `lib/components/surfaces/SurfaceWorkflow.svelte` | 420 | `card ... preset-tonal-surface` |
| `routes/hosts/[id]/+page.svelte` | 584 | `<a class="btn btn-sm preset-tonal">` |
| `routes/hosts/[id]/+page.svelte` | 624, 643 | `preset-tonal-surface`, `preset-tonal` |
| `routes/software/[id]/+page.svelte` | 848 | `badge preset-tonal` |

---

## Category B — Skeleton surface/primary color tokens (not design tokens)

Sub-spec: `2026-04-23-token-migration-design.md` (same spec as A)

| File | Issue |
| --- | --- |
| `lib/components/Modal.svelte:22` | `bg-surface-50 dark:bg-surface-900` |
| `lib/components/CheckboxList.svelte:34,38,52,59` | `rounded-container-token`, `border-surface-*-token`, `hover:bg-surface-*-token`, `text-surface-500` |
| `lib/components/BatchActionBar.svelte:105,110,112,155` | `bg-surface-*`, `border-surface-*`, `border-t-primary-500`, `text-surface-500` |
| `lib/components/BatchResultDialog.svelte:21,29,37,38,39` | `text-success-500`, `text-error-500`, `bg-surface-100/800`, `text-surface-500` |
| `lib/components/AddSoftwareModal.svelte:59` | `text-surface-500` |
| `lib/components/AssignToHostModal.svelte:261,375,501` | `text-surface-500`, `text-surface-400` |
| `lib/components/EditHostAssignmentModal.svelte` | `text-surface-*`, `bg-surface-*`, `border-surface-*` (multiple) |
| `lib/components/SoftwareMergeWizard.svelte` | `bg-primary-*`, `text-primary-*`, `bg-surface-*`, `text-surface-*` |
| `lib/components/surfaces/SurfaceKeyValue.svelte:16,20` | `text-surface-500`, `divide-surface-200/700` |
| `lib/components/surfaces/SurfaceWorkflow.svelte` | `bg-primary-*`, `text-primary-*`, `border-primary-500`, `text-surface-*` |
| `routes/+page.svelte:161` | `text-surface-500` |
| `routes/surfaces/[id]/+page.svelte:54,62` | `text-surface-500` |
| `routes/history/+page.svelte:682,711` | `bg-surface-50/900`, `text-error-500` |
| `routes/audit-logs/+page.svelte:216` | `text-surface-500` |
| `routes/profile/+page.svelte:182` | `bg-surface-100/800` |
| `routes/hosts/+page.svelte:437,563` | `text-surface-400`, `text-error-500` |
| `routes/host-tags/+page.svelte:480` | `text-surface-400` |
| `routes/settings/GlobalSettingsTab.svelte` | `text-surface-*`, `bg-surface-100-900`, `preset-filled-warning-500`, `preset-filled-surface-400-600` |
| `routes/settings/PluginConfigsTab.svelte:1276,1288` | `text-surface-500`, `text-error-500` |
| `routes/settings/SchedulerTab.svelte:124,141` | `text-surface-500`, `text-error-500` |
| `routes/settings/SystemServicesSettings.svelte:200` | `text-surface-600 dark:text-surface-400` |
| `routes/settings/+page.svelte:250` | `text-surface-600` |

---

## Category C — Raw form elements where spec primitives exist

Sub-spec: `2026-04-23-form-primitive-adoption-design.md`

| File | Line(s) | Violation |
| --- | --- | --- |
| `lib/components/CheckboxList.svelte` | 40 | Raw `<input type="checkbox">` |
| `lib/components/AddSoftwareModal.svelte` | 67, 86 | Raw `<input>` |
| `lib/components/AddSoftwareModal.svelte` | 99 | Raw `<input type="checkbox">` |
| `lib/components/AssignToHostModal.svelte` | 302 | Raw `<input type="checkbox">` |
| `lib/components/AssignToHostModal.svelte` | 339, 465 | Raw `<input>` |
| `lib/components/EditHostAssignmentModal.svelte` | 738, 803 | Raw `<input>` |
| `lib/components/EditHostAssignmentModal.svelte` | 770, 891, 1090, 1220 | Raw `<textarea>` |
| `lib/components/EditHostAssignmentModal.svelte` | 789, 915, 1106 | Raw `<input type="checkbox">` |
| `lib/components/SoftwareMergeWizard.svelte` | 261 | Raw `<input type="search">` |
| `lib/components/surfaces/SurfaceForm.svelte` | 137 | Raw `<textarea>` |
| `routes/settings/AuthenticationSettings.svelte` | 50 | Raw `<input type="checkbox">` |
| `routes/settings/AgentCertificateSettings.svelte` | 69, 77 | Raw checkbox + raw input |
| `routes/settings/EnrollmentTokenSettings.svelte` | 246 | Raw `<input type="checkbox">` |
| `routes/settings/NotificationRulesSettings.svelte` | 284 | Raw `<input type="checkbox">` |
| `routes/settings/OidcProvidersSettings.svelte` | 336, 348 | Raw `<input type="checkbox">` |
| `routes/settings/RegistrationSettings.svelte` | 90 | Raw `<input type="checkbox">` |
| `routes/software/+page.svelte` | 889, 940, 972, 1499 | Raw `<input type="checkbox">` |
| `routes/software/+page.svelte` | 1487 | Raw `<input type="text">` |
| `routes/software/[id]/+page.svelte` | 1119 | Raw `<input type="text">` |
| `routes/software/[id]/+page.svelte` | 1129 | Raw `<input type="checkbox">` |
| `routes/software/IgnoreRulesTab.svelte` | 162, 184 | Raw `<input type="checkbox">` |
| `routes/software/IgnoreRulesTab.svelte` | 249 | Raw `<input type="text">` |
| `routes/services/+page.svelte` | 688 | Raw `<input>` |
| `routes/system-services/+page.svelte` | 661 | Raw `<input>` |

---

## Category D — Primitive component spec deviations

Sub-spec: `2026-04-23-primitive-conformance-design.md`

| File | Line | Issue |
| --- | --- | --- |
| `lib/components/ui/Callout.svelte` | 28 | `rounded-xl` violates §2.3 panel radius |
| `lib/components/ui/EmptyState.svelte` | 16 | `rounded-2xl` violates §2.3 card radius |
| `lib/components/ui/SectionCard.svelte` | 17 | `rounded-2xl` violates §2.3 card radius |
| `lib/components/ui/PageShell.svelte` | 25 | `text-3xl` violates §2.4 h1 typography |
| `lib/components/ui/TabStrip.svelte` | 101 | Outer container `rounded-xl` unsanctioned |
| `lib/components/ui/TabStrip.svelte` | 111 | Tab buttons `rounded-lg` (8px) — spec §4.11 = 3px |
| `lib/components/ui/TabStrip.svelte` | 113–114 | Active state solid `--accent` fill — spec §4.11 = tint |
| `lib/components/ui/DataTable.svelte` | 76 | `px-4` header padding — spec §4.12 = 10px |
| `lib/components/ui/ProviderSelector.svelte` | 55 | `rounded-xl` on select — spec §4.10 = 3px |
| `lib/components/surfaces/SurfaceSlot.svelte` | 38, 40 | Skeleton `card` utility + `h3` typography class |
| `lib/components/surfaces/SurfaceRenderer.svelte` | 89 | Skeleton `h3` typography class |
| `lib/components/surfaces/SurfaceWorkflow.svelte` | 471 | `border-primary-500` spinner — use `border-[var(--accent)]` |
| `lib/components/SoftwareMergeWizard.svelte` | 236, 357–405 | Skeleton `h4` typography classes |
| `lib/components/Modal.svelte` | 30 | Skeleton `h3` typography class |

---

## Not violations (false positives filtered)

- `--bg-hover` — valid design token from sub-spec #1 (tokens adapter)
- Raw `<select>` — no `<Select>` primitive in spec scope
- Raw `<input type="radio">` — no Radio primitive in spec scope
- `rounded-2xl` on `device/+page.svelte` code block — device enrollment display, not a card
- `<label>` elements wrapping `<Checkbox>` — valid associated HTML
