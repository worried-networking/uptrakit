# Secure Redirect Handling for Plugin HTTP Clients

- **Date:** 2026-08-13
- **Last revised:** 2026-08-20 (spec split: digest/attestation concerns extracted to bead
  `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`; see § Split rationale and Decision log)
- **Status:** Draft (spec written, not yet planned)
- **Trigger:**

  ```text
  2026-08-13T08:44:49.231934Z WARN fetch_releases: uptrakit_plugin_releases_github::plugin:
  checksums file download returned non-success status status=302 Found
  url=https://github.com/pelican-dev/panel/releases/download/v1.0.0-beta36/checksum.txt
  ```

## Problem

The trigger WARN exposed a cluster of HTTP-client defects. The original centerpiece — fixing
the checksums-file download so attestation digests populate — is **no longer this spec's
job**: contrarian review (2026-08-20) found the checksums-file design fatally flawed and
simultaneously obsolete (§ Split rationale), so the digest/attestation redesign was
extracted to its own speccing bead, `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`. What remains here is the HTTP hygiene
cluster that review confirmed is real regardless of attestation:

Redirect policy across the workspace is ad-hoc per call site: docker manifests deliberately
`Policy::none()` with a documented rationale (`crates/plugins/releases/docker/src/registry.rs:88-91`),
docker blobs `limited(5)`, cargo `limited(10)`, uv `limited(10)`
(`package-managers/uv/src/plugin.rs:166`, same Strict/Permissive-by-index-source pattern as
cargo), the github install download client
`limited(10)` (`plugin.rs:665`), everything else the builder default `none()` — and six
production clients outside the shared builder silently inherit **reqwest's own default
`limited(10)`** with no one having decided that, plus a seventh that chose `Policy::none()`
explicitly but made no DNS-resolver decision (see § Enforcement). Body reads on scheduled
controller paths are unbounded (OOM surface independent of redirects), `Link: rel="next"`
pagination follows server-supplied URLs with no origin check, and nothing stops a new client
from bypassing the shared builder entirely.

This spec makes **no client newly follow redirects** — every site keeps its current
effective policy, typed and guarded — so no intermediate state is worse than baseline.

## Split rationale (digest/attestation → `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`)

Three validated findings killed the checksums-file attestation design that earlier
revisions of this spec tried to repair:

1. **F1 — obsolete:** the GitHub release API now returns a per-asset `digest` field
   (`"sha256:<hex>"`) directly (verified live 2026-08-20 against `pelican-dev/panel` and
   `cli/cli`; present on every asset of recent releases, `null` on 2022-era releases). The
   checksums-file download — the very request that 302s — is an unnecessary, weaker digest
   source.
2. **F2 — forgeable:** any single-digest gate fed from a repo-writable `checksum.txt` is
   attacker-constructible: a one-line file naming an unchanged attested asset A while the
   release installs unattested asset B passes, while honest multi-digest GoReleaser layouts
   block — the passing shape is the forged one.
3. **F3 — silent no-op:** `check_attestation_gate` returns `Ok(())` before any flag check
   when `release_url` isn't `https://github.com/...`
   (`crates/shared/agent-core/src/update.rs:956-958`; parser at `update.rs:853-854`), so
   `require_attestation` never fires for non-github URLs.

The redesign (API digest field as sole source, checksums-path deletion, binding at the
asset-selection site, pipeline backstop, fail-closed semantics) is fully decision-settled
and recorded in bead `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`, which gates the follow-up rollback-detection spec
`uptrakit-te4i9`. Constraint inherited here: the checksums fetch keeps its non-following
client until `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate` deletes it — **do not enable redirects on that path**.

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
- The existing convention "**Auth headers are applied per-request**, not as default headers
  on the client" (`docs/development/plugin-guidelines.md:817`, `crates/plugins/AGENTS.md:130-131`)
  is already violated by the github plugin (`plugin.rs:147` puts `AUTHORIZATION` in
  `default_headers`). Its recorded rationale explicitly warned against "re-enabling redirects
  while keeping auth in default headers" (dissolved into beads 2026-08-16 as tasks
  `uptrakit-plan-2026-07-13-plugin-guidelines-realignment-t01..t02` under bead epic
  `uptrakit-spec-2026-07-12-plugin-guidelines-realignment-design`; full text at
  `pre-beads-archive`; formerly plan line 67). This spec
  supersedes that rule with a header-specific one (§ ADR).
