# ATK-19: RCE on Controller via API or Network Input

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | Controller / HTTP API |
| Prerequisites | Authenticated API access, or network access to the controller |
| STRIDE | Elevation of Privilege |

## Attack description

This scenario examines whether an attacker can achieve code execution directly on the
controller host (as opposed to on managed agent hosts).

### Path 1: No template engine

The controller does not use any server-side template engine (Handlebars, Tera,
Minijinja, etc.). Notification messages are built with Rust `format!()` macros.
User-controlled strings (software names, versions, hostnames) appear in plain text and
HTML notification bodies but are not interpreted as code on the controller.

**Verdict: Not exploitable.**

### Path 2: No dynamic code evaluation

Rust does not have `eval()`. There are no `unsafe` blocks related to code execution
in the API surface code. The controller does not load dynamic libraries, execute
external processes based on API input, or interpret user-supplied code.

**Verdict: Not exploitable.**

### Path 3: SQL injection

All database queries use SeaORM's query builder with parameterized bindings. The
single raw SQL statement (in `rate_limit.rs`) uses a static string literal with
`sea_orm::Value` parameters. The `key` parameter is constructed from parsed socket
addresses, not user-controlled headers in the standard path.

**Verdict: Not exploitable** with the current codebase. Any future raw SQL must
maintain parameterized bindings.

### Path 4: Path traversal

File system operations use the `uptrakit-directories` crate which validates path
components via `validate_path_name()`, rejecting path separators, relative components
(`.`, `..`), and absolute paths. Config and state paths are constructed by joining
validated names to fixed base directories.

Update hook paths (`compose_file`, `project_dir`) are validated at API write-time
with character whitelists and `..` rejection. However, these paths execute on agents,
not on the controller.

**Verdict: Not exploitable** on the controller.

### Path 5: HTTP header injection

The controller processes several security-sensitive HTTP headers:

- `Authorization: Bearer <token>` — parsed by the auth middleware.
- `X-Forwarded-Client-Cert-Info` — parsed only from trusted proxy IPs.
- `X-Tenant-Id` — currently ignored (returns default tenant).

Headers from untrusted sources are stripped by `strip_proxy_headers()`. The
`Referrer-Policy: no-referrer` header is set on OIDC redirects. CORS headers are
managed by the framework.

**Verdict: Limited risk.** The trusted proxy header trust model is configuration-
dependent. A misconfigured `--trusted-proxy` CIDR could allow an attacker to spoof
agent identity via forwarded certificate headers.

### Path 6: Notification HTML injection

User-controlled values (software names, version strings) are interpolated into HTML
notification bodies via `format!()` without HTML escaping:

```rust
let body_html = format!(
    "A new version of <b>{software_label}</b> is available...",
);
```

If an attacker controls a software item name or version string containing HTML/
JavaScript, the injected content appears in email notifications. This is an XSS
vector in email clients that render HTML, but not a controller-side RCE.

**Verdict: XSS in email clients, not controller RCE.** Plain text bodies use
`escape_html()` for `<`, `>`, `&`, `"`.

### Path 7: Reverse proxy header spoofing

If `--trusted-proxy` is misconfigured (e.g., set to `0.0.0.0/0`), any client can
send `X-Forwarded-Client-Cert-Info` or PEM headers to impersonate an agent's mTLS
identity. This grants WebSocket access as an agent, not code execution on the
controller, but the agent can influence controller state via fabricated messages.

**Verdict: Identity spoofing, not controller RCE.** See
[ATK-02](02-rogue-compromised-agent.md).

## Worst-case impact

Direct code execution on the controller host via API or network input is **not
achievable** with the current codebase. The controller does not execute external
processes, evaluate dynamic code, or use template engines that could be exploited.

The most impactful indirect attacks are:

- **Proxy header spoofing** (misconfiguration-dependent): agent identity impersonation
  leading to state manipulation.
- **HTML injection in notifications**: XSS in downstream email clients, not controller
  compromise.
- **API-driven RCE on agents**: via plugin config manipulation
  ([ATK-16](16-rce-plugin-config-manipulation.md)), which executes on agents, not the
  controller.

## Current mitigations

- **No external process execution on the controller.** The controller uses
  `NoopCommandExecutor` for controller-side plugin operations, which panics if called.
  All actual command execution happens on agents.
- **Parameterized SQL everywhere.** SeaORM generates parameterized queries. The single
  raw SQL statement uses static literals with parameterized values.
- **Strict path validation.** The `uptrakit-directories` crate prevents path traversal
  for all file system operations.
- **Header stripping for untrusted sources.** Certificate-related headers, forwarding
  headers, and origin headers are stripped from requests not originating from
  configured trusted proxies.
- **Rust memory safety.** The Rust language provides memory safety guarantees (no
  buffer overflows, use-after-free, or format string vulnerabilities) in safe code.
  No `unsafe` blocks exist in the API surface.
- **Type-safe deserialization.** All HTTP request bodies are deserialized into
  strongly-typed Rust structs with explicit `Validate` implementations.
- **Input validation at API boundary.** All HTTP request types implement `Validate`,
  which is called in route handlers before processing.

## Residual risk

- **Future code changes.** The current absence of RCE vectors depends on the
  continued avoidance of template engines, dynamic code evaluation, external process
  execution, and raw SQL in the controller. Future changes that introduce any of
  these patterns must be reviewed carefully.
- **Notification HTML injection.** While not controller RCE, unescaped HTML in
  notification bodies is a security concern for downstream email clients.
- **Plugin execution on controller.** Controller-side `fetch_releases` plugins
  (GitHub, Docker, GitLab, Forgejo) make outbound HTTP requests with user-configured
  URLs and credentials. While these do not execute processes, they do create SSRF
  vectors (see [ATK-07](07-ssrf-plugin-configuration.md)).
- **Trusted proxy misconfiguration.** An overly broad `--trusted-proxy` CIDR allows
  identity spoofing. This is a configuration error, not a code vulnerability, but it
  has high impact.

## Recommended improvements

- Add HTML escaping for user-controlled values in notification `body_html` templates
  to prevent XSS in email clients.
- Add a CI lint or architectural test that ensures no `std::process::Command` or
  `tokio::process::Command` is used in controller crates (only in agent/shared/plugin
  crates).
- Document the `NoopCommandExecutor` pattern as an architectural invariant: the
  controller must never execute external processes based on user input.
- Add validation for `--trusted-proxy` that warns when overly broad CIDRs are
  configured (e.g., `/0` or `/8`).
- Consider adding Content Security Policy headers to notification email bodies to
  mitigate HTML injection impact in supporting email clients.

## References

- [Security Architecture](../security/security-architecture.md)
- [Secure Development](../security/secure-development.md)
- [Reverse Proxy Security](../security/reverse-proxy-security.md)
- [ATK-07: SSRF via Plugin Configuration](07-ssrf-plugin-configuration.md)
- [ATK-16: RCE via Plugin Config Manipulation](16-rce-plugin-config-manipulation.md)
- `crates/ui/web-api/src/middleware/resolve_proxy_headers.rs` — header trust and
  stripping
- `crates/ui/web-api/src/auth/rate_limit.rs` — the single raw SQL statement
- `crates/ui/web-api/src/notifications/message_builder.rs` — notification body
  construction
