# Textarea Primitive — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.10 Form Validation, §2.6
Interaction)

**Sub-spec #2d of the UI design-language rollout.** Depends on sub-spec #1 (tokens + adapter). Ships alongside #2b
primitive family. Consumer migrations land in sub-specs #3d (PluginConfigsTab), #3e (OidcProvidersSettings), plus #3j
(host-tags), #3k (EditHostAssignmentModal), #4 (SchemaForm, SurfaceForm), plus any pass over GlobalSettingsTab's
textarea sites.

## Overview

Introduce `Textarea.svelte` primitive alongside the `Input` / `Checkbox` / `Link` primitives from #2b. Sub-spec #2b
deferred textarea as "niche" based on one known consumer (plugin config editor). Actual consumer count during #3
drafting: 7 files, 75+ textarea sites. Textarea is no longer niche.

## Design decisions

**Q1 — API shape vs Input discriminated union.**

- Options:
  - (chosen) Standalone `Textarea.svelte` primitive. Input primitive stays scoped to single-line element types from #2b.
    Textarea owns its own API because multiline-specific props (rows, cols, resize policy) don't fit the Input union
    cleanly.
  - Extend `InputType` union with `'textarea'`. Rejected — structural mismatch: `<input>` and `<textarea>` are different
    DOM elements with different reserved attributes (e.g. `value` is attribute on input, child text on textarea); union
    would leak exceptions.
- Reasoning: DOM shape dictates primitive boundary; different element tags want different primitives.

**Q2 — Auto-resize behavior.**

- Options:
  - (chosen) No auto-resize. `rows` prop sets initial height; user resizes via the native `resize-y` affordance (allowed
    unless disabled). Matches native browser behavior consumers already expect.
  - Ship auto-resize-on-content. Rejected — adds layout churn during typing; not part of parent spec.
  - Disable resize entirely. Rejected — loses accessibility (users with longer content want to expand).
- Reasoning: defer auto-resize to a future version; native `resize-y` is sufficient.

**Q3 — Monospace variant for config/secret editing.**

- Options:
  - (chosen) `variant?: 'default' | 'mono' = 'default'`. Mono variant applies `font-mono` class for plugin config JSON,
    shell commands, etc. Two known consumer classes in PluginConfigsTab want this.
  - Boolean `mono?: boolean`. Rejected — leaves room for future variants cleanly; boolean locks the API.
  - Always default (no monospace). Rejected — plugin config field readability regresses without mono.
- Reasoning: variant enum scales better than boolean when a third variant emerges (e.g. "compact" for narrow layouts).

**Q4 — Validation API: mirror Input's `error` prop.**

- Options:
  - (chosen) `error?: string` prop; same semantics as Input — non-empty triggers `aria-invalid="true"` + error border +
    error bg. Consumer pairs with `FormFieldRow` error copy.
  - Separate validation shape. Rejected — consumers think about validation uniformly; mirror Input.
- Reasoning: API consistency across form primitives reduces consumer cognitive load.

**Q5 — `cols` prop omission.**

- Options:
  - (chosen) No `cols` prop. Width is owned by the enclosing layout via `w-full` on the base class contract; `cols` is
    redundant when the textarea stretches to its container.
  - Add `cols?: number`. Rejected — noise at every call site (CSS already controls width) and encourages inconsistent
    widths across forms.
- Reasoning: layout-driven width > attribute-driven width; matches every known consumer.

**Q6 — `onchange` event omission.**

- Options:
  - (chosen) Expose `oninput` + `onblur` only. The `change` event fires on blur-after-modify for `<textarea>`; `onblur`
    covers the same consumer needs (dirty-tracking, validation-on-leave) with finer control.
  - Add `onchange?: (e: Event) => void`. Rejected — redundant given `onblur`; adds a prop whose semantics confuse
    consumers who expect input-style "fires every keystroke" change events.
- Reasoning: if a future consumer genuinely needs `change`, add the prop additively then; YAGNI today.

**Q7 — Location in `$lib/components/`.**

- Options:
  - (chosen) `frontend/src/lib/components/Textarea.svelte` — peer of Input / Checkbox / Link. Consistent with #2b
    placement decision.
  - Nest under a `form/` subfolder. Rejected — sub-spec #2b already chose flat layout for primitives.
- Reasoning: consistency with #2b.

## Goals

