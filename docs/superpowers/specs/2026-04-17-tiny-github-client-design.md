# Tiny GitHub Client Design

Date: 2026-04-17

## Summary

Replace `octocrab` in the global GitHub provider runtime with a small
Uptrakit-owned GitHub client crate built on `reqwest`.

The new crate provides:

- first-class authenticated and anonymous GitHub requests
- a small endpoint builder
- typed response decoding
- GitHub-aware error classification
- retry and cooldown recommendations for a single request attempt

The Web API runtime keeps ownership of process-wide caching, shared concurrency
gates, and retry execution. The new crate computes retry guidance, but it does
not automatically retry or sleep.

V1 only needs repository-tree fetches for `dashboard-icons`, but the crate is
shaped so `releases_github` can become a future consumer without redesigning
the boundary.

## Goals

- Remove the `octocrab` dependency from the global GitHub provider path.
- Keep the plugin-facing boundary unchanged.
- Support both anonymous and bearer-token GitHub requests as first-class modes.
- Centralize GitHub HTTP protocol handling in a small dedicated crate.
- Keep retry and cooldown policy computation close to HTTP classification.
- Preserve the current host-owned shared-runtime model in `uptrakit-web-api`.
- Leave a clean future path for `releases_github`.

## Non-Goals

- Migrating `releases_github` in this implementation slice.
- Building a general-purpose GitHub SDK.
- Supporting write APIs or GraphQL.
- Adding automatic retries inside the new GitHub client crate.
- Changing the global-plugin-only scope of the shared provider design.

## Current Context

Today, the global GitHub provider runtime in
`crates/ui/web-api/src/global_providers/github.rs` uses `octocrab` for a very
small amount of GitHub API usage.

This is heavier than necessary because:

- the public Uptrakit-facing trait is already tiny and typed
- the only implemented operation is repository-tree fetch
- most of the complexity Uptrakit cares about is not GitHub API surface area,
  but runtime policy:
  - credentials loading
  - cache invalidation
  - shared rate-limit gates
  - retry orchestration
  - plugin injection

So `octocrab` mainly serves as a transport/protocol dependency in a place where
Uptrakit already owns the real abstraction.

## Decision

Introduce a small new crate, `uptrakit-github-client`, and use it to replace
`octocrab` inside the global GitHub provider runtime.

The new split becomes:

- `uptrakit-github-client`
  - endpoint builder
  - request execution
  - JSON decoding
  - GitHub response classification
  - retry/cooldown recommendation computation
- `uptrakit-global-github-provider`
  - stable provider trait and shared response models
- `uptrakit-web-api`
  - credential loading
  - provider runtime cache
  - shared concurrency and cooldown coordination
  - retry execution
  - plugin injection

The crate computes what should happen after a failed attempt, but the runtime
decides whether and when to retry.

## Architecture

### 1. New Crate: `uptrakit-github-client`

This crate is a small, focused GitHub REST client. It should live in a shared
crate location and stay independent from plugin runtime code.

It owns:

- GitHub base URL normalization inputs
- auth mode representation
- endpoint construction
- one-attempt HTTP execution via `reqwest`
- JSON deserialization into Uptrakit-owned models
- GitHub error parsing and classification
- retry/cooldown recommendation calculation

It does not own:

- process-wide runtime caches
- cross-request cooldown state
- concurrency semaphores
- retry loops
- settings storage
- plugin lookup or injection

### 2. Public Config Model

The crate should expose a small constructor/config surface:

- `GitHubClientConfig`
  - `base_url`
  - `auth`
  - `user_agent`
- `GitHubAuth`
  - `Anonymous`
  - `BearerToken(String)`

Both anonymous and authenticated requests are first-class inputs to the same
client path. Anonymous access is not a special-case runtime branch.

### 3. Endpoint Builder

The crate should expose an endpoint builder rather than hardcoding URL
construction only inside method bodies.

V1 requires one endpoint:

- `RepositoryTree { owner, repo, git_ref, recursive }`

The builder is designed to add future read-only endpoints without redesign:

- `Releases`
- `LatestRelease`
- `ReleaseByTag`
- `Tags`

The endpoint builder is part of the crate contract so future consumers such as
`releases_github` can share a single request-construction model.

### 4. Typed Client Surface

The public typed surface remains minimal in V1.

The crate should expose a GitHub client with a typed method for the current
need:

- `fetch_repository_tree(...)`

This is intentionally narrow. Future methods can be added incrementally as
new consumers migrate.

The public consumer surface should stay typed and GitHub-specific, not devolve
into a generic `send_json(method, path, query)` API.

### 5. Single-Attempt Outcome Model

The new crate should execute one request attempt at a time and return both the
result and guidance for the caller.

Recommended shape:

- `AttemptOutcome<T>`
  - `Success(T, ResponseMetadata)`
  - `Failure(GitHubClientError, RetryDecision, ResponseMetadata)`

Where:

- `RetryDecision`
  - `DoNotRetry`
  - `RetryAfter(Duration)`
  - `Backoff(Duration)`
- `ResponseMetadata`
  - status code, if available
  - parsed rate-limit metadata, if available
  - auth mode kind, if useful for metrics/debugging

This keeps retry/cooldown intelligence in the crate without letting the crate
silently perform extra work.

### 6. Error Model

The crate should normalize GitHub/HTTP failures into a small set of typed
errors:

- `AuthFailed`
- `Forbidden`
- `NotFound`
- `RateLimited`
- `UpstreamUnavailable`
- `InvalidResponse`
- `Misconfigured`

