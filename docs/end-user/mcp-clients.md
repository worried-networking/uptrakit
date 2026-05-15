# Connecting MCP Clients to the Controller

This guide explains how to connect browser-capable MCP clients (Claude Desktop, Cursor, MCP Inspector)
to a controller using OAuth 2.1. The controller issues access tokens directly — you sign in with your
existing account.

## Prerequisites

Before connecting an MCP client:

- The controller must be running with OAuth enabled (`oauth.mcp_enabled = true`). If you are not sure,
  ask your administrator.
- Your user account must exist on the controller. If you do not have an account, ask your administrator
  to create one for you.
- Your account must have at least the `AccessMcp` permission. Ask your administrator if you are
  unsure.

## Connect Claude Desktop

1. Open Claude Desktop settings and navigate to **Integrations** or **MCP Servers**.
2. Click **Add server** and paste the controller URL (e.g., `https://controller.example.com/mcp`).
3. Claude Desktop opens your browser and redirects you to the controller login page.
4. Sign in with your uptrakit account (username and password, or your organization's SSO provider if
   configured).
5. After signing in, the **Authorize Access** consent screen appears. Review:
   - The client name ("Claude Desktop" or similar).
   - The redirect URI hostname shown in the "Redirect URI" section.
   - The scopes listed in "Permissions requested".
6. If everything looks correct, click **Allow access**. If the client is "Unverified", you must first
   type the redirect URI hostname into the confirmation field before the button becomes available.
7. Your browser returns to Claude Desktop. The integration is now active.

## Connect Cursor

The flow is identical to Claude Desktop:

1. Open Cursor settings and navigate to **MCP Servers**.
2. Click **Add** and paste the controller URL.
3. Cursor opens your browser and redirects you to the controller login page.
4. Sign in, review the consent screen, and click **Allow access**.
5. Your browser returns to Cursor. The integration is now active.

If Cursor shows a "registration" step first (it may call the Dynamic Client Registration endpoint),
this is normal behavior — the controller registers Cursor as a client automatically. DCR must be
enabled by your administrator for this to work.

## What the Consent Screen Shows

The consent screen always shows:

- **Client name** — the name the MCP client registered under (e.g., "Claude Desktop", "Cursor").
- **Redirect URI** — the callback address where the authorization code is sent after you approve.
  The screen emphasizes the **hostname** portion of the URI. Verify this matches where the client
  says it will receive the callback.
- **Permissions requested** — a human-readable list of what the client will be able to do:
  - "Read your uptrakit data (update history, host info, account profile)" — corresponds to `mcp:read`.
  - "Trigger software updates on your behalf" — corresponds to `mcp:write`.
- A note: "This client will act using your existing permissions — it cannot do anything you cannot
  already do."

If the client is **Unverified** (the administrator has not yet reviewed and trusted it), a red
"Unverified client" badge appears. You must type the redirect URI hostname into a confirmation field
before the Allow button becomes available.

## Reviewing Your Authorized Apps

To see which MCP clients you have authorized:

1. Sign in to the controller dashboard.
2. Navigate to **Settings → Account → Authorized Apps**.
3. The list shows each client you have authorized, the date you granted access, when the client last
   used its grant, and the scopes you approved.

## Revoking Access

To revoke an MCP client's access to your account:

1. Navigate to **Settings → Account → Authorized Apps**.
2. Find the client you want to revoke.
3. Click **Revoke** next to that client.
4. Confirm the revocation when prompted.

The client's refresh token and any access tokens it holds are immediately invalidated. The client will
ask you to authorize it again the next time it needs to connect.

## Troubleshooting `WWW-Authenticate` Errors

### 401 Unauthorized

A 401 response means the controller did not accept the access token. Common causes:

- The token expired (access tokens are valid for 15 minutes by default). The MCP client should
  automatically refresh the token. If it does not, disconnect and reconnect from the client settings.
- The controller was restarted with a new signing secret. All previously issued tokens are
  immediately invalid. Disconnect and reconnect to go through the authorization flow again.
- The client is connecting to a different controller URL than the one you authorized. Check the URL in
  the client settings.

### 403 Forbidden with `insufficient_scope`

A 403 with `error="insufficient_scope"` in the `WWW-Authenticate` header means the token does not
have the scope required by the tool you tried to use. For example, triggering a software update
requires `mcp:write`, but your token was issued for `mcp:read` only.

To fix this, the MCP client should prompt you to re-authorize with the expanded scope set. If it does
not do this automatically, disconnect and reconnect — during the new authorization flow, approve the
additional scope when the consent screen shows it.

If you do not want to grant the broader scope, the tool that requires it will continue to be
unavailable to that client.

## Reporting Suspicious Consent Prompts

If you see a consent screen that looks unexpected — an unfamiliar client name, an unusual redirect
URI hostname, a scope you do not expect — do **not** click Allow.

Steps to take:

1. Click **Deny** on the consent screen.
2. Contact your administrator immediately.
3. Include in your report:
   - The **client name** shown on the consent screen.
   - The **redirect URI hostname** shown in the "Redirect URI" section.
   - The **date and time** you saw the prompt.
   - Which MCP client (Claude Desktop, Cursor, other) triggered the flow, if known.

Your administrator can look up the client in the OAuth Clients management view and revoke it if it
is malicious.
