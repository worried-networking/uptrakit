# Global GitHub Provider For Global Plugins Design

Date: 2026-04-17

## Summary

Introduce a host-owned global GitHub provider client for global or singleton
plugins, starting with `dashboard-icons`.

The provider is backed by one global owner-managed GitHub credentials record
(`auth_token`, `api_base_url`) and exposed only through application-layer
injection. Plugins do not read shared settings directly. The provider client is
implemented with `octocrab` behind an Uptrakit-owned abstraction and enforces
shared outbound policy for authentication, retry, and rate-limit handling across
all global GitHub consumers.

In V1, regular tenant-scoped plugins such as `releases_github` do not consume
this provider. `dashboard-icons` may still operate unauthenticated when no
global GitHub credentials are configured.

## Goals

- Allow global or singleton plugins to use one shared global GitHub credential.
- Keep shared credential resolution outside plugin code.
- Centralize GitHub outbound policy, especially rate-limit and retry handling.
- Support unauthenticated fallback for `dashboard-icons`.
- Preserve a clean upgrade path to future global GitHub consumers.

## Non-Goals

- Tenant overrides.
- Multiple GitHub provider instances.
- Making tenant-scoped plugins consume the global provider.
- Generic cross-provider or cross-plugin provider selection.
- Reworking the `releases_github` plugin crate in V1.

## Current Context

Today:

- Global GitHub credentials are stored in the host layer as global settings.
- The pre-cutover branch had shared-provider fallback behavior for regular
  GitHub-backed config materialization paths, but this V1 design removes that
  fallback.
- `dashboard-icons` is a singleton enhancement plugin built from
  `CatalogConfig`, not a `plugin_config` instance.
- `dashboard-icons` fetches GitHub-backed data without a shared provider
  abstraction.
- GitHub request logic and rate-limit behavior are still largely consumer-local.

This creates two problems:

- global plugins have no clean shared-credential injection seam
- GitHub API consumers do not coordinate rate limits centrally

## Decision

V1 narrows the shared provider concept:

- there is one global owner-managed GitHub credentials record
- it is usable only by global or singleton plugins
- the host application builds one shared GitHub provider client from it
- global plugins declare that they consume the global GitHub provider
- the host injects that provider into eligible global plugins at construction
  time
- `dashboard-icons` is the first consumer
- `dashboard-icons` still works without credentials, using unauthenticated
  requests

Regular plugins, including tenant-scoped ones like `releases_github`, do not
consume the global provider in V1. As part of V1, the current branch-only
shared-provider fallback for regular plugin materialization is removed. Regular
plugins continue to use only their own plugin-local config paths.

## Architecture

### 1. Global Provider Storage

The persisted data remains simple and GitHub-specific in V1:

- `auth_token`
- `api_base_url`

Record invariants:

- no record means unauthenticated public GitHub access is allowed for eligible
  global plugins
- a record with non-empty `auth_token` and empty or absent `api_base_url` means
  authenticated access against `https://api.github.com`
- a record with non-empty `auth_token` and non-empty `api_base_url` means
  authenticated access against the validated custom base URL
- a record with empty or absent `auth_token` and non-empty `api_base_url` is
  invalid and must be treated as misconfiguration, not as unauthenticated
  fallback
- a record with both fields empty is treated as no record

Responsibilities:

- load/store the global GitHub credentials record
- encrypt the token at rest
- expose typed load/store helpers
- enforce the existing `api_base_url` validation and outbound-host policy before
  a custom base URL is accepted

Responsibilities explicitly excluded:

- deciding which plugins may use the provider
- building HTTP clients
- handling rate limits or retry policy

Storage remains dumb. Consumer applicability is moved out of storage code and
into host-layer runtime construction.

### 2. Host-Owned GitHub Provider Runtime

Add an application-layer GitHub provider runtime that:

- loads the global GitHub credentials record
- builds one shared GitHub provider client
- caches it for process-wide reuse
- exposes a GitHub-specific trait or handle owned by Uptrakit
- tracks provider-client generations so config changes rebuild the client

This runtime owns:

