# Secure Redirect Handling for Plugin HTTP Clients

- **Date:** 2026-08-13
- **Status:** Draft (spec written, not yet planned)
- **Trigger:**

  ```text
  2026-08-13T08:44:49.231934Z WARN fetch_releases: uptrakit_plugin_releases_github::plugin:
  checksums file download returned non-success status status=302 Found
  url=https://github.com/pelican-dev/panel/releases/download/v1.0.0-beta36/checksum.txt
  ```

## Problem

The GitHub releases plugin downloads a release's checksums file with the plugin's API client
(`crates/plugins/releases/github/src/plugin.rs:327-334` calls `self.client()`, built at
`plugin.rs:150` with the shared builder default `reqwest::redirect::Policy::none()` from
`crates/plugins/infrastructure/core/src/http_client.rs:65`). `browser_download_url` on
github.com always answers 302 to `objects.githubusercontent.com`, so the fetch degrades to
`AttestationStatus::Unverified` for **every** public GitHub repo: SHA-256 digests never
populate, install-time checksum verification never runs, and the attestation feature is
silently dead.

Redirect policy across the workspace is ad-hoc per call site: docker manifests deliberately
`Policy::none()` with a documented rationale (`crates/plugins/releases/docker/src/registry.rs:88-91`),
docker blobs `limited(5)`, cargo `limited(10)`, the github install download client
`limited(10)` (`plugin.rs:665`), everything else the builder default `none()` — and seven
production clients outside the shared builder silently inherit **reqwest's own default
`limited(10)`** with no one having decided that (see § Enforcement).

Fixing the redirect bug naively is dangerous: adversarial review found the 302 failure is
currently _masking_ two worse defects (§ Sequencing constraint). This spec covers the whole
cluster, sequenced so no intermediate state is worse than baseline.

## Verified facts (all re-checked at source, 2026-08-13)

- reqwest is 0.13.4 (Cargo.lock). On a redirect hop whose host, port, **or** scheme differs
  from the previous URL, reqwest strips `Authorization`, `Cookie`, `cookie2`,
  `Proxy-Authorization`, `WWW-Authenticate` — including client `default_headers`, because
  defaults are merged before the tower `FollowRedirect` layer and the strip runs in
  `on_request` on the cloned headers (reqwest `src/redirect.rs:239-252`,
  `src/async_impl/client.rs:2614-2618`).
- The strip list is **fixed**: custom credential headers are not stripped. GitLab's client
  puts its PAT in a client-wide `PRIVATE-TOKEN` default header
  (`crates/plugins/releases/gitlab/src/plugin.rs:124-132`) — redirect-following must never be
  enabled on that client as-is.
- `SsrfSafeResolver` is a DNS `Resolve` impl (`crates/shared/types/src/ssrf.rs`); it applies
  to every hop **except** IP-literal hosts: hyper-util short-circuits DNS for literals
  (hyper-util 0.1.20 `src/client/legacy/connect/http.rs:538-554`), so a
  `Location: https://10.0.0.5/…` hop is never checked by the resolver. The redirect policy
  callback is the only place that sees the redirect target URL.
- `reqwest::redirect::Policy::custom` does **not** inherit hop counting or loop detection;
  `PolicyKind::Limit` counts `previous.len() > max` where `previous[0]` is the initial URL.
  The documented composition pattern is delegating to `Policy::limited(n).redirect(attempt)`
  after custom checks (reqwest `src/redirect.rs:66-69`, `:113-127`, `:135-142`).
- `Policy` action `stop()` returns the 3xx as an `Ok` response — indistinguishable from the
  bug being fixed. Custom checks must fail via `attempt.error(…)`.
- `Response::chunk()` is available without extra cargo features; `bytes_stream()` needs the
  `stream` feature (reqwest `src/async_impl/response.rs:310`, `:349-351`). The capped-read
  helper therefore uses `content_length()` pre-check + a `chunk()` loop — **no new
  dependencies, no feature changes**.
- `GitHubAsset` (`crates/plugins/releases/github/src/api_types.rs:19-24`) has no `id` field
  today; switching the checksums fetch to the API asset endpoint requires adding it.