- HTTP mock idiom in this workspace is `httpmock` 0.8 (workspace `Cargo.toml:220`).
- Private-IP classification helper exists: `uptrakit_shared_types::network::is_private_ip`
  (used by `ssrf.rs:97`).

## Design

### Milestone 1 — Capped body reads

Unbounded body reads on scheduled controller paths are an OOM surface regardless of
redirect policy (a hostile or misconfigured upstream can serve an arbitrarily large body
today). Add a shared helper to `crates/plugins/infrastructure/core` (module of
`http_client.rs` or a sibling), roughly:

```rust
/// Read a response body as text, failing if it exceeds `max_bytes`.
///
/// Rejects early when `Content-Length` already exceeds the cap, then reads
/// chunk-by-chunk so peak memory is bounded even when the header lies.
pub async fn read_text_capped(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<String>
```

plus a bytes variant reused by JSON consumers (`read_bytes_capped` → caller runs
`serde_json::from_slice`). Typed error enum (`thiserror`), distinct variants for
`TooLarge { limit, seen }` vs transport errors, so call sites can log the cap breach loudly
instead of an opaque fetch failure. Per the error-handling boundary convention the module
declares `pub type Result<T> = std::result::Result<T, Report<BodyReadError>>` (rootcause
`Report`, never bare `BodyReadError` — the existing bare `PluginHttpClientBuildError`
return in `http_client.rs` is a local shortcut, not precedent to extend). Both helpers are
public fallible APIs in a shared contract crate, so each documents a `# Errors` rustdoc
section (coding-standards § Shared Contract Crates).

Apply with per-site constants (named `const`, not magic numbers):

| Site                                                                             | Cap   | Notes                                                                                                                                             |
| -------------------------------------------------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| PHS script fetches (`proxmox-helper-scripts/src/plugin.rs:164-184` `fetch_text`) | 1 MiB | must surface the breach in the error path, not `.ok()?` — the caller aborts the whole discovery run, so the message must name the URL and the cap |
| npm packument (`package-managers/npm/src/releases.rs:62`, inside its retry loop) | 8 MiB | packuments for huge packages reach several MiB                                                                                                    |

The github checksums fetch (`plugin.rs:334-358`, unbounded `.text()` at `plugin.rs:339`) is
**not** capped here: the whole path is slated for deletion by `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`, and it
stays dormant meanwhile — its client keeps `Policy::none()` (§ M2 site mapping), so the
302 short-circuits before any body is read.

Out of scope for the cap (deferred, § Deferred): the github install-path
`download_resp.bytes()` (`plugin.rs:688`) which buffers a whole release asset — it has
`ReleaseAsset.size` available for a pre-check and deserves streaming-to-disk treatment of its
own.

### Milestone 2 — Typed `RedirectMode` in the shared builder