- auth header injection
- base URL selection
- `octocrab` construction
- public-HTTPS validation for any custom `api_base_url`
- retry policy
- rate-limit coordination
- metrics and tracing

`octocrab` is an implementation detail behind the Uptrakit abstraction, not the
plugin-facing contract.

For user-controlled custom `api_base_url` values, the runtime must validate the
URL as a public HTTPS endpoint before the provider client is constructed.

#### Cache Invalidation And Refresh

The runtime cache must be refreshable at runtime.

V1 contract:

- the settings update path that persists the global GitHub record must also
  invalidate the cached GitHub provider client
- the next provider acquisition rebuilds the client lazily from the latest
  stored settings
- in-flight requests are allowed to finish on the old client generation
- token change or `api_base_url` change always creates a new client generation
- the provider runtime stores rate-limit state and concurrency gates per
  generation key, so rotation creates a new state object while the old one
  drains naturally with its in-flight requests

For multi-instance controller deployments, V1 uses process-local immediate
invalidation plus bounded lazy revalidation on other instances:

- the writing instance invalidates immediately after a successful settings write
- other instances must re-check the derived generation on acquisition with a
  maximum staleness window of 30 seconds
- the derived generation is a hash of the canonicalized stored record contents,
  not a separate persisted field in V1

V1 does not require hot-cancelling in-flight requests during rotation.

### 3. Global Plugin Consumer Declaration

Global or singleton plugins need a declaration that they consume the global
GitHub provider.

This declaration must be:

- plugin metadata
- provider-aware
- limited to global-plugin construction paths in V1
- independent from storage code

The generic plugin core should only carry provider-agnostic metadata such as an
opaque provider identifier or consumer declaration. It must not grow
GitHub-specific types.

The identifier shape must be string-based, not an enum, to avoid V2 churn.
V1 uses an opaque provider ID with the literal `"github"` and keeps the generic
core independent from provider-specific Rust types.

### 4. Injection Into Global Plugin Construction

The provider handle is injected during singleton/global plugin construction.

The concrete singleton construction seam in V1 is `CatalogConfig`.

`CatalogConfig` gains a provider-agnostic lookup surface owned by the host
layer. The generic plugin core only knows that singleton constructors can look
up global provider handles by provider ID. A GitHub-specific helper outside the
generic core resolves the typed GitHub handle from that lookup surface.

V1 lookup contract:

- `CatalogConfig` carries `global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>`
- `GlobalProviderLookup` is a generic-core trait returning opaque handles by
  string provider ID
- the opaque handle type is `Arc<dyn std::any::Any + Send + Sync>`
- GitHub-specific helper code outside generic core resolves and downcasts the
  opaque handle into `Arc<dyn GitHubProviderClient>`

Although `CatalogConfig` is the concrete V1 seam, the lookup interface is
designed to be reusable by any future singleton construction path rather than
being semantically tied to `dashboard-icons`.

For `dashboard-icons`:

- the host sees that the plugin consumes the global GitHub provider
- it resolves the shared GitHub provider handle
- it injects the handle through singleton construction context

The plugin never:

- reads shared settings
- reads the DB
- builds its own shared rate-limit policy

This keeps `dashboard-icons` testable: unit tests can inject a fake GitHub
provider handle.

## Provider Client Design

The host-owned abstraction should be GitHub-specific, but only outside generic
plugin infrastructure.

Expected behavior:

- provide a small GitHub-oriented interface suited to current global consumers
- expose typed operations or request helpers for GitHub REST access
- hide raw credential resolution from plugins
- hide `octocrab` from plugins
- require consumer identity on each call for metrics and tracing

The injected type is always a GitHub provider handle, never `Option`.

V1 behavior:

- when no global GitHub credentials record exists, inject an unauthenticated
  GitHub provider handle configured for public GitHub access
- when a credentials record exists, inject an authenticated provider handle
- plugins do not branch on whether credentials exist

This keeps `dashboard-icons` unaware of authentication mode while still
supporting unauthenticated fallback.

The V1 trait must be concrete enough to anchor implementation. It should expose
at least:

- a consumer identifier parameter on every call
- one tree-fetch operation needed by `dashboard-icons`, such as
  `fetch_repository_tree(...)`
