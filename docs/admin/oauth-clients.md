# OAuth Clients — Operator Guide

This guide is for operators deploying a controller with the MCP OAuth 2.1 Authorization Server enabled.
Read `docs/security/oauth-mcp.md` before enabling Dynamic Client Registration (DCR) or Client ID
Metadata Documents (CIMD).

Related: [ADR 0010](../adr/0010-mcp-oauth-authorization-server-placement.md) ·
[Security guide](../security/oauth-mcp.md) · [Developer guide](../development/oauth-mcp.md) ·
[End-user guide](../end-user/mcp-clients.md)

## First-Run Checklist

Complete these steps in order before exposing the MCP OAuth surface to users.

1. **Set `oauth.canonical_host`** to the publicly reachable hostname of the controller. This is the
   hostname (optionally with port) that clients use to reach the controller — no scheme, no path, no
   trailing slash. Examples: `controller.example.com`, `controller.example.com:9443`. This value is
   used to derive the AS issuer (`https://<canonical_host>`) and the MCP resource URL
   (`https://<canonical_host>/mcp`). Setting `oauth.canonical_host` alone does not enable OAuth — see
   step 4. If this setting is unset when `oauth.mcp_enabled = true`, the controller refuses to start.

2. **Set `oauth.accepted_audience_hosts`** if the controller is behind a reverse proxy or split DNS,
   or if clients reach it under a different hostname than the primary. The RS validates that the token
   `aud` claim equals `https://{h}/mcp` for `h` in the set formed by `oauth.canonical_host` plus any
   hosts in `oauth.accepted_audience_hosts`. The list is capped at 5 entries. Common scenarios:
   - **Reverse proxy / TLS termination**: set `oauth.canonical_host` to the public hostname; the
     controller process binds to `127.0.0.1:8080` behind nginx. No additional accepted hosts needed
     unless clients also connect directly.
   - **Split DNS**: internal clients reach `controller.corp.internal`, external clients reach
     `controller.corp.example`. Set `oauth.canonical_host = "controller.corp.example"` and add
     `"controller.corp.internal"` to `oauth.accepted_audience_hosts`.
   - **Hostname migration**: add the new hostname as the primary and temporarily list the old hostname
     in `oauth.accepted_audience_hosts` during DNS cutover. Remove the old hostname once outstanding
     refresh tokens drain (at most `oauth.refresh_family_max_ttl_secs`, default 90 days).

3. **(Optional) Set `oauth.jwt_signing_secret`** to a stable secret that persists across restarts. The
   secret must be at least 32 bytes of random data. If this setting is omitted, the controller
   generates a per-boot secret and logs a WARN message at startup. Per-boot secrets mean every restart
   invalidates all issued tokens — all MCP clients must re-authenticate after each restart. For
   production deployments, always set a stable secret.

4. **Set `oauth.mcp_enabled = true`.** This is a required, explicit opt-in — OAuth stays disabled by
   default even with `oauth.canonical_host` set. A missing `oauth.mcp_enabled` row is treated as
   `false`. Once `oauth.mcp_enabled = true`, the controller requires `oauth.canonical_host` to be set
   and refuses to start otherwise (see step 1). To disable OAuth again, write
   `oauth.mcp_enabled = false`.

5. **(Optional) Enable DCR and/or CIMD** after reading `docs/security/oauth-mcp.md`. Both default to
   off because they expand the phishing surface of the consent screen. Enable them only if your MCP
   clients require them.
   - `oauth.dcr_enabled = true` — allows any network-reachable caller to register an OAuth client
     without authentication via `POST /oauth/register`.
   - `oauth.cimd_enabled = true` — allows HTTPS-URL `client_id` values to be resolved via Client ID
     Metadata Document fetch.

## Rate-Limit Knobs

All limits are sliding-window, DB-backed, and read at request time — changes take effect without a
restart.