Replace the raw field `redirect_policy: reqwest::redirect::Policy` in
`PluginHttpClientConfig` with a typed enum — no raw-`Policy` field remains in the shared
builder config (clients built outside the builder are governed by § M4's gate instead):

```rust
/// Redirect-following behavior for a plugin HTTP client.
#[non_exhaustive]
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
   redirect-hop bypass of `SsrfSafeResolver` for plain IPv4 literals; hostname targets still
   go through the resolver on connect. Known limitation: IPv4-mapped (`::ffff:0:0/96`), 6to4
   (`2002::/16`), and NAT64 (`64:ff9b::/96`) IPv6 forms evade `is_private_ip` itself — a
   pre-existing gap in the helper, tracked as bug bead `uptrakit-amd3d`; this guard calls the
   same helper and inherits that fix when it lands. Permissive clients skip this guard —
   their resolver already admits private addresses by design. Building the policy therefore
   needs the `SsrfMode` in scope: derive it inside `build_plugin_http_client`, which already
   has both.
   Guard failures (both causes above) go through a small local `thiserror` enum
   (`HopGuardError`, one variant per cause) — not raw string literals, per the workspace
   typed-error convention. Per the boundary convention the hop-decision function returns
   `Result<(), Report<HopGuardError>>` (module `Result` alias, `report!()` at raise sites);
   the `Policy::custom` closure is the fixed-foreign-signature site (error-handling
   Pattern 13): it alone converts the `Report` into the boxed
   `Into<Box<dyn StdError + Send + Sync>>` that `attempt.error(…)` accepts. Unit tests
   assert the failing variant via `current_context()`, never string-match.

3. **Hop cap + loop detection:** delegate to `Policy::limited(hops).redirect(attempt)` —
   never hand-roll the count (off-by-one: `previous[0]` is the initial URL; `custom` drops
   loop detection).

`Default` for the config keeps `RedirectMode::None`. Struct variant (`Limited { hops }`)
rather than tuple so a later host-allowlist or same-origin mode extends without call-site
churn. `#[non_exhaustive]` per the coding-standards default (the enum explicitly
anticipates future modes, so the closed-enum exception does not apply; matches
`HostCompatibilityError` (`host_requirements.rs:76`) / `PluginError` (`error.rs:5`)
precedent in the same crate and `PluginCapability` in `uptrakit-shared-types`) —
cross-crate match sites carry a wildcard arm; existing struct-variant construction
(`RedirectMode::Limited { hops: 5 }`) is unaffected.

Site mapping (every existing override, with intent — every row is behavior-preserving on
hop counts; `Limited` rows additionally gain the guards):

| Site                                                                  | Today            | After                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| --------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| shared builder default (`http_client.rs:65`)                          | `none()`         | `RedirectMode::None` (unchanged)                                                                                                                                                                                                                                                                                                                                                                                                            |
| base client (`controller-runtime/src/boot/components.rs:222`)         | `limited(5)`     | `Limited { hops: 5 }`                                                                                                                                                                                                                                                                                                                                                                                                                       |
| github API client (`github/src/plugin.rs:150`)                        | default `none()` | `None` — the dormant checksums fetch rides this client until `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate` deletes it; never enable redirects here                                                                                                                                                                                                                                                                        |
| github download client (`github/src/plugin.rs:665`)                   | `limited(10)`    | `Limited { hops: 10 }`                                                                                                                                                                                                                                                                                                                                                                                                                      |
| docker manifests (`docker/src/registry.rs:98`)                        | `none()`         | `None` (keep the documented rationale; a followed manifest redirect would also lose `Docker-Content-Digest`)                                                                                                                                                                                                                                                                                                                                |
| docker blobs (`docker/src/registry.rs:112`)                           | `limited(5)`     | `Limited { hops: 5 }`                                                                                                                                                                                                                                                                                                                                                                                                                       |
| cargo (`package-managers/cargo/src/plugin.rs:139`)                    | `limited(10)`    | `Limited { hops: 10 }` (Permissive when custom registry — per-hop guard intentionally off there)                                                                                                                                                                                                                                                                                                                                            |
| uv (`package-managers/uv/src/plugin.rs:166`)                          | `limited(10)`    | `Limited { hops: 10 }` — same decision as cargo: Strict + per-hop guard for the pypi.org default, Permissive with guard intentionally off when a custom `index_url` is configured                                                                                                                                                                                                                                                           |
| gitlab, forgejo, npm, telegram, webhook, PHS, web-api github provider | default `none()` | `None` — explicit or default; gitlab **must** stay `None` while `PRIVATE-TOKEN` is a default header; webhook's 3xx-reject behavior is deliberate and documented (`docs/hackme/12-webhook-notification-ssrf.md`); PHS relies on URL normalization to the canonical non-redirecting `raw.githubusercontent.com` form (prefix builders `discovery.rs:68-98`, normalization doc comment `discovery.rs:869` region) — keep those comments intact |

### Milestone 3 — `Link: rel="next"` pagination origin check

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

### Milestone 4 — Enforcement gate