- a typed error surface that distinguishes throttling, authentication failure,
  upstream unavailability, and non-retryable request failures

Concrete V1 anchor:

```rust
async fn fetch_repository_tree(
    &self,
    consumer: GlobalProviderConsumerId,
    owner: &str,
    repo: &str,
    git_ref: &str,
    recursive: bool,
) -> Result<GitHubRepositoryTree, GitHubProviderError>;
```

Where:

- `GlobalProviderConsumerId` is an Uptrakit-owned newtype over `&'static str`
  with exported constants, starting with `DASHBOARD_ICONS`
- `GitHubRepositoryTree` is an Uptrakit-owned response model, not an `octocrab`
  type
- `GitHubProviderError` is the classified error surface from this spec

Future global consumers may justify additional operations later, but V1 should
start with the minimal operation set required by `dashboard-icons`.

The provider client is implemented with `octocrab` because it is an actively
maintained GitHub API crate and supports injecting a custom service or layer
stack. That allows Uptrakit to keep ownership of middleware and policy while
still using a supported GitHub client internally.

The `octocrab` dependency belongs in a controller-side application/runtime crate
or module, not in plugin infrastructure core and not in plugin crates.

## Rate Limiting And Retry Policy

All global GitHub consumers share one provider client policy surface.

The provider runtime must centralize:

- primary rate-limit handling
- `Retry-After` handling
- secondary-rate-limit backoff
- bounded retries with exponential backoff
- cooldown coordination across consumers

The provider client is the only retry owner. Plugins must not add their own
GitHub-specific retry loops on top.

V1 baseline policy:

- initial request plus up to 2 retries for retryable throttling or transient
  upstream failures
- exponential backoff
- base delay `500ms`
- max computed backoff `30s`
- explicit `Retry-After` or primary reset windows override the computed delay
- a process-wide concurrency gate starts at `8` in-flight requests per
  provider runtime

These values are conservative anchors for V1 and can be tuned later with
metrics.

When the process-wide concurrency gate is full, new requests wait for a permit
instead of returning an immediate throttling error. Queue wait time does not
consume retry budget.

V1 bounds queue wait at 30 seconds. If no permit is acquired within that
window, the request fails as `Throttled`.

The shared cooldown and concurrency state is process-wide for the V1 global
GitHub provider runtime. Credential or base-URL changes rebuild the client
generation and reset the cached runtime state instead of preserving separate
per-generation buckets.

The shared policy should:

- avoid a naive global mutex around all GitHub traffic
- gate only when cooldown or budget state requires it
- surface transient provider-throttled errors consistently

### Time And Sleep Injection

The provider runtime keeps injectable sleep primitives for retry and cooldown
tests. Generation recheck timing continues to use `tokio::time::Instant` in V1.

### Error Classification

The provider runtime must classify at least these outcomes:

- `Throttled` for primary or secondary rate limiting
- `AuthFailed` for invalid configured credentials or missing scopes
- `UpstreamUnavailable` for retryable transient upstream failures
- `RequestFailed` for durable non-retryable failures such as `404` or `422`
- `Misconfigured` for invalid local runtime construction inputs

Invalid configured credentials must not auto-fallback to unauthenticated mode.
Only absence of a configured global credentials record permits unauthenticated
fallback.

## `dashboard-icons` Behavior

`dashboard-icons` becomes the first global consumer of the GitHub provider.

Behavior rules:

- the provider runtime injects an authenticated handle when a valid global
  credentials record exists
- the provider runtime injects an unauthenticated public-GitHub handle when no
  global credentials record exists
- `dashboard-icons` always uses the injected handle and never branches on
  credential presence itself
- if rate-limited or GitHub is temporarily unavailable, preserve existing cache
  behavior and fail gracefully
- if configured credentials are invalid, preserve cache, surface durable auth
  failures in logs and metrics, and do not auto-fallback to unauthenticated
  mode

On `AuthFailed` or `RequestFailed`, `dashboard-icons` keeps serving its existing
cache and skips only the failing refresh attempt. The next scheduled refresh may
try again through the shared provider runtime, which applies the same
classification and backoff rules.