1. Ship `<Textarea>` primitive implementing §4.10 field conventions + §2.6 focus-ring rules.
2. Provide `variant: 'default' | 'mono'` for monospace use cases.
3. Encode error state as `error?: string` prop mirroring Input.
4. Add a Textarea section to the `/dev/form-primitive-preview` route (created in #2b) for Playwright snapshot coverage.
   No new route.

## Non-goals

- Rich-text / markdown editor — separate concern; no primitive needed until a consumer requests it.
- Auto-resize on content — deferred.
- Code-editor features (syntax highlight, line numbers) — PluginConfigsTab uses CodeMirror elsewhere; Textarea is
  plain-text only.
- Consumer migration of existing sites — per-consumer sub-specs own that (#3d, #3e, #3j, #3k, #4).

## Components

### Textarea primitive

**Files:**

- `frontend/src/lib/components/Textarea.svelte`
- `frontend/src/lib/components/Textarea.test.ts`

**API (Svelte 5 runes):**

```ts
export type TextareaVariant = "default" | "mono";

export type TextareaProps = {
  id: string;
  value: string;
  name?: string;
  placeholder?: string;
  rows?: number;
  disabled?: boolean;
  required?: boolean;
  error?: string;
  variant?: TextareaVariant;
  oninput?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  class?: string;
};
```

Note: `value` is bindable via `bind:value` — implementation declares it with `$bindable()` at `$props()` destructure
time, matching #2b Input: `let { value = $bindable(''), ... }: TextareaProps = $props();`.

`aria-describedby` wiring follows the same contract as #2b Input (Q3): the enclosing `FormFieldRow` owns the error-copy
node and its stable `id`, and sets `aria-describedby` on the nested `<Textarea>` via the class/attrs passthrough.
`<Textarea>` itself does not render error copy or emit an internal describedby id. Callers rendering a textarea outside
a `FormFieldRow` must render their own `<p id="...">` and pass `aria-describedby` through.

**Class contract:**

Base (all variants):

```text
w-full rounded-[3px] p-2
bg-[var(--bg-surface)] border border-[var(--border-default)]
text-sm text-[var(--text-primary)]
placeholder:text-[var(--text-muted)]
focus-visible:outline-none
focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]
disabled:opacity-40 disabled:cursor-not-allowed
aria-[invalid=true]:border-[var(--color-error-border)]
aria-[invalid=true]:bg-[var(--color-error-bg)]
transition-[background,border-color] duration-[0.12s]
resize-y min-h-[4rem]
```

`default` variant adds no extra class.

`mono` variant adds:

```text
font-mono text-[13px]
```

When `rows` is omitted, primitive renders without the `rows` attribute; browser default applies (typically 2). Most
consumers pass `rows={4}` or higher.

## Data flow

Build-time only. No runtime state.

## Error handling

- `error` prop non-empty → `aria-invalid="true"` automatically.
- TS prop typing rejects invalid `variant` values at compile time.

## Testing

### Unit tests

`Textarea.test.ts`:

- Variant snapshot matrix: `default` × `mono` × `error | no-error` × `disabled | enabled` (8 permutations); mono renders
  `font-mono`; error state triggers `aria-invalid="true"` + error border / bg class fragments; disabled renders
  `disabled` attribute + `opacity-40` (class-presence assertion, jsdom does not resolve `var(--*)`).
- `bind:value` round-trip; `oninput` / `onblur` callbacks fire with correct event types.
- `rows` prop forwards to DOM attribute; `required` prop forwards to DOM attribute.
- `aria-describedby` passthrough: when supplied, renders on the underlying `<textarea>`; when omitted, attribute absent.

### Integration / e2e

Extend `/dev/form-primitive-preview` route (from #2b) with a Textarea section:

- Default variant × error toggle.
- Mono variant × error toggle.
- Disabled state.

Playwright snapshot gate covers the new section in both themes.

## Rollout

Single PR, pure addition, titled "feat(frontend): add Textarea primitive (sub-spec #2d)".

1. Add `frontend/src/lib/components/Textarea.svelte`.
2. Add `frontend/src/lib/components/Textarea.test.ts`.
3. Extend `/dev/form-primitive-preview` +page.svelte with Textarea section.
4. Extend `frontend/tests/e2e/form-primitive.spec.ts` with Textarea coverage.
5. Re-baseline preview route snapshot.
6. Full frontend gate.

### Risk + rollback

Reverting one PR removes two new files plus the preview-route section. No consumers touched — zero downstream
regression.

### Dependencies + ordering

- **Blocks on:** sub-spec #1 PR2 merged.
- **Blocks:** #3d2 / #3e2 / #3j2 / #3k2 form-input migration waves (textarea sites specifically); #4 SchemaForm textarea
  dispatch.
- **Parallel-safe with:** sub-spec #2b, #2c, #3a, all #3 Button-only migration waves.