Convention without a gate is advisory (six production clients currently inherit reqwest's
default `limited(10)` without any decision: `crates/ui/web-api/src/oauth/cimd.rs:151` — an
URL supplied by an unauthenticated OAuth client, `crates/ui/web-api/src/oidc_http_client.rs:36`,
`crates/plugins/infrastructure/proxmox/src/client.rs:69`, `crates/shared/openapi-client/src/lib.rs:168`,
`crates/shared/service-sdk/src/ca.rs:51`, `crates/ui/cli/src/commands/auth.rs:78`; a seventh
audited site, `crates/shared/agent-core/src/update.rs:884`, already sets `Policy::none()`
explicitly — its gap is the missing DNS-resolver decision, not redirects).

- Add to `clippy.toml` `disallowed-methods` (precedent: the 11 sea-orm entries with reason
  strings and a documented `#[expect]` escape hatch):

  following the file's two-part convention (terse `reason` string + header comment carrying
  the escape-hatch policy, as the sea-orm block does):

  ```toml
  # Every outbound HTTP client must carry an audited redirect-policy and
  # DNS-resolver decision. Escape hatch: #[expect(clippy::disallowed_methods,
  # reason = "...")] naming both decisions (see § M4 for the audited sites).
  # Known bypass: Default-trait constructors (Client::default(),
  # ClientBuilder::default()) are invisible to this lint — see bead uptrakit-6bsg7.
  { path = "reqwest::Client::builder", reason = "use build_plugin_http_client — direct builders skip the SSRF resolver and typed RedirectMode" }
  { path = "reqwest::Client::new", reason = "use build_plugin_http_client — direct builders skip the SSRF resolver and typed RedirectMode" }
  { path = "reqwest::ClientBuilder::new", reason = "use build_plugin_http_client — direct builders skip the SSRF resolver and typed RedirectMode" }
  ```

  Banning only `Client::builder` would leave the equivalent constructors
  (`Client::new()`, `ClientBuilder::new()`, each a distinct def-path) as silent bypasses.
  All three entries were proven live by an isolated probe crate (2026-08-20). The same
  probe confirmed the lint **silently never fires** on `reqwest::Client::default()`,
  `reqwest::ClientBuilder::default()`, or `let _: Client = Default::default()` — even with
  `::default` entries present in `clippy.toml` — so no dead `::default` entries are added;
  the bypass is documented in the header comment and its mitigation is decided in bead
  `uptrakit-6bsg7`, not here. The workspace does not
  enable reqwest's `blocking` feature; if it ever does, the `reqwest::blocking::*`
  constructor paths join this list. Known limitation (documented in the header comment,
  no extra gate): clients constructed _inside_ dependency crates (octocrab-style) are
  invisible to this lint — covered by dependency review, not by M4.

- `build_plugin_http_client` carries the sole plugin-tree `#[expect(clippy::disallowed_methods, reason = …)]`.
- Each of the seven sites above gets, in this milestone: either migration to
  `build_plugin_http_client` where the dependency graph allows (`proxmox/src/client.rs` is a
  plugin crate; `cimd.rs` and `oidc_http_client.rs` live in web-api, which already depends on
  infrastructure-core), or an explicit `.redirect(…)` choice **and an explicit DNS-resolver
  decision** + scoped `#[expect]` whose reason names both (`openapi-client`, `service-sdk`,
  `cli`, `agent-core` — the first two are published to crates.io and their transitive dep
  trees must not contain `uptrakit-shared-db`/`uptrakit-tenant-db`/`uptrakit-crypto`
  (coding-standards § Publishable Crate Dependency Hygiene), which infrastructure-core
  optionally pulls in (`infrastructure/core/Cargo.toml:41-43`); that hygiene blocks only the
  shared-_builder_ migration, not resolver adoption: `SsrfSafeResolver` lives in
  `uptrakit-shared-types` behind the `http-ssrf` feature, and all four crates already
  depend on that crate). Recorded resolver
  decisions: `agent-core/src/update.rs:882` (attestation verify client) adopts
  `SsrfSafeResolver::new()` — the host is the fixed `api.github.com` today, so this is
  defense-in-depth, a one-line `http-ssrf` feature enable on an existing dependency;
  `openapi-client`/`service-sdk`/`cli` talk to the operator-configured controller where
  private addresses are legitimate, so they adopt `SsrfSafeResolver::permissive()` — the
  self-hosted carve-out the SSRF MUST-FOLLOW rule names explicitly — and their `#[expect]`
  reason records "permissive resolver — operator-configured endpoint" as the decision.