The plugin should not need to know whether it is running authenticated or
unauthenticated. That distinction is owned by the provider runtime.

## Observability

Add provider-level metrics and tracing for:

- request count
- retry count
- throttled count
- cooldown duration
- authentication failures
- consumer identity, such as `dashboard-icons`

This is necessary because one shared client now mediates multiple consumers.

The provider API must therefore require a consumer identifier on every request
path. V1 can model this as a small consumer ID newtype or constant set, starting
with `dashboard-icons`.

Example metric schema:

- `uptrakit_global_provider_requests_total{provider,consumer,status}`
- `uptrakit_global_provider_retries_total{provider,consumer,reason}`
- `uptrakit_global_provider_cooldown_seconds{provider,key_kind}`
- `uptrakit_global_provider_auth_failures_total{provider,consumer}`

In V1, `key_kind` values are:

- `authenticated`
- `anonymous`

## Testing

### Unit Tests

- provider storage load/store helpers
- provider runtime creation with and without credentials
- deterministic retry and cooldown behavior with injected clock/transport
- `dashboard-icons` using a fake provider handle
- client invalidation after settings update

### Integration Tests

- global provider credentials configured -> `dashboard-icons` uses provider path
- no global provider credentials -> `dashboard-icons` still works
- two global consumers share one cooldown window
- invalid credentials produce durable auth failures without aggressive retry
- tenant-scoped GitHub plugins ignore the global provider path in V1

The "two global consumers" test uses one real consumer and one test-only fake
global consumer wired through the same provider runtime.

### Regression Requirements

- no plugin code path may read shared settings directly
- no storage helper may decide plugin applicability
- no tenant-scoped plugin may start consuming the global provider in V1

## Migration

### V1

- keep the global GitHub settings persistence shape
- narrow its semantics to "global GitHub provider for global plugins"
- move consumer applicability out of storage helpers
- add provider runtime and injection path
- migrate `dashboard-icons` to use the injected provider
- remove the current branch-only shared-provider fallback from regular plugin
  materialization
- keep `releases_github` on plugin-local configuration only

This does not change behavior inside the `releases_github` plugin crate itself.
The V1 change is limited to host/query-side removal of the branch-only global
fallback path.

#### Operator Impact

Any installation or test environment currently relying on the branch-only global
GitHub fallback for tenant-scoped plugins must move those credentials into the
plugin-local config before rollout of this V1 design.

The global GitHub settings record remains available after rollout, but it is
used only by global plugins.

V1 diagnostics are limited to invalid global GitHub record states, including
custom `api_base_url` with missing `auth_token`. Those diagnostics surface
through the admin-event channel and the system-alerts route after settings
reload.

#### Rollback And Transition

If rollout reveals missing migration coverage for tenant-scoped consumers, the
safe fallback is to restore plugin-local credentials and disable the global
provider integration for the affected global plugin until the migration issue is
resolved.

### V2-Compatible Seams

V1 should leave room for:

- multiple GitHub provider instances
- tenant-scoped selection
- provider instance resolution keyed by more than one credentials record

The critical V2 preparation is that plugin-facing contracts do not assume a
single hardcoded global GitHub settings source.

## Rejected Alternatives

### Make `dashboard-icons` a `plugin_config` Plugin

Rejected because it changes the plugin's semantics from one global enhancement
to one or more assignable config instances without a V1 need.

### Let Plugins Read Shared Settings Directly

Rejected because it violates the boundary, harms testability, and makes future
provider evolution expensive.

### Make Regular Plugins Consume The Global Provider In V1

Rejected to keep scope small and avoid locking tenant-scoped plugins into the
wrong provider-selection model before V2.

### Use Raw `reqwest` Instead Of A GitHub Crate

Possible, but not preferred. `octocrab` gives a supported GitHub API surface
while still allowing Uptrakit to own the service stack and policy through an
abstraction layer.

## Concrete V1 Outcome

After V1:

- one global GitHub credentials record exists
- one host-owned GitHub provider client exists per process
- `dashboard-icons` can use it
- future global plugins can opt in the same way
- GitHub rate limiting for global consumers is coordinated centrally
- tenant-scoped plugins remain unchanged
