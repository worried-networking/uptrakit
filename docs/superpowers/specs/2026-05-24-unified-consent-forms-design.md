# Unified Auth Consent Forms

**Date:** 2026-05-24
**Status:** Approved

## Problem

The OAuth consent form (`/oauth/consent/[request_id]`) is visually inconsistent with the device
auth form (`/device`):

- Uses `PageShell` (authenticated shell with sidebar/nav) instead of `PublicEntryShell`
- Five or more `SectionCard` blocks — verbose, tall, hard to scan
- Shows a typed redirect URI confirmation input that is friction with no security value
- Shows raw OAuth scope names (`mcp:read`) that are meaningless to users
- Approval prompt on device page is a bare `Callout` — no structured client display

## Goals

1. Both forms use `PublicEntryShell` — focused, distraction-free, consistent with the device flow
2. Shared `ConsentPrompt` component owns client display + trust signals + Approve/Deny buttons
3. OAuth consent form is compact — no `SectionCard` wrappers, minimal noise
4. Typed redirect URI confirmation removed entirely
5. All token/primitive rules from `docs/development/ui/` followed

## Out of Scope

- `DeviceLookup` type unchanged
- OTP input grid on device page unchanged

---

## Backend change

**File:** `crates/ui/web-api/src/routes/oauth/consent.rs`

Remove the `typed_confirmation` gate from the approve handler (lines ~273–286):

```rust
// DELETE this entire block:
if client.trusted_at.is_none() {
    let expected_confirmation = loopback_or_host(&row.redirect_uri);
    let provided = body
        .typed_confirmation
        .as_deref()
        .unwrap_or("")
        .to_lowercase();
    if provided != expected_confirmation {
        return oauth_400(
            "unverified_typed_confirmation_mismatch",
            "typed confirmation does not match redirect URI hostname",
        );
    }
}
```

The `danger` Callout in `ConsentPrompt` is the sole friction for unverified clients. A string-match
gate on a hostname the user typed provides no meaningful security — a user who would type the
hostname would also approve without the check.

`requires_typed_confirmation` and `typed_confirmation_value` fields in the GET response become
vestigial. Leave them in the response for now (no breaking API change); they can be pruned in
a follow-up cleanup. The frontend ignores both fields.

**Tests to update/remove:**

- `consent_approve_unverified_wrong_typed_confirmation` — delete (gate no longer exists)
- Any test asserting `"error": "unverified_typed_confirmation_mismatch"` — delete

---

## Architecture

### New component: `ConsentPrompt`

**File:** `frontend/src/lib/components/ConsentPrompt.svelte`

```ts
import type { Snippet } from 'svelte';  // required — do not use a local alias

export type ConsentPromptTrust =
  | 'verified'
  | 'unverified'
  | 'dcr'
  | 'open-metadata'
  | 'manual';

// Props (destructured via $props())
{
  clientName: string;
  clientUri?: string | null;
  trust: ConsentPromptTrust;
  approveDisabled?: boolean;
  approving: boolean;
  onApprove: () => void;
  onDeny: () => void;
  children?: Snippet;   // page-specific content between client header and buttons
}
```

**Rendered structure:**

```text
[client name — text-page-title font-bold text-[var(--text-primary)]]
[client_uri — inline-flex items-center gap-1 text-sm text-[var(--text-secondary)]]   ← omitted when null/undefined

[Callout tone="danger"  "Unverified client…"]    ← 'unverified' only
[Callout tone="warning" "Newly registered…"]     ← 'dcr' only
                                                 ← no callout for verified/manual/open-metadata

{@render children?.()}

[Button secondary "Deny"] [Button primary loading={approving} disabled={approveDisabled||approving} "Approve"]
```

**Trust callout messages:**

| Trust value  | Tone      | Message                                                            |
| ------------ | --------- | ------------------------------------------------------------------ |
| `unverified` | `danger`  | "This client has not been verified. Proceed only if you trust it." |
| `dcr`        | `warning` | "This client was recently registered and has not been reviewed."   |
| all others   | —         | no callout                                                         |

**Buttons:** Deny is `variant="secondary"` — no `loading` prop (disabled only when `approving`).
Approve is `variant="primary" loading={approving}`.
Both are `disabled` when `approving`. Approve is also `disabled` when `approveDisabled` is true.
Button row: `flex justify-end gap-2` — Deny left, Approve right.

**Client URI:** rendered with `ExternalLink` size 14, `target="_blank" rel="noopener noreferrer"`.

---

### OAuth consent page

**File:** `frontend/src/routes/oauth/consent/[request_id]/+page.svelte`

#### Shell

`PageShell` → `PublicEntryShell`. The `pageTitle` derived value drives the title slot:

```ts
const pageTitle = $derived(
  details ? `${details.client_name} wants access` : "Authorize Access",
);
```

```svelte
<PublicEntryShell eyebrow="Authorize Access" title={pageTitle}>
```

Footer slot: signed-in-as info (replaces the "Signed in as" `SectionCard`).
Uses `Link` from `$lib/components/Link.svelte` for consistent focus ring and color token:

```svelte
{#snippet footer()}
  <p class="text-sm text-[var(--text-secondary)]">
    Signed in as {getUser()?.email ?? ''}
    · <Link href="/login?_auth_context=oauth">Switch account</Link>
  </p>
{/snippet}
```

#### Trust derivation

`clientBadge()` helper removed. Replaced with:

```ts
function clientTrust(d: ConsentDetails): ConsentPromptTrust {
  if (d.trusted_at === null) return "unverified";
  if (d.created_via === "dcr") return "dcr";
  if (d.created_via === "cimd_cache") return "open-metadata";
  return "manual";
}
```

#### Loading state

While `details` is null (before `onMount` resolves), show a loading callout — the `ConsentPrompt`
block is guarded by `{:else if details !== null}`:

```svelte
{#if loadError}
  <Callout tone="danger" message={loadError} />
{:else if details !== null}
  <!-- ConsentPrompt rendered here -->
{:else}
  <Callout tone="info" message="Loading…" />
{/if}
```

#### `approveConsent` call site

`typedConfirmation` is removed. Always pass `null` as the second argument:

```ts
const resp = await approveConsent(requestId, null);
```

#### ConsentPrompt usage

```svelte
<ConsentPrompt
  clientName={details.client_name}
  clientUri={details.client_uri}
  trust={clientTrust(details)}
  approveDisabled={submitting}
  approving={submitting}
  onApprove={handleAllow}
  onDeny={handleDeny}
>
  <!-- Metadata change notice (conditional) -->
  {#if details.metadata_change_diff}
    <Callout
      tone="warning"
      message="This client's published metadata has changed since you last authorized it. Review before continuing."
    />
  {/if}

  <!-- Local redirect warning (conditional) -->
  {#if isLocalRedirect}
    <Callout tone="warning">
      <div class="flex items-center gap-2">
        <AlertTriangle size={16} aria-hidden="true" />
        <span>This client will receive credentials at a local address. Make sure it is running on this machine.</span>
      </div>
    </Callout>
  {/if}

  <!-- Scopes -->
  <ul class="space-y-1">
    {#each details.scopes as scope (scope)}
      <li class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">
        <CheckCircle size={14} class="mt-0.5 shrink-0 text-[var(--color-success)]" aria-hidden="true" />
        {scopeDescription(scope)}
      </li>
    {/each}
  </ul>

  <!-- Permissions note -->
  <Callout
    tone="info"
    message="{details.client_name} will act using your existing permissions — it cannot do anything you cannot already do."
  />
</ConsentPrompt>
```

#### State removed

- `typedConfirmation: string` state — removed
- `requires_typed_confirmation` branch in `allowDisabled` — removed
- `allowDisabled` renamed to inline `approveDisabled={submitting}` (the `!details` guard is covered by the `{:else if details !== null}` block)
- "Confirm redirect URI" `SectionCard` block — removed
- `clientBadge()` function and `ClientBadge` interface — removed
- `BadgeCheck` import — removed (replaced by no icon in client header)
- `StatusBadge` import — removed (was used for trust badge and scope names)
- All `SectionCard` imports and usages — removed

#### Redirect URI

- "Redirect URI" `SectionCard` removed entirely
- `isLocalRedirect` derived value kept — used only to conditionally render the Callout above
- `LOCAL_REDIRECT_HOSTS` const kept

---

### Device auth page

**File:** `frontend/src/routes/device/+page.svelte`

Only the `lookupPhase === 'done'` block changes.

**Before:**

```svelte
{#if lookup?.client_name}
  <Callout tone="info" title="Approve sign-in" message="Approve sign-in from {lookup.client_name}?…" />
{:else}
  <Callout tone="info" message="Your CLI is requesting access.…" />
{/if}
<div class="flex gap-3">
  <Button variant="primary" … onclick={onApprove}>Approve</Button>
  <Button variant="secondary" … onclick={onDeny}>Deny</Button>
</div>
```

**After:**

```svelte
<ConsentPrompt
  clientName={lookup?.client_name ?? 'CLI'}
  trust="verified"
  approving={processing}
  onApprove={onApprove}
  onDeny={onDeny}
/>
```

No children needed — device flow has no scope list. `actionError` Callout stays above `ConsentPrompt`
(already outside the replaced block).

**Button order change:** the existing device page has Approve left / Deny right. `ConsentPrompt`
standardises to Deny left / Approve right (primary action rightmost — consistent with industry
convention and the rest of the app's confirm/cancel button order).

**`trust="verified"` rationale:** device-flow clients are controller-internal — they are the
uptrakit CLI. No third-party DCR registration path exists for device clients. Hard-coding
`'verified'` is correct and intentional.

Shell, OTP input grid, and all other code unchanged.

---

## Forbidden patterns (enforced)

- No hardcoded hex/rgb — all colors via `var(--token-name)`
- No `preset-tonal-*` badge classes — use `StatusBadge`
- No `preset-filled-error-500` — use `Callout`
- `BadgeCheck` icon removed from consent page (was decorative, replaced by cleaner layout)

---

## Deferred follow-ups

- **Improve visibility and verification for unverified OAuth clients** — the `danger` Callout is
  the only friction after this spec ships. A follow-up should explore richer trust signals: e.g.
  an admin verification workflow, client registration review queue, or a stronger UI gate (require
  explicit checkbox or separate confirmation step) for unverified clients specifically. Out of scope
  here to avoid complexity creep, but should be tracked as a security UX improvement.

---

## Documentation deliverables

- `docs/development/ui/primitives.md` — add `ConsentPrompt` section after `ConfirmDialog`:
  - Props table with types
  - Trust value table with callout behavior
  - Usage examples for both call sites (consent + device)
- No ADR needed — not architecturally surprising, no contested tradeoff
- No `CONTEXT.md` changes — no new domain terms
