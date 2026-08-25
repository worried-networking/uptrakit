# 47. Typed redirect policy for outbound HTTP clients

Date: 2026-08-25

## Status

Accepted

## Context

Outbound HTTP clients in plugin code ran ad-hoc redirect policies: some clients disabled
redirects, some followed them without a hop limit, and none applied a consistent guard to
what a redirect hop is allowed to target. A client that follows redirects and also carries
default headers can leak credentials cross-origin — a redirect from the configured host to
an attacker-controlled host will still carry any header set on the client itself, unless
that header is one reqwest strips.

## Decision

Every outbound HTTP client built via `build_plugin_http_client`
(`crates/plugins/infrastructure/core/src/http_client.rs`) declares a typed `RedirectMode`
(`crates/plugins/infrastructure/core/src/redirect.rs`):

- `RedirectMode::None` — never follow redirects. This is the default.
- `RedirectMode::Limited { hops }` — follow up to `hops` redirects. Every hop passes
  `check_hop`, which rejects an https-to-http scheme downgrade unconditionally, and — under
  `SsrfMode::Strict` — rejects a hop whose target is a private-IP literal. Under
  `SsrfMode::Permissive` a guarded hop is followed and logged, never silently allowed.

Direct construction of a `reqwest::Client` is clippy-banned outside
`build_plugin_http_client` (`clippy.toml` `disallowed-methods`: `reqwest::Client::builder`,
`reqwest::Client::new`, `reqwest::ClientBuilder::new`) so every plugin HTTP client goes
through the typed `RedirectMode` and the SSRF resolver.

### Default-header rule (supersedes the blanket per-request rule)

The previous rule — "auth headers are applied per-request, never as default headers" — is
too broad now that clients declare an explicit redirect mode. The header-specific rule:

A default header on a redirect-following client (`RedirectMode::Limited`) is acceptable only
when the credential it carries is one of:

(a) absent — the header carries no credential;
(b) applied per-request instead of as a client default header; or
(c) carried in a header reqwest strips on a cross-origin redirect.

The stripped-header carve-out (c) is exhaustive: it covers `Authorization`, `Cookie`, and
`Proxy-Authorization`, and no other header. Every other credential-bearing header — a custom
auth header such as `PRIVATE-TOKEN` or `X-Api-Key`, or a Gitea/Forgejo `token` header if ever
moved off `Authorization` — is never eligible for the Limited-redirect default-header
exception. A client that sets such a header as a client default must use
`RedirectMode::None`.

Counterexample: GitLab's client (`crates/plugins/releases/gitlab/src/plugin.rs`) sets
`PRIVATE-TOKEN` as a default header for personal-access-token auth. `PRIVATE-TOKEN` is a
custom header reqwest does not strip on redirect, so the GitLab client must stay
`RedirectMode::None` for as long as that header is a client default.

JIT-review note: no leak exists today. GitHub's redirect-following release-asset download
client (`crates/plugins/releases/github/src/plugin.rs:818`, `RedirectMode::Limited { hops: 10
}`) carries only `Authorization: Bearer <token>`, which reqwest strips on a cross-origin
redirect. GitLab's `PRIVATE-TOKEN` client uses `RedirectMode::None`. GitLab is one
redirect-mode change away from a token leak if someone later switches it to `Limited` without
also moving the token off the client-default `PRIVATE-TOKEN` header — hence stating the rule
as a general prohibition rather than as a fact about today's two clients.

### Pagination `Link` following is not a redirect

Release-fetcher plugins (GitHub, GitLab, Forgejo) follow response `Link: rel="next"` headers
to paginate. This is a manual HTTP request the plugin code issues itself, not a
`reqwest::redirect::Policy` hop, so `RedirectMode`/`check_hop` do not apply to it. Each
plugin instead calls `rebase_to_origin(&current_page_url, &candidate)`
(`crates/plugins/infrastructure/core/src/http_client.rs`) to pin the next page's host to the
original request's origin, so a malicious or misconfigured `Link` header cannot redirect the
authenticated pagination request to an attacker-controlled host.

## Consequences

Adding a client now requires an explicit `RedirectMode` choice instead of an implicit
default, and adding a default header to any `Limited` client requires checking it against
the exhaustive stripped-header list above. This is more upfront friction, but it removes a
class of silent cross-origin credential leaks and gives clippy a hard backstop against
bypassing the shared client builder.

Two residual risks remain, both documented in
[secure-development.md](../security/secure-development.md#ssrf-protection):

1. reqwest itself retains `Authorization` across a same-scheme, same-host, same-port,
   different-path redirect (reqwest `redirect.rs:241-243`) — this is not reachable on any
   client mapped in this ADR today, since none pairs `Authorization` as a default header
   with `Limited` redirects to the same host on a different path, but it is a reqwest
   behavior this design does not override.
2. An initial-URL private-IP literal bypasses the `SsrfSafeResolver` DNS-resolution guard,
   because an IP literal never reaches the resolver. `check_hop`'s private-IP check covers
   redirect targets, not the first request. Hardening the initial URL is deferred.