- The existing convention "**Auth headers are applied per-request**, not as default headers
  on the client" (`docs/development/plugin-guidelines.md:812`, `crates/plugins/AGENTS.md:130-131`)
  is already violated by the github plugin (`plugin.rs:147` puts `AUTHORIZATION` in
  `default_headers`). Its recorded rationale explicitly warned against "re-enabling redirects
  while keeping auth in default headers"
  (`docs/superpowers/plans/2026-07-13-plugin-guidelines-realignment.md:67`). This spec
  supersedes that rule with a header-specific one (§ ADR).
- HTTP mock idiom in this workspace is `httpmock` 0.8 (workspace `Cargo.toml:218`).
- Private-IP classification helper exists: `uptrakit_shared_types::network::is_private_ip`
  (used by `ssrf.rs:97`).

## Sequencing constraint (why this is more than a redirect fix)

Two dormant defects become live the moment redirects are followed:

1. **Unbounded body read on a scheduled controller path.** `find_checksums_asset`
   (`plugin.rs:263-268`) matches the _first_ asset whose lowercase name merely contains
   `"sha256"` or `"checksum"` — a multi-GiB `checksums.tar.gz` qualifies. The fetch then does
   an unbounded `.text()` (`plugin.rs:339`). The plugin is `CONTROLLER_ONLY`, so this runs on
   the controller on every scheduled version check. Today the 302 short-circuits before the
   body is read; after the redirect fix, a hostile or misconfigured repo can OOM the
   controller. **The body cap must land before or with the redirect fix.**
2. **Unsound attestation verdict.** The controller marks the whole release `Verified` from
   the _first_ asset with a digest (`plugin.rs:369-379`), and the agent trusts a controller
   `Verified` unconditionally (`crates/shared/agent-core/src/update.rs`, the
   "Trust the controller's Verified verdict" arm); the agent's independent re-verify also
   picks the _first_ digest via `find_map`. Today every digest is `None`, so this is dead
   code and `require_attestation` is a silent no-op. After the fix, a release where attested
   asset A coexists with installed-but-unattested asset B installs under `Verified` — a
   bypass in exactly the threat model attestation exists for (repo write access without
   workflow control). **Verdict binding must land before or with the redirect fix.**

## Design

### Milestone 1 — Capped body reads (prerequisite)

Add a shared helper to `crates/plugins/infrastructure/core` (module of `http_client.rs` or a
sibling), roughly:

```rust
/// Read a response body as text, failing if it exceeds `max_bytes`.
///
/// Rejects early when `Content-Length` already exceeds the cap, then reads
/// chunk-by-chunk so peak memory is bounded even when the header lies.
pub async fn read_text_capped(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<String, BodyReadError>
```

plus a bytes variant reused by JSON consumers (`read_bytes_capped` → caller runs
`serde_json::from_slice`). Typed error enum (`thiserror`), distinct variants for
`TooLarge { limit, seen }` vs transport errors, so call sites can log the cap breach loudly
instead of an opaque fetch failure.

Apply with per-site constants (named `const`, not magic numbers):

| Site                                                                             | Cap   | Notes                                                                                                                                             |
| -------------------------------------------------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| GitHub checksums fetch (`github/src/plugin.rs:334-358`)                          | 1 MiB | real checksums files are tens of KB                                                                                                               |
| PHS script fetches (`proxmox-helper-scripts/src/plugin.rs:133-140` `fetch_text`) | 1 MiB | must surface the breach in the error path, not `.ok()?` — the caller aborts the whole discovery run, so the message must name the URL and the cap |
| npm packument (`npm/src/releases.rs:62`, inside its retry loop)                  | 8 MiB | packuments for huge packages reach several MiB                                                                                                    |

Out of scope for the cap (deferred, § Deferred): the github install-path
`download_resp.bytes()` (`plugin.rs:688`) which buffers a whole release asset — it has
`ReleaseAsset.size` available for a pre-check and deserves streaming-to-disk treatment of its
own.

### Milestone 2 — Attestation verdict binding

Invariant: **an attestation verdict may only gate or bless the asset actually being
installed.** Mechanically:

