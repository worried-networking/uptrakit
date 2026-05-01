# Profile Page Redesign

**Date:** 2026-05-01
**Status:** Approved

## Scope

Redesign `frontend/src/routes/profile/+page.svelte` for design-language alignment:
introduce a `TabStrip`, move inline email/password forms into modals, hide revoked tokens,
add `EmptyState` for the tokens tab, wire inline copy affordance, and add `lucide-svelte`
as the project icon library. Update `profile.test.ts` to match.

## Decisions

### 1. Tab Structure

Replace the single-scroll card stack with a `TabStrip` containing two tabs:

| id           | Label      | Default |
| ------------ | ---------- | ------- |
| `account`    | Account    | Yes     |
| `api-tokens` | API Tokens | No      |

Tab state syncs to `?tab=<id>` query param using `replaceState: true`, `keepFocus: true`,
`noScroll: true` — identical to `settings/+page.svelte`. The default tab (`account`) is
omitted from the URL (empty search string maps to `account`).

`PageShell` keeps `title="Profile"`. The `description` prop is dropped — tabs make the
structure self-evident.

### 2. Account Tab — Profile Card

`SectionCard title="Profile"`:

- `FormFieldRow label="First name"` + `Input` (editable, bound to `firstName`)
- `FormFieldRow label="Last name"` + `Input` (editable, bound to `lastName`)
- `FormFieldRow label="Email"`:
  - Disabled `Input` showing `user.email`
  - (password auth only) `Button variant="secondary" size="sm"` "Change email" — opens change-email modal
  - (password auth only, when `user.has_pending_email_change`) `StatusBadge tone="warning" label="Change pending"` rendered inline after the button
- Error `Callout tone="danger"` when `profileError` is set
- `<div class="flex justify-end">` → `Button variant="primary" loading={profileSaving}` "Save"

### 3. Account Tab — Change Email Modal

Triggered by the "Change email" button. `ModalShell title="Change Email" maxWidth="max-w-lg"`.

**Pending-change state** (`user.has_pending_email_change || emailChangeSuccess`):

- `Callout tone="info"` — "A confirmation email has been sent. Check your inbox. If you did
  not request this change, you can cancel it."
- Footer: `Button variant="ghost"` "Cancel email change" (calls `handleCancelEmailChange`) +
  `Button variant="primary"` "Close" (closes modal)

**Not-pending state** (default form):

- `FormFieldRow label="New email address"` + `Input type="email"` bound to `newEmail`
- `FormFieldRow label="Current password"` + `Input type="password"` bound to `emailCurrentPassword`
- Error `Callout tone="danger"` when `emailError` is set
- Footer: `Button variant="ghost"` "Cancel" (closes modal) + `Button variant="primary"
loading={emailChanging}` "Send confirmation email"

Success path: `emailChangeSuccess = true` flips the modal to pending-change state (shows the
info Callout). No auto-close — user explicitly closes. Closing the modal (Cancel or Close)
resets `emailChangeSuccess = false` and `showChangeEmailModal = false`.

`handleCancelEmailChange` calls `cancelEmailChange`, then `showSuccess`, then `initialize()`,
then closes modal.

### 4. Account Tab — Security Card

`SectionCard title="Security"`:

**Password auth (`authMethod === 'password'`)**:

- `FormFieldRow label="Password"` with a static `<span class="text-sm
text-[var(--text-secondary)]">••••••••</span>` and `Button variant="secondary" size="sm"`
  "Change" in the field slot — opens change-password modal

**OIDC auth (all other `authMethod` values)**:

- `Callout tone="info"` — "Your account uses single sign-on. Password and email are managed
  by your identity provider."

### 5. Account Tab — Change Password Modal

Triggered by the "Change" button in the Security card. `ModalShell title="Change Password"
maxWidth="max-w-lg"`.

**Success state** (`passwordChangeSuccess`): close modal + `showSuccess('Password changed.
Other sessions have been signed out.')`.

**Form state** (default):

- `FormFieldRow label="Current password" inputId="pw-current"` + `Input type="password"`
- `FormFieldRow label="New password" inputId="pw-new" hint="8–128 characters."` +
  `Input type="password"`
