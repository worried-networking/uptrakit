# ATK-12: Webhook Notification SSRF

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | Notifications (webhook channel) |
| Prerequisites | Authenticated user with `manage_notifications` permission |
| STRIDE | Information Disclosure |

## Attack description

1. An attacker with `manage_notifications` permission creates a webhook notification
   channel with a URL targeting an internal network resource:
   `http://169.254.169.254/latest/meta-data/iam/security-credentials/`
2. The attacker creates a notification rule that triggers on a common event (e.g.,
   `UpdateAvailable`).
3. When the event fires, the notification dispatcher calls `WebhookChannel::deliver()`,
   which sends an HTTP POST request to the attacker-specified URL.
4. The response is processed by the controller. While the response body is not returned
   to the attacker directly, error messages, connection timing, and HTTP status codes
   may leak information about internal services.
5. If the URL points to an attacker-controlled external server, the request body
   contains the notification payload (software names, versions, host identifiers).

## Worst-case impact

- **Cloud metadata access.** In AWS/GCP/Azure environments, the controller can reach
  the instance metadata service at `169.254.169.254`, potentially exposing IAM
  credentials, API tokens, and instance configuration.
- **Internal service scanning.** The attacker uses the webhook channel as a port
  scanner, probing internal services by observing connection success/failure timing
  and error messages.
- **Data exfiltration.** Webhook payloads contain software item names, version numbers,
  host identifiers, and update status — information that reveals the infrastructure's
  software inventory and patch state.
- **Custom header injection.** The webhook channel supports arbitrary custom headers
  via `config["headers"]`. An attacker can inject `Authorization`, `Cookie`, or other
  security-sensitive headers, potentially authenticating to internal services.

## Current mitigations

- **Authentication required.** Webhook channel creation requires `manage_notifications`
  permission. Only authorized users can configure webhook URLs.
- **HMAC signature.** When a `secret` is configured, outbound requests include an
  `X-Uptrakit-Signature: sha256=<hex>` header for payload authenticity. However, this
  protects the receiver, not the sender.
- **HTTP client timeouts.** The `reqwest` client is configured with
  `connect_timeout(10s)` and `timeout(60s)`, preventing indefinite connections.
- **Encrypted config storage.** Webhook URLs and secrets are stored encrypted in the
  `notification_channels.config` column via `EncryptedString`.
- **Secret masking in API responses.** The `secret` field is masked in API responses
  via `mask_config_secrets()`.

## Residual risk

- **No URL validation beyond scheme.** The `validate_config()` function only checks
  that the URL starts with `http://` or `https://`. It does not block:
  - `http://127.0.0.1`, `http://localhost` — loopback addresses.
  - `http://169.254.169.254` — cloud metadata endpoints.
  - `http://10.x.x.x`, `http://192.168.x.x` — private network addresses.
  - Encoded IP addresses (e.g., `http://0x7f000001`).
- **No redirect validation.** The `reqwest` client follows HTTP redirects by default.
  An external URL could redirect to an internal address, bypassing any future URL
  validation.
- **Arbitrary custom headers.** The `config["headers"]` field is forwarded verbatim
  to the outbound request with no header name blocklist. An attacker can override
  `Host`, `Authorization`, or other security-critical headers.
- **DNS rebinding.** A URL like `http://evil.com` could resolve to an internal IP at
  request time, bypassing any hostname-based validation.
- **Notification payload leakage.** The webhook body contains operational data
  (software names, versions, hosts) sent to any configured URL without encryption.

## Recommended improvements

- Implement `is_private_host()` validation (similar to the plugin URL validation) for
  webhook URLs, blocking loopback, private, link-local, and cloud metadata addresses.
- Add a header name blocklist that rejects security-sensitive headers like
  `Authorization`, `Cookie`, `Host`, `X-Forwarded-For`, and `Proxy-Authorization` in
  the custom headers configuration.
- Disable HTTP redirect following in the webhook client, or validate redirect targets
  against the same private-host rules.
- Implement DNS resolution validation at connection time to prevent DNS rebinding
  attacks.
- Add a "test delivery" dry-run that shows the resolved IP address and response
  status before committing a webhook configuration, giving admins visibility into
  where requests will go.
- Consider requiring HTTPS-only for webhook URLs in production, blocking `http://`
  to prevent plaintext payload transmission.

## References

- [Notification Subsystem Security](../security/notifications-security.md)
- [ATK-07: SSRF via Plugin Configuration](07-ssrf-plugin-configuration.md)
- `crates/shared/notification-channels/src/webhook.rs` — `WebhookChannel`,
  `validate_config()`
- `crates/ui/web-api/src/routes/notifications.rs` — notification channel handlers
- `crates/ui/web-api/src/notifications/dispatcher.rs` — notification dispatcher
