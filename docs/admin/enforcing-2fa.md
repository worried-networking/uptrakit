# Enforcing 2FA for All Operators

Administrators can require every password-authenticated user in a tenant to enroll in 2FA
before accessing the application.

## Enable enforcement

1. Go to **Settings → Authentication**.
2. Toggle **Require two-factor authentication** to on.
3. Click **Save**.

Once enabled, users who log in without 2FA enrolled will receive a restricted session. They
can only access the 2FA enrollment flow (`/api/v1/auth/me/2fa/*`). All other routes return
`403 { "error": "2fa_setup_required" }` until enrollment is completed.

## Important prerequisite

The enforcement toggle requires the refresh token handler to re-check enrollment state.
This check is included in the implementation — do not expose the toggle in the UI until
the refresh handler check is deployed. See the spec for details.

## OIDC users

OIDC-authenticated users are not affected by this setting. 2FA for OIDC is handled by the
identity provider.