- **Canary (gate-inertness guard):** the scoped `#[expect(clippy::disallowed_methods, …)]`
  inside `build_plugin_http_client` itself is the canary for `Client::builder` — if the ban
  entry's path ever stops resolving (rename, dep bump), the expectation goes unfulfilled and
  `unfulfilled_lint_expectations = "deny"` fails the build. The `Client::new` entry gets the
  same guarantee from the test-site `#[expect]`s below. `ClientBuilder::new` has **no** call
  site anywhere in the workspace (checked 2026-08-20), so no organic `#[expect]` ever
  exercises that entry; it gets a dedicated `#[cfg(test)]` canary in
  `build_plugin_http_client`'s test module — an `#[expect(clippy::disallowed_methods)]` fn
  invoking `reqwest::ClientBuilder::new()` directly, using the same mechanism as the
  sea-orm canary block in
  `crates/shared/db-tx/src/lib.rs` that exercises every banned path regardless of production
  usage. (Note `clippy.toml`'s `allow-*-in-tests` keys cover unwrap/expect/panic/dbg/
  indexing only — they never pre-suppress `disallowed_methods`, so these expectations stay
  live everywhere.)
- **Test-code call sites (in scope for this milestone):** `cargo clippy --all-targets`
  lints test targets, so the ban also trips the ~53 existing test/integration-test
  `reqwest::Client::builder()`/`Client::new()` sites (in-crate `#[tokio::test]`s in
  `proxmox-helper-scripts/src/plugin.rs`, `shared/types/src/ssrf.rs`,
  `shared/github-client/src/lib.rs`, `plugins/releases/docker/src/auth.rs`,
  `notifications/{webhook,telegram}/src/plugin.rs`, plus `core/integration-tests`'
  `helpers/api_client.rs`, `tests/oauth_end_to_end.rs`, the reverse-proxy suites, and
  `crates/ui/mcp/tests/` — enumerate by grep at implementation time, the count drifts).
  Policy: a crate with two or more test sites gets one small local test-helper
  fn carrying a single `#[expect(clippy::disallowed_methods, reason = "test client to
local/mock endpoint — no redirect or SSRF exposure")]`; single-site crates annotate the
  site directly with the same reason. Test clients never migrate to
  `build_plugin_http_client` (they intentionally talk to local mock/compose endpoints the
  strict resolver would block). Doc-comment examples (e.g. `ssrf.rs:24`) are unaffected —
  clippy does not lint doctests.

### Milestone 5 — ADR + documentation

ADR (created with `adrs new`, never hand-numbered): "Typed redirect policy for outbound HTTP
clients". Must:

- State the convention: `RedirectMode` enum, default `None`, `Limited` guards
  (downgrade block, per-hop private-IP literal check on Strict, composed hop cap), the
  clippy gate, and the canary.
- **Supersede by name** the per-request-auth-headers rule at
  `docs/development/plugin-guidelines.md:817` / `crates/plugins/AGENTS.md:130-131` and its
  recorded rationale (dissolved into beads 2026-08-16 as tasks
  `uptrakit-plan-2026-07-13-plugin-guidelines-realignment-t01..t02` under bead epic
  `uptrakit-spec-2026-07-12-plugin-guidelines-realignment-design`; formerly plan line 67; full
  text at `pre-beads-archive`),
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