Recommended classification:

- `401` -> `AuthFailed` + `DoNotRetry`
- `403` with rate-limit evidence -> `RateLimited` + retry recommendation
- `403` without rate-limit evidence -> `Forbidden` + `DoNotRetry`
- `404` -> `NotFound` + `DoNotRetry`
- `429` -> `RateLimited` + `RetryAfter(...)`
- `5xx` -> `UpstreamUnavailable` + `Backoff(...)`
- invalid JSON / missing required fields -> `InvalidResponse` + `DoNotRetry`
- impossible URL or request construction -> `Misconfigured` + `DoNotRetry`

The crate should parse `Retry-After` and GitHub rate-limit headers when present
and produce a deterministic retry recommendation from them.

### 7. Runtime Ownership In `uptrakit-web-api`

The Web API runtime continues to own the shared provider behavior:

- loading provider settings from storage
- selecting anonymous vs bearer auth
- building the GitHub client from current settings
- caching the active client generation
- invalidating on settings changes
- shared concurrency limits
- shared cooldown windows
- deciding whether to wait and retry after a failed attempt
- injecting the provider handle into global plugins

The runtime should consume the new crate’s outcomes instead of `octocrab`
errors.

The important rule is:

- the crate computes retry guidance
- the runtime decides whether to apply it

### 8. Boundary With `uptrakit-global-github-provider`

The existing provider trait crate remains the stable host/plugin-facing
boundary.

It should continue to own:

- `GitHubProviderClient`
- shared response models such as `GitHubRepositoryTree`
- consumer IDs like `DASHBOARD_ICONS`

`uptrakit-github-client` is an implementation dependency for the runtime, not a
new plugin-facing abstraction.

## Data Flow

For `dashboard-icons`:

1. The Web API runtime loads the global GitHub provider record.
2. It creates `GitHubClientConfig` with:
   - validated base URL
   - `GitHubAuth::Anonymous` or `GitHubAuth::BearerToken(...)`
   - Uptrakit user agent
3. The runtime constructs the tiny GitHub client.
4. `dashboard-icons` calls the injected provider handle.
5. The runtime invokes one client attempt against the `RepositoryTree`
   endpoint.
6. The client returns:
   - decoded tree on success, or
   - a typed error plus retry guidance on failure
7. The runtime decides whether to:
   - return immediately
   - back off and retry
   - trigger shared cooldown behavior

Plugins remain unaware of:

- settings storage
- auth token shape
- HTTP headers
- GitHub rate-limit headers

## Future Compatibility With `releases_github`

V1 does not migrate `releases_github`, but the new crate must leave a clean
path for that future work.

That means:

- endpoint builder is not tree-only in design, even if only one variant is
  implemented now
- error model is generic enough for releases endpoints
- client config supports both anonymous and authenticated access uniformly
- response classification does not assume only one endpoint family

When `releases_github` migrates later, the expected change should be additive:

- add release/tag endpoint variants
- add typed response models and methods
- move that plugin’s runtime path over to the same GitHub client crate

## Testing Strategy

### Unit Tests In `uptrakit-github-client`

Cover:

- endpoint URL construction
- auth header behavior for anonymous vs bearer
- response decoding for repository tree
- error classification from status/body/headers
- retry decision calculation from:
  - `Retry-After`
  - primary rate-limit headers
  - secondary-rate-limit style `403`
  - `5xx`

### Runtime Tests In `uptrakit-web-api`

Keep or adapt existing runtime tests to verify:

- client invalidation and lazy rebuild
- shared cooldown behavior
- shared queue wait timeout behavior
- startup/settings diagnostics still work
- `dashboard-icons` still uses the injected provider path

### Migration Safety

Add coverage that proves removing `octocrab` does not change the current
visible behavior of:

- settings updates
- startup diagnostics
- `dashboard-icons` cold refresh path
- throttling and auth-failure classification seen by the runtime

## Migration Plan

1. Add `uptrakit-github-client`.
2. Implement only the repository-tree endpoint and typed method.
3. Replace the `octocrab`-backed factory in the global provider runtime with
   the tiny client.
4. Keep the runtime traits and plugin-facing provider trait unchanged.
5. Remove `octocrab` from the workspace and Web API dependencies.
6. Update docs/spec/plan references that currently describe an `octocrab`-based
   runtime.

This is an internal implementation swap, not a behavior redesign.

## Trade-Offs

### Pros

- smaller dependency surface
- simpler control over request/response behavior
- easier to reason about exactly what GitHub API usage Uptrakit depends on
- cleaner future migration path for `releases_github`
- less translation code from third-party error models into Uptrakit models

### Cons

- Uptrakit owns more low-level GitHub REST details
- future GitHub endpoint expansion requires explicit incremental work
- some protocol edge cases previously hidden by `octocrab` become our
  responsibility

The trade is worth it because the current API usage is small and the real
complexity already lives in Uptrakit’s runtime policy, not in broad GitHub API
coverage.

## Rejected Alternatives

### Keep `octocrab`

Rejected because current GitHub API usage is too small to justify a large
dependency and adapter layer.

### Replace `octocrab` With Ad Hoc `reqwest` Calls Inside `web-api`

Rejected because it keeps HTTP protocol details tangled with runtime policy and
makes future reuse by `releases_github` harder.

### Expose A Generic REST Helper Instead Of Typed Methods

Rejected because it would leak GitHub protocol details back into consumers and
weaken the current clean boundary.