| Setting key                        | Default | Bucket              | Notes                                                                            |
| ---------------------------------- | ------- | ------------------- | -------------------------------------------------------------------------------- |
| `oauth.rate.dcr_per_hour`          | 20      | source IP           | DCR registrations per IP per hour. Tighten if you see registration bursts.       |
| `oauth.rate.authorize_per_min`     | 30      | source IP           | Authorization requests per IP per minute.                                        |
| `oauth.rate.token_per_min`         | 60      | `client_id`         | Token requests per client per minute. Applies to both code exchange and refresh. |
| `oauth.rate.consent_per_min`       | 10      | `user_id`           | Consent screen loads and approval/deny submissions combined.                     |
| `oauth.rate.mcp_auth_fail_per_min` | 20      | source IP           | Failed MCP RS authentication attempts per IP per minute.                         |
| `oauth.rate.cimd_per_min`          | 10      | `ip × metadata_url` | CIMD metadata fetch requests per IP × URL pair per minute.                       |

**Tuning guidance:**

- If legitimate clients (Claude Desktop, Cursor) hit the `authorize_per_min` limit during normal
  use, raise it. The default of 30 is conservative for a single operator.
- If you see `OAUTH_CLIENT_REGISTRATION_RATE_LIMITED` audit events from IP addresses you do not
  recognize, lower `dcr_per_hour` or temporarily set `oauth.dcr_enabled = false`.
- The `consent_per_min` limit of 10 is intentionally low — the consent screen is a human-facing page
  that should not be polled programmatically. Raise it only if you have many concurrent users
  authorizing new clients simultaneously.
- `mcp_auth_fail_per_min` limits brute-force attempts against the RS. Lower it in environments where
  all MCP traffic originates from a known IP range, and consider IP allowlisting at the network layer.

## Reviewing OAuth Clients in the Dashboard

Navigate to **Settings → Authentication → OAuth Clients** to see all registered clients.

The list view shows:

- **Name** — the `client_name` from the registration (HTML-escaped; attacker-controlled values cannot
  inject markup).
- **Source** — how the client was created: `DCR` (Dynamic Client Registration), `CIMD` (Client ID
  Metadata Document), or `Manual`.
- **Created** — registration timestamp.
- **Last used** — the last time the client successfully exchanged a token.
- **Status** — current state:
  - Active — client is in good standing.
  - Unverified — client exists but has not been promoted to Trusted by an Operator. Users see a
    danger-toned badge on the consent screen and must complete a typed-confirmation step.
  - Rate-limited — client has exceeded the `token_per_min` limit. Resets automatically within the
    sliding window.
  - Revoked — client has been revoked; all associated tokens are invalidated.

**Per-row actions:**

- **View details** — shows `client_id`, `redirect_uris`, `grant_types`, `default_scope`, timestamps,
  and the CIMD metadata JSON for CIMD-sourced clients.
- **Promote to Trusted** — sets `oauth_clients.trusted_at`. Trusted clients lose the "Unverified"
  badge; users no longer need to complete the typed-confirmation step for this client.
- **Revoke** — sets `oauth_clients.revoked_at` and cascades to all associated `oauth_consents` and
  `oauth_refresh_tokens` rows. Revocation is immediate; in-flight access tokens remain valid until
  their TTL expires (default 15 minutes). To reduce that window, lower `oauth.access_token_ttl_secs`.

## Monitoring

### Audit Events to Alert On

Configure your alerting or SIEM to trigger on these audit event types:

- **`OAUTH_REFRESH_REPLAY_DETECTED`** — a refresh token was used after it had already been rotated.
  This is a strong signal that a refresh token was stolen and used by both the legitimate client and
  the attacker. Investigate the `family_id` and `replayed_refresh_id` fields. The entire token family
  is immediately revoked by the controller; the affected user must re-authorize their MCP client.

- **`OAUTH_CLIENT_REGISTRATION_RATE_LIMITED`** — a DCR registration was blocked by the rate limit.
  Clusters of this event from new IP addresses with no corresponding `OAUTH_CLIENT_REGISTERED` events
  suggest a DCR-driven client-enumeration or phishing preparation attempt.

- **`OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED`** — `oauth.canonical_host` or
  `oauth.accepted_audience_hosts` was modified. Changes to the accepted audience set can silently
  broaden token acceptance. Verify that any change was intentional and authorized.