| File                                                        | Change                                                                                                                                                                                                       |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `docs/development/plugin-guidelines.md` (§ around line 812) | replace the absolute per-request-auth rule with the header-specific rule + `RedirectMode` guidance; fix the drift vs `github/plugin.rs:147`                                                                  |
| `crates/plugins/AGENTS.md:130-131`                          | same rule replacement (scoped file, ≤250-line budget)                                                                                                                                                        |
| `docs/hackme/07-ssrf-plugin-configuration.md:76-91`         | "All plugin clients use `redirect(Policy::none())`" is already false and becomes policy: rewrite to describe `RedirectMode`, the per-hop guard, and the IP-literal rationale                                 |
| `docs/hackme/12-webhook-notification-ssrf.md:71-89`         | update wording to the typed mode; webhook stays non-following by design                                                                                                                                      |
| `docs/security/secure-development.md` (SSRF section)        | document the IP-literal resolver bypass; the hop guard covers redirect hops only — the section must state the deferred initial-URL literal residual explicitly, never present the hop guard as full coverage |
| root `AGENTS.md` (SSRF MUST-FOLLOW rule)                    | extend by one sentence: redirect policy via `RedirectMode`, `reqwest::Client::builder` clippy-banned outside audited sites; link the ADR (respect the 500-line budget)                                       |
| `docs/adr/README.md`                                        | regenerated by `bash scripts/regen-adr-toc.sh` after `adrs new` (gate-checked; never hand-edited)                                                                                                            |
| new ADR under `docs/adr/`                                   | as above, created via `adrs new` (ADR-0014 hard-fail rules apply: no `...`/placeholder tokens in required sections)                                                                                          |

Attestation doc deliverables (`docs/security/github-attestation.md`, the `CONTEXT.md`
attestation-verdict glossary entry, and the `docs/hackme/11-supply-chain-dependency.md`
claims re-check) moved to `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate` with the rest of that concern.

No REST contract change is expected; the plan still includes a `./scripts/regen-api.sh`
no-diff verification step.
No wire-type changes (no asyncapi regen). Frontend untouched.

## Testing strategy