- **Agent side (authoritative):** in `agent-core`'s attestation check, drop the
  "controller said Verified → skip" shortcut and select the digest of the **asset selected
  for install** (the same selection the install path makes), not `find_map`-first. If the
  selected asset has no digest: with `require_attestation` set, block with a message naming
  the asset; without it, warn and proceed (current no-digest behavior).
- **Controller side (advisory/UI):** `check_release_attestation` keeps populating per-asset
  `sha256_digest`s (that part is sound). The release-level status **stays release-level and
  becomes advisory-only** — documented as a UI hint, never a gate — since the agent no
  longer trusts it. No representation change (per-asset statuses rejected: larger storage
  churn for a value nothing may gate on).
- Two attacker-selectable knobs noted for the plan's test matrix: `find_checksums_asset`
  takes the first name match; `parse_checksums_content` keeps the last line for a duplicated
  filename (`plugin.rs:293`).

User-visible behavior change (release-notes worthy, `security:`/`fix:` conventional commit):
once digests populate, `require_attestation = true` starts genuinely blocking updates for
repos without attestations where it previously always proceeded. Existing stored release
metadata self-heals on the next scheduled `fetch_releases`.

### Milestone 3 — Typed `RedirectMode` in the shared builder

Replace the raw field `redirect_policy: reqwest::redirect::Policy` in
`PluginHttpClientConfig` with a typed enum — no raw-`Policy` field remains in the shared
builder config (clients built outside the builder are governed by § M6's gate instead):

```rust
/// Redirect-following behavior for a plugin HTTP client.
pub enum RedirectMode {
    /// Never follow redirects; a 3xx response is returned to the caller. Default.
    None,
    /// Follow up to `hops` redirects with security guards (see below).
    Limited { hops: usize },
}
```

`Limited` compiles to `reqwest::redirect::Policy::custom` that, in order:

1. **Scheme-downgrade guard:** if `attempt.url().scheme() == "http"` and
   `attempt.previous().last()` has scheme `https`, fail via `attempt.error(…)` with a
   distinct message. Comparing against the _previous_ hop (reqwest's own `cross_host`
   semantics) means a chain that _started_ on plain http is not blocked — a client that began
   plaintext has no confidentiality to preserve; only downgrades die.
2. **Per-hop private-target guard (Strict clients only):** if the client's `SsrfMode` is
   `Strict` and the redirect target's host is an IP literal, reject private/loopback/
   link-local/CGNAT literals via `is_private_ip` (`attempt.error(…)`). This closes the
   IP-literal bypass of `SsrfSafeResolver`; hostname targets still go through the resolver on
   connect. Permissive clients skip this guard — their resolver already admits private
   addresses by design. Building the policy therefore needs the `SsrfMode` in scope: derive
   it inside `build_plugin_http_client`, which already has both.
3. **Hop cap + loop detection:** delegate to `Policy::limited(hops).redirect(attempt)` —
   never hand-roll the count (off-by-one: `previous[0]` is the initial URL; `custom` drops
   loop detection).

`Default` for the config keeps `RedirectMode::None`. Struct variant (`Limited { hops }`)
rather than tuple so a later host-allowlist or same-origin mode extends without call-site
churn. No `#[non_exhaustive]` — nothing matches on it externally and it would hurt
`..Default::default()` ergonomics.

Site mapping (every existing override, with intent):

| Site                                                                  | Today            | After                                                                                                                                                                                                                                                                                                                                                                         |
| --------------------------------------------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| shared builder default (`http_client.rs:65`)                          | `none()`         | `RedirectMode::None` (unchanged)                                                                                                                                                                                                                                                                                                                                              |
| base client (`controller-runtime/src/boot/components.rs:222`)         | `limited(5)`     | `Limited { hops: 5 }`                                                                                                                                                                                                                                                                                                                                                         |
| github API client (`github/src/plugin.rs:150`)                        | default `none()` | `None` (API only — checksums fetch moves off this client, § M4)                                                                                                                                                                                                                                                                                                               |
| github download client (`github/src/plugin.rs:665`)                   | `limited(10)`    | `Limited { hops: 10 }`, shared with checksums fetch (§ M4)                                                                                                                                                                                                                                                                                                                    |
| docker manifests (`docker/src/registry.rs:98`)                        | `none()`         | `None` (keep the documented rationale; a followed manifest redirect would also lose `Docker-Content-Digest`)                                                                                                                                                                                                                                                                  |
| docker blobs (`docker/src/registry.rs:112`)                           | `limited(5)`     | `Limited { hops: 5 }`                                                                                                                                                                                                                                                                                                                                                         |
| cargo (`cargo/src/plugin.rs:139`)                                     | `limited(10)`    | `Limited { hops: 10 }` (Permissive when custom registry — per-hop guard intentionally off there)                                                                                                                                                                                                                                                                              |
| gitlab, forgejo, npm, telegram, webhook, PHS, web-api github provider | default `none()` | `None` — explicit or default; gitlab **must** stay `None` while `PRIVATE-TOKEN` is a default header; webhook's 3xx-reject behavior is deliberate and documented (`docs/hackme/12-webhook-notification-ssrf.md`); PHS relies on URL normalization to the canonical non-redirecting `raw.githubusercontent.com` form (`discovery.rs:757-758` region) — keep that comment intact |

### Milestone 4 — GitHub plugin fixes

- **Checksums fetch via the documented API asset endpoint:** add `id: u64` to `GitHubAsset`;
  fetch `{api_base_url}/repos/{owner}/{repo}/releases/assets/{id}` with
  `Accept: application/octet-stream`. This 302s cross-host to the CDN exactly like
  `browser_download_url`, but works uniformly for public **and** private repos
  (`browser_download_url` on a private repo 302s same-host to `/login` and returns HTML,
  which would parse to an empty digest map). Uses the redirect-following download client.
- **One shared download client:** factor the install path's ad-hoc client construction
  (`plugin.rs:645-668`) into a `download_client()` helper used by both the checksums fetch
  and the install download. Bearer token stays in `default_headers` on this client —
  cross-host strip verified at reqwest source; the github.com hop needs the token for
  private repos (it mints the signed CDN URL), and the CDN hop authenticates via the signed
  query string.
- **Body read via `read_text_capped`** (§ M1).

### Milestone 5 — `Link: rel="next"` pagination origin check

Pagination follows a server-supplied URL as a **fresh request** — reqwest's redirect
machinery (and its header strip) never runs, so a hostile/compromised server could point
`next` at any host and the client would send its credentials there. All three release
plugins share the `parse_link_next` shape (`github/src/plugin.rs:438`,
`gitlab/src/plugin.rs:237`, `forgejo/src/plugin.rs:237-238`).

Fix: validate that the parsed `next` URL's origin (scheme, host, port) equals the initial
API base origin before following; on mismatch, stop pagination with a warn naming both
origins (results collected so far are still returned — same partial-page semantics as a
missing header). A tiny shared helper in `infrastructure/core` beats three copies; the three
`parse_link_next` copies can adopt it without merging the parsers themselves.

### Milestone 6 — Enforcement gate

Convention without a gate is advisory (seven production clients currently inherit reqwest's
default `limited(10)` without any decision: `crates/ui/web-api/src/oauth/cimd.rs:151` — an
URL supplied by an unauthenticated OAuth client, `crates/ui/web-api/src/oidc_http_client.rs:36`,
`crates/plugins/infrastructure/proxmox/src/client.rs:69`, `crates/shared/openapi-client/src/lib.rs:168`,
`crates/shared/service-sdk/src/ca.rs:51`, `crates/ui/cli/src/commands/auth.rs:78`;
`crates/shared/agent-core/src/update.rs:882` already sets `Policy::none()` explicitly but
uses no SSRF resolver).

- Add to `clippy.toml` `disallowed-methods` (precedent: the 11 sea-orm entries with reason
  strings and a documented `#[expect]` escape hatch):

  ```toml
  { path = "reqwest::Client::builder", reason = "use build_plugin_http_client (plugin/controller code) or carry a scoped #[expect] whose reason states the audited redirect policy and DNS-resolver decision" }
  ```

- `build_plugin_http_client` carries the sole plugin-tree `#[expect(clippy::disallowed_methods, reason = …)]`.
- Each of the seven sites above gets, in this milestone: either migration to
  `build_plugin_http_client` where the dependency graph allows (`proxmox/src/client.rs` is a
  plugin crate; `cimd.rs` and `oidc_http_client.rs` live in web-api, which already depends on
  infrastructure-core), or an explicit `.redirect(…)` choice + scoped `#[expect]` with a
  reason naming the decision (`openapi-client`, `service-sdk`, `cli`, `agent-core` — the
  first two must never depend on plugin/db/crypto crates per publishable-crate hygiene).
- **Canary (gate-inertness guard):** the scoped `#[expect(clippy::disallowed_methods, …)]`
  inside `build_plugin_http_client` itself is the canary — if the ban entry's path ever
  stops resolving (rename, dep bump), the expectation goes unfulfilled and
  `unfulfilled_lint_expectations = "deny"` fails the build. Every other `#[expect]` site
  from this milestone provides the same guarantee; no separate test-only canary construct
  is needed. (Note `clippy.toml`'s `allow-*-in-tests` keys cover unwrap/expect/panic/dbg/
  indexing only — they never pre-suppress `disallowed_methods`, so these expectations stay
  live everywhere.)

### Milestone 7 — ADR + documentation

ADR (created with `adrs new`, never hand-numbered): "Typed redirect policy for outbound HTTP
clients". Must:

- State the convention: `RedirectMode` enum, default `None`, `Limited` guards
  (downgrade block, per-hop private-IP literal check on Strict, composed hop cap), the
  clippy gate, and the canary.
- **Supersede by name** the per-request-auth-headers rule at
  `docs/development/plugin-guidelines.md:812` / `crates/plugins/AGENTS.md:130-131` and its
  recorded rationale in `docs/superpowers/plans/2026-07-13-plugin-guidelines-realignment.md:67`,
  replacing it with the header-specific rule: _redirect-following (`Limited`) is permitted
  only on clients whose credentials are (a) absent, (b) applied per-request, or (c) carried
  in a header reqwest strips cross-host (`Authorization`, `Cookie`, `Proxy-Authorization`)_ —
  naming GitLab's `PRIVATE-TOKEN` as the live counterexample.
- Name `Link: rel="next"` pagination as an in-scope manual redirect covered by the
  same-origin check.
- Record the accepted residual risk: same host+port+scheme, different-path redirects retain
  `Authorization` (reqwest `redirect.rs:241-243`); not reachable on mapped sites (github host
  is validated https+public, gitlab/forgejo stay `None`, docker auth is per-request, base
  client carries no auth).

Doc deliverables (each an explicit implementation deliverable; grep-derived, not
hand-listed):

| File                                                        | Change                                                                                                                                                                       |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/development/plugin-guidelines.md` (§ around line 812) | replace the absolute per-request-auth rule with the header-specific rule + `RedirectMode` guidance; fix the drift vs `github/plugin.rs:147`                                  |
| `crates/plugins/AGENTS.md:130-131`                          | same rule replacement (scoped file, ≤250-line budget)                                                                                                                        |
| `docs/hackme/07-ssrf-plugin-configuration.md:76-91`         | "All plugin clients use `redirect(Policy::none())`" is already false and becomes policy: rewrite to describe `RedirectMode`, the per-hop guard, and the IP-literal rationale |
| `docs/hackme/12-webhook-notification-ssrf.md:71-89`         | update wording to the typed mode; webhook stays non-following by design                                                                                                      |
| `docs/security/github-attestation.md`                       | verdict-binding semantics, API-asset-endpoint fetch, private-repo behavior, `require_attestation` now enforceable                                                            |
| `docs/hackme/11-supply-chain-dependency.md`                 | attestation/checksum claims re-checked against the new behavior (plan verifies which sentences drift)                                                                        |
| `docs/security/secure-development.md` (SSRF section)        | document the IP-literal resolver bypass and the redirect-policy hop guard as the compensating control                                                                        |
| root `AGENTS.md` (SSRF MUST-FOLLOW rule)                    | extend by one sentence: redirect policy via `RedirectMode`, `reqwest::Client::builder` clippy-banned outside audited sites; link the ADR (respect the 500-line budget)       |
| `docs/adr/README.md`                                        | regenerated by `bash scripts/regen-adr-toc.sh` after `adrs new` (gate-checked; never hand-edited)                                                                            |
| new ADR under `docs/adr/`                                   | as above, created via `adrs new` (ADR-0014 hard-fail rules apply: no `...`/placeholder tokens in required sections)                                                          |

No REST contract change is expected (attestation status travels inside stored release
metadata JSON); the plan still includes a `./scripts/regen-api.sh` no-diff verification step.
No wire-type changes (no asyncapi regen). Frontend untouched.

## Testing strategy

- **Custom policy unit tests** (infrastructure-core): downgrade blocked with `error()` (not
  a 3xx `Ok`), http-initial chain not blocked, private-IP-literal hop rejected on Strict,
  allowed on Permissive, hop cap honored at exactly `hops`, loop detected. Direct
  `Policy::check`-level tests plus `httpmock`-backed client-level tests (httpmock 0.8 is the
  workspace idiom).
- **Capped read**: under-cap passes; `Content-Length` over cap rejects before body read;
  chunked over-cap (lying/absent `Content-Length`) rejects mid-stream; error names limit and
  observed size. Vacuity guard: the over-cap fixture derives its size from the site constant
  (`CAP + 1`), never a magic literal.
- **Checksums fetch**: httpmock server 302→200 chain proves the fix red→green (the red is
  today's `Unverified`); private-repo shape (302 to same-host HTML) covered.
- **Attestation binding**: selected-asset-has-no-digest + `require_attestation` blocks;
  first-asset-attested-but-selected-asset-absent no longer yields `Verified` (the
  regression the fix exists for — must be red on current code); duplicate-filename
  last-line-wins pinned.
- **Pagination origin**: cross-origin `next` stops with partial results; same-origin
  follows.
- All new logic covers success + failure paths per repo rule; no real sleeps; no new
  endpoint tests (no REST change), so no `TestApp`/`db_access_policy.toml` impact expected —
  if a plan task adds handler tests anyway, it must follow the gate's scope rules.

## Dependencies

No new external dependencies. No new cargo features (capped read uses un-gated
`Response::chunk()`). reqwest stays at workspace `0.13` (resolving 0.13.4).

## Deferred / out of scope

- Streaming (or size-pre-checked) install-path asset download — `github/src/plugin.rs:688`
  buffers a full release asset in RAM under a 600 s timeout; `ReleaseAsset.size` is
  available for a pre-check.
- npm private-registry 301 handling (`npm/src/releases.rs:56-60` permanently errors on
  path-normalizing registries) and `NpmConfig::registry_url` validation gaps (no
  `validate_inner`, absent from `form_schema()`).
- 3xx classification arm in the web-api GitHub global provider
  (`global_providers/github.rs:1088`) — renamed/transferred repos surface as unclassified
  `RequestFailed`.
- IP-literal SSRF bypass for **initial** URLs on Strict clients (same hyper-util
  short-circuit, pre-redirect); today mitigated by per-plugin config-time host validation
  where it exists.
- Migrating `openapi-client` / `service-sdk` / `cli` / `agent-core` clients onto a shared
  builder (blocked by publishable-crate hygiene; they receive explicit-policy `#[expect]`
  sites in M6 instead).

## Decision log (grilling outcomes)

| Decision                | Choice                                                                                                  |
| ----------------------- | ------------------------------------------------------------------------------------------------------- |
| Scope                   | Centralize typed redirect policy in shared builder + fix checksums fetch (not point-fix)                |
| https→http downgrade    | Blocked on redirect hops; initial-http chains unaffected                                                |
| Auth on checksum fetch  | Keep Bearer in `default_headers`, rely on source-verified cross-host strip; header-specific rule in ADR |
| Body caps               | In scope, prerequisite (M1)                                                                             |
| Attestation soundness   | In scope, sequenced before redirect fix (M2)                                                            |
| Pagination origin check | In scope (M5)                                                                                           |
| Enforcement             | clippy `disallowed-methods` ban + scoped `#[expect]` + canary (M6)                                      |
| Checksums URL           | API asset endpoint (`/releases/assets/{id}`), not `browser_download_url`                                |
