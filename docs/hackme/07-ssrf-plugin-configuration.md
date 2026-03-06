# ATK-07: SSRF via Plugin Configuration

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | Plugin system (API base URLs) |
| Prerequisites | Authenticated user with `manage_software` permission |
| STRIDE | Information Disclosure |

## Attack description

1. An authenticated user with `manage_software` permission creates or updates a plugin
   config for the GitHub, GitLab, or Forgejo release plugin.
2. The user sets `api_base_url` to an internal network address, attempting to reach
   services not intended to be publicly accessible from the controller.
3. When the scheduler or a manual version check runs, the controller-side plugin
   constructs HTTP requests to `{api_base_url}/repos/{owner}/{repo}/releases` and
   sends them using the `reqwest` client.
4. The response (or connection error) is processed by the plugin, and error messages
   may leak information about internal network topology.

Alternatively, the Docker plugin's `package_identifier` contains a registry hostname
that is used to construct `https://{registry}/v2/...` URLs. Setting the identifier to
an internal hostname like `169.254.169.254/latest` causes the controller to make
requests to cloud metadata endpoints.

## Worst-case impact

- **Internal service discovery.** The attacker probes internal network addresses
  through the controller, mapping out accessible services and ports.
- **Cloud metadata access.** In cloud environments, reaching `169.254.169.254` exposes
  instance metadata including IAM credentials, API tokens, and configuration data.
- **Internal API abuse.** The attacker accesses internal APIs (databases, caches,
  management interfaces) that are not exposed to the public network.
- **Credential exfiltration.** If the `api_base_url` points to an attacker-controlled
  server, the `Authorization: Bearer {auth_token}` header (containing the GitHub/
  GitLab/Forgejo API token) is sent to the attacker.

## Current mitigations

- **Private host validation for release plugins.** GitHub, GitLab, and Forgejo configs
  validate `api_base_url` via a shared `is_private_host()` function
  (`uptrakit_shared_types::network`) which blocks:
  - `localhost`, `*.local`, `*.internal`, `*.localhost` hostnames.
  - IPv4 private ranges (`10.x`, `172.16-31.x`, `192.168.x`), loopback (`127.x`),
    link-local (`169.254.x`), and CGNAT (`100.64-127.x`).
  - IPv6 loopback (`::1`), unspecified (`::`), ULA (`fc00::/7`), and
    link-local (`fe80::/10`).
- **HTTPS-only enforcement.** All three release plugins require `api_base_url` to use
  the `https://` scheme. `http://`, `file://`, and other schemes are rejected.
- **HTTP client timeouts.** All plugin HTTP clients are configured with
  `connect_timeout(10s)` and `timeout(60s)`, preventing indefinite connections to
  slow or unresponsive internal services.
- **Authentication required.** Plugin config creation and modification require the
  `manage_software` permission, limiting the attack to authenticated and authorized
  users.

## Residual risk

- **DNS rebinding.** `is_private_host()` checks the hostname string at validation
  time, not at connection time. A hostname like `evil.com` could resolve to `127.0.0.1`
  when the HTTP request is actually made, bypassing the static hostname check.
- ~~IPv6 private ranges not fully blocked.~~ **Fixed.** `is_private_host()` now blocks
  IPv6 ULA (`fc00::/7`) and link-local (`fe80::/10`) addresses.
- ~~CGNAT range not blocked.~~ **Fixed.** `is_private_host()` now blocks the
  `100.64.0.0/10` (Carrier-Grade NAT) range.
- ~~Docker registry has no private-host check.~~ **Fixed.** `validate_identifier()`
  now strips the port from the registry hostname and validates it against
  `is_private_host()`, rejecting private registries like `localhost`, `10.0.0.1`,
  `192.168.1.1:5000`, and `169.254.169.254`.
- **Error message information leakage.** Connection errors from SSRF attempts may
  include internal IP addresses, port numbers, or service banners in error messages
  returned to the API caller or logged in version check results.
- ~~Redirect following.~~ **Fixed.** All plugin HTTP clients now use
  `redirect(Policy::none())`, disabling automatic redirect following. API endpoints
  should not redirect; any 3xx response is treated as an error.

## Recommended improvements

- Implement DNS resolution validation at connection time (not just at config
  validation time) by using a custom `reqwest` DNS resolver that rejects private IP
  addresses. This prevents DNS rebinding attacks.
- ~~Add `is_private_host()` validation to the Docker plugin's registry hostname.~~
  **Done.** `validate_identifier()` now checks the registry hostname.
- ~~Block IPv6 ULA, link-local, and CGNAT ranges~~ — **Done.** Consolidated shared
  `is_private_host()` in `uptrakit_shared_types::network` covers all ranges.
- ~~Disable HTTP redirect following in plugin HTTP clients~~ — **Done.** All plugin
  clients use `redirect(Policy::none())`.
- Sanitize error messages from failed SSRF attempts to avoid leaking internal network
  information in API responses and logs.
- Consider an allowlist-based approach for `api_base_url` where operators explicitly
  approve base URLs for self-hosted instances, rather than relying on a blocklist.

## References

- [Secure Development — Plugin Input Validation](../security/secure-development.md#plugin-input-validation)
- [Plugin Guidelines](../development/plugin-guidelines.md)
- `crates/shared/types/src/network.rs` — shared `is_private_host()` (IPv4/IPv6/hostname)
- `crates/plugins/releases/github/src/config.rs` — `GitHubConfig::validate()`
- `crates/plugins/releases/gitlab/src/config.rs` — `GitLabConfig::validate()`
- `crates/plugins/releases/forgejo/src/config.rs` — `ForgejoConfig::validate()`
- `crates/plugins/releases/docker/src/registry.rs` — `get_manifest_digest()`