- **Custom policy unit tests** (infrastructure-core): downgrade blocked with `error()` (not
  a 3xx `Ok`), http-initial chain not blocked, private-IP-literal hop rejected on Strict,
  allowed on Permissive, and a composition test that the guards never short-circuit the
  delegation to `Policy::limited(hops)` (a passing guard still yields the upstream hop-cap
  decision — do **not** assert reqwest's own counting or loop detection as separate cases;
  that would test upstream behavior). Test seam
  (required — `reqwest::redirect::Attempt` has no public constructor and `Policy::check` is
  `pub(crate)`, so `Attempt`-level tests cannot be written outside reqwest, and a local
  `httpmock` server lives on a loopback literal that the Strict guard rejects by design):
  factor the per-hop decision into a plain function taking parsed URLs
  (`(mode, target_url, previous_urls) -> hop decision`) that the `Policy::custom` closure
  delegates to; unit-test every branch there, including Strict-follows-public-host with
  fabricated public addresses. `httpmock`-backed client-level tests (httpmock 0.8 is the
  workspace idiom) then cover only wiring: Strict client rejects a redirect hop to a
  loopback literal (the guard's expected behavior against a local mock); Permissive client
  follows the same chain.
- **Capped read**: under-cap passes; `Content-Length` over cap rejects before body read;
  chunked over-cap (lying/absent `Content-Length`) rejects mid-stream; error names limit and
  observed size. Vacuity guard: the over-cap fixture derives its size from the site constant
  (`CAP + 1`), never a magic literal.
- **Pagination origin**: cross-origin `next` stops with partial results; same-origin
  follows.
- All new logic covers success + failure paths per repo rule; no real sleeps; no new
  endpoint tests (no REST change), so no `TestApp`/`db_access_policy.toml` impact expected —
  if a plan task adds handler tests anyway, it must follow the gate's scope rules.

## Dependencies

No new external dependencies. No new cargo features (capped read uses un-gated
`Response::chunk()`). reqwest stays at workspace `0.13` (resolving 0.13.4).

## Deferred / out of scope

- **Digest/attestation redesign** — extracted whole to bead `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate` (write-spec
  gate; blocks the `uptrakit-te4i9` provenance/rollback follow-up): GitHub API per-asset
  `digest` field as sole source, deletion of the checksums-file path (the trigger WARN's
  request), attestation binding at the asset-selection site with a pipeline backstop,
  fail-closed semantics, and the attestation doc set. Verifier retry for the
  unavailable class is its own deferred bead, `uptrakit-qpwd1`.
- clippy `disallowed-methods` Default-trait bypass (`Client::default()` et al., probe-confirmed
  invisible to the lint) — mitigation decided in bead `uptrakit-6bsg7`.
- Streaming (or size-pre-checked) install-path asset download — `github/src/plugin.rs:688`
  buffers a full release asset in RAM under a 600 s timeout; `ReleaseAsset.size` is
  available for a pre-check.
- npm private-registry 301 handling: the client never follows redirects, so a 301 falls
  into the non-success catch-all (`package-managers/npm/src/releases.rs:54-58`, whose
  comment says "4xx" but also catches 3xx) → terminal `Configuration` error, no retry —
  permanent failure against path-normalizing registries. Plus `NpmConfig::registry_url`
  validation gaps (no `validate_inner`, absent from `form_schema()`).
- 3xx classification arm in the web-api GitHub global provider
  (`global_providers/github.rs:1088`) — renamed/transferred repos surface as unclassified
  `RequestFailed`.
- Initial-URL IP-literal bypass of `SsrfSafeResolver` (hyper-util's literal short-circuit
  skips DNS for a configured `https://10.0.0.5/…` request URL — one config write, no
  redirect needed). A fallible request-creation wrapper was pulled into M2 during review
  and reverted (round 3): `uptrakit-github-client` exposes a bare
  `pub http_client: reqwest::Client` and cannot depend on plugin infrastructure-core, so
  the wrapper cannot cover all Strict clients without a crate-boundary break. Partially
  mitigated where plugins validate configured hosts at config time; revisit as its own
  design if pursued.
- `is_private_ip` IPv6 gaps (IPv4-mapped `::ffff:0:0/96`, 6to4 `2002::/16`, NAT64
  `64:ff9b::/96`) and bracketed-IPv6 `Url::host_str()` parsing — separate P1 bug, bead
  `uptrakit-amd3d`; explicitly **not** a prerequisite for M2 (owner decision 2026-08-20).
- Migrating `openapi-client` / `service-sdk` / `cli` / `agent-core` clients onto a shared
  builder (blocked by publishable-crate hygiene — the builder lives in
  infrastructure-core, whose optional `uptrakit-shared-db`/`uptrakit-tenant-db`/
  `uptrakit-crypto` deps are banned from the published crates' dep trees; they receive
  explicit-policy `#[expect]` sites with recorded redirect + resolver decisions in M4
  instead).

## Decision log (grilling outcomes)

| Decision                 | Choice                                                                                                                                                                                                                                                                     |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Scope split (2026-08-20) | This spec = HTTP hygiene only (caps, `RedirectMode`, pagination origin, clippy gate, docs); digest/attestation redesign → bead `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate` (gates `uptrakit-te4i9`)                                                    |
| Checksums-file path      | Retired, not repaired: GitHub API per-asset `digest` field (F1) obsoletes it; F2 proved any checksums-fed gate forgeable; deletion is spec'd in `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`. The dormant path keeps `Policy::none()` until then        |
| Attestation gate         | Extracted whole to `uptrakit-write-spec-2026-08-20-github-digest-attestation-gate`: binding at asset-selection site + pipeline backstop (closes F3), authenticated verifier, fail-closed distinct messages (absent vs unavailable), no retry (retry bead `uptrakit-qpwd1`) |
| https→http downgrade     | Blocked on redirect hops; initial-http chains unaffected                                                                                                                                                                                                                   |
| Auth in default headers  | github Bearer stays in `default_headers` on the redirect-following download client — cross-host strip verified at reqwest source; header-specific rule in ADR supersedes the absolute per-request rule                                                                     |
| Body caps                | In scope, first milestone (M1); github checksums site excluded (path slated for deletion, dormant meanwhile)                                                                                                                                                               |
| Pagination origin check  | In scope (M3)                                                                                                                                                                                                                                                              |
| Enforcement              | clippy `disallowed-methods` ban (3 probe-verified entries) + scoped `#[expect]` + canary (M4); Default-trait bypass probe-confirmed → mitigation bead `uptrakit-6bsg7`                                                                                                     |
| Initial-URL IP literal   | Stays deferred — wrapper crossed the `uptrakit-github-client` crate boundary (round 3)                                                                                                                                                                                     |
| IPv6 private-IP forms    | Separate bug bead `uptrakit-amd3d`, parallel work — not an M2 prerequisite (owner decision 2026-08-20)                                                                                                                                                                     |
