---
title: Profile and API Tokens
weight: 90
description: The Profile page lets you view your account information and manage API tokens for programmatic access to the Uptrakit API and CLI.
---

# Profile and API Tokens

The **Profile** page lets you view your account information and manage API tokens for
programmatic access to the Uptrakit API and CLI.

## Accessing the Profile Page

### Web UI

Navigate to **Profile** in the main navigation (or use `/profile` directly). The page shows:

- Your account details (username, email, roles)
- All API tokens associated with your account with their names and creation dates

### CLI

Your current authentication status and identity can be checked without navigating the web UI:

```bash
uptrakit auth status
```

## API Tokens

API tokens are long-lived credentials that grant access to the Uptrakit API on behalf of your
account. They are intended for automation, CI pipelines, scripts, and CLI usage where interactive
browser-based login is not practical.

Each token:

- Is scoped to your account and inherits your permissions
- Is displayed in full **only once** at creation time
- Cannot be retrieved again after the creation dialog is closed
- Can be revoked at any time from the Profile page

### Creating a Token

**Web UI:**

1. Go to **Profile**.
2. Click **Create Token**.
3. Enter a descriptive name (e.g. `ci-pipeline`, `home-server-scripts`).
4. Click **Create**. The token value is displayed once — copy it immediately and store it
   securely.

**CLI:**

```bash
uptrakit auth token create --name "ci-pipeline"
```

The full token value is printed once. Store it in your secrets manager or environment variable
configuration.

### Listing Tokens

**Web UI:** All tokens are listed on the Profile page with their names and creation dates. The
token value itself is never shown again after creation.

**CLI:**

```bash
uptrakit auth token list
```

### Revoking a Token

**Web UI:** Click **Revoke** on the token you want to delete. The token is immediately
invalidated and any in-flight requests using it will receive `401 Unauthorized` responses.

**CLI:**

```bash
uptrakit auth token revoke <TOKEN_ID>
```

## Using Tokens

### CLI

Pass the token via the `--token` flag or the `UPTRAKIT_TOKEN` environment variable:

```bash
# Using the environment variable (preferred for automation)
export UPTRAKIT_TOKEN="your-token-here"
uptrakit hosts list

# Using the flag directly (visible in process listings — avoid in shared environments)
uptrakit --token "your-token-here" hosts list
```

Using `UPTRAKIT_TOKEN` is strongly preferred over `--token` in automated contexts because
environment variables are not visible in process listings (`ps`, `top`, shell history).

### API Requests

Include the token as a `Bearer` token in the `Authorization` header:

```bash
curl -H "Authorization: Bearer your-token-here" \
  https://uptrakit.example.com/api/v1/hosts
```

## Security Best Practices

**Treat tokens like passwords.**

- Store tokens in a secrets manager (e.g. HashiCorp Vault, GitHub Actions Secrets,
  a `.env` file with restricted permissions).
- Never commit tokens to source control.
- Never include tokens in log output or error messages.
- Use `UPTRAKIT_TOKEN` instead of `--token` in scripts to keep the value out of process
  listings.

**Use descriptive names.**

Name tokens after their purpose and location (e.g. `ci-github-actions`,
`home-server-cron`). This makes it easy to identify and revoke specific tokens when a
system is decommissioned or credentials are rotated.

**Rotate regularly.**

Create a replacement token before revoking the existing one to avoid downtime. Update your
secrets manager or environment configuration with the new value, then revoke the old token.

**Revoke unused tokens promptly.**

If a system is decommissioned or a token may have been exposed, revoke it immediately from
the Profile page or via the CLI. There is no waiting period — revocation takes effect
instantly.

## Related Documentation

- [Auth Flows](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) — token issuance, storage, and denylist behavior.
- [Auth and Authorization](../security/auth-and-authorization.md) — permissions model and
  roles.
- [CLI Usage Guide](cli-usage.md) — `auth token` commands and `--token` / `UPTRAKIT_TOKEN`
  usage.