### Spotting DCR-Driven Phishing

Phishing via DCR produces a pattern in the audit log:

1. A burst of `OAUTH_CLIENT_REGISTERED` events from a new IP address (attacker registers multiple
   clients).
2. Shortly after, `OAUTH_AUTHORIZE_REQUEST` events for those `client_id` values — users being sent
   crafted `/oauth/authorize` URLs.
3. If users deny: `OAUTH_CONSENT_DENY` events cluster around the same `client_id` values.
4. If users approve: `OAUTH_TOKEN_ISSUED` followed by `MCP_OAUTH_AUTHENTICATE { reason: "success" }`
   events at unusual hours or from unusual client IPs.

Alert on any `OAUTH_CLIENT_REGISTERED` event followed within 60 seconds by
`OAUTH_AUTHORIZE_REQUEST` for the same `client_id`, especially from distinct source IPs. This
indicates the attacker has already distributed the phishing link before the registration event
completes.

## Multi-Controller Deployments

**OAuth is not supported in multi-controller (active-active) deployments in v1.**

If you run multiple controller nodes, OAuth tokens issued by one node cannot be validated by another
node unless both nodes share exactly the same `oauth.jwt_signing_secret`. The controller enforces this
via a boot check against the `oauth_controller_instances` table: if a second node starts with a
different `kid` fingerprint, it refuses to start with a clear error message.

`oauth.allow_multi_controller_unsafe = true` bypasses the boot check. Setting this flag is an
explicit acknowledgement that:

- Tokens issued by node A may fail validation on node B if the secrets differ.
- This will produce intermittent 401 errors for users whose requests are load-balanced to a different
  node than the one that issued their token.
- Anthropic support cannot diagnose intermittent 401s in multi-controller deployments without first
  confirming all nodes share the same signing secret.

For multi-controller deployments that need OAuth, ensure all nodes read `oauth.jwt_signing_secret`
from the same secret store (e.g., a shared environment variable injected via your orchestrator).
Active-active multi-controller with per-node key management is a Phase 2 design goal.

## Rotating `oauth.jwt_signing_secret`

v1 supports only a hard-cut rotation. Plan for user impact before rotating.

**Impact:** All issued access tokens become immediately invalid when the controller restarts with the
new secret. All issued refresh tokens are also immediately invalid. Every active MCP client must
re-authenticate by going through the browser consent flow again.

**Procedure:**

1. Communicate the maintenance window to users. Access-token TTL is 15 minutes by default; in-flight
   requests that fail after the restart can be retried by the client.
2. Update `oauth.jwt_signing_secret` in your secret store or environment.
3. Restart the controller. The new `kid` fingerprint is logged at INFO on startup.
4. Verify the AS metadata endpoint returns the new issuer correctly:
   `GET /.well-known/oauth-authorization-server`.
5. Instruct users to disconnect and reconnect their MCP clients if they experience persistent 401
   errors (their clients will attempt to refresh and, on failure, should trigger re-authorization
   automatically).

**Phase 2** will introduce an overlap window via the `oauth_jwt_keys` table, allowing tokens signed
with the old key to remain valid for one TTL cycle after rotation. Until then, hard-cut rotation is
the only safe procedure.

## Upgrading from a Deployment That Never Enabled OAuth

If your controller has been running without OAuth enabled (the default), no data migration is needed.
The OAuth database tables are created by the standard migration runner on first boot after the upgrade.

To enable OAuth on an existing deployment:

1. Upgrade the controller binary to a version that includes OAuth support.
2. The migration runner creates all required tables (`oauth_clients`, `oauth_consents`,
   `oauth_authorization_requests`, `oauth_authorization_codes`, `oauth_refresh_tokens`,
   `oauth_controller_instances`) with no data.
3. Follow the [First-Run Checklist](#first-run-checklist) above.

There is no conflict between the existing opaque API-token path (`upk_*`) and the new OAuth path.
Both paths continue to work in parallel. Existing API tokens remain valid and do not need to be
rotated as part of this upgrade.