- `FormFieldRow label="Confirm new password" inputId="pw-confirm"` + `Input type="password"` +
  `error` prop wired to `confirmPasswordError` (passwords-don't-match check)
- Error `Callout tone="danger"` when `passwordError` is set
- Footer: `Button variant="ghost"` "Cancel" (closes modal) + `Button variant="primary"
loading={passwordSaving}` "Change password"

`confirmPasswordError` uses `FormFieldRow`'s `error` prop throughout. The raw `<p
class="text-sm text-(--color-danger)">` pattern is removed.

On success: close modal, reset all password fields and error state, call
`showSuccess('Password changed. Other sessions have been signed out.')`.

### 6. API Tokens Tab

Single `SectionCard`:

- `title="API Tokens"`
- `description="API tokens allow programmatic access to Uptrakit. Treat tokens like
passwords and rotate them regularly."`
- `{#snippet actions()}` → `Button variant="primary"` "New Token" (opens create modal)

**Token filtering:** active tokens only. Revoked tokens (`revoked_at !== null`) are filtered
client-side before passing to `DataTable`. The Status column is removed — all displayed tokens
are active.

**DataTable columns:** Name, Created, Actions (Revoke button).

**Empty state:** when the filtered token list is empty, `EmptyState title="No API tokens"
description="Create a token to access Uptrakit programmatically."` with `{#snippet actions()}`
→ `Button variant="ghost"` "New Token" (same handler as header button).

**Revoke:** existing `ConfirmDialog` flow unchanged.

### 7. Create Token Modal

`ModalShell title="New API Token" maxWidth="max-w-lg"`. Two internal states:

**Pre-reveal (name entry):**

- `FormFieldRow label="Token Name" inputId="new-token-name"` + `Input` with Enter-key handler
- Footer: `Button variant="secondary"` "Cancel" + `Button variant="primary"
loading={creating} disabled={!newTokenName.trim()}` "Create"

**Post-reveal (token display):**

- `Callout tone="warning" title="Save this token now" message="It will not be shown again
after you close this dialog."`
- `<div class="relative">` wrapping:
  - `<pre class="rounded-panel bg-[var(--bg-raised)] p-3 font-mono text-sm break-all
whitespace-pre-wrap">{createdToken}</pre>`
  - `<div class="absolute top-2 right-2">` → `Button variant="ghost" size="sm"
ariaLabel="Copy token" onclick={() => copyToken(createdToken!)}` containing
    `<Copy size={14} />` from lucide-svelte
- Footer: `Button variant="secondary"` "Copy" (calls `copyToken`) + `Button variant="primary"`
  "Done" (calls `closeCreateModal`)

### 8. Icon Library

Add `lucide-svelte` to `frontend/package.json` dependencies.

Import pattern:

```typescript
import { Copy } from "lucide-svelte";
```

Used in the token reveal modal. Available for all future icon needs across the frontend.

### 9. Test Updates (`profile.test.ts`)

Remove tests tied to the old inline-form structure:

- Inline change-email form rendering
- Inline change-password form rendering
- Separate "Change email" / "Change password" section card assertions
- Status column in token table

Add tests:

- Both tabs present in `TabStrip`; "Account" tab active by default
- Tab switching changes active tab
- `StatusBadge "Change pending"` visible when `has_pending_email_change: true`
- "Change email" button present (password auth); absent (OIDC)
- Change email modal opens on button click; closes on Cancel
- Change password modal opens on "Change" button click; closes on Cancel
- SSO `Callout` rendered for non-password auth method
- Revoked tokens filtered out of DataTable
- `EmptyState` rendered when all tokens are revoked (active list empty)
- Inline copy icon button present in post-reveal modal state
- Footer "Copy" button present in post-reveal modal state

Keep:

- Clipboard copy tests
- Revoke `ConfirmDialog` launch and confirm flow
- Token creation name-entry form
- Profile save button and form field tests

## Out of Scope

- Token expiry dates or scopes.
- Profile avatar upload.
- Session management (list/revoke active sessions).
- Any backend or API changes.
